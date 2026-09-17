// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! `SegmentStore` contract (STORAGE-PLAN.md phase 2): records route to the
//! right stream, a batch costs one sync per touched stream, segments roll
//! and seal, the store survives a restart, a crash in either active
//! segment or a lost manifest, refuses a second writer, names the partition
//! of a corrupt payload, and a legacy `graph.log` migrates verifiably.

use ontology_graph::{
    Concept, ConceptId, ConceptType, Ontology, OntologyGraph, Relation, RelationType,
};
use ontology_storage::manifest::{DEFAULT_NS_ID, META_NS_ID};
use ontology_storage::segment::{data_path, idx_path, IdxEntry, Kind, RECORD_HEADER_LEN};
use ontology_storage::{
    legacy_present, migrate_legacy, FileStore, LogRecord, Manifest, RollPolicy, SegmentStore,
    SegmentStoreConfig, Store, StoreError,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn tempdir(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "ontology-segstore-{tag}-{}-{}",
        std::process::id(),
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
    o.add_relation_type(RelationType {
        name: "knows".into(),
        domain: "Person".into(),
        range: "Person".into(),
        ..Default::default()
    })
    .unwrap();
    o.add_relation_type(RelationType {
        name: "likes".into(),
        domain: "Person".into(),
        range: "Person".into(),
        symmetric: true,
        ..Default::default()
    })
    .unwrap();
    o
}

fn small_roll() -> SegmentStoreConfig {
    SegmentStoreConfig {
        roll: RollPolicy {
            max_bytes: u64::MAX,
            max_records: 10,
        },
        ..Default::default()
    }
}

/// Drive a graph + store through the write-ahead sequence, like the server.
async fn add_concept(graph: &Arc<OntologyGraph>, store: &dyn Store, name: &str) -> ConceptId {
    let mut c =
        Concept::new(ConceptId(0), "Person", name).with_description(format!("about {name}"));
    graph.prepare_concept(&mut c).unwrap();
    store.append(&LogRecord::concept(c.clone())).await.unwrap();
    graph.apply_prepared_concept(c).unwrap()
}
async fn add_relation(
    graph: &Arc<OntologyGraph>,
    store: &dyn Store,
    rt: &str,
    a: ConceptId,
    b: ConceptId,
) {
    let mut r = Relation::new(Default::default(), rt, a, b);
    graph.prepare_relation(&mut r).unwrap();
    store.append(&LogRecord::relation(r.clone())).await.unwrap();
    graph.apply_prepared_relation(r).unwrap();
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
async fn records_route_by_kind_and_a_batch_syncs_once_per_touched_stream() {
    let root = tempdir("route");
    let store = SegmentStore::open(&root).await.unwrap();
    assert!(store.open_report().created);
    assert_eq!(store.next_seq(), 1);
    assert!(root.join("MANIFEST.json").exists());
    assert!(root.join("LOCK").exists());
    assert_eq!(partitions(&root.join("meta")), vec![1]);
    assert_eq!(partitions(&root.join("graph/default")), vec![2]);

    // Ontology alone: meta only → one sync.
    store
        .append(&LogRecord::ontology(ontology()))
        .await
        .unwrap();
    assert_eq!(store.sync_count(), 1);
    // Two concepts in one batch: graph only → one sync.
    store
        .append_batch(&[
            LogRecord::concept(Concept::new(ConceptId(1), "Person", "a")),
            LogRecord::concept(Concept::new(ConceptId(2), "Person", "b")),
        ])
        .await
        .unwrap();
    assert_eq!(store.sync_count(), 2);
    // Mixed batch: both streams → two syncs, seqs still global and ordered.
    store
        .append_batch(&[
            LogRecord::concept(Concept::new(ConceptId(3), "Person", "c")),
            LogRecord::ontology(ontology()),
            LogRecord::relation(Relation::new(
                Default::default(),
                "knows",
                ConceptId(1),
                ConceptId(2),
            )),
        ])
        .await
        .unwrap();
    assert_eq!(store.sync_count(), 4);
    assert_eq!(store.next_seq(), 7);
    assert_eq!(store.record_count(), 6);

    // The relation type got a frozen symbol, persisted in the manifest.
    let m = store.manifest();
    assert_eq!(m.relation_type_sym("knows"), Some(1));
    let on_disk = Manifest::load(&root).unwrap().unwrap();
    assert_eq!(on_disk.relation_type_sym("knows"), Some(1));

    // Index entries carry ids, endpoints and the symbol (D2).
    let idx = std::fs::read(idx_path(&root.join("graph/default"), 2)).unwrap();
    let entries: Vec<IdxEntry> = (0..IdxEntry::count_in(idx.len() as u64))
        .map(|i| IdxEntry::decode(&idx, IdxEntry::file_offset(i)).unwrap())
        .collect();
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[0].kind, Kind::Concept);
    assert_eq!(entries[0].entity_id, 1);
    assert_eq!(entries[0].ns_id, DEFAULT_NS_ID);
    let rel = entries[3];
    assert_eq!(rel.kind, Kind::Relation);
    assert_eq!(rel.rtype_sym, 1);
    assert_eq!(rel.endpoints, (1u64 << 32) | 2);
    assert_eq!(rel.seq, 6);
    let midx = std::fs::read(idx_path(&root.join("meta"), 1)).unwrap();
    let me = IdxEntry::decode(&midx, IdxEntry::file_offset(0)).unwrap();
    assert_eq!((me.kind, me.ns_id, me.seq), (Kind::Ontology, META_NS_ID, 1));

    // Replays into a fresh graph.
    let g = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&g).await.unwrap();
    assert_eq!(g.concept_count(), 3);
    assert_eq!(g.relation_count(), 1);
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn segments_roll_and_seal_and_the_store_survives_a_restart() {
    let root = tempdir("roll");
    let graph = OntologyGraph::with_arc(Ontology::new());
    let (a_names, syncs_before);
    {
        let store = SegmentStore::open_with(&root, small_roll()).await.unwrap();
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
        let mut ids = Vec::new();
        for i in 0..27 {
            ids.push(add_concept(&graph, &store, &format!("p{i}")).await);
        }
        for w in ids.windows(2) {
            add_relation(&graph, &store, "knows", w[0], w[1]).await;
        }
        add_relation(&graph, &store, "likes", ids[0], ids[5]).await;
        // 27 concepts + 26 knows + 1 likes = 54 graph records → 5 sealed
        // partitions of 10 plus an active one with 4.
        let m = store.manifest();
        let graph_stream = m.stream(DEFAULT_NS_ID).unwrap();
        assert_eq!(graph_stream.sealed.len(), 5, "{:?}", graph_stream.sealed);
        assert!(graph_stream.sealed.iter().all(|p| p.records == 10));
        assert_eq!(graph_stream.sealed[0].entity_min, ids[0].0);
        assert_eq!(graph_stream.sealed[0].entity_max, ids[9].0);
        assert!(graph_stream.sealed[3].edges > 0);
        assert_eq!(m.stream(META_NS_ID).unwrap().sealed.len(), 0);
        assert_eq!(partitions(&root.join("graph/default")).len(), 6);
        a_names = graph.all_concepts().len();
        syncs_before = store.sync_count();
        assert!(syncs_before >= 55);
    }

    // Restart: everything replays, seq continues, sealed segments are mapped.
    let store = SegmentStore::open_with(&root, small_roll()).await.unwrap();
    let r = store.open_report();
    assert!(!r.created);
    let g = &r.graph[&DEFAULT_NS_ID];
    assert_eq!(g.sealed, 5);
    assert_eq!(g.active_records, 4);
    assert_eq!(g.truncated_bytes, 0);
    assert!(g.resealed.is_empty());
    assert_eq!(r.next_seq, 56);
    let fresh = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&fresh).await.unwrap();
    assert_eq!(fresh.concept_count(), a_names);
    assert_eq!(
        fresh.relation_count(),
        26 + 2,
        "likes is symmetric → inverse"
    );
    assert_eq!(
        fresh.find_by_name("Person", "p26"),
        graph.find_by_name("Person", "p26")
    );

    // Writes resume with the next seq and the id allocators are past the
    // replayed ids.
    let id = add_concept(&fresh, &store, "after").await;
    assert!(id.0 > 27);
    assert_eq!(store.next_seq(), 57);
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn a_torn_active_segment_in_each_stream_and_a_lost_manifest_recover() {
    let root = tempdir("crash");
    let graph = OntologyGraph::with_arc(Ontology::new());
    {
        let store = SegmentStore::open_with(&root, small_roll()).await.unwrap();
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
        for i in 0..13 {
            add_concept(&graph, &store, &format!("p{i}")).await;
        }
        store
            .append(&LogRecord::ontology(ontology()))
            .await
            .unwrap();
    }
    // Crash simulation: append garbage tails to both active segments (the
    // graph stream rolled at 10, so its active is partition 3 with 3
    // records; meta is partition 1 with 2 records), drop the manifest.
    let gdir = root.join("graph/default");
    let mdir = root.join("meta");
    let ids = partitions(&gdir);
    let active = *ids.last().unwrap();
    for p in [data_path(&gdir, active), data_path(&mdir, 1)] {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        // A header with a plausible length but no payload behind it.
        let mut torn = vec![0u8; RECORD_HEADER_LEN];
        torn[16..20].copy_from_slice(&500u32.to_le_bytes());
        torn[24] = 2;
        f.write_all(&torn).unwrap();
        f.write_all(b"partial").unwrap();
    }
    std::fs::remove_file(root.join("MANIFEST.json")).unwrap();

    let store = SegmentStore::open_with(&root, small_roll()).await.unwrap();
    let r = store.open_report();
    let g = &r.graph[&DEFAULT_NS_ID];
    assert!(g.truncated_bytes > 0 && r.meta.truncated_bytes > 0);
    assert_eq!(g.sealed, 1);
    assert_eq!(g.active_records, 3);
    assert_eq!(r.meta.active_records, 2);
    assert!(r.manifest_rewritten);
    // Symbols were rebuilt from the payloads (none here — no relation);
    // sealed zone maps recomputed from the directory.
    let m = store.manifest();
    assert_eq!(m.stream(DEFAULT_NS_ID).unwrap().sealed.len(), 1);
    assert_eq!(m.stream(DEFAULT_NS_ID).unwrap().sealed[0].records, 10);
    assert_eq!(m.next_partition_id, active + 1);
    let fresh = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&fresh).await.unwrap();
    assert_eq!(fresh.concept_count(), 13);
    assert_eq!(store.next_seq(), 16);
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn a_seal_interrupted_before_the_manifest_write_is_completed_at_open() {
    let root = tempdir("half-seal");
    {
        let store = SegmentStore::open_with(&root, small_roll()).await.unwrap();
        for i in 0..10 {
            store
                .append(&LogRecord::concept(Concept::new(
                    ConceptId(i + 1),
                    "Person",
                    format!("p{i}"),
                )))
                .await
                .unwrap();
        }
        // Partition 2 just sealed (10 records), partition 3 is the new active.
    }
    let gdir = root.join("graph/default");
    assert_eq!(partitions(&gdir), vec![2, 3]);
    // Undo the seal's header stamp: count back to 0 as if the seal had not
    // finished — SealedSegment::open must refuse it and the stream must
    // recover + reseal.
    let ipath = idx_path(&gdir, 2);
    let mut ib = std::fs::read(&ipath).unwrap();
    ib[20..24].copy_from_slice(&0u32.to_le_bytes());
    std::fs::write(&ipath, &ib).unwrap();
    // And pretend the manifest never learned about it.
    let mut m = Manifest::load(&root).unwrap().unwrap();
    m.stream_mut(DEFAULT_NS_ID).unwrap().sealed.clear();
    m.save(&root).unwrap();

    let store = SegmentStore::open_with(&root, small_roll()).await.unwrap();
    assert_eq!(store.open_report().graph[&DEFAULT_NS_ID].resealed, vec![2]);
    assert_eq!(
        store.manifest().stream(DEFAULT_NS_ID).unwrap().sealed.len(),
        1
    );
    let fresh = OntologyGraph::with_arc(ontology());
    store.load_into(&fresh).await.unwrap();
    assert_eq!(fresh.concept_count(), 10);
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn a_second_writer_is_refused_while_the_first_holds_the_lock() {
    let root = tempdir("lock");
    let first = SegmentStore::open(&root).await.unwrap();
    let err = SegmentStore::open(&root)
        .await
        .expect_err("second open must fail");
    assert!(matches!(err, StoreError::Locked(_)), "{err}");
    drop(first);
    // Released on drop.
    SegmentStore::open(&root).await.unwrap();
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn a_corrupt_payload_in_a_sealed_partition_is_named_on_hydration() {
    let root = tempdir("crc");
    {
        let store = SegmentStore::open_with(&root, small_roll()).await.unwrap();
        for i in 0..12 {
            store
                .append(&LogRecord::concept(Concept::new(
                    ConceptId(i + 1),
                    "Person",
                    format!("p{i}"),
                )))
                .await
                .unwrap();
        }
    }
    let gdir = root.join("graph/default");
    let dpath = data_path(&gdir, 2); // sealed
    let mut bytes = std::fs::read(&dpath).unwrap();
    // Flip a byte inside the 4th record's payload.
    let idx = std::fs::read(idx_path(&gdir, 2)).unwrap();
    let e = IdxEntry::decode(&idx, IdxEntry::file_offset(3)).unwrap();
    bytes[e.offset as usize + RECORD_HEADER_LEN + 5] ^= 0x40;
    std::fs::write(&dpath, &bytes).unwrap();

    let store = SegmentStore::open_with(&root, small_roll()).await.unwrap();
    let fresh = OntologyGraph::with_arc(ontology());
    let err = store
        .load_into(&fresh)
        .await
        .expect_err("crc must fail hydration");
    let msg = err.to_string();
    assert!(msg.contains("partition 2"), "{msg}");
    assert!(msg.contains("crc mismatch"), "{msg}");
    assert_eq!(
        fresh.concept_count(),
        3,
        "records before the bad one were applied"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn a_legacy_log_and_snapshot_migrate_verifiably() {
    let data = tempdir("migrate");
    // Build a legacy store the old way: snapshot after 3 concepts, then 2
    // more concepts and a relation in the WAL, then a deletion.
    let graph = OntologyGraph::with_arc(Ontology::new());
    let (deleted, kept): (ConceptId, ConceptId);
    {
        let legacy = FileStore::open(&data).await.unwrap();
        legacy
            .append(&LogRecord::ontology(ontology()))
            .await
            .unwrap();
        graph
            .extend_ontology(|o| {
                *o = ontology();
                Ok(())
            })
            .unwrap();
        let mut ids = Vec::new();
        for i in 0..3 {
            ids.push(add_concept(&graph, &legacy, &format!("old{i}")).await);
        }
        add_relation(&graph, &legacy, "likes", ids[0], ids[1]).await;
        legacy.snapshot(&graph).await.unwrap();
        for i in 3..5 {
            ids.push(add_concept(&graph, &legacy, &format!("old{i}")).await);
        }
        add_relation(&graph, &legacy, "knows", ids[3], ids[4]).await;
        let cascade = graph.incident_relation_ids(ids[2]).unwrap();
        let mut recs = vec![LogRecord::delete_concept(ids[2], "Person")];
        recs.extend(cascade.into_iter().map(|rid| {
            LogRecord::delete_relation(rid, graph.get_relation(rid).unwrap().relation_type)
        }));
        legacy.append_batch(&recs).await.unwrap();
        graph.remove_concept(ids[2]).unwrap();
        deleted = ids[2];
        kept = ids[4];
    }
    assert!(legacy_present(&data));
    let store_dir = data.join("store");

    let report = migrate_legacy(&data, &store_dir).await.unwrap();
    assert_eq!(
        report.ontology, 1,
        "the snapshot's ontology; the WAL one is under the watermark"
    );
    assert_eq!(report.concepts, 3 + 2);
    assert_eq!(
        report.relations,
        1 + 1,
        "snapshot dedupes the symmetric inverse"
    );
    assert_eq!(report.deletes_and_updates, 1);
    assert_eq!(report.renamed.len(), 2);
    assert!(!legacy_present(&data));
    assert!(data.join("graph.log.migrated").exists());
    assert!(data.join("graph.snap.migrated").exists());
    assert!(report.store_bytes > 0);

    let store = SegmentStore::open(&store_dir).await.unwrap();
    let fresh = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&fresh).await.unwrap();
    assert_eq!(fresh.concept_count(), 4);
    assert!(fresh.get_concept(deleted).is_err());
    assert_eq!(fresh.get_concept(kept).unwrap().name, "old4");
    assert_eq!(fresh.relation_count(), graph.relation_count());

    // Migrating again is refused: nothing legacy left, store not empty.
    let err = migrate_legacy(&data, &store_dir).await.unwrap_err();
    assert!(err.to_string().contains("nothing to migrate"), "{err}");
    std::fs::remove_dir_all(&data).ok();
}

#[tokio::test]
async fn migration_refuses_an_existing_store_and_redoes_an_interrupted_one() {
    let data = tempdir("migrate-nonempty");
    let legacy_bytes;
    {
        let legacy = FileStore::open(&data).await.unwrap();
        legacy
            .append(&LogRecord::ontology(ontology()))
            .await
            .unwrap();
        legacy
            .append(&LogRecord::concept(Concept::new(
                ConceptId(1),
                "Person",
                "a",
            )))
            .await
            .unwrap();
        legacy_bytes = std::fs::read(data.join("graph.log")).unwrap();
    }
    let store_dir = data.join("store");
    // An existing store, whatever it holds: refused, nothing touched.
    {
        SegmentStore::open(&store_dir).await.unwrap();
    }
    let err = migrate_legacy(&data, &store_dir).await.unwrap_err();
    assert!(err.to_string().contains("already exists"), "{err}");
    assert!(legacy_present(&data), "legacy files untouched");
    assert_eq!(std::fs::read(data.join("graph.log")).unwrap(), legacy_bytes);
    std::fs::remove_dir_all(&store_dir).unwrap();

    // A leftover staging directory from an interrupted run is discarded
    // and the migration redone from the legacy files.
    let staging = ontology_storage::staging_dir_for(&store_dir);
    std::fs::create_dir_all(staging.join("meta")).unwrap();
    std::fs::write(staging.join("MANIFEST.json"), b"{ garbage").unwrap();
    let report = migrate_legacy(&data, &store_dir).await.unwrap();
    assert_eq!(report.concepts, 1);
    assert!(!staging.exists(), "staging renamed away");
    assert!(store_dir.join("MANIFEST.json").exists());
    assert!(!legacy_present(&data));
    // The legacy log was read, never rewritten.
    assert_eq!(
        std::fs::read(data.join("graph.log.migrated")).unwrap(),
        legacy_bytes
    );
    let store = SegmentStore::open(&store_dir).await.unwrap();
    let g = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&g).await.unwrap();
    assert_eq!(g.concept_count(), 1);
    std::fs::remove_dir_all(&data).ok();
}

#[tokio::test]
async fn a_poisoned_segment_store_refuses_appends_until_reopened() {
    let root = tempdir("poison");
    let store = SegmentStore::open(&root).await.unwrap();
    store
        .append(&LogRecord::ontology(ontology()))
        .await
        .unwrap();
    store.poison_for_test();
    assert!(store.is_poisoned());
    let err = store
        .append(&LogRecord::concept(Concept::new(
            ConceptId(1),
            "Person",
            "a",
        )))
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::Poisoned(_)), "{err}");
    assert_eq!(store.record_count(), 1, "nothing written while poisoned");
    drop(store);
    let store = SegmentStore::open(&root).await.unwrap();
    assert!(!store.is_poisoned());
    store
        .append(&LogRecord::concept(Concept::new(
            ConceptId(1),
            "Person",
            "a",
        )))
        .await
        .unwrap();
    assert_eq!(store.record_count(), 2);
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn an_active_segment_whose_header_never_landed_is_recreated() {
    let root = tempdir("empty-active");
    {
        let store = SegmentStore::open_with(&root, small_roll()).await.unwrap();
        for i in 0..10 {
            store
                .append(&LogRecord::concept(Concept::new(
                    ConceptId(i + 1),
                    "Person",
                    format!("p{i}"),
                )))
                .await
                .unwrap();
        }
        // Partition 2 sealed, partition 3 freshly created and empty.
    }
    let gdir = root.join("graph/default");
    assert_eq!(partitions(&gdir), vec![2, 3]);
    // Crash between create_new and the header sync: a zero-length file.
    std::fs::write(data_path(&gdir, 3), b"").unwrap();
    let store = SegmentStore::open_with(&root, small_roll()).await.unwrap();
    assert_eq!(partitions(&gdir), vec![2, 3]);
    let fresh = OntologyGraph::with_arc(ontology());
    store.load_into(&fresh).await.unwrap();
    assert_eq!(fresh.concept_count(), 10);
    store
        .append(&LogRecord::concept(Concept::new(
            ConceptId(11),
            "Person",
            "p10",
        )))
        .await
        .unwrap();
    assert_eq!(store.record_count(), 11);
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn an_empty_store_reopens_and_replays_nothing() {
    let root = tempdir("empty");
    {
        SegmentStore::open(&root).await.unwrap();
    }
    let store = SegmentStore::open(&root).await.unwrap();
    assert!(!store.open_report().created);
    assert_eq!(store.next_seq(), 1);
    let g = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&g).await.unwrap();
    assert_eq!(g.concept_count(), 0);
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn meta_and_graph_records_replay_in_global_seq_order() {
    // Regression: a rule (meta stream) referencing concepts (graph stream)
    // written just before it, and an ontology change (meta) followed by a
    // concept of the new type (graph). Stream-by-stream replay fails on
    // both; the merge by seq (H12) must succeed.
    let root = tempdir("interleave");
    let graph = OntologyGraph::with_arc(Ontology::new());
    {
        let store = SegmentStore::open(&root).await.unwrap();
        let mut onto = ontology();
        onto.add_rule_type(ontology_graph::RuleType {
            name: "must_review".into(),
            when: String::new(),
            then: String::new(),
            applies_to: vec!["Person".into()],
            strict: false,
            description: String::new(),
        })
        .unwrap();
        store
            .append(&LogRecord::ontology(onto.clone()))
            .await
            .unwrap();
        graph
            .extend_ontology(|o| {
                *o = onto.clone();
                Ok(())
            })
            .unwrap();
        let a = add_concept(&graph, &store, "a").await;
        let b = add_concept(&graph, &store, "b").await;
        let mut rule = ontology_graph::Rule::new(Default::default(), "must_review", "r1");
        rule.applies_to = vec![a, b];
        graph.prepare_rule(&mut rule).unwrap();
        store.append(&LogRecord::rule(rule.clone())).await.unwrap();
        graph.apply_prepared_rule(rule).unwrap();
        // Schema grows after instances exist; a concept of the new type follows.
        onto.add_concept_type(ConceptType {
            name: "City".into(),
            ..Default::default()
        });
        store
            .append(&LogRecord::ontology(onto.clone()))
            .await
            .unwrap();
        graph
            .extend_ontology(|o| {
                *o = onto.clone();
                Ok(())
            })
            .unwrap();
        let mut c = Concept::new(ConceptId(0), "City", "Geneva");
        graph.prepare_concept(&mut c).unwrap();
        store.append(&LogRecord::concept(c.clone())).await.unwrap();
        graph.apply_prepared_concept(c).unwrap();
    }
    let store = SegmentStore::open(&root).await.unwrap();
    let fresh = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&fresh).await.unwrap();
    assert_eq!(fresh.concept_count(), 3);
    assert_eq!(fresh.rule_count(), 1);
    assert_eq!(fresh.ontology().concept_types.len(), 2);
    assert!(fresh.find_by_name("City", "Geneva").is_some());
    std::fs::remove_dir_all(&root).ok();
}

/// `reset` is a compaction from an empty graph: durable, crash-safe, and the
/// store keeps accepting data afterwards (fresh partitions, seq continues).
#[tokio::test]
async fn reset_empties_the_store_durably_and_accepts_new_data() {
    let dir = tempdir("reset");
    {
        let graph = OntologyGraph::with_arc(ontology());
        let store = SegmentStore::open(&dir).await.unwrap();
        store
            .append(&LogRecord::ontology(ontology()))
            .await
            .unwrap();
        let a = add_concept(&graph, &store, "a").await;
        let b = add_concept(&graph, &store, "b").await;
        add_relation(&graph, &store, "knows", a, b).await;
        store.reset().await.unwrap();

        let fresh = OntologyGraph::with_arc(Ontology::new());
        store.load_into(&fresh).await.unwrap();
        assert_eq!(fresh.concept_count(), 0);
        assert_eq!(fresh.relation_count(), 0);
        assert!(fresh.with_ontology(|o| o.concept_types.is_empty()));
    }
    // Reopen: nothing of the old content survives on disk either.
    let fresh = OntologyGraph::with_arc(Ontology::new());
    let store = SegmentStore::open(&dir).await.unwrap();
    store.load_into(&fresh).await.unwrap();
    assert_eq!(fresh.concept_count(), 0);
    assert!(fresh.with_ontology(|o| o.concept_types.is_empty()));

    // The store still works: schema again, a concept, reopen, still there.
    store
        .append(&LogRecord::ontology(ontology()))
        .await
        .unwrap();
    fresh
        .extend_ontology(|o| {
            *o = ontology();
            Ok(())
        })
        .unwrap();
    add_concept(&fresh, &store, "c").await;
    drop(store);
    let again = OntologyGraph::with_arc(Ontology::new());
    let store = SegmentStore::open(&dir).await.unwrap();
    store.load_into(&again).await.unwrap();
    assert_eq!(again.concept_count(), 1);
    assert!(again.with_ontology(|o| o.concept_types.contains_key("Person")));
}

/// Hydration runs in bulk mode (derived indexes rebuilt once at the end):
/// after `load_into`, the sorted listing, per-type listing, trigram search
/// and relation listing must equal those of the graph that wrote the store
/// — including renames and deletes replayed from the journal.
#[tokio::test]
async fn hydration_rebuilds_the_derived_indexes_like_the_live_graph() {
    let dir = tempdir("derived");
    let live = OntologyGraph::with_arc(ontology());
    let store = SegmentStore::open(&dir).await.unwrap();
    store
        .append(&LogRecord::ontology(ontology()))
        .await
        .unwrap();
    let mut ids = Vec::new();
    for name in ["alice", "bob", "carol", "dave", "erin", "frank"] {
        ids.push(add_concept(&live, &store, name).await);
    }
    add_relation(&live, &store, "knows", ids[0], ids[1]).await;
    add_relation(&live, &store, "likes", ids[2], ids[3]).await;
    // A rename and a delete go through the journal too.
    let renamed = live
        .update_concept(
            ids[4],
            ontology_graph::ConceptPatch {
                name: Some("zed".into()),
                ..Default::default()
            },
        )
        .unwrap();
    store
        .append(&LogRecord::update_concept(renamed))
        .await
        .unwrap();
    let cascade = live.incident_relation_ids(ids[3]).unwrap();
    store
        .append(&LogRecord::delete_concept(ids[3], "Person".to_string()))
        .await
        .unwrap();
    for rid in &cascade {
        store
            .append(&LogRecord::delete_relation(*rid, "likes".to_string()))
            .await
            .unwrap();
    }
    live.remove_concept(ids[3]).unwrap();
    drop(store);

    let loaded = OntologyGraph::with_arc(Ontology::new());
    let store = SegmentStore::open(&dir).await.unwrap();
    store.load_into(&loaded).await.unwrap();
    assert!(!loaded.is_bulk(), "the guard ended bulk mode");

    let view = |g: &OntologyGraph| {
        let (total, page) = g.list_concepts_page(None, None, 0, 100, true, true);
        let (_, by_type) = g.list_concepts_page(Some("Person"), None, 0, 100, true, false);
        let (_, hits) = g.list_concepts_page(None, Some("zed"), 0, 100, true, true);
        let (_, nohit) = g.list_concepts_page(None, Some("dav"), 0, 100, true, true);
        let (_, rels) = g.list_relations_page(None, None, None, 0, 100, true);
        (
            total,
            page.iter()
                .map(|c| (c.id, c.name.clone()))
                .collect::<Vec<_>>(),
            by_type.iter().map(|c| c.id).collect::<Vec<_>>(),
            hits.iter().map(|c| c.id).collect::<Vec<_>>(),
            nohit.len(),
            rels.iter()
                .map(|r| (r.id, r.source, r.target, r.relation_type.clone()))
                .collect::<Vec<_>>(),
            g.concepts_generation() > 0 && g.relations_generation() > 0,
        )
    };
    assert_eq!(view(&live), view(&loaded));
    assert_eq!(view(&loaded).4, 0, "the deleted `dave` is not searchable");
    assert_eq!(view(&loaded).3.len(), 1, "the renamed `zed` is searchable");
}
