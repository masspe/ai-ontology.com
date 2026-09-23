// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! P1, storage side (`STORAGE.md` §6.2, §8.2; `STORAGE-PLAN.md` §7.2):
//! concept locations, the lock-free sealed reader, seal notifications and
//! relocation by compaction. The graph side is stubbed on this branch, so
//! eviction itself is asserted in an `#[ignore]`d test.

use ontology_graph::{
    Concept, ConceptId, ConceptType, Loc, Ontology, OntologyGraph, PayloadSource, Relation,
    RelationId, RelationType,
};
use ontology_storage::segment::{Kind, SealedSegment};
use ontology_storage::{LogRecord, RollPolicy, SegmentStore, SegmentStoreConfig, Store, Tier};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn tempdir(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "ontology-p1-{tag}-{}-{}-{}",
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

/// Two domains: `Doc` in `docs`, `Person` in `people`; `wrote: Person -> Doc`.
fn ontology() -> Ontology {
    let mut o = Ontology::new();
    for (ty, ns) in [("Doc", "docs"), ("Person", "people")] {
        o.add_concept_type(ConceptType {
            name: ty.into(),
            ns: Some(ns.into()),
            ..Default::default()
        });
    }
    o.add_relation_type(RelationType {
        name: "wrote".into(),
        domain: "Person".into(),
        range: "Doc".into(),
        ..Default::default()
    })
    .unwrap();
    o
}

fn small_roll() -> SegmentStoreConfig {
    SegmentStoreConfig {
        roll: RollPolicy {
            max_bytes: u64::MAX,
            max_records: 7,
        },
        ..Default::default()
    }
}

const PERSON_BASE: u64 = 1000;

fn doc_name(id: u64, updated: bool) -> String {
    if updated {
        format!("Doc-{id}-v2")
    } else {
        format!("Doc-{id}")
    }
}

/// `n` docs (`ConceptId(1..=n)`), `n` people (`ConceptId(1001..)`), every
/// third doc renamed by an `UpdateConcept`, one `wrote` per pair. Several
/// partitions seal in each domain with the small roll policy.
async fn build(dir: &Path, n: u64) {
    let store = SegmentStore::open_with(dir, small_roll()).await.unwrap();
    store
        .append(&LogRecord::ontology(ontology()))
        .await
        .unwrap();
    for i in 1..=n {
        store
            .append(&LogRecord::concept(Concept::new(
                ConceptId(i),
                "Doc",
                doc_name(i, false),
            )))
            .await
            .unwrap();
        let p = PERSON_BASE + i;
        store
            .append(&LogRecord::concept(Concept::new(
                ConceptId(p),
                "Person",
                format!("Person-{p}"),
            )))
            .await
            .unwrap();
    }
    for i in (3..=n).step_by(3) {
        store
            .append(&LogRecord::update_concept(Concept::new(
                ConceptId(i),
                "Doc",
                doc_name(i, true),
            )))
            .await
            .unwrap();
    }
    for i in 1..=n {
        store
            .append(&LogRecord::relation(Relation::new(
                RelationId(i),
                "wrote",
                ConceptId(PERSON_BASE + i),
                ConceptId(i),
            )))
            .await
            .unwrap();
    }
}

fn p1_everywhere(store: &SegmentStore) -> HashMap<u16, Tier> {
    store
        .manifest()
        .graph_ns_ids()
        .into_iter()
        .map(|ns| (ns, Tier::P1))
        .collect()
}

/// Every sealed partition of every graph stream, mapped a second time from
/// the files the manifest names: `(ns_id, segment)`.
fn sealed_segments(store: &SegmentStore) -> Vec<(u16, SealedSegment)> {
    let m = store.manifest();
    let mut out = Vec::new();
    for ns in m.graph_ns_ids() {
        let entry = m.stream(ns).unwrap();
        for p in &entry.sealed {
            out.push((
                ns,
                SealedSegment::open(&store.root().join(&entry.dir), p.id).unwrap(),
            ));
        }
    }
    out
}

/// `Concept` has no `PartialEq`: compare what a payload holds.
fn keys(concepts: &[Concept]) -> Vec<(u64, String, String, String)> {
    let mut v: Vec<_> = concepts
        .iter()
        .map(|c| {
            (
                c.id.0,
                c.concept_type.clone(),
                c.name.clone(),
                c.description.clone(),
            )
        })
        .collect();
    v.sort();
    v
}

fn expected_name(ns_name: &str, kind: Kind, id: u64) -> String {
    match ns_name {
        "docs" => doc_name(id, kind == Kind::UpdateConcept),
        _ => format!("Person-{id}"),
    }
}

/// Reads every concept entry of `seg` back through `reader` and checks
/// id and name. Returns the number of concept entries checked.
fn check_segment(
    store: &SegmentStore,
    reader: &dyn PayloadSource,
    ns: u16,
    seg: &SealedSegment,
    name_of: &dyn Fn(Kind, u64) -> String,
) -> usize {
    let ns_name = store.manifest().ns_name(ns).unwrap().to_string();
    let mut checked = 0;
    for e in seg.entries() {
        let loc = Loc {
            ns_id: ns,
            partition: seg.partition_id(),
            offset: e.offset,
        };
        match e.kind {
            Kind::Concept | Kind::UpdateConcept => {
                let c = reader
                    .read(loc)
                    .unwrap_or_else(|err| panic!("{loc:?}: {err}"));
                assert_eq!(c.id.0, e.entity_id, "{loc:?}");
                assert_eq!(c.name, name_of(e.kind, e.entity_id), "{ns_name} {loc:?}");
                checked += 1;
            }
            _ => {
                let err = reader.read(loc).unwrap_err();
                assert!(err.contains("not a concept"), "{loc:?}: {err}");
            }
        }
    }
    checked
}

/// 1. Without `set_tiers`, every domain is P0: nothing is evicted and the
/// report names no P1 domain.
#[tokio::test]
async fn p0_by_default_keeps_every_payload_resident() {
    let dir = tempdir("p0");
    build(&dir, 12).await;
    let store = SegmentStore::open_with(&dir, small_roll()).await.unwrap();
    let g = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&g).await.unwrap();
    assert_eq!(g.concept_count(), 24);
    assert_eq!(g.resident_payloads(), g.concept_count());
    let report = store.last_hydration().unwrap();
    assert!(report.p1_domains.is_empty(), "{report:?}");
    assert!(
        !store.payload_source().partitions().is_empty(),
        "the sealed index is kept whatever the tier"
    );
}

