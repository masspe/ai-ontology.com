// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

use axum::body::{to_bytes, Body};
use http::{Request, StatusCode};
use ontology_graph::{ConceptType, Ontology, OntologyGraph, RelationType};
use ontology_index::HybridIndex;
use ontology_rag::{EchoModel, RagPipeline};
use ontology_server::{
    build_router, build_router_with_auth, build_router_with_config, build_router_with_jwt,
    AppState, JwtAuth, RateLimit, RouterConfig,
};
use ontology_storage::{MemoryStore, Store};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

fn make_state() -> AppState {
    let graph = OntologyGraph::with_arc(ontology());
    let index = Arc::new(HybridIndex::with_default_embedder(graph.clone()));
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let pipeline = Arc::new(RagPipeline::new(index.clone(), Arc::new(EchoModel)));
    AppState::new(graph, index, store, pipeline)
}

fn ontology() -> Ontology {
    let mut o = Ontology::new();
    o.add_concept_type(ConceptType {
        name: "Topic".into(),
        parent: None,
        properties: None,
        description: "topic".into(),
    });
    o.add_relation_type(RelationType {
        name: "related_to".into(),
        domain: "Topic".into(),
        range: "Topic".into(),
        cardinality: Default::default(),
        symmetric: true,
        description: "".into(),
    })
    .unwrap();
    o
}

