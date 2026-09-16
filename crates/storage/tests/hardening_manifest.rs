// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! `MANIFEST.json` at the store boundary (STORAGE.md §4.5, R10, D2): a
//! foreign format version or an unreadable manifest is an error, never a
//! silently recreated empty store; a leftover `.tmp` from an interrupted
//! atomic write is harmless; domain ids and relation-type symbols are
//! frozen across reopens and keep growing from their maximum.

mod hardening_common;

use hardening_common::*;
use ontology_graph::{Ontology, OntologyGraph};
use ontology_storage::manifest::MANIFEST_FILE;
use ontology_storage::{LogRecord, Manifest, SegmentStore, Store, StoreError};

/// A store written by a build with another layout version is refused with
/// `Format`, and its files are not touched.
#[tokio::test]
async fn a_manifest_with_a_foreign_format_version_is_refused_with_format_error() {
    let root = tempdir("format-version");
    {
        let store = SegmentStore::open(&root).await.unwrap();
        populate(&store).await;
    }
    // 1 (JSON only) and 2 (non-JSON payload codec in use) are this build's
    // versions; anything else comes from another layout.
    let mut m = Manifest::load(&root).unwrap().unwrap();
    m.format_version = 9;
    m.save(&root).unwrap();
    let before = partitions(&root.join("graph/parties"));

    let err = SegmentStore::open(&root).await.unwrap_err();
    assert!(matches!(err, StoreError::Format(_)), "{err}");
    assert!(err.to_string().contains("format version 9"), "{err}");
    assert_eq!(partitions(&root.join("graph/parties")), before);
    assert_eq!(Manifest::load(&root).unwrap().unwrap().format_version, 9);
    std::fs::remove_dir_all(&root).ok();
}

/// An unreadable manifest (bad JSON, or a manifest version this build does
/// not know) fails the open: the symbol tables it holds are authoritative,
/// so the store must not come up as fresh over existing partitions.
#[tokio::test]
async fn a_corrupt_manifest_is_an_error_not_a_fresh_store() {
    let root = tempdir("corrupt-manifest");
    {
        let store = SegmentStore::open(&root).await.unwrap();
        populate(&store).await;
    }
    let path = root.join(MANIFEST_FILE);
    let good = std::fs::read(&path).unwrap();
    let dirs_before: Vec<Vec<u32>> = STREAM_DIRS
        .iter()
        .map(|d| partitions(&root.join(d)))
        .collect();

    std::fs::write(&path, b"{ this is not json").unwrap();
    let err = SegmentStore::open(&root).await.unwrap_err();
    assert!(matches!(err, StoreError::Io(_)), "{err}");
    assert!(err.to_string().contains("MANIFEST"), "{err}");

    let mut v: serde_json::Value = serde_json::from_slice(&good).unwrap();
    v["version"] = serde_json::json!(99);
    std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    let err = SegmentStore::open(&root).await.unwrap_err();
    assert!(err.to_string().contains("version 99"), "{err}");

    let dirs_after: Vec<Vec<u32>> = STREAM_DIRS
        .iter()
        .map(|d| partitions(&root.join(d)))
        .collect();
    assert_eq!(dirs_after, dirs_before, "no partition created or removed");

    // The original manifest back: everything is still there.
    std::fs::write(&path, &good).unwrap();
    let store = SegmentStore::open(&root).await.unwrap();
    let g = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&g).await.unwrap();
    assert_eq!(g.concept_count(), 6);
    std::fs::remove_dir_all(&root).ok();
}