/// 2. Every `Concept` / `UpdateConcept` entry of every sealed partition
/// reads back through the lock-free reader with the right id and name; a
/// relation's location, an offset inside a record and an unknown partition
/// are errors naming the location.
#[tokio::test]
async fn reader_returns_every_sealed_concept_and_refuses_the_rest() {
    let dir = tempdir("reader");
    build(&dir, 20).await;
    let store = SegmentStore::open_with(&dir, small_roll()).await.unwrap();
    store.set_tiers(p1_everywhere(&store));
    let g = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&g).await.unwrap();
    let report = store.last_hydration().unwrap();
    assert_eq!(report.p1_domains, ["default", "docs", "people"]);

    let reader = store.payload_source();
    let segs = sealed_segments(&store);
    assert!(segs.len() >= 5, "{} sealed partitions", segs.len());
    assert_eq!(
        reader.partitions(),
        segs.iter()
            .map(|(ns, s)| (*ns, s.partition_id()))
            .collect::<Vec<_>>(),
        "the index holds exactly the sealed partitions of the graph streams"
    );
    let ns_name = |ns: u16| store.manifest().ns_name(ns).unwrap().to_string();
    let mut concepts = 0;
    let mut relations = 0;
    for (ns, seg) in &segs {
        let name = ns_name(*ns);
        concepts += check_segment(&store, &*reader, *ns, seg, &|k, id| {
            expected_name(&name, k, id)
        });
        relations += seg.entries().filter(|e| e.kind == Kind::Relation).count();
    }
    assert!(concepts >= 20, "{concepts} concept entries read back");
    assert!(relations > 0, "a relation loc was refused");

    // An offset inside a record.
    let (ns, seg) = &segs[0];
    let first = seg.entry(0).unwrap();
    let inside = Loc {
        ns_id: *ns,
        partition: seg.partition_id(),
        offset: first.offset + 8,
    };
    let err = reader.read(inside).unwrap_err();
    assert!(err.contains(&format!("offset {}", inside.offset)), "{err}");
    // An unknown partition.
    let err = reader
        .read(Loc {
            ns_id: *ns,
            partition: 9_999,
            offset: first.offset,
        })
        .unwrap_err();
    assert!(err.contains("partition 9999"), "{err}");
    // An unknown domain.
    let err = reader
        .read(Loc {
            ns_id: 777,
            partition: seg.partition_id(),
            offset: first.offset,
        })
        .unwrap_err();
    assert!(err.contains("ns 777"), "{err}");
}

