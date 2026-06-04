// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// End-to-end integration test for the LLM-assisted ingest review flow.
//
// Wires a `FakeLlm` into the RAG pipeline so the harness is offline and
// deterministic, then drives:
//
//   1. `POST /ingest/analyze` with a multipart text upload — asserts the
//      returned proposal contains the canned concepts/relations and that
//      the `source.encoding` reflects what `decode_to_utf8` detected.
//   2. `POST /ingest/apply` with decisions accepting every item —
//      asserts that the graph now contains the new concept and relation
//      and that the report counts `created` correctly.
//   3. Re-running `/ingest/analyze` returns the same concept marked
//      `exists` since the previous apply step landed it in the graph.

use axum::body::{to_bytes, Body};
use http::{Request, StatusCode};
use ontology_graph::{ConceptType, Ontology, OntologyGraph, RelationType};
use ontology_index::HybridIndex;
use ontology_rag::{
    model::{LanguageModel, LlmRequest, LlmResponse, TokenUsage},
    LlmError, RagPipeline,
};
use ontology_server::{build_router, AppState};
use ontology_storage::{MemoryStore, Store};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

const CANNED_JSON: &str = r#"{
  "concept_types": [],
  "relation_types": [],
  "concepts": [
    {"client_ref":"c0","concept_type":"Party","name":"Acme","description":"buyer","confidence":0.9,"evidence":"Acme Corp purchased ..."},
    {"client_ref":"c1","concept_type":"Party","name":"Globex","description":"seller","confidence":0.85}
  ],
  "relations": [
    {"client_ref":"r0","relation_type":"buys_from","source_ref":"Party:Acme","target_ref":"Party:Globex","confidence":0.8}
  ],
  "rules": [],
  "actions": []
}"#;

#[derive(Debug, Clone, Default)]
struct FakeLlm;

#[async_trait::async_trait]
impl LanguageModel for FakeLlm {
    async fn generate(&self, _req: &LlmRequest) -> Result<LlmResponse, LlmError> {
        Ok(LlmResponse {
            content: CANNED_JSON.to_string(),
            model: "fake".into(),
            stop_reason: Some("end_turn".into()),
            usage: TokenUsage::default(),
        })
    }
}

fn ontology() -> Ontology {
    let mut o = Ontology::new();
    o.add_concept_type(ConceptType {
        name: "Party".into(),
        parent: None,
        properties: None,
        description: "trading party".into(),
        ..Default::default()
    });
    o.add_relation_type(RelationType {
        name: "buys_from".into(),
        domain: "Party".into(),
        range: "Party".into(),
        cardinality: Default::default(),
        symmetric: false,
        description: "".into(),
        ..Default::default()
    })
    .unwrap();
    o
}

fn make_state() -> AppState {
    make_state_with_store().0
}

/// Like [`make_state`] but also hands back the backing store so tests can
/// replay the WAL into a fresh graph and assert that applied items were
/// actually persisted (not just held in memory).
fn make_state_with_store() -> (AppState, Arc<dyn Store>) {
    let graph = OntologyGraph::with_arc(ontology());
    let index = Arc::new(HybridIndex::with_default_embedder(graph.clone()));
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let pipeline = Arc::new(RagPipeline::new(index.clone(), Arc::new(FakeLlm::default())));
    let state = AppState::new(graph, index, store.clone(), pipeline);
    (state, store)
}

/// Build a multipart body the dumb way — sufficient for our static fields.
fn multipart_body(file_name: &str, file_bytes: &[u8]) -> (String, Vec<u8>) {
    let boundary = "----testboundary12345";
    let mut body: Vec<u8> = Vec::new();
    let header = format!(
        "--{b}\r\n\
         Content-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\n\
         Content-Type: text/plain\r\n\r\n",
        b = boundary,
        name = file_name,
    );
    body.extend_from_slice(header.as_bytes());
    body.extend_from_slice(file_bytes);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());
    (format!("multipart/form-data; boundary={}", boundary), body)
}

