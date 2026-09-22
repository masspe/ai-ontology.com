// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Memory socle on a real store (`STORAGE-PLAN.md` §7.1, R14, R17): the
//! per-domain estimate is built from the MANIFEST zone maps plus the active
//! segments, before any data file is read; the plan refuses or narrows the
//! load; the plan is remembered for the API.

use ontology_graph::{
    Concept, ConceptId, ConceptType, Ontology, OntologyGraph, Relation, RelationId, RelationType,
};
use ontology_storage::budget::{
    estimate_bytes, CONCEPT_INDEX_BYTES, REBUILD_PEAK_PER_CONCEPT, RELATION_BYTES,
};
use ontology_storage::{
    BudgetSource, LogRecord, MemoryBudget, MemoryMode, RollPolicy, SegmentStore,
    SegmentStoreConfig, Store,
};
use std::path::PathBuf;
use std::sync::Arc;

fn tempdir(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "ontology-budget-{tag}-{}-{}-{}",
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

/// Three domains of very different sizes: `big` (many concepts + edges),
/// `mid`, `tiny`.
fn ontology() -> Ontology {
    let mut o = Ontology::new();
    for (ty, ns) in [("Big", "big"), ("Mid", "mid"), ("Tiny", "tiny")] {
        o.add_concept_type(ConceptType {
            name: ty.into(),
            ns: Some(ns.into()),
            ..Default::default()
        });
    }
    o.add_relation_type(RelationType {
        name: "big_rel".into(),
        domain: "Big".into(),
        range: "Big".into(),
        ..Default::default()
    })
    .unwrap();
    o.add_relation_type(RelationType {
        name: "mid_rel".into(),
        domain: "Mid".into(),
        range: "Mid".into(),
        ..Default::default()
    })
    .unwrap();
    o
}

struct Written {
    records: u64,
    edges: u64,
    payload: u64,
}

async fn fill(
    graph: &Arc<OntologyGraph>,
    store: &SegmentStore,
    ty: &str,
    n: usize,
    rel: Option<&str>,
    desc_len: usize,
) -> Written {
    let mut w = Written {
        records: 0,
        edges: 0,
        payload: 0,
    };
    let mut ids = Vec::new();
    for i in 0..n {
        let mut c = Concept::new(ConceptId(0), ty, format!("{ty}-{i}"))
            .with_description("x".repeat(desc_len));
        graph.prepare_concept(&mut c).unwrap();
        let rec = LogRecord::concept(c.clone());
        w.payload += serde_json::to_vec(&rec.kind).unwrap().len() as u64;
        w.records += 1;
        store.append(&rec).await.unwrap();
        ids.push(graph.apply_prepared_concept(c).unwrap());
    }
    if let Some(rt) = rel {
        for i in 1..n {
            let mut r = Relation::new(RelationId(0), rt, ids[i - 1], ids[i]);
            graph.prepare_relation(&mut r).unwrap();
            let rec = LogRecord::relation(r.clone());
            w.payload += serde_json::to_vec(&rec.kind).unwrap().len() as u64;
            w.records += 1;
            w.edges += 1;
            store.append(&rec).await.unwrap();
            graph.apply_prepared_relation(r).unwrap();
        }
    }
    w
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

/// The estimate of each domain equals the formula applied to exactly the
/// records, edges and payload bytes written to it — across sealed
/// partitions (small roll policy) and the active segment, before and after
/// a reopen.
#[tokio::test]
async fn estimates_come_from_the_manifest_and_the_active_segments() {
    let dir = tempdir("estimate");
    let graph = OntologyGraph::with_arc(ontology());
    let (big, mid, tiny) = {
        let store = SegmentStore::open_with(&dir, small_roll()).await.unwrap();
        store
            .append(&LogRecord::ontology(ontology()))
            .await
            .unwrap();
        let big = fill(&graph, &store, "Big", 30, Some("big_rel"), 400).await;
        let mid = fill(&graph, &store, "Mid", 10, Some("mid_rel"), 100).await;
        let tiny = fill(&graph, &store, "Tiny", 2, None, 10).await;

        let est = store.estimate_domains().unwrap();
        assert_eq!(
            est.iter().map(|d| d.ns.as_str()).collect::<Vec<_>>(),
            ["default", "big", "mid", "tiny"],
            "every graph domain, meta excluded, by ns_id"
        );
        for (d, w) in est.iter().skip(1).zip([&big, &mid, &tiny]) {
            assert_eq!(
                (d.records, d.edges, d.payload_bytes),
                (w.records, w.edges, w.payload),
                "{}",
                d.ns
            );
            assert_eq!(
                d.estimated_bytes,
                estimate_bytes(w.records, w.edges, w.payload)
            );
            let concepts = w.records - w.edges;
            assert_eq!(
                d.estimated_bytes,
                w.payload
                    + concepts * (CONCEPT_INDEX_BYTES + REBUILD_PEAK_PER_CONCEPT)
                    + w.edges * RELATION_BYTES
            );
        }
        assert!(
            est[0].records == 0 && est[0].estimated_bytes == 0,
            "empty default domain"
        );
        assert!(
            store.manifest().stream(2).unwrap().sealed.len() >= 5,
            "the small roll policy sealed partitions"
        );
        (big, mid, tiny)
    };
    // Reopen: the same figures, now entirely from sealed zone maps + a
    // recovered active segment.
    let store = SegmentStore::open_with(&dir, small_roll()).await.unwrap();
    let est = store.estimate_domains().unwrap();
    for (d, w) in est.iter().skip(1).zip([&big, &mid, &tiny]) {
        assert_eq!(
            (d.records, d.edges, d.payload_bytes),
            (w.records, w.edges, w.payload),
            "{} after reopen",
            d.ns
        );
    }
    // The fixture is built big > mid > tiny (estimates are listed by
    // ns_id; the plan sorts by size itself), which the next test relies on.
    assert!(
        est[1].estimated_bytes > est[2].estimated_bytes
            && est[2].estimated_bytes > est[3].estimated_bytes
    );
}

/// Strict mode refuses with both figures; adaptive narrows the load to the
/// domains that fit (smallest first) and the store then hydrates exactly
/// those; the plan is remembered; `--ns` wins.
#[tokio::test]
async fn plan_refuses_or_narrows_and_hydration_follows_it() {
    let dir = tempdir("plan");
    let graph = OntologyGraph::with_arc(ontology());
    let store = SegmentStore::open(&dir).await.unwrap();
    store
        .append(&LogRecord::ontology(ontology()))
        .await
        .unwrap();
    fill(&graph, &store, "Big", 30, Some("big_rel"), 400).await;
    fill(&graph, &store, "Mid", 10, Some("mid_rel"), 100).await;
    fill(&graph, &store, "Tiny", 2, None, 10).await;
    let est = store.estimate_domains().unwrap();
    let by = |ns: &str| est.iter().find(|d| d.ns == ns).unwrap().estimated_bytes;
    let (big, mid, tiny) = (by("big"), by("mid"), by("tiny"));
    let total = big + mid + tiny;
    assert!(store.last_plan().is_none());

    // Everything fits.
    let plan = store
        .plan_load(MemoryBudget::fixed(total), MemoryMode::Strict, None)
        .unwrap();
    assert_eq!(plan.domains, None);
    assert!(!plan.is_partial());
    assert_eq!(store.last_plan().as_ref(), Some(&plan));

    // Strict, one byte short: refused, both figures in the message (R17),
    // through the store's own error type without a misleading prefix.
    let err = store
        .plan_load(MemoryBudget::fixed(total - 1), MemoryMode::Strict, None)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("strict") && err.contains("MiB") && err.contains("big"),
        "{err}"
    );
    assert!(
        err.starts_with("the store needs") && err.contains(&format!("({total} bytes)")),
        "{err}"
    );

    // Adaptive on a free-memory reading never trims: everything, flagged.
    let soft = MemoryBudget::from_available(mid + tiny + 1, BudgetSource::MemAvailable, 1.0);
    let plan = store.plan_load(soft, MemoryMode::Adaptive, None).unwrap();
    assert_eq!(plan.domains, None);
    assert!(plan.over_budget && !plan.is_partial());

    // Adaptive with room for mid + tiny only.
    let budget = MemoryBudget::fixed(mid + tiny + 1);
    let plan = store.plan_load(budget, MemoryMode::Adaptive, None).unwrap();
    assert_eq!(
        plan.domains,
        Some(vec![
            "default".to_string(),
            "mid".to_string(),
            "tiny".to_string()
        ])
    );
    assert_eq!(
        plan.skipped
            .iter()
            .map(|s| s.ns.as_str())
            .collect::<Vec<_>>(),
        ["big"]
    );
    assert_eq!(plan.estimated_loaded_bytes, mid + tiny);
    assert_eq!(plan.estimated_total_bytes, total);

    // Hydrating what the plan chose loads exactly those domains.
    let loaded = OntologyGraph::with_arc(Ontology::new());
    store
        .load_domains(&loaded, plan.domains.as_ref().unwrap())
        .await
        .unwrap();
    assert_eq!(loaded.concept_count(), 12, "10 Mid + 2 Tiny, no Big");
    assert_eq!(loaded.relation_count(), 9, "mid_rel only");

    // An explicit --ns is honoured (and its estimate checked in strict).
    let want = vec!["big".to_string()];
    let plan = store
        .plan_load(MemoryBudget::fixed(big), MemoryMode::Strict, Some(&want))
        .unwrap();
    assert!(plan.explicit && plan.domains == Some(want.clone()));
    let err = store
        .plan_load(
            MemoryBudget::fixed(big - 1),
            MemoryMode::Strict,
            Some(&want),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("strict"), "{err}");
    let err = store
        .plan_load(
            MemoryBudget::fixed(big),
            MemoryMode::Adaptive,
            Some(&["nope".to_string()]),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("nope") && err.contains("big"), "{err}");

    // Unknown budget: everything, no refusal even in strict.
    let plan = store
        .plan_load(MemoryBudget::unknown(0.6), MemoryMode::Strict, None)
        .unwrap();
    assert_eq!(plan.domains, None);
}

/// A fresh store plans to load nothing in particular and never refuses.
#[tokio::test]
async fn an_empty_store_always_fits() {
    let dir = tempdir("empty");
    let store = SegmentStore::open(&dir).await.unwrap();
    let plan = store
        .plan_load(MemoryBudget::fixed(1), MemoryMode::Strict, None)
        .unwrap();
    assert_eq!(plan.estimated_total_bytes, 0);
    assert_eq!(plan.domains, None);
    let est = store.estimate_domains().unwrap();
    assert_eq!(est.len(), 1, "only the default domain exists");
    assert_eq!(est[0].ns, "default");
}
