// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Phase 3 — partitioning by domain (`ns`): records route to the stream of
//! their type's domain, tombstones follow their entity, cross-domain
//! relations leave an `.xref` entry on the target side, a batch costs one
//! sync per touched domain, selective hydration loads a subset, and
//! compaction rewrites the whole store verifiably.

use ontology_graph::{
    Concept, ConceptId, ConceptPatch, ConceptType, Ontology, OntologyGraph, Relation, RelationType,
    Rule, RuleType,
};
use ontology_storage::manifest::META_NS_ID;
use ontology_storage::segment::{idx_path, IdxEntry, Kind};
use ontology_storage::{
    LogRecord, RollPolicy, SegmentStore, SegmentStoreConfig, Store, StoreError,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn tempdir(tag: &str) -> PathBuf {
    // pid + counter + clock: the clock alone repeats on Windows (coarse
    // ticks) when two tests create their directory in the same instant.
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "ontology-domains-{tag}-{}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn ct(name: &str, ns: Option<&str>, parent: Option<&str>) -> ConceptType {
    ConceptType {
        name: name.into(),
        ns: ns.map(str::to_string),
        parent: parent.map(str::to_string),
        ..Default::default()
    }
}

/// Three domains: parties (Company, Person), contrats (Contract),
/// facturation (Invoice) — the finance example's layout.
fn ontology() -> Ontology {
    let mut o = Ontology::new();
    o.add_concept_type(ct("Company", Some("parties"), None));
    o.add_concept_type(ct("Subsidiary", None, Some("Company")));
    o.add_concept_type(ct("Person", Some("parties"), None));
    o.add_concept_type(ct("Contract", Some("contrats"), None));
    o.add_concept_type(ct("Invoice", Some("facturation"), None));
    o.add_relation_type(RelationType {
        name: "between".into(),
        domain: "Contract".into(),
        range: "Company".into(),
        ..Default::default()
    })
    .unwrap();
    o.add_relation_type(RelationType {
        name: "employed_by".into(),
        domain: "Person".into(),
        range: "Company".into(),
        ..Default::default()
    })
    .unwrap();
    o.add_relation_type(RelationType {
        name: "covers_contract".into(),
        domain: "Invoice".into(),
        range: "Contract".into(),
        ..Default::default()
    })
    .unwrap();
    o.add_relation_type(RelationType {
        name: "partner_of".into(),
        domain: "Company".into(),
        range: "Company".into(),
        symmetric: true,
        ..Default::default()
    })
    .unwrap();
    o.add_rule_type(RuleType {
        name: "must_review".into(),
        when: String::new(),
        then: String::new(),
        applies_to: vec!["Contract".into()],
        strict: false,
        description: String::new(),
    })
    .unwrap();
    o
}

fn small_roll() -> SegmentStoreConfig {
    SegmentStoreConfig {
        roll: RollPolicy {
            max_bytes: u64::MAX,
            max_records: 5,
        },
        ..Default::default()
    }
}

struct Fixture {
    graph: Arc<OntologyGraph>,
}

impl Fixture {
    async fn new(store: &dyn Store) -> Self {
        let graph = OntologyGraph::with_arc(Ontology::new());
        store
            .append(&LogRecord::ontology(ontology()))
            .await
            .unwrap();
        graph
            .extend_ontology(|o| {
                *o = ontology();
                Ok(())
            })
            .unwrap();
        Self { graph }
    }
    async fn concept(&self, store: &dyn Store, ty: &str, name: &str) -> ConceptId {
        let mut c = Concept::new(ConceptId(0), ty, name);
        self.graph.prepare_concept(&mut c).unwrap();
        store.append(&LogRecord::concept(c.clone())).await.unwrap();
        self.graph.apply_prepared_concept(c).unwrap()
    }
    async fn relation(&self, store: &dyn Store, rt: &str, a: ConceptId, b: ConceptId) {
        let mut r = Relation::new(Default::default(), rt, a, b);
        self.graph.prepare_relation(&mut r).unwrap();
        store.append(&LogRecord::relation(r.clone())).await.unwrap();
        self.graph.apply_prepared_relation(r).unwrap();
    }
    async fn delete_concept(&self, store: &dyn Store, id: ConceptId) {
        let ty = self.graph.get_concept(id).unwrap().concept_type;
        let cascade = self.graph.incident_relation_ids(id).unwrap();
        let mut recs = vec![LogRecord::delete_concept(id, ty)];
        for rid in cascade {
            let rt = self.graph.get_relation(rid).unwrap().relation_type;
            recs.push(LogRecord::delete_relation(rid, rt));
        }
        store.append_batch(&recs).await.unwrap();
        self.graph.remove_concept(id).unwrap();
    }
}

fn entries(dir: &Path, partition: u32) -> Vec<IdxEntry> {
    let bytes = std::fs::read(idx_path(dir, partition)).unwrap();
    (0..IdxEntry::count_in(bytes.len() as u64))
        .map(|i| IdxEntry::decode(&bytes, IdxEntry::file_offset(i)).unwrap())
        .collect()
}

fn partitions(dir: &Path) -> Vec<u32> {
    let mut ids: Vec<u32> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let p = e.path();
            (p.extension()? == "data").then(|| p.file_stem()?.to_str()?.parse::<u32>().ok())?
        })
        .collect();
    ids.sort_unstable();
    ids
}