async fn read_body(b: Body) -> Value {
    let bytes = to_bytes(b, 1 << 20).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn create_retrieve_delete_round_trip() {
    let graph = OntologyGraph::with_arc(ontology());
    let index = Arc::new(HybridIndex::with_default_embedder(graph.clone()));
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let pipeline = Arc::new(RagPipeline::new(index.clone(), Arc::new(EchoModel)));

    let app = build_router(AppState::new(graph.clone(), index, store, pipeline));

    // Create concept.
    let body = json!({ "id": 0, "concept_type": "Topic", "name": "RAG",
        "description": "retrieval augmented generation",
        "properties": {} });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/concepts")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let id = read_body(resp.into_body()).await["id"].as_u64().unwrap();
    assert!(id > 0);

    // Retrieve.
    let req_body = json!({ "query": "retrieval augmented generation",
        "top_k": 4, "lexical_weight": 0.5, "expansion": {"max_depth": 1} });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/retrieve")
                .header("content-type", "application/json")
                .body(Body::from(req_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = read_body(resp.into_body()).await;
    assert_eq!(v["scored"][0]["id"].as_u64(), Some(id));

    // Stats.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/stats")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let v = read_body(resp.into_body()).await;
    assert_eq!(v["concepts"].as_u64(), Some(1));

    // Delete.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/concepts/{}", id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert_eq!(graph.concept_count(), 0);
}

#[tokio::test]
async fn patch_concept_renames_and_metrics_reflect_state() {
    let state = make_state();
    let graph = state.graph.clone();
    let app = build_router(state);

    // Seed a concept.
    let body = json!({ "id": 0, "concept_type": "Topic", "name": "RAG",
        "description": "first", "properties": {} });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/concepts")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let id = read_body(resp.into_body()).await["id"].as_u64().unwrap();

    // PATCH it.
    let patch = json!({ "name": "Retrieval Augmented Generation", "description": "renamed" });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/concepts/{}", id))
                .header("content-type", "application/json")
                .body(Body::from(patch.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = read_body(resp.into_body()).await;
    assert_eq!(v["name"], "Retrieval Augmented Generation");
    assert_eq!(v["description"], "renamed");

    // GET round-trips.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/concepts/{}", id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let v = read_body(resp.into_body()).await;
    assert_eq!(v["name"], "Retrieval Augmented Generation");

    // Old name binding removed in the graph.
    assert!(graph.find_by_name("Topic", "RAG").is_none());

    // /metrics is plain-text Prometheus exposition.
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(
        text.contains("ontology_concepts 1"),
        "metrics body:\n{text}"
    );
    assert!(text.contains("# TYPE ontology_concepts gauge"));
}

#[tokio::test]
async fn bearer_auth_blocks_missing_or_wrong_token() {
    let app = build_router_with_auth(make_state(), Some("s3cret".into()));

    // /healthz is unauthenticated.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // No Authorization header → 401.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/stats")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Wrong token → 401.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/stats")
                .header("authorization", "Bearer wrong")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Right token → 200.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/stats")
                .header("authorization", "Bearer s3cret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn rate_limiter_rejects_after_burst() {
    // 2 req/min — third request inside the window should 429.
    let app = build_router_with_config(
        make_state(),
        RouterConfig {
            bearer_token: None,
            jwt: None,
            rate_limit: Some(RateLimit {
                max_requests: 2,
                window: Duration::from_secs(60),
            }),
        },
    );

    for expected in [
        StatusCode::OK,
        StatusCode::OK,
        StatusCode::TOO_MANY_REQUESTS,
    ] {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), expected);
    }
}

fn mint_jwt(secret: &[u8], sub: &str, iss: &str, aud: &str, expires_in: i64) -> String {
    use jsonwebtoken::{encode, EncodingKey, Header};
    #[derive(serde::Serialize)]
    struct Claims<'a> {
        sub: &'a str,
        iss: &'a str,
        aud: &'a str,
        exp: usize,
        email: &'a str,
        name: &'a str,
    }
    let exp = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        + expires_in) as usize;
    encode(
        &Header::default(),
        &Claims { sub, iss, aud, exp, email: "alice@example.com", name: "Alice" },
        &EncodingKey::from_secret(secret),
    )
    .unwrap()
}

#[tokio::test]
async fn jwt_auth_accepts_valid_token() {
    let secret = b"super-shared-secret-must-be-long-enough";
    let app = build_router_with_jwt(make_state(), JwtAuth::from_secret(secret.to_vec()));

    // Healthz is open.
    let resp = app
        .clone()
        .oneshot(Request::builder().uri("/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // No token → 401.
    let resp = app
        .clone()
        .oneshot(Request::builder().uri("/stats").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Valid token → 200.
    let token = mint_jwt(secret, "user-1", "ai-ontology", "web", 3600);
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/stats")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn jwt_auth_rejects_bad_signature_wrong_claims_and_expired() {
    let secret = b"the-real-secret-the-real-secret-the-real";
    let app = build_router_with_jwt(make_state(), JwtAuth::from_secret(secret.to_vec()));

    // Wrong signing key → 401.
    let token = mint_jwt(b"wrong-key-wrong-key-wrong-key-wrong", "u", "ai-ontology", "web", 60);
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/stats")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Wrong audience → 401.
    let token = mint_jwt(secret, "u", "ai-ontology", "other", 60);
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/stats")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Expired → 401 (well past the 60s leeway).
    let token = mint_jwt(secret, "u", "ai-ontology", "web", -3600);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/stats")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn jwt_and_static_token_coexist() {
    // Both creds accepted: a JWT for users, a service token for backend jobs.
    let secret = b"coexist-secret-coexist-secret-coexist";
    let app = build_router_with_config(
        make_state(),
        RouterConfig {
            bearer_token: Some("service-token".into()),
            jwt: Some(JwtAuth::from_secret(secret.to_vec())),
            rate_limit: None,
        },
    );

    // Service static token works.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/stats")
                .header("authorization", "Bearer service-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // User JWT works.
    let token = mint_jwt(secret, "user-1", "ai-ontology", "web", 3600);
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/stats")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Garbage rejected.
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/stats")
                .header("authorization", "Bearer nope")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn request_id_is_minted_and_round_trips() {
    let app = build_router(make_state());

    // No inbound id → server mints one.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let id = resp
        .headers()
        .get("x-request-id")
        .expect("server must mint id");
    assert!(!id.to_str().unwrap().is_empty());

    // Inbound id → echoed back unchanged.
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/healthz")
                .header("x-request-id", "trace-42")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.headers().get("x-request-id").unwrap(), "trace-42");
}


// ---------------------------------------------------------------------------
// Smoke tests for the new endpoints
// ---------------------------------------------------------------------------

#[tokio::test]
async fn settings_round_trip() {
    let app = build_router(make_state());

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/settings")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = read_body(resp.into_body()).await;
    assert_eq!(v["retrieval"]["top_k"].as_u64(), Some(8));

    let patch = json!({ "retrieval": { "top_k": 12 }, "ui": { "theme": "dark" } });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/settings")
                .header("content-type", "application/json")
                .body(Body::from(patch.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = read_body(resp.into_body()).await;
    assert_eq!(v["retrieval"]["top_k"].as_u64(), Some(12));
    assert_eq!(v["ui"]["theme"], "dark");
}

#[tokio::test]
async fn saved_queries_crud_and_run() {
    let app = build_router(make_state());

    let create = json!({ "name": "RAG basics", "query": "what is RAG", "top_k": 4 });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/queries")
                .header("content-type", "application/json")
                .body(Body::from(create.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = read_body(resp.into_body()).await;
    let id = v["id"].as_u64().unwrap();
    assert_eq!(v["name"], "RAG basics");

    // List.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/queries")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let v = read_body(resp.into_body()).await;
    assert_eq!(v["queries"].as_array().unwrap().len(), 1);

    // Run (EchoModel returns deterministic answer).
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/queries/{id}/run"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = read_body(resp.into_body()).await;
    assert!(v["answer"].as_str().unwrap().starts_with("[echo]"));

    // Delete.
    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/queries/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn subgraph_returns_concepts() {
    let state = make_state();
    // Seed two concepts so the subgraph isn't empty.
    state
        .graph
        .upsert_concept(ontology_graph::Concept::new(Default::default(), "Topic", "A"))
        .unwrap();
    state
        .graph
        .upsert_concept(ontology_graph::Concept::new(Default::default(), "Topic", "B"))
        .unwrap();
    let app = build_router(state);

    let body = json!({ "limit": 50, "expansion_depth": 1 });
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/subgraph")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = read_body(resp.into_body()).await;
    assert_eq!(v["subgraph"]["concepts"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn export_jsonl_streams() {
    let state = make_state();
    state
        .graph
        .upsert_concept(ontology_graph::Concept::new(Default::default(), "Topic", "A"))
        .unwrap();
    let app = build_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/export?format=jsonl")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(text.contains("\"ontology\""), "export body:\n{text}");
    assert!(text.contains("\"concept\""));
}

#[tokio::test]
async fn stats_history_grows_after_calls() {
    let app = build_router(make_state());

    // /stats records a sample.
    for _ in 0..2 {
        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/stats")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
    }
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/stats/history")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let v = read_body(resp.into_body()).await;
    // The 60s dedupe means we'll have exactly one sample for two quick calls.
    assert!(v["samples"].as_array().unwrap().len() >= 1);
}


#[tokio::test]
async fn openapi_spec_endpoint_returns_valid_document() {
    let app = build_router(make_state());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = read_body(resp.into_body()).await;
    assert_eq!(v["openapi"], "3.0.3");
    assert!(v["info"]["title"].as_str().unwrap().contains("ai-ontology"));
    // A handful of routes we expect to be documented.
    let paths = v["paths"].as_object().unwrap();
    for p in [
        "/healthz",
        "/stats",
        "/concepts",
        "/relations",
        "/relations/{id}",
        "/rules",
        "/rules/{id}",
        "/actions",
        "/actions/{id}",
        "/ask",
    ] {
        assert!(paths.contains_key(p), "missing path in spec: {p}");
    }
}

#[tokio::test]
async fn swagger_ui_page_is_served() {
    let app = build_router(make_state());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/docs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(ct.starts_with("text/html"), "got content-type: {ct}");
    let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    let body = std::str::from_utf8(&bytes).unwrap();
    assert!(body.contains("SwaggerUIBundle"));
}

#[tokio::test]
async fn openapi_and_docs_are_unauthenticated() {
    let app = build_router_with_auth(make_state(), Some("s3cret".into()));
    for path in ["/openapi.json", "/docs"] {
        let resp = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "auth-gated: {path}");
    }
}
