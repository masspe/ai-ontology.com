// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! P1 through the HTTP API (STORAGE.md §8.2): a domain whose payloads live
//! on disk serves `GET /concepts/{id}` with its description, a `PATCH`
//! writes a new record and the read that follows sees it, `/stats` shows
//! the tier, and a payload the source cannot read is a 500 — never a panic.

mod hardening_common;

use axum::body::Body;
use hardening_common::*;
use http::{Request, StatusCode};
use ontology_graph::{Concept, ConceptId, Loc, Ontology, OntologyGraph, PayloadSource};
use ontology_server::build_router;
use ontology_storage::{LogRecord, RollPolicy, SegmentStore, SegmentStoreConfig, Store, Tier};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

fn tempdir(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "ontology-p1http-{tag}-{}-{}-{}",
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

/// A P1 store of `n` topics with a description, every partition sealed but
/// the last (roll every 5 records).
async fn p1_app(n: u64) -> (axum::Router, Arc<OntologyGraph>, Arc<SegmentStore>) {
    let dir = tempdir("store");
    let cfg = SegmentStoreConfig {
        roll: RollPolicy {
            max_bytes: u64::MAX,
            max_records: 5,
        },
        ..Default::default()
    };
    {
        let store = SegmentStore::open_with(&dir, cfg).await.unwrap();
        store
            .append(&LogRecord::ontology(ontology()))
            .await
            .unwrap();
        for i in 1..=n {
            store
                .append(&LogRecord::concept(
                    Concept::new(ConceptId(i), "Topic", format!("topic-{i}"))
                        .with_description(format!("about topic {i}")),
                ))
                .await
                .unwrap();
        }
    }
    let store = Arc::new(SegmentStore::open_with(&dir, cfg).await.unwrap());
    let tiers = store
        .manifest()
        .graph_ns_ids()
        .into_iter()
        .map(|id| (id, Tier::P1))
        .collect();
    store.set_tiers(tiers);
    let graph = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&graph).await.unwrap();
    let app = build_router(state_with(store.clone(), graph.clone()));
    (app, graph, store)
}

#[tokio::test]
async fn a_p1_domain_is_served_read_and_patched_over_http() {
    let (app, graph, _store) = p1_app(12).await;
    // Ten of the twelve are in sealed partitions and evicted; the last two
    // sit in the active segment and stay resident.
    assert_eq!(graph.concept_count(), 12);
    assert_eq!(graph.resident_payloads(), 2, "the active tail is resident");

    let (st, v) = get(&app, "/concepts/3").await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["name"], "topic-3");
    assert_eq!(v["description"], "about topic 3", "payload read from disk");

    // The listing reads every payload through the source too.
    let (st, v) = get(&app, "/concepts?limit=12").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["concepts"].as_array().unwrap().len(), 12);
    assert!(v["concepts"]
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c["description"]
            .as_str()
            .unwrap()
            .starts_with("about topic")));

    // PATCH: a new record goes to the active segment, the graph applies it,
    // the next read sees it; the concept is resident again until a roll.
    let (st, v) = call(
        &app,
        "PATCH",
        "/concepts/3",
        Some(json!({ "description": "rewritten" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (_, v) = get(&app, "/concepts/3").await;
    assert_eq!(v["description"], "rewritten");
    assert_eq!(graph.resident_payloads(), 3);

    // Enough further writes to roll: the patched concept is evicted again
    // and still reads back as patched.
    for i in 0..6u64 {
        let (st, _) = call(
            &app,
            "PATCH",
            "/concepts/4",
            Some(json!({ "description": format!("v{i}") })),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
    }
    let (_, v) = get(&app, "/concepts/3").await;
    assert_eq!(v["description"], "rewritten");
    let (_, v) = get(&app, "/concepts/4").await;
    assert_eq!(v["description"], "v5");
    assert!(
        graph.resident_payloads() < 8,
        "{}",
        graph.resident_payloads()
    );

    // /stats without a plan has no `memory` object (the CLI makes the plan);
    // the graph-side counter is what the plan would report.
    let (st, v) = get(&app, "/stats").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["concepts"], 12);
    assert!(v["memory"].is_null());
}

struct Broken;
impl PayloadSource for Broken {
    fn read(&self, loc: Loc) -> Result<Concept, String> {
        Err(format!("disk gone at {loc:?}"))
    }
}

#[tokio::test]
async fn an_unreadable_payload_is_a_500_not_a_panic() {
    let (app, graph, _store) = flaky_app();
    let id = graph
        .upsert_concept(Concept::new(ConceptId(0), "Topic", "t").with_description("d"))
        .unwrap();
    graph.set_payload_source(Arc::new(Broken));
    graph.set_loc(
        id,
        Loc {
            ns_id: 1,
            partition: 1,
            offset: 64,
        },
        false,
    );
    assert_eq!(graph.resident_payloads(), 0);

    let (st, v) = get(&app, &format!("/concepts/{}", id.0)).await;
    assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR, "{v}");
    assert!(error_text(&v).contains("payload"), "{v}");
    // The listing and the subgraph seeds fail the same way, explicitly.
    let (st, v) = get(&app, "/concepts").await;
    assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR, "{v}");
    let (st, _, _) = send(
        &app,
        Request::builder()
            .method("POST")
            .uri("/subgraph")
            .header("content-type", "application/json")
            .body(Body::from(json!({ "limit": 5 }).to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR);
    // Type and name never need the payload: the name lookup still works.
    assert_eq!(graph.find_by_name("Topic", "t"), Some(id));
}