#[tokio::test]
async fn records_route_to_their_types_domain_and_tombstones_follow_the_entity() {
    let root = tempdir("route");
    let store = SegmentStore::open(&root).await.unwrap();
    let fx = Fixture::new(&store).await;
    // Ontology alone: only meta and the default domain exist so far.
    assert!(root.join("graph/default").exists());
    assert!(!root.join("graph/parties").exists());

    let acme = fx.concept(&store, "Company", "Acme").await;
    let sub = fx.concept(&store, "Subsidiary", "Acme Labs").await; // inherits parties
    let alice = fx.concept(&store, "Person", "Alice").await;
    let c1 = fx.concept(&store, "Contract", "C-1").await;
    let inv = fx.concept(&store, "Invoice", "I-1").await;
    fx.relation(&store, "between", c1, acme).await; // contrats → parties
    fx.relation(&store, "employed_by", alice, acme).await; // parties → parties
    fx.relation(&store, "covers_contract", inv, c1).await; // facturation → contrats

    // Streams were created lazily, ids frozen in the manifest.
    let m = store.manifest();
    let parties = m.ns_id("parties").unwrap();
    let contrats = m.ns_id("contrats").unwrap();
    let facturation = m.ns_id("facturation").unwrap();
    assert_eq!(m.stream(parties).unwrap().dir, "graph/parties");
    let counts = store.record_counts_by_domain();
    assert_eq!(counts["meta"], 1);
    assert_eq!(counts["parties"], 4, "{counts:?}"); // 3 concepts + employed_by
    assert_eq!(counts["contrats"], 2); // C-1 + between
    assert_eq!(counts["facturation"], 2); // I-1 + covers_contract
    assert_eq!(counts["default"], 0);

    // Index entries: ns of the stream, target domain on cross-domain edges.
    let cdir = root.join("graph/contrats");
    let cpart = partitions(&cdir)[0];
    let ce = entries(&cdir, cpart);
    assert_eq!(ce.len(), 2);
    assert_eq!(ce[0].kind, Kind::Concept);
    assert_eq!(ce[0].ns_id, contrats);
    assert_eq!(ce[1].kind, Kind::Relation);
    assert_eq!(ce[1].target_ns_id, parties, "between targets parties");
    let pdir = root.join("graph/parties");
    let pe = entries(&pdir, partitions(&pdir)[0]);
    let employed = pe.iter().find(|e| e.kind == Kind::Relation).unwrap();
    assert_eq!(
        employed.target_ns_id, 0,
        "same-domain edge carries no target ns"
    );
    let _ = (sub, facturation);

    // A deletion lands in the entity's domain, with its cascade.
    fx.delete_concept(&store, c1).await;
    let ce = entries(&cdir, cpart);
    assert_eq!(ce.len(), 4, "{ce:?}");
    assert_eq!(ce[2].kind, Kind::DeleteConcept);
    assert_eq!(ce[2].entity_id, c1.0);
    assert_eq!(
        ce[3].kind,
        Kind::DeleteRelation,
        "between's tombstone in contrats"
    );
    // covers_contract lives in facturation (its source domain): its tombstone too.
    let fdir = root.join("graph/facturation");
    let fe = entries(&fdir, partitions(&fdir)[0]);
    assert_eq!(fe.last().unwrap().kind, Kind::DeleteRelation);
    assert_eq!(fe.len(), 3);

    // Everything replays to the live state.
    let fresh = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&fresh).await.unwrap();
    assert_eq!(fresh.concept_count(), fx.graph.concept_count());
    assert_eq!(fresh.relation_count(), fx.graph.relation_count());
    assert_eq!(fresh.relation_count(), 1);
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn an_unrouted_tombstone_is_refused_before_anything_is_written() {
    let root = tempdir("unrouted");
    let store = SegmentStore::open(&root).await.unwrap();
    let fx = Fixture::new(&store).await;
    let acme = fx.concept(&store, "Company", "Acme").await;
    let before = store.record_count();
    let err = store
        .append(&LogRecord::delete_concept_unrouted(acme))
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::Format(_)), "{err}");
    assert!(err.to_string().contains("routing hint"), "{err}");
    assert_eq!(store.record_count(), before);
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn a_batch_costs_one_sync_per_touched_domain() {
    let root = tempdir("syncs");
    let store = SegmentStore::open(&root).await.unwrap();
    let fx = Fixture::new(&store).await;
    let base = store.sync_count();
    // 1 000 concepts over two domains, in 10 batches of 100.
    let mut records = Vec::new();
    for i in 0..1000u64 {
        let (ty, name) = if i % 2 == 0 {
            ("Company", format!("co{i}"))
        } else {
            ("Contract", format!("ct{i}"))
        };
        let mut c = Concept::new(ConceptId(0), ty, name);
        fx.graph.prepare_concept(&mut c).unwrap();
        records.push(c);
    }
    for chunk in records.chunks(100) {
        let recs: Vec<LogRecord> = chunk.iter().cloned().map(LogRecord::concept).collect();
        store.append_batch(&recs).await.unwrap();
        for c in chunk {
            fx.graph.apply_prepared_concept(c.clone()).unwrap();
        }
    }
    assert_eq!(store.sync_count() - base, 20, "2 domains × 10 batches");
    let counts = store.record_counts_by_domain();
    assert_eq!(counts["parties"], 500);
    assert_eq!(counts["contrats"], 500);
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn cross_domain_edges_are_materialized_in_the_targets_xref_at_open() {
    let root = tempdir("xref");
    let (acme, c1, c2);
    {
        let store = SegmentStore::open_with(&root, small_roll()).await.unwrap();
        let fx = Fixture::new(&store).await;
        acme = fx.concept(&store, "Company", "Acme").await;
        let globex = fx.concept(&store, "Company", "Globex").await;
        c1 = fx.concept(&store, "Contract", "C-1").await;
        c2 = fx.concept(&store, "Contract", "C-2").await;
        fx.relation(&store, "between", c1, acme).await;
        fx.relation(&store, "between", c2, acme).await;
        fx.relation(&store, "between", c2, globex).await;
        // Enough parties records to roll (5): 2 concepts so far → add more.
        for i in 0..6 {
            fx.concept(&store, "Person", &format!("p{i}")).await;
        }
        let inv = fx.concept(&store, "Invoice", "I-1").await;
        fx.relation(&store, "covers_contract", inv, c1).await;
    }
    // Reopen: xrefs are rebuilt from the other domains' indexes.
    let store = SegmentStore::open_with(&root, small_roll()).await.unwrap();
    assert_eq!(
        store.open_report().xref_entries,
        4,
        "3 between + 1 covers_contract"
    );
    let parties = store.xrefs("parties").unwrap();
    let all: Vec<_> = parties
        .iter()
        .flat_map(|(_, e)| e.iter().copied())
        .collect();
    assert_eq!(all.len(), 3);
    assert!(all
        .iter()
        .all(|e| e.target_id == acme.0 || e.source_id == c2.0));
    assert!(all
        .iter()
        .any(|e| e.target_id == acme.0 && e.source_id == c1.0));
    // Entries are filed by seq into the partition whose range holds them.
    assert!(parties.len() >= 2, "parties rolled: {}", parties.len());
    let contrats = store.xrefs("contrats").unwrap();
    let all_c: Vec<_> = contrats
        .iter()
        .flat_map(|(_, e)| e.iter().copied())
        .collect();
    assert_eq!(all_c.len(), 1);
    assert_eq!(all_c[0].target_id, c1.0);
    assert!(store
        .xrefs("facturation")
        .unwrap()
        .iter()
        .all(|(_, e)| e.is_empty()));
    assert!(store.xrefs("nope").is_err());
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn selective_hydration_loads_a_subset_and_skips_dangling_edges() {
    let root = tempdir("selective");
    {
        let store = SegmentStore::open(&root).await.unwrap();
        let fx = Fixture::new(&store).await;
        let acme = fx.concept(&store, "Company", "Acme").await;
        let alice = fx.concept(&store, "Person", "Alice").await;
        let c1 = fx.concept(&store, "Contract", "C-1").await;
        fx.relation(&store, "between", c1, acme).await;
        fx.relation(&store, "employed_by", alice, acme).await;
        let mut rule = Rule::new(Default::default(), "must_review", "r1");
        rule.applies_to = vec![c1];
        fx.graph.prepare_rule(&mut rule).unwrap();
        store.append(&LogRecord::rule(rule.clone())).await.unwrap();
        fx.graph.apply_prepared_rule(rule).unwrap();
    }
    let store = SegmentStore::open(&root).await.unwrap();

    // Only parties: 2 concepts, the same-domain edge, no contract, the
    // cross-domain edge skipped. The schema is always loaded.
    let g = OntologyGraph::with_arc(Ontology::new());
    let r = store
        .load_domains_report(&g, &["parties".to_string()])
        .await
        .unwrap();
    assert_eq!(g.concept_count(), 2);
    assert_eq!(g.relation_count(), 1);
    assert_eq!(g.ontology().concept_types.len(), 5);
    assert!(g.find_by_name("Contract", "C-1").is_none());
    // `between` lives in the contrats stream, which is not loaded at all —
    // it is absent, not skipped. The rule (meta, always loaded) references
    // the unloaded contract: skipped and counted.
    assert_eq!(g.rule_count(), 0);
    assert_eq!(r.skipped_cross_domain, 1, "{r:?}");
    assert_eq!(r.applied, 4, "ontology + 2 concepts + employed_by");

    // Only contrats: the contract, and `between` skipped (target missing).
    let g = OntologyGraph::with_arc(Ontology::new());
    let r = store
        .load_domains_report(&g, &["contrats".to_string()])
        .await
        .unwrap();
    assert_eq!(g.concept_count(), 1);
    assert_eq!(g.relation_count(), 0);
    assert_eq!(r.skipped_cross_domain, 1);
    assert_eq!(g.rule_count(), 1, "rule applies to a loaded contract");

    // Both: complete.
    let g = OntologyGraph::with_arc(Ontology::new());
    let r = store
        .load_domains_report(&g, &["contrats".to_string(), "parties".to_string()])
        .await
        .unwrap();
    assert_eq!(g.concept_count(), 3);
    assert_eq!(g.relation_count(), 2);
    assert_eq!(r.skipped_cross_domain, 0);

    // Unknown domain: warned, nothing loaded from it.
    let g = OntologyGraph::with_arc(Ontology::new());
    store
        .load_domains_report(&g, &["nope".to_string()])
        .await
        .unwrap();
    assert_eq!(g.concept_count(), 0);
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn compaction_rewrites_the_whole_store_and_the_result_replays_identically() {
    let root = tempdir("compact");
    let store = SegmentStore::open_with(&root, small_roll()).await.unwrap();
    let fx = Fixture::new(&store).await;
    let acme = fx.concept(&store, "Company", "Acme").await;
    let globex = fx.concept(&store, "Company", "Globex").await;
    let alice = fx.concept(&store, "Person", "Alice").await;
    let mut contracts = Vec::new();
    for i in 0..8 {
        contracts.push(fx.concept(&store, "Contract", &format!("C-{i}")).await);
    }
    for c in &contracts {
        fx.relation(&store, "between", *c, acme).await;
    }
    fx.relation(&store, "employed_by", alice, acme).await;
    // Churn: rename, delete, so the log holds superseded records.
    let renamed = fx
        .graph
        .preview_concept_update(
            alice,
            &ConceptPatch {
                name: Some("Alicia".into()),
                ..Default::default()
            },
        )
        .unwrap();
    store
        .append(&LogRecord::update_concept(renamed.clone()))
        .await
        .unwrap();
    fx.graph.apply_concept_update(renamed).unwrap();
    for c in &contracts[..4] {
        fx.delete_concept(&store, *c).await;
    }
    fx.delete_concept(&store, globex).await;
    let mut rule = Rule::new(Default::default(), "must_review", "r1");
    rule.applies_to = vec![contracts[5]];
    fx.graph.prepare_rule(&mut rule).unwrap();
    store.append(&LogRecord::rule(rule.clone())).await.unwrap();
    fx.graph.apply_prepared_rule(rule).unwrap();

    let before = store.record_count();
    let parts_before: usize = ["meta", "graph/default", "graph/parties", "graph/contrats"]
        .iter()
        .map(|d| partitions(&root.join(d)).len())
        .sum();
    assert!(
        parts_before > 4,
        "several partitions rolled: {parts_before}"
    );

    let report = store.compact_store(&fx.graph).await.unwrap();
    assert!(report.records_after < report.records_before, "{report:?}");
    assert_eq!(report.records_before, before);
    // Live state: ontology + 2 (Acme, Alicia) + 4 contracts + 4 between +
    // 1 employed_by + 1 rule = 13.
    assert_eq!(report.records_after, 13, "{report:?}");
    assert_eq!(report.partitions_removed, parts_before);
    // Each stream is now one sealed partition + one empty active.
    for d in ["meta", "graph/parties", "graph/contrats"] {
        let ids = partitions(&root.join(d));
        assert_eq!(ids.len(), 2, "{d}: {ids:?}");
    }
    // No leftovers on disk.
    for d in ["meta", "graph/default", "graph/parties", "graph/contrats"] {
        let olds = std::fs::read_dir(root.join(d))
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|x| x == "old")
            })
            .count();
        assert_eq!(olds, 0, "{d} has .old leftovers");
    }

    // Replays to the live graph, and writes continue after compaction.
    let fresh = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&fresh).await.unwrap();
    assert_eq!(fresh.concept_count(), 6);
    assert_eq!(fresh.relation_count(), 5);
    assert_eq!(fresh.rule_count(), 1);
    assert_eq!(fresh.get_concept(alice).unwrap().name, "Alicia");
    assert!(fresh.get_concept(globex).is_err());
    let next = store.next_seq();
    let bob = fx.concept(&store, "Person", "Bob").await;
    assert_eq!(store.next_seq(), next + 1);

    // …and after a restart, xrefs included.
    drop(store);
    let store = SegmentStore::open_with(&root, small_roll()).await.unwrap();
    assert_eq!(store.open_report().xref_entries, 4, "4 live between edges");
    let fresh = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&fresh).await.unwrap();
    assert_eq!(fresh.concept_count(), 7);
    assert_eq!(fresh.get_concept(bob).unwrap().name, "Bob");
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn a_schema_change_reroutes_the_records_that_follow_it() {
    let root = tempdir("reroute");
    let store = SegmentStore::open(&root).await.unwrap();
    let fx = Fixture::new(&store).await;
    fx.concept(&store, "Company", "Acme").await;
    // Declare a new domain for a new type in the same batch as its first
    // instance: the Ontology record precedes it, so routing sees it.
    let mut onto = fx.graph.ontology();
    onto.add_concept_type(ct("Asset", Some("actifs"), None));
    fx.graph
        .extend_ontology(|o| {
            *o = onto.clone();
            Ok(())
        })
        .unwrap();
    let mut asset = Concept::new(ConceptId(0), "Asset", "Truck");
    fx.graph.prepare_concept(&mut asset).unwrap();
    store
        .append_batch(&[LogRecord::ontology(onto), LogRecord::concept(asset.clone())])
        .await
        .unwrap();
    fx.graph.apply_prepared_concept(asset).unwrap();
    let counts = store.record_counts_by_domain();
    assert_eq!(counts["actifs"], 1, "{counts:?}");
    assert_eq!(counts["meta"], 2);
    assert!(root.join("graph/actifs").exists());
    // The manifest froze the id; a reopen finds the stream and the router.
    drop(store);
    let store = SegmentStore::open(&root).await.unwrap();
    assert!(store.manifest().ns_id("actifs").is_some());
    let fresh = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&fresh).await.unwrap();
    assert!(fresh.find_by_name("Asset", "Truck").is_some());
    let meta_ns = META_NS_ID;
    assert_eq!(meta_ns, 0);
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn a_single_domain_store_keeps_the_phase_2_layout() {
    let root = tempdir("single");
    let store = SegmentStore::open(&root).await.unwrap();
    let mut o = Ontology::new();
    o.add_concept_type(ct("Person", None, None));
    store.append(&LogRecord::ontology(o.clone())).await.unwrap();
    store
        .append(&LogRecord::concept(Concept::new(
            ConceptId(1),
            "Person",
            "a",
        )))
        .await
        .unwrap();
    assert_eq!(partitions(&root.join("meta")), vec![1]);
    assert_eq!(partitions(&root.join("graph/default")), vec![2]);
    let m = store.manifest();
    assert_eq!(m.ns.len(), 2, "meta + default only");
    assert_eq!(store.record_counts_by_domain()["default"], 1);
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn compaction_keeps_relation_ids_so_later_tombstones_still_apply() {
    // Symmetric relations are two live relations with two ids; compaction
    // writes both as RelationExact, so after a restart the ids on disk are
    // the ids the live graph deletes with.
    let root = tempdir("compact-ids");
    let store = SegmentStore::open_with(&root, small_roll()).await.unwrap();
    let fx = Fixture::new(&store).await;
    let acme = fx.concept(&store, "Company", "Acme").await;
    let globex = fx.concept(&store, "Company", "Globex").await;
    let initech = fx.concept(&store, "Company", "Initech").await;
    fx.relation(&store, "partner_of", acme, globex).await;
    fx.relation(&store, "partner_of", globex, initech).await;
    assert_eq!(fx.graph.relation_count(), 4, "two symmetric pairs");
    let live_before: Vec<Relation> = fx.graph.all_relations();

    store.compact_store(&fx.graph).await.unwrap();

    // Replay of the compacted store yields the very same relations, ids included.
    let fresh = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&fresh).await.unwrap();
    assert_eq!(fresh.relation_count(), 4);
    for r in &live_before {
        let got = fresh.get_relation(r.id).unwrap();
        assert_eq!(
            (got.source, got.target),
            (r.source, r.target),
            "id {} moved",
            r.id
        );
    }

    // Delete one pair through the live graph (its ids), restart: gone.
    let pair = fx.graph.incident_relation_ids(initech).unwrap();
    assert_eq!(pair.len(), 2);
    let recs: Vec<LogRecord> = pair
        .iter()
        .map(|rid| {
            LogRecord::delete_relation(*rid, fx.graph.get_relation(*rid).unwrap().relation_type)
        })
        .collect();
    store.append_batch(&recs).await.unwrap();
    for rid in &pair {
        fx.graph.remove_relation(*rid).unwrap();
    }
    drop(store);
    let store = SegmentStore::open_with(&root, small_roll()).await.unwrap();
    let fresh = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&fresh).await.unwrap();
    assert_eq!(fresh.relation_count(), 2, "the deleted pair stays deleted");
    assert!(fresh.get_relation(pair[0]).is_err());
    assert_eq!(fresh.incident_relation_ids(initech).unwrap().len(), 0);
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn an_interrupted_compaction_is_finished_or_discarded_at_open() {
    use ontology_storage::manifest::CompactionMarker;
    use ontology_storage::Manifest;

    let root = tempdir("compact-crash");
    let (acme, before_ids);
    {
        let store = SegmentStore::open_with(&root, small_roll()).await.unwrap();
        let fx = Fixture::new(&store).await;
        acme = fx.concept(&store, "Company", "Acme").await;
        for i in 0..7 {
            fx.concept(&store, "Company", &format!("c{i}")).await;
        }
        before_ids = fx
            .graph
            .all_concepts()
            .iter()
            .map(|c| c.id)
            .collect::<Vec<_>>();
    }
    let pdir = root.join("graph/parties");
    let old = partitions(&pdir);
    assert_eq!(old.len(), 2, "one sealed + one active: {old:?}");

    // (a) Aborted staging — files in compacting/, no marker: removed at open,
    //     nothing else changes.
    let staging = pdir.join("compacting");
    std::fs::create_dir_all(&staging).unwrap();
    std::fs::write(staging.join("000099.data"), b"junk").unwrap();
    {
        let store = SegmentStore::open_with(&root, small_roll()).await.unwrap();
        assert!(!staging.exists(), "aborted staging discarded");
        assert_eq!(partitions(&pdir), old);
        let g = OntologyGraph::with_arc(Ontology::new());
        store.load_into(&g).await.unwrap();
        assert_eq!(g.concept_count(), 8);
    }

    // (b) Commit point passed — staged partition present, marker written,
    //     process died before the swap: open finishes it.
    //     Build the staged partition as a real compaction would: a copy of
    //     the live state. Simplest faithful way: run a compaction, then
    //     recreate the "just committed" state from what it produced.
    let store = SegmentStore::open_with(&root, small_roll()).await.unwrap();
    let g = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&g).await.unwrap();
    store.compact_store(&g).await.unwrap();
    let after = partitions(&pdir); // [sealed compacted, fresh active]
    assert_eq!(after.len(), 2);
    drop(store);
    // Move the compacted sealed partition back into compacting/ and write a
    // marker naming it as staged and the (now absent) old ids as to-remove,
    // plus the current active as old. Exactly the on-disk picture after the
    // commit point.
    let ns_id = Manifest::load(&root)
        .unwrap()
        .unwrap()
        .ns_id("parties")
        .unwrap();
    let staged_pid = after[0];
    std::fs::create_dir_all(&staging).unwrap();
    for ext in ["data", "idx", "xref"] {
        let f = pdir.join(format!("{staged_pid:06}.{ext}"));
        if f.exists() {
            std::fs::rename(&f, staging.join(format!("{staged_pid:06}.{ext}"))).unwrap();
        }
    }
    let mut m = Manifest::load(&root).unwrap().unwrap();
    m.compaction = Some(CompactionMarker {
        staged: vec![(ns_id, staged_pid)],
        remove: vec![(ns_id, vec![after[1]])],
    });
    m.save(&root).unwrap();

    let store = SegmentStore::open_with(&root, small_roll()).await.unwrap();
    assert!(store.manifest().compaction.is_none(), "marker cleared");
    assert!(!staging.exists());
    let now = partitions(&pdir);
    assert!(
        now.contains(&staged_pid),
        "staged partition moved in: {now:?}"
    );
    assert!(!now.contains(&after[1]), "old active removed: {now:?}");
    let g = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&g).await.unwrap();
    assert_eq!(g.concept_count(), 8);
    assert!(g.get_concept(acme).is_ok());
    let mut ids: Vec<_> = g.all_concepts().iter().map(|c| c.id).collect();
    let mut expect = before_ids.clone();
    ids.sort();
    expect.sort();
    assert_eq!(ids, expect);
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn xref_entries_are_filed_in_the_partition_whose_seq_range_holds_them() {
    let root = tempdir("xref-ranges");
    {
        let store = SegmentStore::open_with(&root, small_roll()).await.unwrap();
        let fx = Fixture::new(&store).await;
        let acme = fx.concept(&store, "Company", "Acme").await;
        // Roll parties several times while contracts keep pointing at Acme.
        for i in 0..12 {
            fx.concept(&store, "Person", &format!("p{i}")).await;
            let c = fx.concept(&store, "Contract", &format!("C-{i}")).await;
            fx.relation(&store, "between", c, acme).await;
        }
    }
    let store = SegmentStore::open_with(&root, small_roll()).await.unwrap();
    let parties = store.xrefs("parties").unwrap();
    assert!(parties.len() >= 3, "{}", parties.len());
    let m = store.manifest();
    let ns_id = m.ns_id("parties").unwrap();
    let mut bases: Vec<(u32, u64)> = m
        .stream(ns_id)
        .unwrap()
        .sealed
        .iter()
        .map(|p| (p.id, p.base_seq))
        .collect();
    bases.sort_by_key(|(id, _)| *id);
    let mut total = 0;
    for (i, (pid, entries)) in parties.iter().enumerate() {
        total += entries.len();
        if let Some((sealed_id, base)) = bases.get(i) {
            assert_eq!(pid, sealed_id);
            let upper = bases.get(i + 1).map(|(_, b)| *b).unwrap_or(u64::MAX);
            for e in entries {
                assert!(
                    e.seq >= *base && e.seq < upper,
                    "seq {} not in [{base}, {upper})",
                    e.seq
                );
            }
        }
    }
    assert_eq!(total, 12);
    std::fs::remove_dir_all(&root).ok();
}
