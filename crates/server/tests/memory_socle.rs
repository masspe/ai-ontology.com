// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! The startup memory decision is visible to operators: `/stats` carries a
//! `memory` object and `/metrics` the matching gauges; without a plan (an
//! in-memory store) `/stats.memory` is null and the gauges are absent.

use axum::body::{to_bytes, Body};
use http::{Request, StatusCode};
use ontology_graph::{Ontology, OntologyGraph};
use ontology_index::HybridIndex;
use ontology_rag::{EchoModel, RagPipeline};
use ontology_server::{build_router, AppState};
use ontology_storage::{
    BudgetSource, DomainEstimate, LoadPlan, MemoryBudget, MemoryMode, MemoryStore, SkippedDomain,
    Store,
};
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

fn state() -> AppState {
    let graph = OntologyGraph::with_arc(Ontology::new());
    let index = Arc::new(HybridIndex::with_default_embedder(graph.clone()));
    let pipeline = Arc::new(RagPipeline::new(index.clone(), Arc::new(EchoModel)));
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    AppState::new(graph, index, store, pipeline)
}

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, String) {
    let resp = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let body = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

fn est(ns: &str, id: u16, bytes: u64) -> DomainEstimate {
    DomainEstimate {
        ns_id: id,
        ns: ns.into(),
        records: 10,
        edges: 4,
        payload_bytes: bytes / 2,
        estimated_bytes: bytes,
    }
}

fn partial_plan() -> LoadPlan {
    LoadPlan {
        mode: MemoryMode::Adaptive,
        budget: MemoryBudget::from_available(10 << 20, BudgetSource::CgroupV2, 0.5),
        estimated_total_bytes: 9 << 20,
        estimated_loaded_bytes: 3 << 20,
        domains: Some(vec!["parties".into(), "contrats".into()]),
        skipped: vec![SkippedDomain {
            ns: "facturation".into(),
            estimated_bytes: 6 << 20,
        }],
        explicit: false,
        over_budget: false,
        estimates: vec![
            est("parties", 2, 1 << 20),
            est("contrats", 3, 2 << 20),
            est("facturation", 4, 6 << 20),
        ],
    }
}

#[tokio::test]
async fn stats_and_metrics_expose_the_memory_plan() {
    let app = build_router(state().with_memory_plan(Some(partial_plan())));
    let (st, body) = get(&app, "/stats").await;
    assert_eq!(st, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).unwrap();
    let m = &v["memory"];
    assert_eq!(m["mode"], "adaptive");
    assert_eq!(m["budget_source"], "cgroup-v2");
    assert_eq!(m["available_mib"], 10);
    assert_eq!(m["budget_mib"], 5);
    assert_eq!(m["estimate_mib"], 9);
    assert_eq!(m["loaded_estimate_mib"], 3);
    assert_eq!(m["partial"], true);
    assert_eq!(m["over_budget"], false);
    assert_eq!(
        m["domains_loaded"],
        serde_json::json!(["parties", "contrats"])
    );
    assert_eq!(m["domains_skipped"], serde_json::json!(["facturation"]));
    assert!(m["rss_mib"].is_number() || m["rss_mib"].is_null());

    let (st, text) = get(&app, "/metrics").await;
    assert_eq!(st, StatusCode::OK);
    for line in [
        &format!("ontology_memory_budget_bytes {}", 5u64 << 20),
        &format!("ontology_memory_estimate_bytes {}", 9u64 << 20),
        &format!("ontology_memory_loaded_estimate_bytes {}", 3u64 << 20),
        "ontology_domains_loaded 2",
        "ontology_domains_skipped 1",
        "ontology_memory_partial 1",
        "ontology_memory_over_budget 0",
        "ontology_memory_budget_known 1",
    ] {
        assert!(text.contains(line), "missing `{line}` in:\n{text}");
    }
    // The pre-existing gauges are still there.
    assert!(text.contains("ontology_concepts 0"));
}

#[tokio::test]
async fn a_full_load_shows_every_domain_as_loaded_and_not_partial() {
    let mut plan = partial_plan();
    plan.domains = None;
    plan.skipped.clear();
    plan.estimated_loaded_bytes = plan.estimated_total_bytes;
    let app = build_router(state().with_memory_plan(Some(plan)));
    let (_, body) = get(&app, "/stats").await;
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["memory"]["partial"], false);
    assert_eq!(
        v["memory"]["domains_loaded"],
        serde_json::json!(["parties", "contrats", "facturation"])
    );
    assert_eq!(v["memory"]["domains_skipped"], serde_json::json!([]));
    let (_, text) = get(&app, "/metrics").await;
    assert!(
        text.contains("ontology_domains_loaded 3") && text.contains("ontology_memory_partial 0")
    );
}

#[tokio::test]
async fn without_a_plan_stats_memory_is_null_and_gauges_are_absent() {
    let app = build_router(state());
    let (_, body) = get(&app, "/stats").await;
    let v: Value = serde_json::from_str(&body).unwrap();
    assert!(v["memory"].is_null(), "{body}");
    let (_, text) = get(&app, "/metrics").await;
    assert!(!text.contains("ontology_memory_budget_bytes"));
    assert!(!text.contains("ontology_domains_loaded"));
}

#[tokio::test]
async fn an_unknown_budget_is_reported_as_such() {
    let mut plan = partial_plan();
    plan.budget = MemoryBudget::unknown(0.6);
    plan.domains = None;
    plan.skipped.clear();
    let app = build_router(state().with_memory_plan(Some(plan)));
    let (_, body) = get(&app, "/stats").await;
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["memory"]["budget_source"], "unknown");
    assert!(v["memory"]["budget_mib"].is_null());
    let (_, text) = get(&app, "/metrics").await;
    assert!(
        text.contains("ontology_memory_budget_bytes 0")
            && text.contains("ontology_memory_budget_known 0"),
        "unknown budget is exported as 0 and flagged unknown:\n{text}"
    );
}

#[tokio::test]
async fn a_soft_budget_exceeded_is_reported_as_over_budget_not_partial() {
    let mut plan = partial_plan();
    plan.budget = MemoryBudget::from_available(4 << 20, BudgetSource::MemAvailable, 1.0);
    plan.domains = None;
    plan.skipped.clear();
    plan.estimated_loaded_bytes = plan.estimated_total_bytes;
    plan.over_budget = true;
    let app = build_router(state().with_memory_plan(Some(plan)));
    let (_, body) = get(&app, "/stats").await;
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["memory"]["budget_source"], "meminfo");
    assert_eq!(v["memory"]["partial"], false);
    assert_eq!(v["memory"]["over_budget"], true);
    assert_eq!(v["memory"]["domains_skipped"], serde_json::json!([]));
    let (_, text) = get(&app, "/metrics").await;
    assert!(
        text.contains("ontology_memory_over_budget 1")
            && text.contains("ontology_memory_partial 0"),
        "{text}"
    );
}
