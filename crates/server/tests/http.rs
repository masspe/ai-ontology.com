use axum::body::{to_bytes, Body};
use http::{Request, StatusCode};
use ontology_graph::{ConceptType, Ontology, OntologyGraph, RelationType};
use ontology_index::HybridIndex;
use ontology_rag::{EchoModel, RagPipeline};
use ontology_server::{build_router, AppState};
use ontology_storage::{MemoryStore, Store};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

fn ontology() -> Ontology {
    let mut o = Ontology::new();
    o.add_concept_type(ConceptType {
        name: "Topic".into(), parent: None, properties: None,
        description: "topic".into(),
    });
    o.add_relation_type(RelationType {
        name: "related_to".into(), domain: "Topic".into(), range: "Topic".into(),
        cardinality: Default::default(), symmetric: true, description: "".into(),
    }).unwrap();
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

    let app = build_router(AppState { graph: graph.clone(), index, store, pipeline });

    // Create concept.
    let body = json!({ "id": 0, "concept_type": "Topic", "name": "RAG",
        "description": "retrieval augmented generation",
        "properties": {} });
    let resp = app.clone()
        .oneshot(Request::builder()
            .method("POST").uri("/concepts")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string())).unwrap())
        .await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let id = read_body(resp.into_body()).await["id"].as_u64().unwrap();
    assert!(id > 0);

    // Retrieve.
    let req_body = json!({ "query": "retrieval augmented generation",
        "top_k": 4, "lexical_weight": 0.5, "expansion": {"max_depth": 1} });
    let resp = app.clone()
        .oneshot(Request::builder()
            .method("POST").uri("/retrieve")
            .header("content-type", "application/json")
            .body(Body::from(req_body.to_string())).unwrap())
        .await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = read_body(resp.into_body()).await;
    assert_eq!(v["scored"][0]["id"].as_u64(), Some(id));

    // Stats.
    let resp = app.clone()
        .oneshot(Request::builder().method("GET").uri("/stats")
            .body(Body::empty()).unwrap())
        .await.unwrap();
    let v = read_body(resp.into_body()).await;
    assert_eq!(v["concepts"].as_u64(), Some(1));

    // Delete.
    let resp = app.clone()
        .oneshot(Request::builder()
            .method("DELETE").uri(&format!("/concepts/{}", id))
            .body(Body::empty()).unwrap())
        .await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert_eq!(graph.concept_count(), 0);
}
