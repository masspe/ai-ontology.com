// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Write-ahead ordering of every mutating endpoint (`STORAGE.md` R8): the
//! record is durable *before* the in-memory graph changes, so a store
//! failure leaves the graph — and its generation counters — untouched; a
//! cascade costs one durability barrier; and a `FileStore` round-trips
//! through a restart.

use axum::body::{to_bytes, Body};
use http::{Request, StatusCode};
use ontology_graph::{ActionType, ConceptType, Ontology, OntologyGraph, RelationType, RuleType};
use ontology_index::HybridIndex;
use ontology_rag::{EchoModel, RagPipeline};
use ontology_server::{build_router, AppState};
use ontology_storage::{FileStore, FlakyStore, SegmentStore, Store};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

fn ontology() -> Ontology {
    let mut o = Ontology::new();
    o.add_concept_type(ConceptType {
        name: "Topic".into(),
        description: "topic".into(),
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

fn state_with(store: Arc<dyn Store>, graph: Arc<OntologyGraph>) -> AppState {
    let index = Arc::new(HybridIndex::with_default_embedder(graph.clone()));
    let pipeline = Arc::new(RagPipeline::new(index.clone(), Arc::new(EchoModel)));
    AppState::new(graph, index, store, pipeline)
}

async fn call(
    app: &axum::Router,
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
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    (status, value)
}

fn topic(name: &str) -> Value {
    json!({ "id": 0, "concept_type": "Topic", "name": name, "description": "", "properties": {} })
}

/// Snapshot of everything a failed write must leave alone.
#[derive(Debug, PartialEq)]
struct Fingerprint {
    concepts: usize,
    relations: usize,
    rules: usize,
    actions: usize,
    concepts_gen: u64,
    relations_gen: u64,
    ontology_types: usize,
}

fn fingerprint(g: &OntologyGraph) -> Fingerprint {
    Fingerprint {
        concepts: g.concept_count(),
        relations: g.relation_count(),
        rules: g.rule_count(),
        actions: g.action_count(),
        concepts_gen: g.concepts_generation(),
        relations_gen: g.relations_generation(),
        ontology_types: g.ontology().concept_types.len(),
    }
}

#[tokio::test]
async fn a_failing_store_leaves_the_graph_untouched_on_every_mutating_endpoint() {
    let store = Arc::new(FlakyStore::new());
    let graph = OntologyGraph::with_arc(ontology());
    let app = build_router(state_with(store.clone(), graph.clone()));

    // Seed while the store works: two topics, one relation, one rule, one action.
    let (st, v) = call(&app, "POST", "/concepts", Some(topic("A"))).await;
    assert_eq!(st, StatusCode::OK);
    let a = v["id"].as_u64().unwrap();
    let (st, v) = call(&app, "POST", "/concepts", Some(topic("B"))).await;
    assert_eq!(st, StatusCode::OK);
    let b = v["id"].as_u64().unwrap();
    let (st, v) = call(
        &app,
        "POST",
        "/relations",
        Some(
            json!({ "id": 0, "relation_type": "related_to", "source": a, "target": b,
                     "weight": 1.0, "properties": {} }),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let rel = v["id"].as_u64().unwrap();
    let (st, v) = call(
        &app,
        "POST",
        "/rules",
        Some(
            json!({ "id": 0, "rule_type": "must_review", "name": "r1", "when": "", "then": "",
                     "applies_to": [a], "strict": false, "description": "", "properties": {} }),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let rule = v["id"].as_u64().unwrap();
    let (st, v) = call(
        &app,
        "POST",
        "/actions",
        Some(
            json!({ "id": 0, "action_type": "archive", "name": "act1", "subject": a,
                     "object": null, "parameters": {}, "effect": "", "description": "" }),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let action = v["id"].as_u64().unwrap();
    let written_before = store.records_written();

    // From here on, every append fails.
    store.set_failing(true);
    let before = fingerprint(&graph);

    let attempts: Vec<(&str, String, Option<Value>)> = vec![
        ("POST", "/concepts".into(), Some(topic("C"))),
        (
            "PATCH",
            format!("/concepts/{a}"),
            Some(json!({ "name": "A-renamed" })),
        ),
        ("DELETE", format!("/concepts/{a}"), None),
        (
            "POST",
            "/relations".into(),
            Some(
                json!({ "id": 0, "relation_type": "related_to", "source": b, "target": a,
                         "weight": 0.5, "properties": {} }),
            ),
        ),
        (
            "PATCH",
            format!("/relations/{rel}"),
            Some(json!({ "weight": 0.1 })),
        ),
        ("DELETE", format!("/relations/{rel}"), None),
        (
            "POST",
            "/rules".into(),
            Some(
                json!({ "id": 0, "rule_type": "must_review", "name": "r2", "when": "", "then": "",
                         "applies_to": [], "strict": false, "description": "", "properties": {} }),
            ),
        ),
        (
            "PATCH",
            format!("/rules/{rule}"),
            Some(json!({ "name": "r1-renamed" })),
        ),
        ("DELETE", format!("/rules/{rule}"), None),
        (
            "POST",
            "/actions".into(),
            Some(
                json!({ "id": 0, "action_type": "archive", "name": "act2", "subject": b,
                         "object": null, "parameters": {}, "effect": "", "description": "" }),
            ),
        ),
        (
            "PATCH",
            format!("/actions/{action}"),
            Some(json!({ "name": "act1-renamed" })),
        ),
        ("DELETE", format!("/actions/{action}"), None),
        (
            "PUT",
            "/ontology".into(),
            Some({
                // A schema change the graph accepts (an extra type), so the
                // failing store is what is exercised, not the schema guard.
                let mut o = ontology();
                o.add_concept_type(ConceptType {
                    name: "Extra".into(),
                    ..Default::default()
                });
                serde_json::to_value(o).unwrap()
            }),
        ),
    ];

    for (method, uri, body) in attempts {
        let (st, v) = call(&app, method, &uri, body).await;
        assert_eq!(
            st,
            StatusCode::INTERNAL_SERVER_ERROR,
            "{method} {uri}: expected 500 from the failing store, got {st} {v}"
        );
        assert_eq!(
            fingerprint(&graph),
            before,
            "{method} {uri}: graph changed although the store rejected the write"
        );
    }
    assert_eq!(
        store.records_written(),
        written_before,
        "nothing reached the store"
    );

    // Names and payloads are also intact, not just counts.
    assert_eq!(
        graph
            .get_concept(ontology_graph::ConceptId(a))
            .unwrap()
            .name,
        "A"
    );
    assert_eq!(
        graph
            .get_relation(ontology_graph::RelationId(rel))
            .unwrap()
            .weight,
        1.0
    );
    assert_eq!(
        graph.get_rule(ontology_graph::RuleId(rule)).unwrap().name,
        "r1"
    );
    assert_eq!(
        graph
            .get_action(ontology_graph::ActionId(action))
            .unwrap()
            .name,
        "act1"
    );

    // And once the store recovers, the same requests succeed.
    store.set_failing(false);
    let (st, _) = call(&app, "POST", "/concepts", Some(topic("C"))).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(graph.concept_count(), 3);
}

#[tokio::test]
async fn unknown_ids_are_404_without_touching_the_store() {
    let store = Arc::new(FlakyStore::new());
    let graph = OntologyGraph::with_arc(ontology());
    let app = build_router(state_with(store.clone(), graph.clone()));

    for (method, uri) in [
        ("DELETE", "/concepts/4242"),
        ("DELETE", "/relations/4242"),
        ("DELETE", "/rules/4242"),
        ("DELETE", "/actions/4242"),
        ("PATCH", "/concepts/4242"),
        ("PATCH", "/relations/4242"),
    ] {
        let body = if method == "PATCH" {
            Some(json!({}))
        } else {
            None
        };
        let (st, _) = call(&app, method, uri, body).await;
        assert!(
            st == StatusCode::NOT_FOUND || st == StatusCode::BAD_REQUEST,
            "{method} {uri}: got {st}"
        );
    }
    assert_eq!(store.append_calls(), 0);
    assert_eq!(store.batch_calls(), 0);
}

#[tokio::test]
async fn deleting_a_concept_journals_its_cascade_as_one_batch() {
    let store = Arc::new(FlakyStore::new());
    let graph = OntologyGraph::with_arc(ontology());
    let app = build_router(state_with(store.clone(), graph.clone()));

    let (_, v) = call(&app, "POST", "/concepts", Some(topic("A"))).await;
    let a = v["id"].as_u64().unwrap();
    let (_, v) = call(&app, "POST", "/concepts", Some(topic("B"))).await;
    let b = v["id"].as_u64().unwrap();
    let (_, v) = call(&app, "POST", "/concepts", Some(topic("C"))).await;
    let c = v["id"].as_u64().unwrap();
    for target in [b, c] {
        let (st, _) = call(
            &app,
            "POST",
            "/relations",
            Some(
                json!({ "id": 0, "relation_type": "related_to", "source": a, "target": target,
                         "weight": 1.0, "properties": {} }),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
    }
    // Symmetric: 2 logical edges, 4 stored relations, all incident to A.
    assert_eq!(graph.relation_count(), 4);
    let batches_before = store.batch_calls();
    let written_before = store.records_written();

    let (st, _) = call(&app, "DELETE", &format!("/concepts/{a}"), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert_eq!(
        store.batch_calls(),
        batches_before + 1,
        "one barrier for the cascade"
    );
    assert_eq!(
        store.records_written(),
        written_before + 1 + 4,
        "DeleteConcept + 4 DeleteRelation records"
    );
    assert_eq!(graph.concept_count(), 2);
    assert_eq!(graph.relation_count(), 0);

    // The journal replays to the same end state (the schema was given at
    // construction here, not journaled, so the fresh graph starts from it).
    let fresh = OntologyGraph::with_arc(ontology());
    store.load_into(&fresh).await.unwrap();
    assert_eq!(fresh.concept_count(), 2);
    assert_eq!(fresh.relation_count(), 0);
}

#[tokio::test]
async fn file_store_survives_a_restart_after_http_writes() {
    let dir = std::env::temp_dir().join(format!(
        "ontology-http-restart-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();

    let (a, b, rel);
    {
        let store = Arc::new(FileStore::open(&dir).await.unwrap());
        let graph = OntologyGraph::with_arc(Ontology::new());
        store.load_into(&graph).await.unwrap();
        let app = build_router(state_with(store.clone(), graph.clone()));

        let (st, _) = call(
            &app,
            "PUT",
            "/ontology",
            Some(serde_json::to_value(ontology()).unwrap()),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        let (_, v) = call(&app, "POST", "/concepts", Some(topic("A"))).await;
        a = v["id"].as_u64().unwrap();
        let (_, v) = call(&app, "POST", "/concepts", Some(topic("B"))).await;
        b = v["id"].as_u64().unwrap();
        let (st, v) = call(
            &app,
            "POST",
            "/relations",
            Some(
                json!({ "id": 0, "relation_type": "related_to", "source": a, "target": b,
                         "weight": 0.7, "properties": {} }),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        rel = v["id"].as_u64().unwrap();
        let (st, _) = call(
            &app,
            "PATCH",
            &format!("/concepts/{b}"),
            Some(json!({ "name": "B2" })),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        // ontology + 2 concepts + 1 relation + 1 update = 5 syncs
        assert_eq!(store.sync_count(), 5);
    }

    // "Restart": reopen the directory and replay.
    let store = FileStore::open(&dir).await.unwrap();
    let graph = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&graph).await.unwrap();
    assert_eq!(graph.concept_count(), 2);
    assert_eq!(
        graph.relation_count(),
        2,
        "symmetric inverse re-materialized"
    );
    assert_eq!(
        graph
            .get_concept(ontology_graph::ConceptId(a))
            .unwrap()
            .name,
        "A"
    );
    assert_eq!(
        graph
            .get_concept(ontology_graph::ConceptId(b))
            .unwrap()
            .name,
        "B2"
    );
    assert_eq!(
        graph
            .get_relation(ontology_graph::RelationId(rel))
            .unwrap()
            .weight,
        0.7
    );
    assert_eq!(graph.ontology().concept_types.len(), 1);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn segment_store_survives_a_restart_after_http_writes() {
    let dir = std::env::temp_dir().join(format!(
        "ontology-http-segstore-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();

    let (a, b, rel);
    {
        let store = Arc::new(SegmentStore::open(&dir).await.unwrap());
        let graph = OntologyGraph::with_arc(Ontology::new());
        store.load_into(&graph).await.unwrap();
        let app = build_router(state_with(store.clone(), graph.clone()));

        let (st, _) = call(
            &app,
            "PUT",
            "/ontology",
            Some(serde_json::to_value(ontology()).unwrap()),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        let (_, v) = call(&app, "POST", "/concepts", Some(topic("A"))).await;
        a = v["id"].as_u64().unwrap();
        let (_, v) = call(&app, "POST", "/concepts", Some(topic("B"))).await;
        b = v["id"].as_u64().unwrap();
        let (_, v) = call(&app, "POST", "/concepts", Some(topic("C"))).await;
        let c = v["id"].as_u64().unwrap();
        let (st, v) = call(
            &app,
            "POST",
            "/relations",
            Some(
                json!({ "id": 0, "relation_type": "related_to", "source": a, "target": b,
                         "weight": 0.7, "properties": {} }),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        rel = v["id"].as_u64().unwrap();
        let (st, _) = call(
            &app,
            "PATCH",
            &format!("/concepts/{b}"),
            Some(json!({ "name": "B2" })),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        let (st, _) = call(
            &app,
            "POST",
            "/rules",
            Some(
                json!({ "id": 0, "rule_type": "must_review", "name": "r1", "when": "", "then": "",
                         "applies_to": [a], "strict": false, "description": "", "properties": {} }),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        let (st, _) = call(&app, "DELETE", &format!("/concepts/{c}"), None).await;
        assert_eq!(st, StatusCode::NO_CONTENT);
        // ontology(meta) + 3 concepts + relation + update + rule(meta) + delete:
        // 8 batches, each touching exactly one stream -> 8 syncs.
        assert_eq!(store.sync_count(), 8);
        assert_eq!(store.record_count(), 8);
    }

    // "Restart": reopen the directory and replay.
    let store = SegmentStore::open(&dir).await.unwrap();
    let graph = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&graph).await.unwrap();
    assert_eq!(graph.concept_count(), 2);
    assert_eq!(
        graph.relation_count(),
        2,
        "symmetric inverse re-materialized"
    );
    assert_eq!(graph.rule_count(), 1);
    assert_eq!(
        graph
            .get_concept(ontology_graph::ConceptId(a))
            .unwrap()
            .name,
        "A"
    );
    assert_eq!(
        graph
            .get_concept(ontology_graph::ConceptId(b))
            .unwrap()
            .name,
        "B2"
    );
    assert_eq!(
        graph
            .get_relation(ontology_graph::RelationId(rel))
            .unwrap()
            .weight,
        0.7
    );
    assert_eq!(graph.ontology().concept_types.len(), 1);
    assert_eq!(store.next_seq(), 9);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn ingest_apply_rolls_back_type_declarations_when_the_store_fails() {
    // Type declarations are applied to the live schema first and journaled
    // as one Ontology record; if that record cannot be written, the schema
    // must be restored — otherwise a later concept of that type would be
    // accepted, journaled, and refused on replay.
    let store = Arc::new(FlakyStore::new());
    let graph = OntologyGraph::with_arc(ontology());
    let app = build_router(state_with(store.clone(), graph.clone()));
    let before = graph.ontology();
    store.set_failing(true);

    let body = json!({
        "proposal": { "concept_types": [ { "client_ref": "t1", "name": "Widget" } ] },
        "decisions": [ { "client_ref": "t1", "action": "create_new" } ],
        "strict": true
    });
    let (st, v) = call(&app, "POST", "/ingest/apply", Some(body.clone())).await;
    assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR, "{v}");
    assert!(!graph.ontology().concept_types.contains_key("Widget"));
    assert_eq!(
        serde_json::to_value(graph.ontology()).unwrap(),
        serde_json::to_value(&before).unwrap()
    );

    // A concept of the un-journaled type is therefore refused, not accepted.
    let (st, _) = call(
        &app,
        "POST",
        "/concepts",
        Some(json!({ "id": 0, "concept_type": "Widget", "name": "w", "description": "", "properties": {} })),
    )
    .await;
    assert_ne!(st, StatusCode::OK);

    // With the store back, the same apply journals the schema and succeeds.
    store.set_failing(false);
    let (st, v) = call(&app, "POST", "/ingest/apply", Some(body)).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(graph.ontology().concept_types.contains_key("Widget"));
    assert_eq!(store.records_written(), 1, "one Ontology record");
}

#[tokio::test]
async fn a_refused_schema_is_never_journaled() {
    let store = Arc::new(FlakyStore::new());
    let graph = OntologyGraph::with_arc(ontology());
    let app = build_router(state_with(store.clone(), graph.clone()));
    let (st, _) = call(&app, "POST", "/concepts", Some(topic("A"))).await;
    assert_eq!(st, StatusCode::OK);
    let written = store.records_written();

    // Invalid domain identifier.
    let mut bad = ontology();
    bad.concept_type_mut("Topic").ns = Some("Not Valid".into());
    let (st, v) = call(
        &app,
        "PUT",
        "/ontology",
        Some(serde_json::to_value(&bad).unwrap()),
    )
    .await;
    assert!(st.is_client_error(), "{st} {v}");
    // Dropping a type that has an instance.
    let mut bad = ontology();
    bad.concept_types.remove("Topic");
    let (st, v) = call(
        &app,
        "PUT",
        "/ontology",
        Some(serde_json::to_value(&bad).unwrap()),
    )
    .await;
    assert!(st.is_client_error(), "{st} {v}");
    assert_eq!(
        store.records_written(),
        written,
        "nothing reached the store"
    );
    assert_eq!(graph.ontology().concept_types.len(), 1);

    // A valid change still goes through, journaled once.
    let mut good = ontology();
    good.add_concept_type(ConceptType {
        name: "Tag".into(),
        ..Default::default()
    });
    let (st, v) = call(
        &app,
        "PUT",
        "/ontology",
        Some(serde_json::to_value(&good).unwrap()),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(store.records_written(), written + 1);
}