async fn read_body(b: Body) -> Value {
    let bytes = to_bytes(b, 4 << 20).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn analyze_then_apply_round_trip() {
    let state = make_state();
    let app = build_router(state);

    // 1. Analyze
    let (ct, body) = multipart_body(
        "contract.txt",
        "Acme Corp purchased widgets from Globex.\n".as_bytes(),
    );
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/ingest/analyze")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let proposal = read_body(resp.into_body()).await;
    assert_eq!(proposal["concepts"].as_array().unwrap().len(), 2);
    assert_eq!(proposal["relations"].as_array().unwrap().len(), 1);
    // First pass: nothing in the graph yet → no `exists` conflicts.
    assert!(proposal["concepts"][0]["conflict"].is_null());
    // The relation's source_ref ("Party:Acme") resolves forward to a
    // proposal concept declared in the same batch → no DanglingRef.
    assert!(proposal["relations"][0]["conflict"].is_null());
    let encoding = proposal["source"]["encoding"].as_str().unwrap_or("");
    assert!(
        encoding.eq_ignore_ascii_case("UTF-8"),
        "expected UTF-8 detection, got {encoding}",
    );

    // 2. Apply with `create_new` for every item.
    let decisions: Vec<Value> = proposal["concepts"]
        .as_array()
        .unwrap()
        .iter()
        .chain(proposal["relations"].as_array().unwrap())
        .map(|item| {
            json!({ "client_ref": item["client_ref"], "action": "create_new" })
        })
        .collect();
    let payload = json!({
        "proposal": proposal,
        "decisions": decisions,
        "strict": false,
        "default_action": "skip"
    });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/ingest/apply")
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let report = read_body(resp.into_body()).await;
    // 2 concepts + 1 relation accepted → 3 created, 0 failed.
    assert_eq!(report["created"].as_u64().unwrap(), 3);
    assert_eq!(report["failed"].as_u64().unwrap(), 0);

    // 3. The concept now exists in the graph → next analyze flags it.
    let (ct, body) = multipart_body("contract2.txt", b"Acme again.");
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/ingest/analyze")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let p2 = read_body(resp.into_body()).await;
    let acme = p2["concepts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Acme")
        .expect("Acme present");
    assert_eq!(
        acme["conflict"]["kind"]["kind"].as_str().unwrap(),
        "exists",
        "Acme should be marked as already existing after first apply",
    );
}

/// Regression test: `POST /ingest/apply` must persist accepted items to the
/// store, not merely mutate the in-memory graph. Previously the handler wrote
/// nothing to the WAL, so concepts/relations vanished on restart even though
/// they showed up in the UI. We assert persistence by replaying the store into
/// a brand-new graph and checking the data is there.
#[tokio::test]
async fn apply_persists_to_store() {
    let (state, store) = make_state_with_store();
    let app = build_router(state);

    // Analyze to obtain a valid proposal.
    let (ct, body) = multipart_body(
        "contract.txt",
        "Acme Corp purchased widgets from Globex.\n".as_bytes(),
    );
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/ingest/analyze")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let proposal = read_body(resp.into_body()).await;

    // Apply, accepting every concept and relation.
    let decisions: Vec<Value> = proposal["concepts"]
        .as_array()
        .unwrap()
        .iter()
        .chain(proposal["relations"].as_array().unwrap())
        .map(|item| json!({ "client_ref": item["client_ref"], "action": "create_new" }))
        .collect();
    let payload = json!({
        "proposal": proposal,
        "decisions": decisions,
        "strict": false,
        "default_action": "skip"
    });
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/ingest/apply")
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let report = read_body(resp.into_body()).await;
    assert_eq!(report["created"].as_u64().unwrap(), 3);

    // Replay the store into a fresh graph — this is what a server restart does.
    let replayed = OntologyGraph::with_arc(ontology());
    store.load_into(&replayed).await.unwrap();

    let acme = replayed
        .find_by_name("Party", "Acme")
        .expect("Acme must survive a store replay");
    assert!(
        replayed.find_by_name("Party", "Globex").is_some(),
        "Globex must survive a store replay",
    );
    // The relation must persist too: Acme should have an outgoing edge.
    let rels = replayed.outgoing(acme);
    assert_eq!(
        rels.len(),
        1,
        "the buys_from relation must survive a store replay, got {rels:?}",
    );
}

#[tokio::test]
async fn analyze_decodes_windows_1252() {
    // "Café Münch" with é=0xE9, ü=0xFC, ' '=0x20 — pure Windows-1252.
    let win1252: &[u8] = &[
        b'C', b'a', b'f', 0xE9, b' ', b'M', 0xFC, b'n', b'c', b'h', b'\n',
    ];
    let state = make_state();
    let app = build_router(state);
    let (ct, body) = multipart_body("legacy.txt", win1252);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/ingest/analyze")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let proposal = read_body(resp.into_body()).await;
    let encoding = proposal["source"]["encoding"].as_str().unwrap_or("");
    // chardetng may classify as windows-1252 or ISO-8859-15 — both decode
    // the bytes to the same UTF-8. Just ensure we did *not* misclassify
    // as UTF-8 (which would have rendered replacement characters).
    assert!(
        !encoding.eq_ignore_ascii_case("UTF-8"),
        "Windows-1252 bytes should not be detected as UTF-8 (got {encoding})",
    );
}
