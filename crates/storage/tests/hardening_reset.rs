// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! `Store::reset` on `SegmentStore`. There is no `Clear` record (D3): a
//! reset is a compaction from an empty graph, so it must be durable,
//! idempotent, keep `seq` monotonic (H12), leave the manifest without a
//! compaction marker and the streams without leftovers, and obey the same
//! poison rule as every other write.

mod hardening_common;

use hardening_common::*;
use ontology_graph::{Concept, ConceptId, Ontology, OntologyGraph};
use ontology_storage::{LogRecord, SegmentStore, Store, StoreError};

/// Resetting a store that never held anything succeeds and the store keeps
/// working afterwards; the only record left is the empty schema.
#[tokio::test]
async fn reset_on_an_empty_store_succeeds_and_the_store_stays_usable() {
    let root = tempdir("reset-empty");
    let store = SegmentStore::open(&root).await.unwrap();
    assert_eq!(store.next_seq(), 1);
    store.reset().await.unwrap();
    assert_eq!(store.next_seq(), 2, "one record: the empty ontology");
    assert_eq!(store.record_count(), 1);
    assert!(store.manifest().compaction.is_none());

    let g = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&g).await.unwrap();
    assert_eq!(g.concept_count(), 0);
    assert!(g.with_ontology(|o| o.concept_types.is_empty()));

    let fx = Fixture::new(&store).await;
    fx.concept(&store, "Company", "Acme").await;
    assert_eq!(store.record_count(), 3);
    assert_eq!(store.next_seq(), 4);
    std::fs::remove_dir_all(&root).ok();
}

/// H12: `seq` never goes backwards, not even across resets; a second reset
/// replaces the first one's output rather than piling on it.
#[tokio::test]
async fn reset_twice_in_a_row_is_idempotent_and_seq_keeps_growing() {
    let root = tempdir("reset-twice");
    let store = SegmentStore::open_with(&root, roll(3)).await.unwrap();
    populate(&store).await;
    let seq_before = store.next_seq();
    assert!(store.record_count() > 10);

    store.reset().await.unwrap();
    assert_eq!(store.next_seq(), seq_before + 1);
    assert_eq!(store.record_count(), 1);

    store.reset().await.unwrap();
    assert_eq!(store.next_seq(), seq_before + 2);
    assert_eq!(
        store.record_count(),
        1,
        "the previous reset's partition was removed, not kept"
    );

    let g = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&g).await.unwrap();
    assert_eq!(g.concept_count(), 0);
    assert_eq!(g.relation_count(), 0);
    assert_eq!(g.rule_count(), 0);
    std::fs::remove_dir_all(&root).ok();
}

/// Selective hydration after a reset sees the same emptiness as a full one:
/// the schema record is applied, nothing is skipped.
#[tokio::test]
async fn reset_then_load_domains_yields_an_empty_graph() {
    let root = tempdir("reset-domains");
    let store = SegmentStore::open(&root).await.unwrap();
    populate(&store).await;
    store.reset().await.unwrap();

    let g = OntologyGraph::with_arc(Ontology::new());
    let report = store
        .load_domains_report(&g, &["parties".to_string(), "contrats".to_string()])
        .await
        .unwrap();
    assert_eq!(report.applied, 1, "the empty ontology only: {report:?}");
    assert_eq!(report.skipped_cross_domain, 0);
    assert_eq!(g.concept_count(), 0);
    assert!(g.with_ontology(|o| o.concept_types.is_empty()));
    std::fs::remove_dir_all(&root).ok();
}