/// 3. Hydrating in P1 yields the same graph as in P0: same concepts (by
/// id, with the latest name), same relations, same listing.
#[tokio::test]
async fn p1_hydration_builds_the_same_graph_as_p0() {
    let dir = tempdir("same");
    build(&dir, 20).await;
    let store = SegmentStore::open_with(&dir, small_roll()).await.unwrap();

    let p0 = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&p0).await.unwrap();
    store.set_tiers(p1_everywhere(&store));
    let p1 = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&p1).await.unwrap();

    assert_eq!(keys(&p0.all_concepts()), keys(&p1.all_concepts()));
    assert_eq!(p0.all_concepts().len(), 40);
    assert_eq!(p0.relation_count(), p1.relation_count());
    assert_eq!(p1.relation_count(), 20);
    let page = |g: &OntologyGraph| {
        let (total, page) = g.list_concepts_page(None, None, 0, 100, true, false);
        // The listing order is part of the contract, so no sort here.
        let ids: Vec<u64> = page.iter().map(|c| c.id.0).collect();
        (total, ids, keys(&page))
    };
    assert_eq!(page(&p0), page(&p1));
    assert_eq!(
        p1.list_concepts_page(Some("Doc"), Some("v2"), 0, 100, true, false)
            .0,
        6
    );
    assert_eq!(p1.get_concept(ConceptId(3)).unwrap().name, "Doc-3-v2");
}

/// Eviction proper: after a P1 hydration only the payloads whose latest
/// record sits in an active segment stay resident.
// passes once feat/p1-graph lands
#[ignore = "passes once feat/p1-graph lands (set_loc / partition_sealed are P0 stubs here)"]
#[tokio::test]
async fn p1_hydration_evicts_sealed_payloads() {
    let dir = tempdir("evict");
    build(&dir, 20).await;
    let store = SegmentStore::open_with(&dir, small_roll()).await.unwrap();
    store.set_tiers(p1_everywhere(&store));
    let g = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&g).await.unwrap();
    let active_records: usize = store
        .open_report()
        .graph
        .values()
        .map(|r| r.active_records as usize)
        .sum();
    assert!(g.resident_payloads() < g.concept_count());
    assert!(g.resident_payloads() <= active_records);
    // Evicted payloads still read through the source.
    assert_eq!(g.get_concept(ConceptId(3)).unwrap().name, "Doc-3-v2");
}

/// 4. Write path: appends on a P1 domain land in the active partition; when
/// the roll seals it, the partition enters the index and the records that
/// were appended while it was active read back through the reader.
#[tokio::test]
async fn appends_seal_into_the_index_and_read_back() {
    let dir = tempdir("append");
    let docs = {
        let store = SegmentStore::open_with(&dir, small_roll()).await.unwrap();
        store
            .append(&LogRecord::ontology(ontology()))
            .await
            .unwrap();
        for i in 1..=3u64 {
            store
                .append(&LogRecord::concept(Concept::new(
                    ConceptId(i),
                    "Doc",
                    doc_name(i, false),
                )))
                .await
                .unwrap();
        }
        store.manifest().ns_id("docs").unwrap()
    };
    let store = SegmentStore::open_with(&dir, small_roll()).await.unwrap();
    store.set_tiers(HashMap::from([(docs, Tier::P1)]));
    let g = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&g).await.unwrap();
    assert_eq!(store.last_hydration().unwrap().p1_domains, ["docs"]);
    let active = store.open_report().graph[&docs].active_partition;
    let reader = store.payload_source();
    assert!(
        !reader.partitions().contains(&(docs, active)),
        "the active partition is not readable through the index"
    );
    assert!(reader
        .read(Loc {
            ns_id: docs,
            partition: active,
            offset: 0,
        })
        .is_err());

    // 3 records in the active segment + 4 = max_records: the roll seals it.
    let batch: Vec<LogRecord> = (4..=7u64)
        .map(|i| LogRecord::concept(Concept::new(ConceptId(i), "Doc", doc_name(i, false))))
        .collect();
    store.append_batch(&batch).await.unwrap();
    assert!(reader.partitions().contains(&(docs, active)), "sealed");
    assert_eq!(
        store
            .manifest()
            .stream(docs)
            .unwrap()
            .sealed
            .last()
            .unwrap()
            .id,
        active
    );
    let seg = SealedSegment::open(
        &store
            .root()
            .join(&store.manifest().stream(docs).unwrap().dir),
        active,
    )
    .unwrap();
    assert_eq!(seg.record_count(), 7);
    let checked = check_segment(&store, &*reader, docs, &seg, &|k, id| {
        expected_name("docs", k, id)
    });
    assert_eq!(checked, 7, "the 3 pre-roll and the 4 new concepts");
    // Every appended concept's name reads back, ids included.
    let mut names: Vec<String> = seg
        .entries()
        .map(|e| {
            reader
                .read(Loc {
                    ns_id: docs,
                    partition: active,
                    offset: e.offset,
                })
                .unwrap()
                .name
        })
        .collect();
    names.sort();
    let mut want: Vec<String> = (1..=7).map(|i| doc_name(i, false)).collect();
    want.sort();
    assert_eq!(names, want);
    // Applying the batch to the graph afterwards (write-ahead order) keeps
    // the graph and the store in step.
    for r in &batch {
        if let ontology_storage::RecordKind::Concept(c) = &r.kind {
            g.upsert_concept(c.clone()).unwrap();
        }
    }
    assert_eq!(g.concept_count(), 7);
    // passes once feat/p1-graph lands: the 7 sealed payloads are dropped
    // (`partition_sealed`) and read back through the source on demand.
    assert_eq!(g.get_concept(ConceptId(5)).unwrap().name, "Doc-5");
}

