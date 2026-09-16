// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Shared harness for the `hardening_*` integration tests: one small but
//! complete schema (two concept types, a symmetric and a many-to-one
//! relation type, one rule type, one action type), an `AppState` wired on
//! any [`Store`], and request helpers that return `(status, json)`.
//!
//! Compiled into every `hardening_*` binary; helpers a given binary does
//! not use are expected, hence the `dead_code` allowance.

#![allow(dead_code)]

use axum::body::{to_bytes, Body};
use axum::Router;
use http::{HeaderMap, Request, StatusCode};
use ontology_graph::{
    ActionType, Cardinality, ConceptType, Ontology, OntologyGraph, RelationType, RuleType,
};
use ontology_index::HybridIndex;
use ontology_rag::{EchoModel, LanguageModel, RagPipeline};
use ontology_server::{build_router, AppState};
use ontology_storage::{FlakyStore, Store};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

/// `Topic` and `Tag` concept types; `related_to` (Topic–Topic, symmetric),
/// `owned_by` (Topic→Topic, many-to-one); rule type `must_review`; action
/// type `archive`.
pub fn ontology() -> Ontology {
    let mut o = Ontology::new();
    o.add_concept_type(ConceptType {
        name: "Topic".into(),
        description: "topic".into(),
        ..Default::default()
    });
    o.add_concept_type(ConceptType {
        name: "Tag".into(),
        description: "tag".into(),
        ..Default::default()
    });
    o.add_relation_type(RelationType {
        name: "related_to".into(),
        domain: "Topic".into(),
        range: "Topic".into(),
        symmetric: true,
        ..Default::default()
    })
    .unwrap();
    o.add_relation_type(RelationType {
        name: "owned_by".into(),
        domain: "Topic".into(),
        range: "Topic".into(),
        cardinality: Cardinality::ManyToOne,
        ..Default::default()
    })
    .unwrap();
    o.add_rule_type(RuleType {
        name: "must_review".into(),
        when: String::new(),
        then: String::new(),
        applies_to: vec!["Topic".into()],
        strict: false,
        description: String::new(),
    })
    .unwrap();
    o.add_action_type(ActionType {
        name: "archive".into(),
        subject: "Topic".into(),
        object: None,
        parameters: Vec::new(),
        effect: String::new(),
        description: String::new(),
    })
    .unwrap();
    o
}

pub fn state_with(store: Arc<dyn Store>, graph: Arc<OntologyGraph>) -> AppState {
    state_with_llm(store, graph, Arc::new(EchoModel))
}

pub fn state_with_llm(
    store: Arc<dyn Store>,
    graph: Arc<OntologyGraph>,
    llm: Arc<dyn LanguageModel>,
) -> AppState {
    let index = Arc::new(HybridIndex::with_default_embedder(graph.clone()));
    let pipeline = Arc::new(RagPipeline::new(index.clone(), llm));
    AppState::new(graph, index, store, pipeline)
}

/// Router over a fresh [`FlakyStore`] and the test schema.
pub fn flaky_app() -> (Router, Arc<OntologyGraph>, Arc<FlakyStore>) {
    let store = Arc::new(FlakyStore::new());
    let graph = OntologyGraph::with_arc(ontology());
    let app = build_router(state_with(store.clone(), graph.clone()));
    (app, graph, store)
}

/// Send a raw request; the body cap is generous so a full 5 000-concept
/// page fits.
pub async fn send(app: &Router, req: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = to_bytes(resp.into_body(), 16 << 20).await.unwrap().to_vec();
    (status, headers, bytes)
}

/// JSON body, or the raw text as a `Value::String` when it is not JSON,
/// or `Null` when empty.
pub fn parse(bytes: &[u8]) -> Value {
    if bytes.is_empty() {
        return Value::Null;
    }
    serde_json::from_slice(bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(bytes).into_owned()))
}

pub async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            req = req.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let (status, _, bytes) = send(app, req.body(body).unwrap()).await;
    (status, parse(&bytes))
}

pub async fn get(app: &Router, uri: &str) -> (StatusCode, Value) {
    call(app, "GET", uri, None).await
}

/// The `error` field of an error body, or the whole body as text.
pub fn error_text(v: &Value) -> String {
    v["error"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| v.to_string())
}

pub fn concept(ty: &str, name: &str) -> Value {
    json!({ "id": 0, "concept_type": ty, "name": name, "description": "", "properties": {} })
}

pub fn topic(name: &str) -> Value {
    concept("Topic", name)
}

