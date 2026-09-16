// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Edge cases of the two bulk paths, `POST /concepts/delete` and
//! `POST /reset` (`STORAGE.md` R8: nothing reaches memory that the store
//! refused, and nothing reaches the store that changes nothing).

mod hardening_common;

use hardening_common::*;
use http::StatusCode;
use ontology_graph::{ConceptId, OntologyGraph};
use ontology_storage::{RecordKind, Store};
use serde_json::json;

/// An empty selection is a 200 with zero counts — and costs no barrier.
#[tokio::test]
async fn bulk_delete_with_no_ids_is_a_no_op_that_never_reaches_the_store() {
    let (app, graph, store) = flaky_app();
    create_topic(&app, "A").await;
    let before = fingerprint(&graph);
    let (appends, batches) = (store.append_calls(), store.batch_calls());

    let (st, v) = call(&app, "POST", "/concepts/delete", Some(json!({ "ids": [] }))).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v, json!({ "deleted": 0, "relations": 0, "missing": [] }));
    assert_eq!(store.append_calls(), appends);
    assert_eq!(
        store.batch_calls(),
        batches,
        "no barrier for an empty selection"
    );
    assert_eq!(fingerprint(&graph), before);
}

/// The same id repeated in `ids` is one concept: one tombstone, `deleted: 1`.
#[tokio::test]
async fn bulk_delete_counts_a_duplicated_id_once() {
    let (app, graph, store) = flaky_app();
    let a = create_topic(&app, "A").await;
    create_topic(&app, "B").await;
    let records_before = store.records().len();

    let (st, v) = call(
        &app,
        "POST",
        "/concepts/delete",
        Some(json!({ "ids": [a, a, a] })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v, json!({ "deleted": 1, "relations": 0, "missing": [] }));
    assert_eq!(graph.concept_count(), 1);
    let tail = &store.records()[records_before..];
    assert_eq!(tail.len(), 1, "exactly one record journaled: {tail:?}");
    assert!(matches!(tail[0].kind, RecordKind::DeleteConcept(ConceptId(id)) if id == a));
}

/// A selection made only of unknown ids is not an error: they are listed
/// under `missing` (sorted, deduplicated) and the store is not written.
#[tokio::test]
async fn bulk_delete_of_only_unknown_ids_reports_them_and_journals_nothing() {
    let (app, graph, store) = flaky_app();
    create_topic(&app, "A").await;
    let before = fingerprint(&graph);
    let batches = store.batch_calls();

    let (st, v) = call(
        &app,
        "POST",
        "/concepts/delete",
        Some(json!({ "ids": [1000, 999, 1000] })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(
        v,
        json!({ "deleted": 0, "relations": 0, "missing": [999, 1000] })
    );
    assert_eq!(store.batch_calls(), batches);
    assert_eq!(fingerprint(&graph), before);
}

/// Pins the current semantics: deleting a concept does not touch the rules
/// (`applies_to`) and actions (`subject`) that reference it — they keep a
/// dangling id, live and after a replay alike.
#[tokio::test]
async fn bulk_delete_leaves_rules_and_actions_pointing_at_the_removed_concept() {
    let (app, graph, store) = flaky_app();
    let a = create_topic(&app, "A").await;
    let rule_id = create_rule(&app, "r1", &[a]).await;
    let action_id = create_action(&app, "act1", a).await;

    let (st, v) = call(
        &app,
        "POST",
        "/concepts/delete",
        Some(json!({ "ids": [a] })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["deleted"], 1);
    assert_eq!(graph.concept_count(), 0);

    let (st, r) = get(&app, &format!("/rules/{rule_id}")).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(r["applies_to"], json!([a]), "rule keeps the dangling id");
    let (st, act) = get(&app, &format!("/actions/{action_id}")).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(act["subject"], a, "action keeps the dangling subject");

    // Replay agrees with the live graph.
    let replayed = OntologyGraph::with_arc(ontology());
    store.load_into(&replayed).await.unwrap();
    assert_eq!(replayed.concept_count(), 0);
    assert_eq!(replayed.rule_count(), 1);
    assert_eq!(replayed.action_count(), 1);
}

/// R8: when the one barrier fails, every concept of the selection and every
/// incident relation are still there, generation counters included.
#[tokio::test]
async fn bulk_delete_with_a_failing_store_is_a_500_that_changes_nothing() {
    let (app, graph, store) = flaky_app();
    let a = create_topic(&app, "A").await;
    let b = create_topic(&app, "B").await;
    relate(&app, "related_to", a, b).await;
    let before = fingerprint(&graph);
    let written = store.records_written();
    store.set_failing(true);

    let (st, v) = call(
        &app,
        "POST",
        "/concepts/delete",
        Some(json!({ "ids": [a, b] })),
    )
    .await;
    assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR, "{v}");
    assert!(error_text(&v).starts_with("store:"), "{v}");
    assert_eq!(fingerprint(&graph), before);
    assert_eq!(store.records_written(), written);
    assert_eq!(graph.get_concept(ConceptId(a)).unwrap().name, "A");

    store.set_failing(false);
    let (st, v) = call(
        &app,
        "POST",
        "/concepts/delete",
        Some(json!({ "ids": [a, b] })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v, json!({ "deleted": 2, "relations": 2, "missing": [] }));
}

/// The body is validated by the JSON extractor before the handler runs:
/// invalid JSON is 400, a well-formed body of the wrong shape is 422, a
/// missing content type is 415 — and none of them reaches the store.
#[tokio::test]
async fn bulk_delete_rejects_a_malformed_body_before_touching_anything() {
    let (app, graph, store) = flaky_app();
    create_topic(&app, "A").await;
    let before = fingerprint(&graph);

    let send_raw = |body: &'static str, content_type: Option<&'static str>| {
        let app = app.clone();
        async move {
            let mut req = http::Request::builder()
                .method("POST")
                .uri("/concepts/delete");
            if let Some(ct) = content_type {
                req = req.header("content-type", ct);
            }
            let (st, _, _) = send(&app, req.body(axum::body::Body::from(body)).unwrap()).await;
            st
        }
    };
    assert_eq!(
        send_raw("{\"ids\": [1", Some("application/json")).await,
        StatusCode::BAD_REQUEST,
        "truncated JSON"
    );
    assert_eq!(
        send_raw("{\"ids\": \"1\"}", Some("application/json")).await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "wrong type for ids"
    );
    assert_eq!(
        send_raw("{}", Some("application/json")).await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "ids is required"
    );
    assert_eq!(
        send_raw("{\"ids\": [1]}", None).await,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "no content type"
    );
    assert_eq!(store.append_calls(), 1, "only the seed reached the store");
    assert_eq!(store.batch_calls(), 0);
    assert_eq!(fingerprint(&graph), before);
}

/// `POST /reset` twice is two 204s; afterwards the file registry, the stats
/// history, every count, the schema and the store are empty.
#[tokio::test]
async fn reset_is_idempotent_and_clears_files_history_and_store() {
    let (app, graph, store) = flaky_app();
    let a = create_topic(&app, "A").await;
    let b = create_topic(&app, "B").await;
    relate(&app, "related_to", a, b).await;
    create_rule(&app, "r1", &[a]).await;
    create_action(&app, "act1", a).await;
    let (st, v) = upload(
        &app,
        "jsonl",
        None,
        "seed.jsonl",
        br#"{"kind":"concept","id":0,"concept_type":"Topic","name":"C"}"#,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (_, files) = get(&app, "/files").await;
    assert_eq!(files["files"].as_array().unwrap().len(), 1);
    let (_, _) = get(&app, "/stats").await; // records a history sample
    let (_, history) = get(&app, "/stats/history").await;
    assert_eq!(history["samples"].as_array().unwrap().len(), 1);
    assert!(!store.records().is_empty());

    for _ in 0..2 {
        let (st, v) = call(&app, "POST", "/reset", None).await;
        assert_eq!(st, StatusCode::NO_CONTENT, "{v}");
    }

    let (st, files) = get(&app, "/files").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(files["files"], json!([]));
    let (_, history) = get(&app, "/stats/history").await;
    assert_eq!(history["samples"], json!([]));
    let (_, stats) = get(&app, "/stats").await;
    for key in [
        "concepts",
        "relations",
        "rules",
        "actions",
        "concept_types",
        "relation_types",
        "rule_types",
        "action_types",
    ] {
        assert_eq!(stats[key], 0, "{key} not reset: {stats}");
    }
    let (_, onto) = get(&app, "/ontology").await;
    assert_eq!(onto["concept_types"], json!({}));
    assert_eq!(onto["relation_types"], json!({}));
    assert_eq!(onto["rule_types"], json!({}));
    assert_eq!(onto["action_types"], json!({}));
    assert!(store.records().is_empty(), "store emptied");
    assert_eq!(graph.concept_count(), 0);
}

/// Ids of every family restart at 1 after a reset, not just concepts.
#[tokio::test]
async fn reset_restarts_every_id_family_at_one() {
    let (app, _graph, _store) = flaky_app();
    let a = create_topic(&app, "A").await;
    let b = create_topic(&app, "B").await;
    relate(&app, "related_to", a, b).await;
    create_rule(&app, "r1", &[a]).await;
    create_action(&app, "act1", a).await;

    let (st, _) = call(&app, "POST", "/reset", None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, v) = call(
        &app,
        "PUT",
        "/ontology",
        Some(serde_json::to_value(ontology()).unwrap()),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");

    assert_eq!(create_topic(&app, "A").await, 1);
    assert_eq!(create_topic(&app, "B").await, 2);
    assert_eq!(relate(&app, "owned_by", 1, 2).await, 1);
    assert_eq!(create_rule(&app, "r1", &[1]).await, 1);
    assert_eq!(create_action(&app, "act1", 1).await, 1);
}

/// R8 for the reset: the store is emptied first; if that fails, the live
/// graph, its schema and the journal are all exactly as they were.
#[tokio::test]
async fn reset_with_a_failing_store_is_a_500_that_changes_nothing() {
    let (app, graph, store) = flaky_app();
    let a = create_topic(&app, "A").await;
    let b = create_topic(&app, "B").await;
    relate(&app, "related_to", a, b).await;
    let before = fingerprint(&graph);
    let records = store.records().len();
    store.set_failing(true);

    let (st, v) = call(&app, "POST", "/reset", None).await;
    assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR, "{v}");
    assert_eq!(fingerprint(&graph), before);
    assert_eq!(store.records().len(), records, "journal untouched");
    let (_, stats) = get(&app, "/stats").await;
    assert_eq!(stats["concepts"], 2);
    assert_eq!(stats["concept_types"], 2);

    store.set_failing(false);
    let (st, _) = call(&app, "POST", "/reset", None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert_eq!(graph.concept_count(), 0);
}