/// 5. Compaction rewrites every domain into one fresh sealed partition: the
/// old partitions leave the index, the new ones enter it, and every concept
/// entry of the new partitions reads back as the live concept.
#[tokio::test]
async fn compaction_relocates_every_sealed_concept() {
    let dir = tempdir("compact");
    build(&dir, 20).await;
    let store = SegmentStore::open_with(&dir, small_roll()).await.unwrap();
    store.set_tiers(p1_everywhere(&store));
    let g = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&g).await.unwrap();
    let reader = store.payload_source();
    let before = reader.partitions();
    assert!(before.len() >= 5);

    store.compact(&g).await.unwrap();

    let after = reader.partitions();
    assert_eq!(after.len(), 2, "one sealed partition per domain: {after:?}");
    assert!(
        after.iter().all(|p| !before.contains(p)),
        "old partitions gone from the index: {before:?} -> {after:?}"
    );
    for (ns, seg) in sealed_segments(&store) {
        assert!(after.contains(&(ns, seg.partition_id())));
        assert!(seg.entries().all(|e| e.kind != Kind::UpdateConcept));
        let checked = check_segment(&store, &*reader, ns, &seg, &|_, id| {
            g.get_concept(ConceptId(id)).unwrap().name
        });
        assert_eq!(checked, 20, "every live concept of the domain");
    }
    for p in &before {
        assert!(reader
            .read(Loc {
                ns_id: p.0,
                partition: p.1,
                offset: 0,
            })
            .is_err());
    }
    // passes once feat/p1-graph lands: every relocated payload is evicted
    // (`set_loc(.., resident = false)`) and still readable.
    assert_eq!(g.get_concept(ConceptId(3)).unwrap().name, "Doc-3-v2");

    // The compacted store hydrates to the same graph, in P1 again.
    drop(store);
    let store = SegmentStore::open_with(&dir, small_roll()).await.unwrap();
    store.set_tiers(p1_everywhere(&store));
    let g2 = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&g2).await.unwrap();
    assert_eq!(keys(&g.all_concepts()), keys(&g2.all_concepts()));
    assert_eq!(g2.relation_count(), 20);
}

/// `set_tiers` with a partial map: the named domain is P1, the other stays
/// P0; the report says which.
#[tokio::test]
async fn tiers_apply_per_domain() {
    let dir = tempdir("tiers");
    build(&dir, 6).await;
    let store = SegmentStore::open_with(&dir, small_roll()).await.unwrap();
    let people = store.manifest().ns_id("people").unwrap();
    store.set_tiers(HashMap::from([(people, Tier::P1)]));
    let g = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&g).await.unwrap();
    assert_eq!(store.last_hydration().unwrap().p1_domains, ["people"]);
    assert_eq!(g.concept_count(), 12);
    let _: Arc<dyn PayloadSource> = store.payload_source();
}
