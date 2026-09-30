// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Backup and restore (ROADMAP §3.8.4): a backup restores to the same
//! graph, a repeated backup copies only what changed, a compaction's
//! removed partitions disappear from the backup, and the refusals hold.

use ontology_graph::{
    Concept, ConceptId, ConceptType, Ontology, OntologyGraph, Relation, RelationType,
};
use ontology_storage::migrate::compare_graphs;
use ontology_storage::{
    restore, LogRecord, MemoryStore, RollPolicy, SegmentStore, SegmentStoreConfig, Store,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn tempdir(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "ontology-backup-{tag}-{}-{}-{}",
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

fn ontology() -> Ontology {
    let mut o = Ontology::new();
    o.add_concept_type(ConceptType {
        name: "Person".into(),
        ..Default::default()
    });
    o.add_concept_type(ConceptType {
        name: "Site".into(),
        ns: Some("sites".into()),
        ..Default::default()
    });
    o.add_relation_type(RelationType {
        name: "works_on".into(),
        domain: "Person".into(),
        range: "Site".into(),
        ..Default::default()
    })
    .unwrap();
    o
}

/// Tiny partitions, so a handful of records spans several sealed ones.
fn small_roll() -> SegmentStoreConfig {
    SegmentStoreConfig {
        roll: RollPolicy {
            max_bytes: 512,
            max_records: 4,
        },
        ..Default::default()
    }
}

async fn open(dir: &Path) -> Arc<SegmentStore> {
    Arc::new(SegmentStore::open_with(dir, small_roll()).await.unwrap())
}

async fn fill(store: &SegmentStore, from: u64, to: u64) {
    let mut batch = Vec::new();
    for i in from..to {
        batch.push(LogRecord::concept(Concept::new(
            ConceptId(i),
            "Person",
            format!("person {i}"),
        )));
        batch.push(LogRecord::concept(Concept::new(
            ConceptId(1000 + i),
            "Site",
            format!("site {i}"),
        )));
        batch.push(LogRecord::relation(Relation::new(
            ontology_graph::RelationId(i),
            "works_on",
            ConceptId(i),
            ConceptId(1000 + i),
        )));
    }
    store.append_batch(&batch).await.unwrap();
}

async fn hydrate(store: &SegmentStore) -> Arc<OntologyGraph> {
    let g = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&g).await.unwrap();
    g
}

fn files(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            if e.file_type().unwrap().is_dir() {
                walk(root, &e.path(), out);
            } else {
                out.push(
                    e.path()
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    walk(dir, dir, &mut out);
    out.sort();
    out
}

#[tokio::test]
async fn a_backup_restores_to_the_same_graph_and_carries_no_lock_or_xref() {
    let root = tempdir("roundtrip");
    let store = open(&root.join("store")).await;
    store
        .append(&LogRecord::ontology(ontology()))
        .await
        .unwrap();
    fill(&store, 1, 30).await;
    let expected = hydrate(&store).await;

    let dest = root.join("backup");
    let report = store.backup(&dest).await.unwrap();
    assert_eq!(report.records, store.record_count());
    assert_eq!(
        report.copied, report.files,
        "first backup copies everything"
    );
    assert!(report.bytes > 0);
    let copied = files(&dest);
    assert!(copied.contains(&"MANIFEST.json".to_string()));
    assert!(copied.iter().any(|f| f.starts_with("graph/sites/")));
    assert!(
        copied
            .iter()
            .all(|f| !f.ends_with(".xref") && f != "LOCK" && !f.ends_with(".tmp")),
        "{copied:?}"
    );

    let restored_dir = root.join("restored");
    let r = restore(&dest, &restored_dir).await.unwrap();
    assert_eq!(r.records, report.records);
    assert_eq!((r.concepts, r.relations), (58, 29));
    assert!(!root.join("restored.restoring").exists());
    let restored = open(&restored_dir).await;
    compare_graphs(&expected, &*hydrate(&restored).await).unwrap();
    // The restored store is a working store: it takes writes and reopens.
    fill(&restored, 30, 32).await;
    drop(restored);
    let again = open(&restored_dir).await;
    assert_eq!(hydrate(&again).await.concept_count(), 62);
}

#[tokio::test]
async fn a_repeated_backup_copies_only_what_changed_and_drops_compacted_partitions() {
    let root = tempdir("incremental");
    let store = open(&root.join("store")).await;
    store
        .append(&LogRecord::ontology(ontology()))
        .await
        .unwrap();
    fill(&store, 1, 20).await;
    let dest = root.join("backup");
    let first = store.backup(&dest).await.unwrap();
    assert!(
        first.files > 6,
        "several sealed partitions expected: {first:?}"
    );

    // Nothing changed: only the manifest and the active partitions move.
    let second = store.backup(&dest).await.unwrap();
    assert_eq!(second.files, first.files);
    assert!(second.copied < first.copied, "{second:?} vs {first:?}");

    // A compaction rewrites the store into fresh partitions: the old ones
    // must not survive at the destination (discovery is by directory).
    let before = files(&dest);
    let graph = hydrate(&store).await;
    store.compact_store(&graph).await.unwrap();
    let third = store.backup(&dest).await.unwrap();
    let after = files(&dest);
    assert_ne!(before, after);
    let stale: Vec<&String> = before
        .iter()
        .filter(|f| *f != "MANIFEST.json" && after.contains(f))
        .collect();
    assert!(
        stale.is_empty(),
        "old partitions left in the backup: {stale:?}"
    );
    assert_eq!(third.records, store.record_count());
    let restored_dir = root.join("restored");
    restore(&dest, &restored_dir).await.unwrap();
    let restored = open(&restored_dir).await;
    compare_graphs(&graph, &*hydrate(&restored).await).unwrap();
}

#[tokio::test]
async fn refusals_destination_inside_the_store_existing_target_and_not_a_backup() {
    let root = tempdir("refusals");
    let store_dir = root.join("store");
    let store = open(&store_dir).await;
    store
        .append(&LogRecord::ontology(ontology()))
        .await
        .unwrap();
    let e = store.backup(&store_dir.join("copy")).await.unwrap_err();
    assert!(e.to_string().contains("outside the store"), "{e}");
    let e = store.backup(&root).await.unwrap_err();
    assert!(e.to_string().contains("outside the store"), "{e}");

    let dest = root.join("backup");
    store.backup(&dest).await.unwrap();
    let e = restore(&dest, &store_dir).await.unwrap_err();
    assert!(e.to_string().contains("already exists"), "{e}");
    let e = restore(&root.join("nowhere"), &root.join("x"))
        .await
        .unwrap_err();
    assert!(e.to_string().contains("not a backup"), "{e}");
    assert!(!root.join("x").exists());

    // An interrupted restore left a staging directory: discarded, redone.
    let target = root.join("fresh");
    std::fs::create_dir_all(root.join("fresh.restoring")).unwrap();
    std::fs::write(root.join("fresh.restoring").join("junk"), b"x").unwrap();
    restore(&dest, &target).await.unwrap();
    assert!(!root.join("fresh.restoring").exists());
    assert!(!target.join("junk").exists());

    // A restored copy is another history: it cannot reuse its origin's
    // backup directory (same partition ids, different contents).
    let copy = open(&target).await;
    let e = copy.backup(&dest).await.unwrap_err();
    assert!(e.to_string().contains("another store"), "{e}");
    copy.backup(&root.join("backup-of-copy")).await.unwrap();

    // A backup interrupted before its manifest is not a backup.
    std::fs::remove_file(dest.join("MANIFEST.json")).unwrap();
    let e = restore(&dest, &root.join("y")).await.unwrap_err();
    assert!(e.to_string().contains("not a backup"), "{e}");
    store.backup(&dest).await.unwrap();
    assert!(dest.join("MANIFEST.json").is_file());

    // A store without files has nothing to back up.
    let e = MemoryStore::new().backup(&dest).await.unwrap_err();
    assert!(e.to_string().contains("no files"), "{e}");
}
