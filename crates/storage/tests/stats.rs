// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! T3 (STORAGE-PLAN.md §8): per-stream figures for `/metrics` — segments,
//! bytes, records, last seq and syncs per domain, the domain's tier, and
//! the last compaction since open.

use ontology_graph::{Concept, ConceptId, ConceptType, Ontology, OntologyGraph};
use ontology_storage::{LogRecord, RollPolicy, SegmentStore, SegmentStoreConfig, Store, Tier};
use std::collections::HashMap;
use std::path::PathBuf;

fn tempdir(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "ontology-stats-{tag}-{}-{}-{}",
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

fn ct(name: &str, ns: &str) -> ConceptType {
    ConceptType {
        name: name.into(),
        ns: Some(ns.into()),
        ..Default::default()
    }
}

/// Two domains: `alpha` (type A) and `beta` (type B).
fn ontology() -> Ontology {
    let mut o = Ontology::new();
    o.add_concept_type(ct("A", "alpha"));
    o.add_concept_type(ct("B", "beta"));
    o
}

fn concept(id: u64, ty: &str) -> LogRecord {
    LogRecord::concept(Concept::new(ConceptId(id), ty, format!("{ty}-{id}")))
}

#[tokio::test]
async fn per_stream_stats_count_segments_records_syncs_tier_and_compaction() {
    let dir = tempdir("streams");
    let cfg = SegmentStoreConfig {
        roll: RollPolicy {
            max_bytes: u64::MAX,
            max_records: 2,
        },
        ..Default::default()
    };
    let store = SegmentStore::open_with(&dir, cfg).await.unwrap();
    // seq 1: the schema, on the meta stream (one sync).
    store
        .append(&LogRecord::ontology(ontology()))
        .await
        .unwrap();
    // seq 2-3: one batch touching both domains costs one sync per stream.
    store
        .append_batch(&[concept(1, "A"), concept(2, "B")])
        .await
        .unwrap();
    // seq 4-7: alpha alone; with a roll every 2 records, two sealed
    // partitions and one record in the active one.
    for i in 3..=6 {
        store.append(&concept(i, "A")).await.unwrap();
    }

    let st = store.stats();
    let by = |ns: &str| {
        st.streams
            .iter()
            .find(|s| s.ns == ns)
            .cloned()
            .unwrap_or_else(|| panic!("no stream {ns} in {st:?}"))
    };
    let meta = by("meta");
    assert_eq!(meta.tier, "meta");
    assert_eq!(meta.records, 1);
    assert_eq!(meta.syncs, 1);
    assert_eq!(meta.last_seq, Some(1));

    let alpha = by("alpha");
    assert_eq!(alpha.tier, "p0");
    assert_eq!(alpha.records, 5);
    assert_eq!(alpha.syncs, 5, "one per batch that touched alpha");
    assert_eq!(alpha.sealed_segments, 2);
    assert_eq!(alpha.last_seq, Some(7));
    assert!(alpha.data_bytes > 0);

    let beta = by("beta");
    assert_eq!(beta.records, 1);
    assert_eq!(beta.syncs, 1);
    assert_eq!(beta.sealed_segments, 0);
    assert_eq!(beta.last_seq, Some(3));

    assert_eq!(st.next_seq, 8);
    assert_eq!(
        st.syncs,
        1 + 2 + 4,
        "meta + both domains + four alpha appends"
    );
    assert!(st.last_compaction.is_none(), "no compaction since open");
    // meta, alpha, beta, and the default graph domain every store opens with.
    assert_eq!(st.streams.len(), 4, "{st:?}");
    assert_eq!(
        st.streams.iter().filter(|s| s.records == 0).count(),
        1,
        "the unused default domain"
    );

    // The tier label follows the tiers set on the store.
    let tiers: HashMap<u16, Tier> = [(alpha.ns_id, Tier::P1)].into_iter().collect();
    store.set_tiers(tiers);
    let again = store.stats();
    assert_eq!(
        again.streams.iter().find(|s| s.ns == "alpha").unwrap().tier,
        "p1"
    );
    assert_eq!(
        again.streams.iter().find(|s| s.ns == "beta").unwrap().tier,
        "p0"
    );

    // A compaction is remembered with its duration, end time and report.
    let graph = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&graph).await.unwrap();
    let report = store.compact_store(&graph).await.unwrap();
    let st = store.stats();
    let c = st.last_compaction.expect("compaction recorded");
    assert_eq!(c.report, report);
    assert!(c.duration_secs >= 0.0);
    assert!(
        c.finished_unix_secs > 1_700_000_000,
        "{}",
        c.finished_unix_secs
    );

    // Through the trait, the segment store answers; stores without streams
    // answer `None` (checked on the server side with `MemoryStore`).
    let dyn_store: &dyn Store = &store;
    assert!(dyn_store.store_stats().is_some());
}
