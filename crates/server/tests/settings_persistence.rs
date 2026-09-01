// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! End-to-end checks on the settings store, which is now the *only* place
//! provider credentials live — no environment variable backs it up.
//!
//! Three properties matter and each has a test here:
//!
//! 1. What the UI saves survives a restart, secret included.
//! 2. A raw key never travels back out over HTTP.
//! 3. A "test connection" / "list models" call with unsaved credentials
//!    probes them without writing anything to the store.
//!
//! Nothing here touches the network: every probe is aimed at a closed local
//! port or fails on missing credentials before any request is dispatched.

use axum::body::{to_bytes, Body};
use http::{Request, StatusCode};
use ontology_graph::{ConceptType, Ontology, OntologyGraph};
use ontology_index::HybridIndex;
use ontology_rag::{EchoModel, RagPipeline};
use ontology_server::{build_router, AppState};
use ontology_storage::{MemoryStore, Store};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower::ServiceExt;

fn ontology() -> Ontology {
    let mut o = Ontology::new();
    o.add_concept_type(ConceptType {
        name: "Topic".into(),
        parent: None,
        properties: None,
        description: "topic".into(),
        ..Default::default()
    });
    o
}

fn state_at(path: &Path) -> AppState {
    let graph = OntologyGraph::with_arc(ontology());
    let index = Arc::new(HybridIndex::with_default_embedder(graph.clone()));
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let pipeline = Arc::new(RagPipeline::new(index.clone(), Arc::new(EchoModel)));
    AppState::new(graph, index, store, pipeline).with_settings_path(path.to_path_buf())
}