/// STORAGE.md §5 protocol, step 4: after the swap the marker is cleared,
/// the staging directories are gone, no `.old` / `.tmp` remains, every
/// graph stream is down to one empty active partition and `meta` to one
/// sealed partition (the schema) plus one empty active. Domain ids stay
/// frozen (R10). A reopen has nothing to sweep.
#[tokio::test]
async fn reset_leaves_no_compaction_marker_and_nothing_to_sweep() {
    let root = tempdir("reset-clean");
    let ns_before;
    {
        let store = SegmentStore::open_with(&root, roll(3)).await.unwrap();
        populate(&store).await;
        let parts: usize = STREAM_DIRS
            .iter()
            .map(|d| partitions(&root.join(d)).len())
            .sum();
        assert!(
            parts > 6,
            "several partitions rolled before the reset: {parts}"
        );
        ns_before = store.manifest().ns.clone();

        store.reset().await.unwrap();

        let m = store.manifest();
        assert!(m.compaction.is_none(), "marker cleared");
        assert_eq!(m.ns, ns_before, "domain table untouched (R10)");
        for d in STREAM_DIRS {
            let dir = root.join(d);
            assert!(!dir.join("compacting").exists(), "{d}: staging left");
            assert_eq!(files_with_ext(&dir, "old"), 0, "{d}: .old left");
            assert_eq!(files_with_ext(&dir, "tmp"), 0, "{d}: .tmp left");
            let expect = if d == "meta" { 2 } else { 1 };
            assert_eq!(
                partitions(&dir).len(),
                expect,
                "{d}: {:?}",
                partitions(&dir)
            );
        }
        let counts = store.record_counts_by_domain();
        assert_eq!(counts["meta"], 1);
        for d in ["default", "parties", "contrats"] {
            assert_eq!(counts[d], 0, "{counts:?}");
        }
        assert!(!root.join("MANIFEST.json.tmp").exists());
    }
    let store = SegmentStore::open_with(&root, roll(3)).await.unwrap();
    let r = store.open_report();
    assert_eq!(r.meta.swept_old, 0);
    assert!(r.graph.values().all(|g| g.swept_old == 0), "{r:?}");
    assert_eq!(r.xref_entries, 0);
    assert_eq!(store.manifest().ns, ns_before);
    let g = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&g).await.unwrap();
    assert_eq!(g.concept_count(), 0);
    // A new instance of a known domain reuses the frozen id and stream.
    let fx = Fixture::new(&store).await;
    fx.concept(&store, "Company", "Acme").await;
    assert_eq!(store.record_counts_by_domain()["parties"], 1);
    std::fs::remove_dir_all(&root).ok();
}

/// A poisoned store refuses every write until restart — a reset and a
/// compaction are writes. The data the poison interrupted is still there
/// after the restart.
#[tokio::test]
async fn reset_and_compact_are_refused_on_a_poisoned_store() {
    let root = tempdir("reset-poison");
    let (concepts, records);
    {
        let store = SegmentStore::open(&root).await.unwrap();
        let fx = populate(&store).await;
        concepts = fx.graph.concept_count();
        records = store.record_count();
        store.poison_for_test();

        let err = store.reset().await.unwrap_err();
        assert!(matches!(err, StoreError::Poisoned(_)), "{err}");
        assert!(
            err.to_string().contains(&root.display().to_string()),
            "names the store root: {err}"
        );
        let err = store.compact_store(&fx.graph).await.unwrap_err();
        assert!(matches!(err, StoreError::Poisoned(_)), "{err}");
        let err = store
            .append(&LogRecord::concept(Concept::new(
                ConceptId(0),
                "Company",
                "x",
            )))
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::Poisoned(_)), "{err}");
        assert_eq!(store.record_count(), records, "nothing written");
        assert!(store.manifest().compaction.is_none());
        for d in STREAM_DIRS {
            assert!(!root.join(d).join("compacting").exists(), "{d}");
        }
    }
    let store = SegmentStore::open(&root).await.unwrap();
    assert!(!store.is_poisoned());
    assert_eq!(store.record_count(), records);
    let g = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&g).await.unwrap();
    assert_eq!(g.concept_count(), concepts, "the reset did not happen");
    store.reset().await.unwrap();
    assert_eq!(store.record_count(), 1);
    std::fs::remove_dir_all(&root).ok();
}
