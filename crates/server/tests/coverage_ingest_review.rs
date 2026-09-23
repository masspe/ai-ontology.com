// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! `/ingest/analyze` and `/ingest/apply` off the happy path: provider
//! selection and form fields, LLM and parse failures, office-format
//! branches, and every `apply` family (relation types, relations, rules,
//! actions) through its skip / created / failed / strict outcomes. The LLM
//! is a scripted fake; nothing leaves the process.

mod hardening_common;

use hardening_common::*;
use http::StatusCode;
use ontology_graph::{ActionId, ConceptId, OntologyGraph, RuleId};
use ontology_rag::{LlmError, LlmResponse, TokenUsage};
use ontology_server::build_router;
use ontology_storage::{FlakyStore, RecordKind};
use serde_json::{json, Value};
use std::sync::Arc;

const ONE_CONCEPT: &str =
    r#"{"concepts":[{"client_ref":"c0","concept_type":"Topic","name":"Acme"}]}"#;

fn review_app(llm: Arc<ScriptedLlm>) -> (axum::Router, Arc<OntologyGraph>, Arc<FlakyStore>) {
    let store = Arc::new(FlakyStore::new());
    let graph = OntologyGraph::with_arc(ontology());
    let app = build_router(state_with_llm(store.clone(), graph.clone(), llm));
    (app, graph, store)
}

fn plain_app() -> (axum::Router, Arc<OntologyGraph>, Arc<FlakyStore>) {
    review_app(ScriptedLlm::new(vec![reply(ONE_CONCEPT)]))
}

async fn analyze(
    app: &axum::Router,
    fields: &[(&str, &str)],
    file: (&str, &[u8]),
) -> (StatusCode, Value) {
    post_multipart(app, "/ingest/analyze", fields, Some(file)).await
}

fn accept_all(proposal: &Value) -> Value {
    let mut decisions = Vec::new();
    for family in [
        "concept_types",
        "relation_types",
        "concepts",
        "relations",
        "rules",
        "actions",
    ] {
        if let Some(items) = proposal[family].as_array() {
            for it in items {
                decisions.push(json!({ "client_ref": it["client_ref"], "action": "create_new" }));
            }
        }
    }
    Value::Array(decisions)
}