/// Fresh directory per test — the tag keeps parallel tests from colliding.
fn temp_dir(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("ontology-settings-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

async fn read_body(b: Body) -> Value {
    let bytes = to_bytes(b, 1 << 20).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn get_settings(path: &Path) -> Value {
    let resp = build_router(state_at(path))
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
    read_body(resp.into_body()).await
}

async fn send_json(path: &Path, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let resp = build_router(state_at(path))
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    (status, read_body(resp.into_body()).await)
}

/// Collect every field name in a JSON tree, so a leak can be asserted
/// against structurally rather than by string matching alone.
fn field_names(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for (k, child) in map {
                out.push(k.clone());
                field_names(child, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                field_names(item, out);
            }
        }
        _ => {}
    }
}

#[tokio::test]
async fn infomaniak_config_survives_a_restart() {
    let dir = temp_dir("restart");
    let file = dir.join("settings.json");

    let (status, patched) = send_json(
        &file,
        "PATCH",
        "/settings",
        json!({
            "llm": {
                "active_provider": "infomaniak",
                "infomaniak_api_key": "tok-secret-1234",
                "infomaniak_product_id": " 101112 ",
                "infomaniak_model": "mixtral"
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // The PATCH response already masks the key it just accepted.
    assert!(patched["llm"].get("infomaniak_api_key").is_none());
    assert_eq!(patched["llm"]["infomaniak_api_key_hint"], "tok...1234");
    // Whitespace around a pasted product id is absorbed.
    assert_eq!(patched["llm"]["infomaniak_product_id"], "101112");

    // The file on disk *is* the credential store: it must hold the raw key,
    // otherwise the provider is silently logged out on the next boot.
    let raw = std::fs::read_to_string(&file).expect("settings.json written");
    assert!(
        raw.contains("tok-secret-1234"),
        "raw key missing from settings.json:\n{raw}"
    );

    // Restart: a brand-new AppState reading the same file.
    let reloaded = get_settings(&file).await;
    assert_eq!(reloaded["llm"]["active_provider"], "infomaniak");
    assert_eq!(reloaded["llm"]["infomaniak_product_id"], "101112");
    assert_eq!(reloaded["llm"]["infomaniak_model"], "mixtral");
    assert_eq!(reloaded["llm"]["infomaniak_api_key_hint"], "tok...1234");
}

#[tokio::test]
async fn get_settings_never_leaks_a_raw_key() {
    let dir = temp_dir("noleak");
    let file = dir.join("settings.json");

    send_json(
        &file,
        "PATCH",
        "/settings",
        json!({
            "llm": {
                "openai_api_key": "sk-openai-1111",
                "anthropic_api_key": "sk-ant-2222",
                "infomaniak_api_key": "tok-info-3333"
            },
            "ocr": { "google_api_key": "goog-4444" }
        }),
    )
    .await;

    let view = get_settings(&file).await;
    let flat = view.to_string();
    for secret in ["sk-openai-1111", "sk-ant-2222", "tok-info-3333", "goog-4444"] {
        assert!(!flat.contains(secret), "{secret} leaked in {flat}");
    }

    let mut names = Vec::new();
    field_names(&view, &mut names);
    let leaked: Vec<_> = names
        .iter()
        .filter(|n| n.ends_with("_api_key") || n.ends_with("_secret") || n.ends_with("_token"))
        .collect();
    assert!(leaked.is_empty(), "secret-shaped fields exposed: {leaked:?}");

    // The hints the UI needs are still there.
    assert_eq!(view["llm"]["openai_api_key_hint"], "sk-...1111");
    assert_eq!(view["ocr"]["google_api_key_hint"], "goo...4444");
}

#[tokio::test]
async fn an_obsolete_infomaniak_base_url_is_cleared_when_the_file_loads() {
    let dir = temp_dir("migrate");
    let file = dir.join("settings.json");

    // A file as an older build wrote it: the dead `/1/ai` base URL, and none
    // of the fields added since.
    std::fs::write(
        &file,
        json!({
            "llm": {
                "active_provider": "infomaniak",
                "infomaniak_api_key": "tok-old-5555",
                "infomaniak_base_url": "https://api.infomaniak.com/1/ai"
            }
        })
        .to_string(),
    )
    .unwrap();

    let view = get_settings(&file).await;
    // Parsing did not fall back to defaults...
    assert_eq!(view["llm"]["active_provider"], "infomaniak");
    assert_eq!(view["llm"]["infomaniak_api_key_hint"], "tok...5555");
    // ...the unreachable base URL is gone, so the derived one takes over...
    assert_eq!(view["llm"]["infomaniak_base_url"], "");
    // ...and fields that did not exist in that file got their defaults.
    assert_eq!(view["retrieval"]["top_k"], 8);
    assert_eq!(view["ocr"]["provider"], "tesseract");
}

#[tokio::test]
async fn probing_with_unsaved_credentials_persists_nothing() {
    let dir = temp_dir("dryrun");
    let file = dir.join("settings.json");

    // Port 1 refuses instantly: enough to prove the request was attempted
    // with the supplied credentials, without leaving the machine.
    let (status, models) = send_json(
        &file,
        "POST",
        "/settings/llm/models",
        json!({
            "provider": "infomaniak",
            "api_key": "tok-unsaved-6666",
            "base_url": "http://127.0.0.1:1/v1"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(models["endpoint"], "http://127.0.0.1:1/v1/models");
    assert!(models["models"].as_array().unwrap().is_empty());
    assert!(models["error"].is_string(), "expected an error: {models}");

    // Nothing was written: no file, and a fresh state still has no key.
    assert!(
        !file.exists(),
        "a read-only probe must not create settings.json"
    );
    let view = get_settings(&file).await;
    assert_eq!(view["llm"]["infomaniak_api_key_hint"], "");
    assert_eq!(view["llm"]["active_provider"], "default");
}

#[tokio::test]
async fn missing_credentials_are_reported_before_any_request() {
    let dir = temp_dir("nocreds");
    let file = dir.join("settings.json");

    let (status, test) = send_json(
        &file,
        "POST",
        "/settings/llm/test",
        json!({ "provider": "infomaniak" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(test["ok"], false);
    assert_eq!(test["provider"], "infomaniak");
    assert!(
        test["endpoint"].is_null(),
        "no endpoint should be resolved yet: {test}"
    );
    assert!(
        test["error"].as_str().unwrap().contains("Clé API Infomaniak"),
        "unhelpful error: {test}"
    );

    // Same for a product id that is missing while the key is present.
    send_json(
        &file,
        "PATCH",
        "/settings",
        json!({ "llm": { "infomaniak_api_key": "tok-7777" } }),
    )
    .await;
    let (_, test) = send_json(
        &file,
        "POST",
        "/settings/llm/test",
        json!({ "provider": "infomaniak" }),
    )
    .await;
    assert_eq!(test["ok"], false);
    assert!(
        test["error"].as_str().unwrap().contains("Product ID"),
        "unhelpful error: {test}"
    );
}
