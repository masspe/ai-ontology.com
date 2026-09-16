// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Operational endpoints: `/healthz`, `/compact` (with a real
//! `SegmentStore` reopened afterwards), the OpenAPI document against the
//! router, and the bearer gate in front of writes.

mod hardening_common;

use axum::body::Body;
use hardening_common::*;
use http::{Request, StatusCode};
use ontology_graph::{ConceptId, Ontology, OntologyGraph};
use ontology_server::{build_router, build_router_with_auth};
use ontology_storage::{MemoryStore, SegmentStore, Store};
use serde_json::json;
use std::collections::BTreeSet;
use std::sync::Arc;

/// `/healthz` is a plain-text `ok`.
#[tokio::test]
async fn healthz_is_plain_ok() {
    let (app, _, _) = flaky_app();
    let (st, headers, bytes) = send(
        &app,
        Request::builder()
            .uri("/healthz")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(bytes, b"ok");
    let ct = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(ct.starts_with("text/plain"), "content-type: {ct}");
}

/// On a store without a WAL to truncate, `/compact` is a 204 no-op.
#[tokio::test]
async fn compact_on_a_memory_store_is_a_204_no_op() {
    let store = Arc::new(MemoryStore::new());
    let graph = OntologyGraph::with_arc(ontology());
    let app = build_router(state_with(store.clone(), graph.clone()));
    let a = create_topic(&app, "A").await;
    let b = create_topic(&app, "B").await;
    relate(&app, "related_to", a, b).await;
    let before = fingerprint(&graph);
    let records = store.len();

    let (st, v) = call(&app, "POST", "/compact", None).await;
    assert_eq!(st, StatusCode::NO_CONTENT, "{v}");
    assert_eq!(fingerprint(&graph), before);
    assert_eq!(store.len(), records);
}

/// `/compact` on a `SegmentStore`: 204, the live graph is untouched, the
/// rewritten store replays to the same graph both immediately and after a
/// close-and-reopen.
#[tokio::test]
async fn compact_keeps_the_graph_and_the_store_replays_identically_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let expected;
    {
        let store = Arc::new(SegmentStore::open(dir.path()).await.unwrap());
        let graph = OntologyGraph::with_arc(Ontology::new());
        store.load_into(&graph).await.unwrap();
        let app = build_router(state_with(store.clone(), graph.clone()));

        let (st, v) = call(
            &app,
            "PUT",
            "/ontology",
            Some(serde_json::to_value(ontology()).unwrap()),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{v}");
        let a = create_topic(&app, "A").await;
        let b = create_topic(&app, "B").await;
        let c = create_topic(&app, "C").await;
        relate(&app, "related_to", a, b).await;
        relate(&app, "owned_by", a, c).await;
        create_rule(&app, "r1", &[a]).await;
        create_action(&app, "act1", a).await;
        let (st, _) = call(
            &app,
            "PATCH",
            &format!("/concepts/{b}"),
            Some(json!({ "name": "B2" })),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        let (st, _) = call(&app, "DELETE", &format!("/concepts/{c}"), None).await;
        assert_eq!(st, StatusCode::NO_CONTENT);
        let before = fingerprint(&graph);
        expected = shape(&graph);

        let (st, v) = call(&app, "POST", "/compact", None).await;
        assert_eq!(st, StatusCode::NO_CONTENT, "{v}");
        assert_eq!(
            fingerprint(&graph),
            before,
            "compaction never touches memory"
        );

        let fresh = OntologyGraph::with_arc(Ontology::new());
        store.load_into(&fresh).await.unwrap();
        assert_eq!(
            shape(&fresh),
            expected,
            "compacted store replays the same graph"
        );
        assert_eq!(
            fresh.get_concept(ConceptId(b)).unwrap().name,
            "B2",
            "the rename survived, not the original name"
        );
    }

    let store = SegmentStore::open(dir.path()).await.unwrap();
    let graph = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&graph).await.unwrap();
    assert_eq!(shape(&graph), expected, "same graph after reopen");
    assert_eq!(graph.concept_count(), 2);
    assert_eq!(
        graph.relation_count(),
        2,
        "A-B both ways; A-C cascaded away"
    );
    assert_eq!(graph.rule_count(), 1);
    assert_eq!(graph.action_count(), 1);
}

/// Every route `build_router` registers, with its methods. Kept by hand —
/// axum does not expose its route table — so adding a route means adding it
/// here **and** to `openapi.rs`; the test below fails otherwise.
const ROUTES: &[(&str, &[&str])] = &[
    ("/healthz", &["get"]),
    ("/openapi.json", &["get"]),
    ("/docs", &["get"]),
    ("/stats", &["get"]),
    ("/stats/history", &["get"]),
    ("/metrics", &["get"]),
    ("/ontology", &["get", "put"]),
    ("/ontology/generate", &["post"]),
    ("/concepts", &["get", "post"]),
    ("/concepts/delete", &["post"]),
    ("/concepts/{id}", &["get", "patch", "delete"]),
    ("/relations", &["get", "post"]),
    ("/relations/{id}", &["get", "patch", "delete"]),
    ("/rules", &["get", "post"]),
    ("/rules/generate", &["post"]),
    ("/rules/{id}", &["get", "patch", "delete"]),
    ("/actions", &["get", "post"]),
    ("/actions/{id}", &["get", "patch", "delete"]),
    ("/retrieve", &["post"]),
    ("/subgraph", &["post"]),
    ("/ask", &["post"]),
    ("/ask/stream", &["post"]),
    ("/path", &["post"]),
    ("/compact", &["post"]),
    ("/reset", &["post"]),
    ("/upload", &["post"]),
    ("/ingest/analyze", &["post"]),
    ("/ingest/apply", &["post"]),
    ("/export", &["get"]),
    ("/files", &["get"]),
    ("/files/{id}", &["get", "delete"]),
    ("/queries", &["get", "post"]),
    ("/queries/{id}", &["get", "patch", "delete"]),
    ("/queries/{id}/run", &["post"]),
    ("/settings", &["get", "patch"]),
    ("/settings/llm/test", &["post"]),
    ("/settings/llm/models", &["get", "post"]),
    ("/settings/llm/infomaniak/products", &["post"]),
    ("/settings/ocr/status", &["get"]),
    ("/feedbacks", &["get", "post"]),
    ("/feedbacks/{id}", &["delete"]),
    ("/logs/tail", &["get"]),
];

/// `/openapi.json` documents exactly the router's paths and methods, and
/// every documented operation is actually routed (no 405, no bare 404).
#[tokio::test]
async fn openapi_documents_exactly_the_routes_the_router_serves() {
    let (app, _, _) = flaky_app();
    let (st, spec) = get(&app, "/openapi.json").await;
    assert_eq!(st, StatusCode::OK);
    let paths = spec["paths"].as_object().unwrap();

    let documented: BTreeSet<&str> = paths.keys().map(String::as_str).collect();
    let routed: BTreeSet<&str> = ROUTES.iter().map(|(p, _)| *p).collect();
    let missing: Vec<_> = routed.difference(&documented).collect();
    let stale: Vec<_> = documented.difference(&routed).collect();
    assert!(
        missing.is_empty(),
        "routes missing from openapi.json: {missing:?}"
    );
    assert!(
        stale.is_empty(),
        "openapi.json paths with no route: {stale:?}"
    );

    for (path, methods) in ROUTES {
        let mut spec_methods: Vec<&str> = paths[*path]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .filter(|k| *k != "parameters")
            .collect();
        spec_methods.sort_unstable();
        let mut want: Vec<&str> = methods.to_vec();
        want.sort_unstable();
        assert_eq!(spec_methods, want, "methods documented for {path}");
    }

    // Every documented operation reaches a handler: the router's own
    // fallbacks are a bodiless 404 (unknown path) or a 405 (known path,
    // wrong method); a handler's 404 carries a JSON error body.
    for (path, methods) in ROUTES {
        for method in *methods {
            let uri = path.replace("{id}", "1");
            let has_body = matches!(*method, "post" | "put" | "patch");
            let mut req = Request::builder()
                .method(method.to_uppercase().as_str())
                .uri(&uri);
            let body = if has_body {
                req = req.header("content-type", "application/json");
                Body::from("{}")
            } else {
                Body::empty()
            };
            let (st, _, bytes) = send(&app, req.body(body).unwrap()).await;
            assert_ne!(st, StatusCode::METHOD_NOT_ALLOWED, "{method} {uri}");
            assert!(
                !(st == StatusCode::NOT_FOUND && bytes.is_empty()),
                "{method} {uri}: not routed"
            );
        }
    }
}

/// With a bearer token configured, a write without (or with the wrong)
/// credential is refused before anything is parsed, journaled or applied;
/// the scheme keyword is case-insensitive.
#[tokio::test]
async fn auth_rejects_writes_before_any_store_or_graph_access() {
    let store = Arc::new(ontology_storage::FlakyStore::new());
    let graph = OntologyGraph::with_arc(ontology());
    let app = build_router_with_auth(
        state_with(store.clone(), graph.clone()),
        Some("s3cret".into()),
    );
    let before = fingerprint(&graph);

    let attempt = |auth: Option<&'static str>| {
        let app = app.clone();
        async move {
            let mut req = Request::builder()
                .method("POST")
                .uri("/concepts")
                .header("content-type", "application/json");
            if let Some(a) = auth {
                req = req.header("authorization", a);
            }
            let (st, _, _) =
                send(&app, req.body(Body::from(topic("A").to_string())).unwrap()).await;
            st
        }
    };
    assert_eq!(attempt(None).await, StatusCode::UNAUTHORIZED);
    assert_eq!(
        attempt(Some("Bearer wrong")).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        attempt(Some("Basic czNjcmV0")).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        attempt(Some("s3cret")).await,
        StatusCode::UNAUTHORIZED,
        "scheme required"
    );
    assert_eq!(store.append_calls(), 0);
    assert_eq!(fingerprint(&graph), before);

    assert_eq!(attempt(Some("bearer s3cret")).await, StatusCode::OK);
    assert_eq!(graph.concept_count(), 1);
    assert_eq!(store.append_calls(), 1);

    // Reads and deletes are gated the same way.
    let (st, _) = get(&app, "/concepts").await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _) = call(&app, "DELETE", "/concepts/1", None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    assert_eq!(graph.concept_count(), 1);
}