pub async fn create(app: &Router, ty: &str, name: &str) -> u64 {
    let (st, v) = call(app, "POST", "/concepts", Some(concept(ty, name))).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    v["id"].as_u64().unwrap()
}

pub async fn create_topic(app: &Router, name: &str) -> u64 {
    create(app, "Topic", name).await
}

pub fn relation(rt: &str, a: u64, b: u64) -> Value {
    json!({ "id": 0, "relation_type": rt, "source": a, "target": b, "weight": 1.0, "properties": {} })
}

pub async fn relate(app: &Router, rt: &str, a: u64, b: u64) -> u64 {
    let (st, v) = call(app, "POST", "/relations", Some(relation(rt, a, b))).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    v["id"].as_u64().unwrap()
}

pub fn rule(name: &str, applies_to: &[u64]) -> Value {
    json!({ "id": 0, "rule_type": "must_review", "name": name, "when": "", "then": "",
            "applies_to": applies_to, "strict": false, "description": "", "properties": {} })
}

pub async fn create_rule(app: &Router, name: &str, applies_to: &[u64]) -> u64 {
    let (st, v) = call(app, "POST", "/rules", Some(rule(name, applies_to))).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    v["id"].as_u64().unwrap()
}

pub fn action(name: &str, subject: u64) -> Value {
    json!({ "id": 0, "action_type": "archive", "name": name, "subject": subject,
            "object": null, "parameters": {}, "effect": "", "description": "" })
}

pub async fn create_action(app: &Router, name: &str, subject: u64) -> u64 {
    let (st, v) = call(app, "POST", "/actions", Some(action(name, subject))).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    v["id"].as_u64().unwrap()
}

/// Multipart body: the given text fields, then (optionally) one `file`
/// part. Returns `(content-type, body)`.
pub fn multipart(fields: &[(&str, &str)], file: Option<(&str, &[u8])>) -> (String, Vec<u8>) {
    let boundary = "----hardening-boundary-7f3a9c";
    let mut body: Vec<u8> = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    if let Some((filename, bytes)) = file {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; \
                 filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

pub async fn post_multipart(
    app: &Router,
    uri: &str,
    fields: &[(&str, &str)],
    file: Option<(&str, &[u8])>,
) -> (StatusCode, Value) {
    let (ct, body) = multipart(fields, file);
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", ct)
        .body(Body::from(body))
        .unwrap();
    let (status, _, bytes) = send(app, req).await;
    (status, parse(&bytes))
}

/// `POST /upload` with `kind` and optional `concept_type`.
pub async fn upload(
    app: &Router,
    kind: &str,
    concept_type: Option<&str>,
    filename: &str,
    bytes: &[u8],
) -> (StatusCode, Value) {
    let mut fields: Vec<(&str, &str)> = vec![("kind", kind)];
    if let Some(ct) = concept_type {
        fields.push(("concept_type", ct));
    }
    post_multipart(app, "/upload", &fields, Some((filename, bytes))).await
}

/// Everything a refused write must leave alone: counts, generation
/// counters (query cache / ETag, `PERFORMANCE.md` §4.7) and the schema.
#[derive(Debug, PartialEq)]
pub struct Fingerprint {
    pub concepts: usize,
    pub relations: usize,
    pub rules: usize,
    pub actions: usize,
    pub concepts_gen: u64,
    pub relations_gen: u64,
    pub ontology: Value,
}

pub fn fingerprint(g: &OntologyGraph) -> Fingerprint {
    Fingerprint {
        concepts: g.concept_count(),
        relations: g.relation_count(),
        rules: g.rule_count(),
        actions: g.action_count(),
        concepts_gen: g.concepts_generation(),
        relations_gen: g.relations_generation(),
        ontology: serde_json::to_value(g.ontology()).unwrap(),
    }
}

/// Generation-free view of a graph, comparable across a restart.
pub fn shape(g: &OntologyGraph) -> Value {
    let mut concepts: Vec<(String, String)> = g
        .all_concepts()
        .into_iter()
        .map(|c| (c.concept_type, c.name))
        .collect();
    concepts.sort();
    let mut relations: Vec<(String, u64, u64)> = g
        .all_relations()
        .into_iter()
        .map(|r| (r.relation_type, r.source.0, r.target.0))
        .collect();
    relations.sort();
    json!({
        "concepts": concepts,
        "relations": relations,
        "rules": g.rule_count(),
        "actions": g.action_count(),
        "ontology": serde_json::to_value(g.ontology()).unwrap(),
    })
}
