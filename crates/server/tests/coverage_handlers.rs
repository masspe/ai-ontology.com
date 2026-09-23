// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Handlers and branches the other integration tests leave out: state
//! assembly and settings loading, every `PATCH /settings` field, feedback
//! and log tail, saved-query patches, `/path`, rule / action / relation
//! update and delete, `/ask` and `/ask/stream`, the LLM-backed generators,
//! `/subgraph` seeding, `/export?format=json`, JWT without a pinned
//! audience, and the provider control plane against a loopback server.
//! No test leaves the machine.

mod hardening_common;

use axum::body::Body;
use hardening_common::*;
use http::{Request, StatusCode};
use ontology_graph::{OntologyGraph, RelationId};
use ontology_index::HybridIndex;
use ontology_rag::model::{
    LanguageModel, LlmError, LlmRequest, LlmResponse, LlmStream, StreamChunk, TokenUsage,
};
use ontology_rag::{EchoModel, Message, RagPipeline};
use ontology_server::{
    build_router, build_router_with_jwt, configured_model, push_recent_log, AppState,
    ConfiguredModel, JwtAuth, LlmOverrides, Settings, SettingsRoutedModel,
};
use ontology_storage::{FlakyStore, MemoryStore, Store};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tower::ServiceExt;

static DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Fresh directory per call: pid + counter + clock keep parallel tests and
/// repeated runs apart.
fn temp_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "ontology-cov-{tag}-{}-{}-{nanos}",
        std::process::id(),
        DIR_COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn graph_and_index() -> (Arc<OntologyGraph>, Arc<HybridIndex>) {
    let graph = OntologyGraph::with_arc(ontology());
    let index = Arc::new(HybridIndex::with_default_embedder(graph.clone()));
    (graph, index)
}

// ---------------------------------------------------------------------------
// State assembly, settings loading, provider resolution outside HTTP
// ---------------------------------------------------------------------------