async fn apply(app: &axum::Router, proposal: Value, decisions: Value, strict: bool) -> Value {
    let (st, report) = call(
        app,
        "POST",
        "/ingest/apply",
        Some(json!({ "proposal": proposal, "decisions": decisions, "strict": strict })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{report}");
    report
}

fn outcome(report: &Value, family: &str, client_ref: &str) -> Value {
    report[family]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o[0] == client_ref)
        .map(|o| o[1].clone())
        .unwrap_or_else(|| panic!("no outcome for {client_ref} in {report}"))
}

// ---------------------------------------------------------------------------
// /ingest/analyze
// ---------------------------------------------------------------------------

/// `language_hint` overrides detection and is stamped on created concepts;
/// `provider=default` and unknown form fields are accepted; a named
/// provider with no stored key is a 400 before any LLM call.
#[tokio::test]
async fn analyze_form_fields_select_the_language_and_the_provider() {
    let (app, graph, _store) = plain_app();

    let (st, proposal) = analyze(
        &app,
        &[
            ("language_hint", "fr"),
            ("provider", "default"),
            ("model", ""),
            ("junk", "x"),
        ],
        ("note.txt", b"Acme achete des widgets.\n"),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{proposal}");
    assert_eq!(proposal["language"]["code"], "fr");
    assert_eq!(proposal["language"]["confidence"], 1.0);
    assert_eq!(proposal["source"]["provider"], "default");

    let report = apply(&app, proposal.clone(), accept_all(&proposal), true).await;
    assert_eq!(report["created"], 1, "{report}");
    let id = graph.find_by_name("Topic", "Acme").unwrap();
    let c = graph.get_concept(id).unwrap();
    assert_eq!(serde_json::to_value(&c.properties).unwrap()["lang"], "fr");

    // A blank hint falls back to detection (short text: none).
    let (st, proposal) = analyze(&app, &[("language_hint", "  ")], ("n.txt", b"Acme.\n")).await;
    assert_eq!(st, StatusCode::OK, "{proposal}");

    for (provider, needle) in [
        ("openai", "OpenAI"),
        ("anthropic", "Anthropic"),
        ("infomaniak", "Infomaniak"),
        ("mistral", "mistral"),
    ] {
        let (st, v) = analyze(&app, &[("provider", provider)], ("n.txt", b"Acme.\n")).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{provider}: {v}");
        assert!(error_text(&v).contains(needle), "{provider}: {v}");
    }
}

/// A configured provider is selected per request and reaches `model_for`:
/// with a key but no model the refusal names the missing model, with a
/// `model` form field a client is built (and no request is dispatched).
#[tokio::test]
async fn analyze_with_a_configured_provider_resolves_through_model_for() {
    let (app, _graph, _store) = plain_app();
    let (st, _) = call(
        &app,
        "PATCH",
        "/settings",
        Some(json!({ "llm": { "openai_api_key": "sk-test-1234", "openai_model": "" } })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);

    let (st, v) = analyze(&app, &[("provider", "openai")], ("n.txt", b"Acme.\n")).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(error_text(&v).contains("Aucun modèle"), "{v}");
}

/// LLM failures keep their class: a provider error is a 502, an answer that
/// is not JSON is a 422 naming the parse problem, and a `max_tokens` cut on
/// a one-line piece that cannot be split is a 422 too.
#[tokio::test]
async fn analyze_maps_llm_and_parse_failures_to_502_and_422() {
    let llm = ScriptedLlm::new(vec![
        Err(LlmError::Api("quota exceeded".into())),
        reply("Sorry, I cannot help with that."),
        reply(r#"{"concepts":"not a list"}"#),
        Ok(LlmResponse {
            content: r#"{"concepts":[{"client_ref":"c0","concept_type":"Topic","name":"Ac"#.into(),
            model: "scripted".into(),
            stop_reason: Some("max_tokens".into()),
            usage: TokenUsage::default(),
        }),
    ]);
    let (app, graph, store) = review_app(llm);

    let (st, v) = analyze(&app, &[], ("n.txt", b"Acme.\n")).await;
    assert_eq!(st, StatusCode::BAD_GATEWAY, "{v}");
    assert!(error_text(&v).contains("quota exceeded"), "{v}");

    let (st, v) = analyze(&app, &[], ("n.txt", b"Acme.\n")).await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert!(error_text(&v).contains("unparseable JSON"), "{v}");

    // Valid JSON of the wrong shape is a parse failure naming the chunk.
    let (st, v) = analyze(&app, &[], ("n.txt", b"Acme.\n")).await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert!(error_text(&v).contains("chunk 0: invalid type"), "{v}");

    let (st, v) = analyze(&app, &[], ("n.txt", b"Acme.\n")).await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert!(error_text(&v).contains("cut off"), "{v}");

    assert_eq!(graph.concept_count(), 0);
    assert_eq!(store.records_written(), 0);
}

/// Office formats: a broken `.docx` and a broken `.xlsx` are 422s naming
/// the format; a zip container with no telling extension is tried as docx
/// then as a spreadsheet (the invoices workbook renamed passes, garbage
/// does not).
#[tokio::test]
async fn analyze_flattens_or_refuses_office_containers() {
    let (app, _graph, _store) = plain_app();

    let (st, v) = analyze(
        &app,
        &[],
        ("memo.docx", b"PK\x03\x04 definitely not a docx"),
    )
    .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert!(error_text(&v).contains("docx"), "{v}");

    let (st, v) = analyze(&app, &[], ("book.xlsx", b"not a workbook at all")).await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert!(error_text(&v).contains("spreadsheet"), "{v}");

    let (st, v) = analyze(&app, &[], ("blob", b"PK\x03\x04\x00\x00garbage zip")).await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert!(error_text(&v).contains("spreadsheet"), "{v}");

    let xlsx = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/finance/invoices.xlsx"
    ))
    .unwrap();
    let (st, v) = analyze(&app, &[], ("blob", &xlsx)).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["source"]["name"], "blob");
}

// ---------------------------------------------------------------------------
// /ingest/apply — relation types
// ---------------------------------------------------------------------------

/// Relation types: skipped, created (journaled once with the concept types
/// as a single `Ontology` record), failed on an unknown domain; in strict
/// mode the failure rolls back every type declared before it.
#[tokio::test]
async fn apply_relation_types_are_created_skipped_or_rolled_back() {
    let (app, graph, store) = plain_app();
    let proposal = json!({
        "concept_types": [{ "client_ref": "t0", "name": "Widget", "description": "w",
                            "properties": ["sku"] }],
        "relation_types": [
            { "client_ref": "rt0", "name": "knows", "domain": "Topic", "range": "Tag", "symmetric": true },
            { "client_ref": "rt1", "name": "ignored", "domain": "Topic", "range": "Topic" },
            { "client_ref": "rt2", "name": "broken", "domain": "Nope", "range": "Topic" }
        ]
    });
    let decisions = json!([
        { "client_ref": "t0", "action": "create_new" },
        { "client_ref": "rt0", "action": "create_new" },
        { "client_ref": "rt2", "action": "create_new" }
    ]);

    // Strict: rt2 fails after t0 and rt0 landed → all three rolled back.
    let report = apply(&app, proposal.clone(), decisions.clone(), true).await;
    assert_eq!(report["created"], 2, "{report}");
    assert_eq!(report["failed"], 1, "{report}");
    assert_eq!(
        outcome(&report, "relation_types", "rt1"),
        json!({ "status": "skipped" })
    );
    assert!(
        outcome(&report, "relation_types", "rt2")["error"]
            .as_str()
            .unwrap()
            .contains("Nope"),
        "{report}"
    );
    let onto = graph.ontology();
    assert!(!onto.concept_types.contains_key("Widget"), "rolled back");
    assert!(!onto.relation_types.contains_key("knows"), "rolled back");
    assert_eq!(store.records_written(), 0);

    // Best effort: the failure is reported and the rest is journaled once.
    let report = apply(&app, proposal, decisions, false).await;
    assert_eq!(report["created"], 2, "{report}");
    assert_eq!(report["failed"], 1, "{report}");
    assert_eq!(
        outcome(&report, "relation_types", "rt0"),
        json!({ "status": "created", "id": "knows" })
    );
    let onto = graph.ontology();
    assert_eq!(
        onto.concept_types["Widget"].properties.as_deref(),
        Some(&["sku".to_string()][..])
    );
    assert!(onto.relation_types["knows"].symmetric);
    assert_eq!(store.records_written(), 1);
    assert!(matches!(store.records()[0].kind, RecordKind::Ontology(_)));
}

/// A concept type that cannot be applied (a parent cycle) is a failed
/// outcome; strict mode stops there and nothing is journaled.
#[tokio::test]
async fn apply_concept_type_failure_is_reported_and_stops_strict_mode() {
    let (app, graph, store) = plain_app();
    let proposal = json!({
        "concept_types": [
            { "client_ref": "t0", "name": "Topic", "parent": "Topic" },
            { "client_ref": "t1", "name": "Never" }
        ]
    });
    let decisions = json!([
        { "client_ref": "t0", "action": "create_new" },
        { "client_ref": "t1", "action": "create_new" }
    ]);
    let report = apply(&app, proposal.clone(), decisions.clone(), true).await;
    assert_eq!(report["failed"], 1, "{report}");
    assert_eq!(
        report["concept_types"].as_array().unwrap().len(),
        1,
        "t1 never processed"
    );
    assert!(
        outcome(&report, "concept_types", "t0")["error"]
            .as_str()
            .unwrap()
            .contains("cycle"),
        "{report}"
    );
    assert!(!graph.ontology().concept_types.contains_key("Never"));
    assert_eq!(store.records_written(), 0);

    let report = apply(&app, proposal, decisions, false).await;
    assert_eq!(report["failed"], 1, "{report}");
    assert_eq!(report["created"], 1, "{report}");
    assert!(graph.ontology().concept_types.contains_key("Never"));
    assert_eq!(store.records_written(), 1);
}

// ---------------------------------------------------------------------------
// /ingest/apply — relations
// ---------------------------------------------------------------------------

/// Relations resolve `client_ref`s and `Type:Name` forms, carry the
/// proposed weight, and report dangling refs, schema refusals and store
/// failures per item; strict mode stops at the first one.
#[tokio::test]
async fn apply_relations_resolve_refs_and_report_each_refusal() {
    let (app, graph, store) = plain_app();
    let b = create_topic(&app, "B").await;
    let written = store.records_written();

    let proposal = json!({
        "concepts": [{ "client_ref": "c0", "concept_type": "Topic", "name": "A" }],
        "relations": [
            { "client_ref": "r0", "relation_type": "related_to", "source_ref": "c0",
              "target_ref": "Topic:B", "weight": 0.25 },
            { "client_ref": "r1", "relation_type": "related_to", "source_ref": "c0",
              "target_ref": "Topic:Ghost" },
            { "client_ref": "r2", "relation_type": "nope", "source_ref": "c0",
              "target_ref": "Topic:B" },
            { "client_ref": "r3", "relation_type": "related_to", "source_ref": "c0",
              "target_ref": "Topic:B" }
        ]
    });
    let decisions = json!([
        { "client_ref": "c0", "action": "create_new" },
        { "client_ref": "r0", "action": "create_new" },
        { "client_ref": "r1", "action": "create_new" },
        { "client_ref": "r2", "action": "create_new" }
    ]);

    let report = apply(&app, proposal.clone(), decisions.clone(), false).await;
    assert_eq!(report["created"], 2, "{report}");
    assert_eq!(report["failed"], 2, "{report}");
    assert_eq!(report["skipped"], 1, "{report}");
    let created = outcome(&report, "relations", "r0");
    assert_eq!(created["status"], "created");
    let rid: u64 = created["id"].as_str().unwrap().parse().unwrap();
    let rel = graph.get_relation(ontology_graph::RelationId(rid)).unwrap();
    assert_eq!(rel.weight, 0.25);
    assert_eq!(rel.target, ConceptId(b));
    let dangling = outcome(&report, "relations", "r1")["error"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        dangling.contains("dangling refs") && dangling.contains("Topic:Ghost"),
        "{dangling}"
    );
    assert!(
        outcome(&report, "relations", "r2")["error"]
            .as_str()
            .unwrap()
            .contains("nope"),
        "{report}"
    );
    assert_eq!(
        outcome(&report, "relations", "r3"),
        json!({ "status": "skipped" })
    );
    assert_eq!(store.records_written(), written + 2, "concept + relation");

    // Strict: the dangling relation ends the apply; r2 is never listed.
    let (app, graph, store) = plain_app();
    create_topic(&app, "B").await;
    let report = apply(&app, proposal.clone(), decisions.clone(), true).await;
    assert_eq!(report["failed"], 1, "{report}");
    assert_eq!(report["relations"].as_array().unwrap().len(), 2, "{report}");
    assert_eq!(
        report["created"], 2,
        "c0 and r0 landed before the stop: {report}"
    );
    assert!(graph.find_by_name("Topic", "A").is_some());
    assert_eq!(store.records_written(), 3, "B, A and r0");

    // Strict with a store failure on the relation itself.
    let (app, graph, store) = plain_app();
    create_topic(&app, "A").await;
    create_topic(&app, "B").await;
    store.set_failing(true);
    let report = apply(
        &app,
        json!({ "relations": [{ "client_ref": "r0", "relation_type": "related_to",
                                 "source_ref": "Topic:A", "target_ref": "Topic:B" }] }),
        json!([{ "client_ref": "r0", "action": "create_new" }]),
        true,
    )
    .await;
    assert_eq!(report["failed"], 1, "{report}");
    assert!(
        outcome(&report, "relations", "r0")["error"]
            .as_str()
            .unwrap()
            .contains("FlakyStore"),
        "{report}"
    );
    assert_eq!(graph.relation_count(), 0);
}

// ---------------------------------------------------------------------------
// /ingest/apply — rules
// ---------------------------------------------------------------------------

/// Rules: `applies_to` resolves refs (unresolvable ones are dropped), an
/// unknown rule type and a store failure are per-item failures, strict
/// mode stops at the first, and a skipped rule writes nothing.
#[tokio::test]
async fn apply_rules_resolve_scope_and_report_refusals() {
    let (app, graph, store) = plain_app();
    let b = create_topic(&app, "B").await;
    let written = store.records_written();

    let proposal = json!({
        "concepts": [{ "client_ref": "c0", "concept_type": "Topic", "name": "A" }],
        "rules": [
            { "client_ref": "u0", "rule_type": "must_review", "name": "Review A",
              "when": "w", "then": "t", "strict": true, "description": "d",
              "applies_to": ["c0", "Topic:B", "bogus", "Topic:Ghost"] },
            { "client_ref": "u1", "rule_type": "unknown_rule", "name": "X" },
            { "client_ref": "u2", "rule_type": "must_review", "name": "Skipped" }
        ]
    });
    let decisions = json!([
        { "client_ref": "c0", "action": "create_new" },
        { "client_ref": "u0", "action": "create_new" },
        { "client_ref": "u1", "action": "create_new" }
    ]);

    let report = apply(&app, proposal.clone(), decisions.clone(), false).await;
    assert_eq!(report["created"], 2, "{report}");
    assert_eq!(report["failed"], 1, "{report}");
    assert_eq!(
        outcome(&report, "rules", "u2"),
        json!({ "status": "skipped" })
    );
    let created = outcome(&report, "rules", "u0");
    let rid: u64 = created["id"].as_str().unwrap().parse().unwrap();
    let rule = graph.get_rule(RuleId(rid)).unwrap();
    let a = graph.find_by_name("Topic", "A").unwrap();
    assert_eq!(rule.applies_to, vec![a, ConceptId(b)]);
    assert!(rule.strict);
    assert_eq!(
        (
            rule.when.as_str(),
            rule.then.as_str(),
            rule.description.as_str()
        ),
        ("w", "t", "d")
    );
    assert!(
        outcome(&report, "rules", "u1")["error"]
            .as_str()
            .unwrap()
            .contains("unknown_rule"),
        "{report}"
    );
    assert_eq!(store.records_written(), written + 2);

    // Strict: the unknown rule type ends the apply.
    let (app, graph, _) = plain_app();
    create_topic(&app, "B").await;
    let report = apply(&app, proposal, decisions, true).await;
    assert_eq!(report["failed"], 1, "{report}");
    assert_eq!(
        report["rules"].as_array().unwrap().len(),
        2,
        "u2 never reached: {report}"
    );
    assert_eq!(graph.rule_count(), 1);

    // Store failure on the rule: failed per item, graph untouched.
    let (app, graph, store) = plain_app();
    create_topic(&app, "A").await;
    store.set_failing(true);
    let report = apply(
        &app,
        json!({ "rules": [{ "client_ref": "u0", "rule_type": "must_review", "name": "R",
                             "applies_to": ["Topic:A"] }] }),
        json!([{ "client_ref": "u0", "action": "create_new" }]),
        true,
    )
    .await;
    assert_eq!(report["failed"], 1, "{report}");
    assert!(
        outcome(&report, "rules", "u0")["error"]
            .as_str()
            .unwrap()
            .contains("FlakyStore"),
        "{report}"
    );
    assert_eq!(graph.rule_count(), 0);
}

// ---------------------------------------------------------------------------
// /ingest/apply — actions
// ---------------------------------------------------------------------------

/// Actions: subject and object resolve through refs, parameters land as
/// text, a dangling subject / unknown type / store failure are per-item
/// failures, strict mode stops at the first, a skipped action writes
/// nothing.
#[tokio::test]
async fn apply_actions_resolve_endpoints_and_report_refusals() {
    let (app, graph, store) = plain_app();
    let b = create_topic(&app, "B").await;
    let written = store.records_written();

    let proposal = json!({
        "concepts": [{ "client_ref": "c0", "concept_type": "Topic", "name": "A" }],
        "actions": [
            { "client_ref": "a0", "action_type": "archive", "name": "Archive A",
              "subject_ref": "c0", "object_ref": "Topic:B", "effect": "e", "description": "d",
              "parameters": [["reason", "old"]] },
            { "client_ref": "a1", "action_type": "archive", "name": "Dangling",
              "subject_ref": "Topic:Ghost" },
            { "client_ref": "a2", "action_type": "not_a_type", "name": "Bad type",
              "subject_ref": "c0", "object_ref": "Topic:Ghost" },
            { "client_ref": "a3", "action_type": "archive", "name": "Skipped", "subject_ref": "c0" }
        ]
    });
    let decisions = json!([
        { "client_ref": "c0", "action": "create_new" },
        { "client_ref": "a0", "action": "create_new" },
        { "client_ref": "a1", "action": "create_new" },
        { "client_ref": "a2", "action": "create_new" }
    ]);

    let report = apply(&app, proposal.clone(), decisions.clone(), false).await;
    assert_eq!(report["created"], 2, "{report}");
    assert_eq!(report["failed"], 2, "{report}");
    assert_eq!(
        outcome(&report, "actions", "a3"),
        json!({ "status": "skipped" })
    );
    let created = outcome(&report, "actions", "a0");
    let aid: u64 = created["id"].as_str().unwrap().parse().unwrap();
    let act = graph.get_action(ActionId(aid)).unwrap();
    assert_eq!(act.subject, graph.find_by_name("Topic", "A").unwrap());
    assert_eq!(act.object, Some(ConceptId(b)));
    assert_eq!(
        serde_json::to_value(&act.parameters).unwrap()["reason"],
        "old"
    );
    assert_eq!((act.effect.as_str(), act.description.as_str()), ("e", "d"));
    assert!(
        outcome(&report, "actions", "a1")["error"]
            .as_str()
            .unwrap()
            .contains("dangling subject"),
        "{report}"
    );
    assert!(
        outcome(&report, "actions", "a2")["error"]
            .as_str()
            .unwrap()
            .contains("not_a_type"),
        "{report}"
    );
    assert_eq!(store.records_written(), written + 2);

    // Strict: the dangling subject ends the apply.
    let (app, graph, _) = plain_app();
    create_topic(&app, "B").await;
    let report = apply(&app, proposal.clone(), decisions.clone(), true).await;
    assert_eq!(report["failed"], 1, "{report}");
    assert_eq!(report["actions"].as_array().unwrap().len(), 2, "{report}");
    assert_eq!(graph.action_count(), 1);

    // Strict: a refused type ends the apply too.
    let (app, graph, _) = plain_app();
    create_topic(&app, "A").await;
    let report = apply(
        &app,
        json!({ "actions": [
            { "client_ref": "a2", "action_type": "not_a_type", "name": "X", "subject_ref": "Topic:A" },
            { "client_ref": "a3", "action_type": "archive", "name": "Y", "subject_ref": "Topic:A" }
        ] }),
        json!([{ "client_ref": "a2", "action": "create_new" }, { "client_ref": "a3", "action": "create_new" }]),
        true,
    )
    .await;
    assert_eq!(report["actions"].as_array().unwrap().len(), 1, "{report}");
    assert_eq!(graph.action_count(), 0);

    // Store failure on the action: failed per item, graph untouched.
    let (app, graph, store) = plain_app();
    create_topic(&app, "A").await;
    store.set_failing(true);
    let report = apply(
        &app,
        json!({ "actions": [{ "client_ref": "a0", "action_type": "archive", "name": "X",
                               "subject_ref": "Topic:A" }] }),
        json!([{ "client_ref": "a0", "action": "create_new" }]),
        true,
    )
    .await;
    assert!(
        outcome(&report, "actions", "a0")["error"]
            .as_str()
            .unwrap()
            .contains("FlakyStore"),
        "{report}"
    );
    assert_eq!(graph.action_count(), 0);
}

/// `merge` with a non-empty description and no properties replaces the
/// description and keeps the stored properties (the counterpart of the
/// empty-description case in `hardening_upload_ingest`).
#[tokio::test]
async fn apply_merge_with_a_description_replaces_it_and_keeps_properties() {
    let (app, graph, _store) = plain_app();
    let (st, v) = call(
        &app,
        "POST",
        "/concepts",
        Some(json!({ "id": 0, "concept_type": "Topic", "name": "Acme",
                     "description": "old", "properties": { "keep": "x" } })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let id = ConceptId(v["id"].as_u64().unwrap());

    let report = apply(
        &app,
        json!({ "concepts": [{ "client_ref": "c0", "concept_type": "Topic", "name": "Acme",
                                "description": "fresh" }] }),
        json!([{ "client_ref": "c0", "action": "merge" }]),
        true,
    )
    .await;
    assert_eq!(report["merged"], 1, "{report}");
    let c = graph.get_concept(id).unwrap();
    assert_eq!(c.description, "fresh");
    assert_eq!(
        serde_json::to_value(&c.properties).unwrap()["keep"],
        "x",
        "no properties in the proposal keeps the stored ones"
    );
}

/// A concept type marked `skip` is reported as such and does not touch the
/// schema; the whole proposal being skipped journals nothing.
#[tokio::test]
async fn apply_skipped_types_leave_the_schema_alone() {
    let (app, graph, store) = plain_app();
    let before = fingerprint(&graph);
    let report = apply(
        &app,
        json!({ "concept_types": [{ "client_ref": "t0", "name": "Widget" }],
                "relation_types": [{ "client_ref": "rt0", "name": "x", "domain": "Topic", "range": "Topic" }] }),
        json!([{ "client_ref": "t0", "action": "skip" }]),
        true,
    )
    .await;
    assert_eq!(report["skipped"], 2, "{report}");
    assert_eq!(
        outcome(&report, "concept_types", "t0"),
        json!({ "status": "skipped" })
    );
    assert_eq!(fingerprint(&graph), before);
    assert_eq!(store.records_written(), 0);
}
