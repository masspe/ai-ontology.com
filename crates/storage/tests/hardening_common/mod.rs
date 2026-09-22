// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Fixtures shared by the `hardening_*` integration tests: a unique temp
//! dir per test, a two-domain schema, a write-ahead driver mirroring the
//! server (`prepare → append → apply`, R8) and small on-disk probes.

#![allow(dead_code)]

use ontology_graph::{
    Concept, ConceptId, ConceptType, Ontology, OntologyGraph, Relation, RelationType, Rule,
    RuleType,
};
use ontology_storage::segment::{idx_path, IdxEntry};
use ontology_storage::{LogRecord, RollPolicy, SegmentStoreConfig, Store};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A fresh, uniquely named directory under the system temp dir.
pub fn tempdir(tag: &str) -> PathBuf {
    // pid + counter + clock: the clock alone repeats on Windows (coarse
    // ticks) when two tests create their directory in the same instant.
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "ontology-hardening-{tag}-{}-{}-{}",
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

pub fn ct(name: &str, ns: Option<&str>, parent: Option<&str>) -> ConceptType {
    ConceptType {
        name: name.into(),
        ns: ns.map(str::to_string),
        parent: parent.map(str::to_string),
        ..Default::default()
    }
}

pub fn rt(name: &str, domain: &str, range: &str, symmetric: bool) -> RelationType {
    RelationType {
        name: name.into(),
        domain: domain.into(),
        range: range.into(),
        symmetric,
        ..Default::default()
    }
}

/// Two graph domains: `parties` (Company, Person) and `contrats`
/// (Contract); `between` crosses from contrats to parties, `employed_by`
/// stays in parties, `partner_of` is symmetric inside parties; one rule
/// type on Contract.
pub fn ontology() -> Ontology {
    let mut o = Ontology::new();
    o.add_concept_type(ct("Company", Some("parties"), None));
    o.add_concept_type(ct("Person", Some("parties"), None));
    o.add_concept_type(ct("Contract", Some("contrats"), None));
    o.add_relation_type(rt("between", "Contract", "Company", false))
        .unwrap();
    o.add_relation_type(rt("employed_by", "Person", "Company", false))
        .unwrap();
    o.add_relation_type(rt("partner_of", "Company", "Company", true))
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

/// Roll by record count only.
pub fn roll(max_records: u32) -> SegmentStoreConfig {
    SegmentStoreConfig {
        roll: RollPolicy {
            max_bytes: u64::MAX,
            max_records,
        },
        ..Default::default()
    }
}

/// Graph + store driven in write-ahead order, like the server does.
pub struct Fixture {
    pub graph: Arc<OntologyGraph>,
}

impl Fixture {
    pub async fn new(store: &dyn Store) -> Self {
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
    pub async fn concept(&self, store: &dyn Store, ty: &str, name: &str) -> ConceptId {
        let mut c = Concept::new(ConceptId(0), ty, name);
        self.graph.prepare_concept(&mut c).unwrap();
        store.append(&LogRecord::concept(c.clone())).await.unwrap();
        self.graph.apply_prepared_concept(c).unwrap()
    }
    pub async fn relation(&self, store: &dyn Store, rt: &str, a: ConceptId, b: ConceptId) {
        let mut r = Relation::new(Default::default(), rt, a, b);
        self.graph.prepare_relation(&mut r).unwrap();
        store.append(&LogRecord::relation(r.clone())).await.unwrap();
        self.graph.apply_prepared_relation(r).unwrap();
    }
    pub async fn rule(&self, store: &dyn Store, name: &str, applies_to: Vec<ConceptId>) {
        let mut rule = Rule::new(Default::default(), "must_review", name);
        rule.applies_to = applies_to;
        self.graph.prepare_rule(&mut rule).unwrap();
        store.append(&LogRecord::rule(rule.clone())).await.unwrap();
        self.graph.apply_prepared_rule(rule).unwrap();
    }
    /// Journal the tombstone and its cascade as one batch, then apply.
    pub async fn delete_concept(&self, store: &dyn Store, id: ConceptId) {
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

/// Schema plus a small graph with churn in every stream: 3 companies /
/// people, 4 contracts, cross-domain and symmetric relations, a rule, and
/// one deletion with its cascade. 17 records; with `roll(3)` several
/// partitions per domain.
pub async fn populate(store: &dyn Store) -> Fixture {
    let fx = Fixture::new(store).await;
    let acme = fx.concept(store, "Company", "Acme").await;
    let globex = fx.concept(store, "Company", "Globex").await;
    let alice = fx.concept(store, "Person", "Alice").await;
    let mut contracts = Vec::new();
    for i in 0..4 {
        contracts.push(fx.concept(store, "Contract", &format!("C-{i}")).await);
    }
    for c in &contracts {
        fx.relation(store, "between", *c, acme).await;
    }
    fx.relation(store, "employed_by", alice, acme).await;
    fx.relation(store, "partner_of", acme, globex).await;
    fx.rule(store, "r1", vec![contracts[1]]).await;
    fx.delete_concept(store, contracts[0]).await;
    fx
}

/// Partition ids present in `dir` (from the `.data` files), ascending.
pub fn partitions(dir: &Path) -> Vec<u32> {
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

/// Decoded `.idx` entries of a partition.
pub fn entries(dir: &Path, partition: u32) -> Vec<IdxEntry> {
    let bytes = std::fs::read(idx_path(dir, partition)).unwrap();
    (0..IdxEntry::count_in(bytes.len() as u64))
        .map(|i| IdxEntry::decode(&bytes, IdxEntry::file_offset(i)).unwrap())
        .collect()
}

/// Number of files in `dir` whose extension is `ext` (not recursive).
pub fn files_with_ext(dir: &Path, ext: &str) -> usize {
    match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == ext))
            .count(),
        Err(_) => 0,
    }
}

/// Recursive copy of a directory tree.
pub fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dst = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &dst);
        } else {
            std::fs::copy(entry.path(), &dst).unwrap();
        }
    }
}

/// The stream directories of a store, relative to its root.
pub const STREAM_DIRS: [&str; 4] = ["meta", "graph/default", "graph/parties", "graph/contrats"];
