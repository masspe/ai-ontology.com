use axum::body::{to_bytes, Body};
use http::{Request, StatusCode};
use ontology_graph::{ConceptType, Ontology, OntologyGraph, RelationType};
use ontology_index::HybridIndex;
use ontology_rag::{EchoModel, RagPipeline};
use ontology_server::{
    build_router, build_router_with_auth, build_router_with_config, AppState, RateLimit,
    RouterConfig,
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
    AppState {
        graph,
        index,
        store,
        pipeline,
    }
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

    let app = build_router(AppState {
        graph: graph.clone(),
        index,
        store,
        pipeline,
    });

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