/// A `MANIFEST.json.tmp` left by a crash between write and rename is
/// ignored at open (the real manifest wins) and consumed by the next save.
#[tokio::test]
async fn a_leftover_manifest_tmp_is_ignored_and_consumed_by_the_next_save() {
    let root = tempdir("manifest-tmp");
    {
        let store = SegmentStore::open(&root).await.unwrap();
        store
            .append(&LogRecord::ontology(ontology()))
            .await
            .unwrap();
    }
    let tmp = root.join("MANIFEST.json.tmp");
    std::fs::write(&tmp, b"{ half-written").unwrap();

    let store = SegmentStore::open(&root).await.unwrap();
    assert!(!store.open_report().created, "the real manifest was used");
    assert_eq!(store.manifest().relation_types.len(), 0);
    // A new relation type forces a manifest save (D2): tmp is renamed away.
    let fx = Fixture {
        graph: OntologyGraph::with_arc(ontology()),
    };
    let a = fx.concept(&store, "Company", "a").await;
    let b = fx.concept(&store, "Company", "b").await;
    fx.relation(&store, "partner_of", a, b).await;
    assert!(!tmp.exists(), "tmp consumed by the atomic save");
    let on_disk = Manifest::load(&root).unwrap().unwrap();
    assert_eq!(on_disk.relation_type_sym("partner_of"), Some(1));
    assert_eq!(on_disk, store.manifest());
    std::fs::remove_dir_all(&root).ok();
}

/// R10 / D2 across a restart: the ids a store handed out are the ids it
/// finds again, in-memory and on disk agree byte for byte, and new names
/// continue from the maximum.
#[tokio::test]
async fn domain_ids_and_relation_symbols_are_frozen_across_reopen() {
    let root = tempdir("frozen");
    let (ids, syms, next_partition);
    {
        let store = SegmentStore::open(&root).await.unwrap();
        let fx = Fixture::new(&store).await;
        // contrats first, then parties: ids follow first use, not the schema.
        let c = fx.concept(&store, "Contract", "C-1").await;
        let acme = fx.concept(&store, "Company", "Acme").await;
        let alice = fx.concept(&store, "Person", "Alice").await;
        fx.relation(&store, "between", c, acme).await;
        fx.relation(&store, "employed_by", alice, acme).await;
        let m = store.manifest();
        ids = (
            m.ns_id("default").unwrap(),
            m.ns_id("contrats").unwrap(),
            m.ns_id("parties").unwrap(),
        );
        assert_eq!(ids, (1, 2, 3));
        syms = (
            m.relation_type_sym("between").unwrap(),
            m.relation_type_sym("employed_by").unwrap(),
        );
        assert_eq!(syms, (1, 2));
        next_partition = m.next_partition_id;
        assert_eq!(Manifest::load(&root).unwrap().unwrap(), m);
    }
    let store = SegmentStore::open(&root).await.unwrap();
    let m = store.manifest();
    assert_eq!(
        (
            m.ns_id("default").unwrap(),
            m.ns_id("contrats").unwrap(),
            m.ns_id("parties").unwrap()
        ),
        ids
    );
    assert_eq!(
        (
            m.relation_type_sym("between").unwrap(),
            m.relation_type_sym("employed_by").unwrap()
        ),
        syms
    );
    assert_eq!(
        m.next_partition_id, next_partition,
        "no partition created by a clean reopen"
    );
    assert_eq!(Manifest::load(&root).unwrap().unwrap(), m);

    // A new domain and a new relation type after the restart: max + 1.
    let mut onto = ontology();
    onto.add_concept_type(ct("Asset", Some("actifs"), None));
    onto.add_relation_type(rt("owns", "Company", "Asset", false))
        .unwrap();
    let g = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&g).await.unwrap();
    g.extend_ontology(|o| {
        *o = onto.clone();
        Ok(())
    })
    .unwrap();
    store.append(&LogRecord::ontology(onto)).await.unwrap();
    let fx = Fixture { graph: g };
    let truck = fx.concept(&store, "Asset", "Truck").await;
    let acme = fx.graph.find_by_name("Company", "Acme").unwrap();
    fx.relation(&store, "owns", acme, truck).await;
    let m = store.manifest();
    assert_eq!(m.ns_id("actifs"), Some(4));
    assert_eq!(m.relation_type_sym("owns"), Some(3));
    assert_eq!(m.graph_ns_ids(), vec![1, 2, 3, 4]);
    assert_eq!(Manifest::load(&root).unwrap().unwrap(), m);
    std::fs::remove_dir_all(&root).ok();
}