/// `assemble` loads the settings file *before* the pipeline is built, so a
/// `SettingsRoutedModel` sees the persisted provider on its first call;
/// `configured_model` reports the three states; a broken or unreadable
/// settings file falls back to defaults instead of failing startup.
#[tokio::test]
async fn assemble_loads_settings_first_and_configured_model_reports_each_state() {
    let dir = temp_dir("assemble");
    let file = dir.join("settings.json");
    std::fs::write(
        &file,
        json!({ "llm": { "active_provider": "infomaniak", "infomaniak_api_key": "tok-assemble-4242",
                          "infomaniak_product_id": "77", "infomaniak_model": "mixtral" } })
        .to_string(),
    )
    .unwrap();

    let (graph, index) = graph_and_index();
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let seen = Arc::new(std::sync::Mutex::new(String::new()));
    let seen_in_builder = seen.clone();
    let idx = index.clone();
    let state = AppState::assemble(graph, index, store, Some(file.clone()), move |settings| {
        *seen_in_builder.lock().unwrap() = settings.read().llm.active_provider.clone();
        Arc::new(RagPipeline::new(
            idx,
            Arc::new(SettingsRoutedModel::new(settings, Arc::new(EchoModel))),
        ))
    });
    assert_eq!(
        *seen.lock().unwrap(),
        "infomaniak",
        "the pipeline builder must see the persisted provider"
    );
    let app = build_router(state);
    let (st, view) = get(&app, "/settings").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(view["llm"]["active_provider"], "infomaniak");
    assert_eq!(view["llm"]["infomaniak_api_key_hint"], "tok...4242");
    assert!(view["llm"].get("infomaniak_api_key").is_none());

    // configured_model: not configured / ready / invalid.
    let settings = Settings::load_or_default(&file);
    assert!(matches!(
        configured_model(&settings, &LlmOverrides::default()),
        ConfiguredModel::Ready(_)
    ));
    assert!(matches!(
        configured_model(&Settings::default(), &LlmOverrides::default()),
        ConfiguredModel::NotConfigured
    ));
    let mut anthropic = Settings::default();
    anthropic.llm.active_provider = "anthropic".into();
    anthropic.llm.anthropic_api_key = "sk-ant-test".into();
    anthropic.llm.anthropic_model.clear();
    match configured_model(&anthropic, &LlmOverrides::default()) {
        ConfiguredModel::Invalid(msg) => assert!(msg.contains("Aucun modèle"), "{msg}"),
        _ => panic!("a provider without a model is invalid, not ready"),
    }
    let ov = LlmOverrides {
        model: Some("claude-x".into()),
        ..Default::default()
    };
    assert!(
        matches!(configured_model(&anthropic, &ov), ConfiguredModel::Ready(_)),
        "an Anthropic client is built from a complete config"
    );

    // Unusable files: garbage content, and a directory in place of a file.
    let garbage = dir.join("garbage.json");
    std::fs::write(&garbage, b"{ not json").unwrap();
    assert_eq!(
        Settings::load_or_default(&garbage).llm.active_provider,
        "default"
    );
    assert_eq!(
        Settings::load_or_default(&dir).llm.active_provider,
        "default"
    );
    assert_eq!(
        Settings::load_or_default(&dir.join("missing.json"))
            .llm
            .active_provider,
        "default"
    );
    let (g, i) = graph_and_index();
    let s: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let state = AppState::new(
        g,
        i.clone(),
        s,
        Arc::new(RagPipeline::new(i, Arc::new(EchoModel))),
    )
    .with_settings_path(garbage);
    assert_eq!(state.settings.read().llm.active_provider, "default");

    // The routed model streams through its fallback while unconfigured.
    let routed = SettingsRoutedModel::new(
        Arc::new(parking_lot::RwLock::new(Settings::default())),
        Arc::new(EchoModel),
    );
    let req = LlmRequest {
        messages: vec![Message::user("hi")],
        max_tokens: 8,
        ..Default::default()
    };
    let chunks: Vec<_> =
        futures::StreamExt::collect::<Vec<_>>(routed.generate_stream(&req).await.unwrap()).await;
    assert!(
        matches!(&chunks[0], Ok(StreamChunk::Text(t)) if t == "[echo] hi"),
        "{chunks:?}"
    );
    assert!(
        matches!(&chunks[1], Ok(StreamChunk::End { .. })),
        "{chunks:?}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// Settings: every patchable field, failed persistence, OCR status
// ---------------------------------------------------------------------------

/// Every field of `PATCH /settings` lands, the response is redacted, and a
/// persistence failure (the parent "directory" is a file) is logged, not
/// surfaced. `/settings/ocr/status` reflects the Google key.
#[tokio::test]
async fn patch_settings_covers_every_field_and_survives_a_failed_persist() {
    let dir = temp_dir("patch");
    let blocker = dir.join("not-a-dir");
    std::fs::write(&blocker, b"x").unwrap();
    let (g, i) = graph_and_index();
    let s: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let state = AppState::new(
        g,
        i.clone(),
        s,
        Arc::new(RagPipeline::new(i, Arc::new(EchoModel))),
    )
    .with_settings_path(blocker.join("settings.json"));
    let app = build_router(state);

    let (st, status) = get(&app, "/settings/ocr/status").await;
    assert_eq!(st, StatusCode::OK, "{status}");
    assert_eq!(status["google_vision"]["configured"], false);
    assert_eq!(status["google_vision"]["auth"], "API key");
    assert!(status["tesseract"]["available"].is_boolean());
    assert!(status["tesseract"]["ghostscript_available"].is_boolean());

    let patch = json!({
        "retrieval": { "top_k": 3, "lexical_weight": 0.75, "expansion_depth": 4 },
        "ui": { "theme": "dark", "graph_layout": "cose" },
        "llm": {
            "active_provider": "anthropic",
            "openai_api_key": "sk-openai-0001", "openai_base_url": "http://o.test", "openai_model": "o1",
            "anthropic_api_key": "sk-ant-0002", "anthropic_base_url": "http://a.test", "anthropic_model": "a1",
            "infomaniak_api_key": "tok-0003", "infomaniak_product_id": " 9 ",
            "infomaniak_base_url": " http://i.test ", "infomaniak_model": "i1",
            "temperature": 0.5, "max_tokens": 42
        },
        "ocr": {
            "provider": "google_vision", "auto_fallback": false, "min_text_threshold": 7,
            "tesseract_languages": "fra", "google_api_key": "goog-0004"
        }
    });
    let (st, view) = call(&app, "PATCH", "/settings", Some(patch)).await;
    assert_eq!(st, StatusCode::OK, "{view}");
    assert_eq!(
        view["retrieval"],
        json!({ "top_k": 3, "lexical_weight": 0.75, "expansion_depth": 4 })
    );
    assert_eq!(
        view["ui"],
        json!({ "theme": "dark", "graph_layout": "cose" })
    );
    let llm = &view["llm"];
    assert_eq!(llm["active_provider"], "anthropic");
    assert_eq!(llm["openai_base_url"], "http://o.test");
    assert_eq!(llm["openai_model"], "o1");
    assert_eq!(llm["anthropic_base_url"], "http://a.test");
    assert_eq!(llm["anthropic_model"], "a1");
    assert_eq!(llm["infomaniak_product_id"], "9", "trimmed");
    assert_eq!(llm["infomaniak_base_url"], "http://i.test", "trimmed");
    assert_eq!(llm["infomaniak_model"], "i1");
    assert_eq!(llm["temperature"], 0.5);
    assert_eq!(llm["max_tokens"], 42);
    assert_eq!(llm["openai_api_key_hint"], "sk-...0001");
    assert_eq!(llm["anthropic_api_key_hint"], "sk-...0002");
    assert_eq!(llm["infomaniak_api_key_hint"], "tok...0003");
    let ocr = &view["ocr"];
    assert_eq!(ocr["provider"], "google_vision");
    assert_eq!(ocr["auto_fallback"], false);
    assert_eq!(ocr["min_text_threshold"], 7);
    assert_eq!(ocr["tesseract_languages"], "fra");
    assert_eq!(ocr["google_api_key_hint"], "goo...0004");
    for secret in ["sk-openai-0001", "sk-ant-0002", "tok-0003", "goog-0004"] {
        assert!(!view.to_string().contains(secret), "leaked {secret}");
    }
    assert!(
        blocker.is_file(),
        "the failed persist must not replace the file"
    );

    let (_, status) = get(&app, "/settings/ocr/status").await;
    assert_eq!(status["google_vision"]["configured"], true);
    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// Feedback, log tail, saved queries
// ---------------------------------------------------------------------------

#[tokio::test]
async fn feedback_crud_and_the_log_tail_are_bounded() {
    let (app, _graph, _store) = flaky_app();

    let (st, v) = call(&app, "POST", "/feedbacks", Some(json!({ "title": "  " }))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(error_text(&v).contains("title"), "{v}");

    let (st, fb) = call(
        &app,
        "POST",
        "/feedbacks",
        Some(
            json!({ "title": "Broken button", "description": "d", "frontend_logs": "console",
                     "user_agent": "ua", "url": "/x", "reporter_email": "a@b.c" }),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{fb}");
    assert_eq!(fb["id"], 1);
    assert_eq!(fb["kind"], "bug", "default kind");
    assert_eq!(fb["title"], "Broken button");
    assert_eq!(fb["reporter_email"], "a@b.c");
    assert!(fb["backend_logs"].is_string());
    assert!(fb["created_at"].as_u64().unwrap() > 0);

    let (st, list) = get(&app, "/feedbacks").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["id"], 1);

    let (st, _) = call(&app, "DELETE", "/feedbacks/1", None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, v) = call(&app, "DELETE", "/feedbacks/1", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND, "{v}");
    let (_, list) = get(&app, "/feedbacks").await;
    assert!(list.as_array().unwrap().is_empty());

    // The ring buffer keeps the newest 500 lines; `limit` trims further.
    for i in 0..600 {
        push_recent_log(format!("cov-marker-{i}"));
    }
    let (st, tail) = get(&app, "/logs/tail?limit=1000").await;
    assert_eq!(st, StatusCode::OK);
    let lines = tail["lines"].as_array().unwrap();
    assert!(lines.len() <= 500, "{}", lines.len());
    assert!(lines.iter().any(|l| l == "cov-marker-599"));
    assert!(!lines.iter().any(|l| l == "cov-marker-0"));
    let (_, tail) = get(&app, "/logs/tail?limit=3").await;
    assert_eq!(tail["lines"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn saved_queries_take_defaults_and_accept_a_full_patch() {
    let (app, _graph, _store) = flaky_app();

    let (st, v) = call(
        &app,
        "POST",
        "/queries",
        Some(json!({ "name": " ", "query": "q" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");

    let (st, q) = call(
        &app,
        "POST",
        "/queries",
        Some(json!({ "name": "n", "query": "q" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{q}");
    assert_eq!(q["top_k"], 8);
    assert_eq!(q["lexical_weight"], 0.5);
    assert_eq!(q["expansion_depth"], 2);
    assert_eq!(q["concept_types"], json!([]));
    let id = q["id"].as_u64().unwrap();

    let (st, v) = call(
        &app,
        "PATCH",
        &format!("/queries/{id}"),
        Some(
            json!({ "name": "n2", "query": "q2", "top_k": 3, "lexical_weight": 0.25,
                     "concept_types": ["Topic"], "expansion_depth": 1 }),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["name"], "n2");
    assert_eq!(v["query"], "q2");
    assert_eq!(v["top_k"], 3);
    assert_eq!(v["lexical_weight"], 0.25);
    assert_eq!(v["concept_types"], json!(["Topic"]));
    assert_eq!(v["expansion_depth"], 1);
    let (_, again) = get(&app, &format!("/queries/{id}")).await;
    assert_eq!(again, v);

    let (st, v) = call(&app, "PATCH", "/queries/999", Some(json!({ "name": "x" }))).await;
    assert_eq!(st, StatusCode::NOT_FOUND, "{v}");
}

// ---------------------------------------------------------------------------
// /path, rule / action / relation updates, stats deltas
// ---------------------------------------------------------------------------

#[tokio::test]
async fn path_finds_a_route_or_says_so() {
    let (app, _graph, _store) = flaky_app();
    let a = create_topic(&app, "A").await;
    let b = create_topic(&app, "B").await;
    create_topic(&app, "C").await;
    relate(&app, "related_to", a, b).await;

    let body = |from: &str, to: &str| json!({ "from_type": "Topic", "from_name": from, "to_type": "Topic", "to_name": to });
    let (st, v) = call(&app, "POST", "/path", Some(body("A", "B"))).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["found"], true);
    assert_eq!(v["path"]["steps"].as_array().unwrap().len(), 1);
    assert_eq!(v["path"]["start"]["name"], "A");

    let (st, v) = call(&app, "POST", "/path", Some(body("A", "C"))).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v, json!({ "found": false }));

    let (st, v) = call(&app, "POST", "/path", Some(body("Ghost", "B"))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(error_text(&v).contains("Ghost"), "{v}");
    let (st, v) = call(&app, "POST", "/path", Some(body("A", "Ghost"))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(error_text(&v).contains("Ghost"), "{v}");
}

#[tokio::test]
async fn rules_actions_and_relations_are_listed_sorted_patched_and_deleted() {
    let (app, graph, store) = flaky_app();
    let a = create_topic(&app, "A").await;
    let b = create_topic(&app, "B").await;
    let rel = relate(&app, "owned_by", a, b).await;

    let r2 = create_rule(&app, "Zulu", &[a]).await;
    let r1 = create_rule(&app, "Alpha", &[a]).await;
    let (st, list) = get(&app, "/rules").await;
    assert_eq!(st, StatusCode::OK);
    let names: Vec<_> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].clone())
        .collect();
    assert_eq!(names, json!(["Alpha", "Zulu"]).as_array().unwrap().clone());

    let written = store.records_written();
    let (st, v) = call(
        &app,
        "PATCH",
        &format!("/rules/{r1}"),
        Some(json!({ "name": "Alpha 2", "strict": true })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["name"], "Alpha 2");
    assert_eq!(v["strict"], true);
    assert_eq!(
        graph.get_rule(ontology_graph::RuleId(r1)).unwrap().name,
        "Alpha 2"
    );
    let (st, _) = call(&app, "DELETE", &format!("/rules/{r2}"), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert_eq!(graph.rule_count(), 1);
    assert_eq!(
        store.records_written(),
        written + 2,
        "one update, one delete"
    );

    let a2 = create_action(&app, "Zip", a).await;
    let a1 = create_action(&app, "Arch", a).await;
    let (st, list) = get(&app, "/actions").await;
    assert_eq!(st, StatusCode::OK);
    let names: Vec<_> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].clone())
        .collect();
    assert_eq!(names, json!(["Arch", "Zip"]).as_array().unwrap().clone());
    let (st, v) = call(
        &app,
        "PATCH",
        &format!("/actions/{a1}"),
        Some(json!({ "name": "Arch 2", "effect": "gone" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["name"], "Arch 2");
    assert_eq!(v["effect"], "gone");
    let (st, _) = call(&app, "DELETE", &format!("/actions/{a2}"), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert_eq!(graph.action_count(), 1);

    let (st, v) = call(
        &app,
        "PATCH",
        &format!("/relations/{rel}"),
        Some(json!({ "weight": 0.5 })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["weight"], 0.5);
    assert_eq!(graph.get_relation(RelationId(rel)).unwrap().weight, 0.5);
}

#[tokio::test]
async fn stats_deltas_measure_growth_since_the_first_sample() {
    let (app, _graph, _store) = flaky_app();
    let (st, first) = get(&app, "/stats").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(first["concepts"], 0);
    assert_eq!(first["deltas"]["concepts_pct"], 0.0);

    create_topic(&app, "A").await;
    let (_, second) = get(&app, "/stats").await;
    assert_eq!(second["concepts"], 1);
    assert_eq!(second["deltas"]["concepts_pct"], 100.0, "{second}");
    assert_eq!(second["deltas"]["relations_pct"], 0.0);
    // Samples closer than 60 s apart are not recorded twice.
    let (_, hist) = get(&app, "/stats/history").await;
    assert_eq!(hist["samples"].as_array().unwrap().len(), 1, "{hist}");
}

// ---------------------------------------------------------------------------
// /ask, /ask/stream, generators
// ---------------------------------------------------------------------------

/// Streams a fixed script: two text deltas around a keep-alive, an end
/// frame, then an upstream error; or refuses upfront.
struct StreamLlm {
    fail_upfront: bool,
}

#[async_trait::async_trait]
impl LanguageModel for StreamLlm {
    async fn generate(&self, _req: &LlmRequest) -> Result<LlmResponse, LlmError> {
        Err(LlmError::Api("generate is not used here".into()))
    }

    async fn generate_stream(&self, _req: &LlmRequest) -> Result<LlmStream, LlmError> {
        if self.fail_upfront {
            return Err(LlmError::Http("connection refused".into()));
        }
        let chunks: Vec<Result<StreamChunk, LlmError>> = vec![
            Ok(StreamChunk::Text("Hel".into())),
            Ok(StreamChunk::KeepAlive),
            Ok(StreamChunk::Text("lo".into())),
            Ok(StreamChunk::End {
                usage: TokenUsage::default(),
                stop_reason: Some("end_turn".into()),
                model: "stream".into(),
            }),
            Err(LlmError::Api("cut mid-stream".into())),
        ];
        Ok(futures::StreamExt::boxed(futures::stream::iter(chunks)))
    }
}

fn app_with(llm: Arc<dyn LanguageModel>) -> axum::Router {
    let store = Arc::new(FlakyStore::new());
    let graph = OntologyGraph::with_arc(ontology());
    build_router(state_with_llm(store, graph, llm))
}

async fn sse(app: &axum::Router, body: Value) -> (StatusCode, String) {
    let req = Request::builder()
        .method("POST")
        .uri("/ask/stream")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let (st, headers, bytes) = send(app, req).await;
    if st == StatusCode::OK {
        assert!(
            headers["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/event-stream"),
            "{headers:?}"
        );
    }
    (st, String::from_utf8(bytes).unwrap())
}

#[tokio::test]
async fn ask_answers_through_the_pipeline_and_maps_llm_failures_to_502() {
    let (app, _graph, _store) = flaky_app();
    create_topic(&app, "Acme").await;
    let (st, ans) = call(
        &app,
        "POST",
        "/ask",
        Some(json!({ "query": "Who is Acme?" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{ans}");
    assert_eq!(ans["query"], "Who is Acme?");
    assert!(
        ans["answer"].as_str().unwrap().starts_with("[echo]"),
        "{ans}"
    );
    assert_eq!(ans["model"], "echo");
    assert!(!ans["subgraph"]["concepts"].as_array().unwrap().is_empty());

    let app = app_with(ScriptedLlm::new(vec![Err(LlmError::Api(
        "upstream down".into(),
    ))]));
    let (st, v) = call(&app, "POST", "/ask", Some(json!({ "query": "x" }))).await;
    assert_eq!(st, StatusCode::BAD_GATEWAY, "{v}");
    assert!(error_text(&v).contains("upstream down"), "{v}");

    // /queries/:id/run shares the mapping.
    let (_, q) = call(
        &app,
        "POST",
        "/queries",
        Some(json!({ "name": "n", "query": "q" })),
    )
    .await;
    let (st, v) = call(&app, "POST", &format!("/queries/{}/run", q["id"]), None).await;
    assert_eq!(st, StatusCode::BAD_GATEWAY, "{v}");
}

#[tokio::test]
async fn ask_stream_emits_retrieved_tokens_end_and_error_events() {
    let app = app_with(Arc::new(StreamLlm {
        fail_upfront: false,
    }));
    create_topic(&app, "Acme").await;
    let (st, text) = sse(&app, json!({ "query": "Acme?" })).await;
    assert_eq!(st, StatusCode::OK, "{text}");
    let events: Vec<&str> = text
        .lines()
        .filter_map(|l| l.strip_prefix("event: "))
        .collect();
    assert_eq!(
        events,
        ["retrieved", "token", "token", "end", "error"],
        "{text}"
    );
    let data: Vec<Value> = text
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .map(|d| serde_json::from_str(d).unwrap())
        .collect();
    assert_eq!(data[0]["query"], "Acme?");
    assert!(!data[0]["subgraph"]["concepts"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(data[1]["text"], "Hel");
    assert_eq!(data[2]["text"], "lo");
    assert_eq!(data[3]["model"], "stream");
    assert!(
        data[4]["message"]
            .as_str()
            .unwrap()
            .contains("cut mid-stream"),
        "{text}"
    );

    let app = app_with(Arc::new(StreamLlm { fail_upfront: true }));
    let (st, text) = sse(&app, json!({ "query": "x" })).await;
    assert_eq!(st, StatusCode::BAD_GATEWAY, "{text}");
    assert!(text.contains("connection refused"), "{text}");
}

#[tokio::test]
async fn ontology_and_rule_generators_validate_input_and_map_llm_outcomes() {
    let schema = serde_json::to_string(&ontology()).unwrap();
    let llm = ScriptedLlm::new(vec![
        reply(&format!("Here you go:\n```json\n{schema}\n```")),
        reply("no json here"),
        Err(LlmError::Api("ontology llm down".into())),
        reply(r#"{"name":"Review A","when":"w","then":"t","description":"d","strict":true}"#),
        reply("{\"when\": \"missing name\"}"),
        Err(LlmError::Api("rule llm down".into())),
    ]);
    let app = app_with(llm);

    let (st, v) = call(
        &app,
        "POST",
        "/ontology/generate",
        Some(json!({ "description": " " })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    let (st, v) = call(
        &app,
        "POST",
        "/ontology/generate",
        Some(json!({ "description": "topics" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["model"], "configured-llm");
    assert!(v["ontology"]["concept_types"].get("Topic").is_some(), "{v}");
    let (st, v) = call(
        &app,
        "POST",
        "/ontology/generate",
        Some(json!({ "description": "topics" })),
    )
    .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert!(error_text(&v).contains("ontology JSON parse failed"), "{v}");
    assert!(error_text(&v).contains("no json here"), "raw echoed: {v}");
    let (st, v) = call(
        &app,
        "POST",
        "/ontology/generate",
        Some(json!({ "description": "topics" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_GATEWAY, "{v}");
    assert!(error_text(&v).contains("ontology llm down"), "{v}");

    let a = create_topic(&app, "A").await;
    let rule_req = |applies_to: Value| json!({ "description": "review A", "rule_type": "must_review", "applies_to": applies_to });
    for (label, body) in [
        (
            "empty description",
            json!({ "description": "", "rule_type": "r", "applies_to": [a] }),
        ),
        (
            "empty rule_type",
            json!({ "description": "d", "rule_type": " ", "applies_to": [a] }),
        ),
        ("no applies_to", rule_req(json!([]))),
        ("unknown concept", rule_req(json!([9999]))),
    ] {
        let (st, v) = call(&app, "POST", "/rules/generate", Some(body)).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{label}: {v}");
    }
    let (st, v) = call(&app, "POST", "/rules/generate", Some(rule_req(json!([a])))).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(
        v,
        json!({ "name": "Review A", "when": "w", "then": "t", "description": "d", "strict": true })
    );
    let (st, v) = call(&app, "POST", "/rules/generate", Some(rule_req(json!([a])))).await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert!(error_text(&v).contains("rule JSON parse failed"), "{v}");
    let (st, v) = call(&app, "POST", "/rules/generate", Some(rule_req(json!([a])))).await;
    assert_eq!(st, StatusCode::BAD_GATEWAY, "{v}");
    assert!(error_text(&v).contains("rule llm down"), "{v}");
}

// ---------------------------------------------------------------------------
// /subgraph seeding, /export json, upload with unknown fields
// ---------------------------------------------------------------------------

#[tokio::test]
async fn subgraph_seeds_by_query_and_by_type_within_the_limit() {
    let (app, _graph, _store) = flaky_app();
    let acme = create_topic(&app, "Acme").await;
    let beta = create_topic(&app, "Beta").await;
    let tag = create(&app, "Tag", "Urgent").await;
    relate(&app, "related_to", acme, beta).await;

    let (st, v) = call(
        &app,
        "POST",
        "/subgraph",
        Some(json!({ "seed_query": "Acme", "expansion_depth": 0 })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let ids: Vec<u64> = v["subgraph"]["concepts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_u64().unwrap())
        .collect();
    assert!(ids.contains(&acme), "{v}");

    let (st, v) = call(
        &app,
        "POST",
        "/subgraph",
        Some(json!({ "seed_concept_types": ["Topic", "Tag"], "limit": 1, "expansion_depth": 0 })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(
        v["subgraph"]["concepts"].as_array().unwrap().len(),
        1,
        "{v}"
    );

    let (st, v) = call(
        &app,
        "POST",
        "/subgraph",
        Some(json!({ "seed_concept_types": ["Tag"], "expansion_depth": 0 })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let ids: Vec<u64> = v["subgraph"]["concepts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids, vec![tag], "{v}");

    // A blank query is ignored and the whole graph seeds the view.
    let (st, v) = call(
        &app,
        "POST",
        "/subgraph",
        Some(json!({ "seed_query": "  " })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(
        v["subgraph"]["concepts"].as_array().unwrap().len(),
        3,
        "{v}"
    );
}

#[tokio::test]
async fn export_json_snapshots_the_graph_and_unknown_formats_are_400() {
    let (app, _graph, _store) = flaky_app();
    let a = create_topic(&app, "A").await;
    let b = create_topic(&app, "B").await;
    relate(&app, "owned_by", a, b).await;

    let (st, v) = get(&app, "/export?format=json").await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(v["ontology"]["concept_types"].get("Topic").is_some());
    assert_eq!(v["concepts"].as_array().unwrap().len(), 2);
    assert_eq!(v["relations"].as_array().unwrap().len(), 1);
    assert_eq!(v["relations"][0]["relation_type"], "owned_by");

    let (st, v) = get(&app, "/export?format=xml").await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(error_text(&v).contains("xml"), "{v}");
}

#[tokio::test]
async fn upload_ignores_unknown_form_fields() {
    let (app, graph, _store) = flaky_app();
    let (st, v) = post_multipart(
        &app,
        "/upload",
        &[("kind", "triples"), ("junk", "ignored")],
        Some(("t.triples", b"Topic:A related_to Topic:B\n")),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["ingested"]["concepts"], 2);
    assert_eq!(graph.concept_count(), 2);
}

// ---------------------------------------------------------------------------
// JWT without a pinned audience
// ---------------------------------------------------------------------------

#[tokio::test]
async fn jwt_without_a_pinned_audience_accepts_tokens_carrying_none() {
    use jsonwebtoken::{encode, EncodingKey, Header};
    #[derive(serde::Serialize)]
    struct Claims<'a> {
        sub: &'a str,
        iss: &'a str,
        exp: usize,
    }
    let secret = b"another-shared-secret-long-enough-for-hs256";
    let exp = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 600) as usize;
    let token = encode(
        &Header::default(),
        &Claims {
            sub: "u1",
            iss: "ai-ontology",
            exp,
        },
        &EncodingKey::from_secret(secret),
    )
    .unwrap();

    let store = Arc::new(FlakyStore::new());
    let graph = OntologyGraph::with_arc(ontology());
    let app = build_router_with_jwt(
        state_with(store, graph),
        JwtAuth {
            audience: None,
            ..JwtAuth::from_secret(&secret[..])
        },
    );
    let req = |auth: Option<&str>| {
        let mut b = Request::builder().method("GET").uri("/stats");
        if let Some(a) = auth {
            b = b.header("authorization", a);
        }
        b.body(Body::empty()).unwrap()
    };
    let resp = app
        .clone()
        .oneshot(req(Some(&format!("Bearer {token}"))))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let resp = app.clone().oneshot(req(None)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// Provider control plane against a loopback server
// ---------------------------------------------------------------------------

/// A fake provider on 127.0.0.1: `/v1/models` answers an Anthropic-style
/// catalogue when `x-api-key` is present and an OpenAI-style one under a
/// bearer token; `/bad/v1/models` is a 401.
async fn fake_provider() -> String {
    use axum::{extract::Request as AxReq, routing::get, Router};
    async fn models(req: AxReq) -> axum::Json<Value> {
        let anthropic = req.headers().contains_key("x-api-key")
            && req.headers().get("anthropic-version").is_some();
        let bearer = req
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.starts_with("Bearer "))
            .unwrap_or(false);
        let id = match (anthropic, bearer) {
            (true, _) => "claude-loopback",
            (_, true) => "gpt-loopback",
            _ => "unauthenticated",
        };
        axum::Json(json!({ "data": [{ "id": id }] }))
    }
    async fn unauthorized() -> (StatusCode, &'static str) {
        (StatusCode::UNAUTHORIZED, "{\"error\":\"bad key\"}")
    }
    let router = Router::new()
        .route("/v1/models", get(models))
        .route("/bad/v1/models", get(unauthorized));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

#[tokio::test]
async fn provider_control_plane_probes_and_lists_models_against_a_loopback_server() {
    let base = fake_provider().await;
    let (app, _graph, _store) = flaky_app();

    // Missing credentials are refused before any request is made.
    for (provider, needle) in [("openai", "OpenAI"), ("anthropic", "Anthropic")] {
        let (_, v) = call(
            &app,
            "POST",
            "/settings/llm/test",
            Some(json!({ "provider": provider, "base_url": base })),
        )
        .await;
        assert_eq!(v["ok"], false, "{v}");
        assert!(v["error"].as_str().unwrap().contains(needle), "{v}");
        assert!(v.get("endpoint").is_none(), "{v}");
    }

    // Connection test, OpenAI dialect, stored model echoed back.
    let (st, v) = call(
        &app,
        "POST",
        "/settings/llm/test",
        Some(json!({ "provider": "openai", "api_key": "k-1", "base_url": base })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["provider"], "openai");
    assert_eq!(v["endpoint"], format!("{base}/v1/models"));
    assert_eq!(v["model"], "gpt-4o", "the stored default model");
    assert!(v.get("error").is_none());

    // Anthropic dialect, no model stored → `model` absent.
    call(
        &app,
        "PATCH",
        "/settings",
        Some(json!({ "llm": { "anthropic_model": "" } })),
    )
    .await;
    let (_, v) = call(
        &app,
        "POST",
        "/settings/llm/test",
        Some(json!({ "provider": "anthropic", "api_key": "k-2", "base_url": base })),
    )
    .await;
    assert_eq!(v["ok"], true, "{v}");
    assert!(v.get("model").is_none(), "{v}");

    // A non-2xx answer is reported with its status and body excerpt.
    let (_, v) = call(
        &app,
        "POST",
        "/settings/llm/test",
        Some(json!({ "provider": "openai", "api_key": "k-3", "base_url": format!("{base}/bad") })),
    )
    .await;
    assert_eq!(v["ok"], false, "{v}");
    assert!(v["error"].as_str().unwrap().contains("401"), "{v}");
    assert!(v["error"].as_str().unwrap().contains("bad key"), "{v}");

    // Model catalogue from stored settings (GET) and from overrides (POST).
    call(
        &app,
        "PATCH",
        "/settings",
        Some(json!({ "llm": { "openai_api_key": "k-4", "openai_base_url": base } })),
    )
    .await;
    let (st, v) = get(&app, "/settings/llm/models?provider=openai").await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(
        v["models"],
        json!([{ "id": "gpt-loopback", "label": "gpt-loopback" }]),
        "{v}"
    );
    assert_eq!(v["endpoint"], format!("{base}/v1/models"));
    let (_, v) = call(
        &app,
        "POST",
        "/settings/llm/models",
        Some(json!({ "provider": "anthropic", "api_key": "k-5", "base_url": base })),
    )
    .await;
    assert_eq!(v["models"][0]["id"], "claude-loopback", "{v}");

    let (_, v) = call(
        &app,
        "POST",
        "/settings/llm/infomaniak/products",
        Some(json!({})),
    )
    .await;
    assert!(v["error"].as_str().unwrap().contains("Infomaniak"), "{v}");
    assert!(v["products"].as_array().unwrap().is_empty());
}
