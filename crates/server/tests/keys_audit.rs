// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Scoped API keys (clearances, read-only, expiry, rotation), the
//! hash-chained audit log (sign-ins, refusals, access changes, keys,
//! questions with the domains they touched; filters; verification) and
//! the on-premises egress policy (a question touching a domain that must
//! not leave the premises goes to the on-premises provider or is refused).

mod hardening_common;

use axum::body::{to_bytes, Body};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use hardening_common::*;
use http::{Request, StatusCode};
use ontology_graph::{ConceptType, Ontology, OntologyGraph, RelationType};
use ontology_server::audit::AuditLog;
use ontology_server::auth::{UserAuth, UserStore};
use ontology_server::{build_router_with_config, JwtAuth, RouterConfig};
use ontology_storage::{MemoryStore, Store};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

const SECRET: &[u8] = b"test-secret-for-the-built-in-auth";

fn tempdir(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "ontology-keys-{tag}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn ontology() -> Ontology {
    let mut o = Ontology::new();
    o.add_concept_type(ConceptType {
        name: "Topic".into(),
        ..Default::default()
    });
    o.add_concept_type(ConceptType {
        name: "Payslip".into(),
        ns: Some("payroll".into()),
        ..Default::default()
    });
    o.add_relation_type(RelationType {
        name: "related_to".into(),
        domain: "Topic".into(),
        range: "Topic".into(),
        ..Default::default()
    })
    .unwrap();
    o
}

struct World {
    app: Router,
    admin: String,
    audit: Arc<AuditLog>,
    dir: PathBuf,
}

async fn world(tag: &str, keep_questions: bool) -> World {
    let dir = tempdir(tag);
    let graph = OntologyGraph::with_arc(ontology());
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    // The model is routed by the live settings, as `ontology serve` does:
    // the on-premises policy is decided there.
    let settings = Arc::new(parking_lot::RwLock::new(
        ontology_server::Settings::default(),
    ));
    let routed = ontology_server::SettingsRoutedModel::new(
        settings.clone(),
        Arc::new(ontology_rag::EchoModel),
    );
    let mut state = state_with_llm(store, graph, Arc::new(routed));
    state.settings = settings;
    let audit = Arc::new(
        AuditLog::open(dir.join("audit.jsonl"))
            .unwrap()
            .keep_questions(keep_questions),
    );
    let app = build_router_with_config(
        state,
        RouterConfig {
            bearer_token: None,
            jwt: Some(JwtAuth::from_secret(SECRET.to_vec())),
            rate_limit: None,
            users: Some(UserAuth {
                store: Arc::new(UserStore::open(dir.join("users.json")).unwrap()),
                allow_signup: true,
            }),
            web_dir: None,
            audit: Some(audit.clone()),
        },
    );
    let (st, v) = call(
        &app,
        "POST",
        "/auth/signup",
        None,
        Some(json!({ "email": "alice@example.com", "name": "Alice", "password": "Passw0rd!x" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let admin = v["token"].as_str().unwrap().to_string();
    for (ty, name, labels) in [
        ("Topic", "public-sheet", json!([])),
        ("Topic", "hr-sheet", json!(["hr"])),
        ("Payslip", "payslip-march", json!([])),
    ] {
        let (st, v) = call(
            &app,
            "POST",
            "/concepts",
            Some(&admin),
            Some(json!({ "id": 0, "concept_type": ty, "name": name, "description": format!("about {name}"), "labels": labels })),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{v}");
    }
    World {
        app,
        admin,
        audit,
        dir,
    }
}

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let req = match body {
        Some(b) => req
            .header("content-type", "application/json")
            .body(Body::from(b.to_string()))
            .unwrap(),
        None => req.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1 << 22).await.unwrap();
    let v = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (status, v)
}

fn names(v: &Value) -> Vec<String> {
    let mut out: Vec<String> = v
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|c| c["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

#[tokio::test]
async fn a_key_is_scoped_by_clearances_read_only_and_expiry_and_rotates() {
    let w = world("keys", false).await;
    let (app, admin) = (&w.app, w.admin.as_str());

    // A scoped key: sees the public sheets only, may not write.
    let (st, v) = call(
        app,
        "POST",
        "/auth/keys",
        Some(admin),
        Some(json!({ "name": "ERP", "clearances": [], "read_only": true, "expires_in_days": 30 })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let key = v["key"].as_str().unwrap().to_string();
    let key_id = v["id"].as_str().unwrap().to_string();
    assert_eq!(v["readOnly"], true);
    assert!(v["expiresAt"].as_u64().unwrap() > 0);
    let (_, v) = call(app, "GET", "/concepts", Some(&key), None).await;
    assert_eq!(names(&v["concepts"]), vec!["payslip-march", "public-sheet"]);
    let (st, _) = call(
        app,
        "POST",
        "/concepts",
        Some(&key),
        Some(json!({ "id": 0, "concept_type": "Topic", "name": "x" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "read-only");
    let (st, _) = call(
        app,
        "POST",
        "/retrieve",
        Some(&key),
        Some(json!({ "query": "public" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "a retrieval is a read");
    let (_, v) = call(app, "GET", "/auth/keys", Some(admin), None).await;
    let listed = v["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["id"] == key_id)
        .unwrap()
        .clone();
    assert_eq!(listed["readOnly"], true);
    assert!(
        listed["lastUsedAt"].as_str().is_some(),
        "last use noted: {listed}"
    );

    // An unscoped key sees everything and writes.
    let (_, v) = call(
        app,
        "POST",
        "/auth/keys",
        Some(admin),
        Some(json!({ "name": "full" })),
    )
    .await;
    let full = v["key"].as_str().unwrap().to_string();
    let (_, v) = call(app, "GET", "/concepts", Some(&full), None).await;
    assert_eq!(v["total"], 3);
    let (st, _) = call(
        app,
        "POST",
        "/concepts",
        Some(&full),
        Some(json!({ "id": 0, "concept_type": "Topic", "name": "by-key" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);

    // Rotation: the old secret is dead, the new one keeps the scope.
    let (st, v) = call(
        app,
        "POST",
        &format!("/auth/keys/{key_id}/rotate"),
        Some(admin),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let rotated = v["key"].as_str().unwrap().to_string();
    assert_ne!(rotated, key);
    let (st, _) = call(app, "GET", "/concepts", Some(&key), None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (_, v) = call(app, "GET", "/concepts", Some(&rotated), None).await;
    assert_eq!(
        v["total"], 3,
        "still scoped: hr-sheet hidden, by-key visible"
    );
    let (st, _) = call(app, "POST", "/auth/keys/nope/rotate", Some(admin), None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _) = call(
        app,
        "POST",
        "/auth/keys",
        Some(admin),
        Some(json!({ "name": "bad", "clearances": ["Not Ok"] })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    // Expiry: a key whose time has passed is refused.
    let users: Value =
        serde_json::from_slice(&std::fs::read(w.dir.join("users.json")).unwrap()).unwrap();
    let mut edited = users.clone();
    for k in edited["keys"].as_array_mut().unwrap() {
        if k["id"] == key_id {
            k["expiresAt"] = json!(1);
        }
    }
    std::fs::write(w.dir.join("users.json"), edited.to_string()).unwrap();
    let store = UserStore::open(w.dir.join("users.json")).unwrap();
    assert!(store.key_by_secret(&rotated).is_none(), "expired");
    assert!(store.key_by_secret(&full).is_some());
}

#[tokio::test]
async fn the_audit_log_chains_every_event_and_is_verified() {
    let w = world("audit", false).await;
    let (app, admin) = (&w.app, w.admin.as_str());

    // Sign-in events: a refusal and a success, outside the protected router.
    let (st, _) = call(
        app,
        "POST",
        "/auth/login",
        None,
        Some(json!({ "email": "alice@example.com", "password": "wrong-pass-1" })),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _) = call(
        app,
        "POST",
        "/auth/login",
        None,
        Some(json!({ "email": "alice@example.com", "password": "Passw0rd!x" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    // A question: logged as a read with the domains it touched; the text hashed.
    let (st, _) = call(
        app,
        "POST",
        "/ask",
        Some(admin),
        Some(json!({ "query": "payslip-march about", "top_k": 3, "lexical_weight": 1.0 })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    // An access change and a key.
    let (_, v) = call(
        app,
        "POST",
        "/auth/users",
        Some(admin),
        Some(json!({ "email": "bob@example.com", "name": "Bob", "password": "Passw0rd!x" })),
    )
    .await;
    let bob = v["user"]["id"].as_str().unwrap().to_string();
    let (st, _) = call(
        app,
        "PATCH",
        &format!("/auth/users/{bob}"),
        Some(admin),
        Some(json!({ "clearances": ["hr"] })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = call(
        app,
        "POST",
        "/auth/keys",
        Some(admin),
        Some(json!({ "name": "ERP" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    let (st, v) = call(app, "GET", "/audit?limit=100", Some(admin), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let entries = v["entries"].as_array().unwrap();
    let find = |path: &str, detail_part: &str| -> Value {
        entries
            .iter()
            .find(|e| e["path"] == path && e["detail"].as_str().unwrap_or("").contains(detail_part))
            .unwrap_or_else(|| panic!("no entry {path} {detail_part} in {v}"))
            .clone()
    };
    let refused = find("/auth/login", "refused");
    assert_eq!(refused["who"], "alice@example.com");
    assert_eq!(refused["status"], 401);
    assert_eq!(find("/auth/login", "sign-in: password")["status"], 200);
    let ask = find("/ask", "domains=");
    assert_eq!(ask["who"], "alice@example.com");
    assert!(
        ask["detail"].as_str().unwrap().contains("domains=payroll"),
        "{ask}"
    );
    assert!(
        ask["detail"].as_str().unwrap().contains("q=sha256:"),
        "hashed: {ask}"
    );
    assert!(!ask["detail"]
        .as_str()
        .unwrap()
        .contains("payslip-march about"));
    let access = find(&format!("/auth/users/{bob}"), "access:");
    assert!(
        access["detail"].as_str().unwrap().contains("clearances=hr"),
        "{access}"
    );
    find("/auth/keys", "key created: ERP");
    // Every line chained to the one before.
    let mut prev = String::new();
    for e in entries {
        assert_eq!(e["prev"].as_str().unwrap_or(""), prev, "{e}");
        prev = e["hash"].as_str().unwrap().to_string();
        assert_eq!(prev.len(), 64);
    }
    let n = w.audit.verify().unwrap();
    assert_eq!(n, entries.len());

    // Filters.
    let (_, v) = call(
        app,
        "GET",
        "/audit?who=alice@example.com&path=/auth/login",
        Some(admin),
        None,
    )
    .await;
    assert!(v["entries"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["path"] == "/auth/login"));
    assert_eq!(v["entries"].as_array().unwrap().len(), 2);
    let since = entries.last().unwrap()["ts"].as_str().unwrap();
    let (_, v) = call(
        app,
        "GET",
        &format!("/audit?since={since}"),
        Some(admin),
        None,
    )
    .await;
    assert!(!v["entries"].as_array().unwrap().is_empty());
    let (_, v) = call(
        app,
        "GET",
        "/audit?since=2999-01-01T00:00:00Z",
        Some(admin),
        None,
    )
    .await;
    assert!(v["entries"].as_array().unwrap().is_empty());

    // Tampering breaks the chain; a reopened log continues it.
    let path = w.dir.join("audit.jsonl");
    let reopened = AuditLog::open(&path).unwrap();
    reopened.event("alice@example.com", "POST", "/x", 200, "after reopen");
    assert_eq!(reopened.verify().unwrap(), entries.len() + 1);
    let raw = std::fs::read_to_string(&path).unwrap();
    let altered = raw.replacen("sign-in: password", "sign-in: forged", 1);
    assert_ne!(raw, altered);
    std::fs::write(&path, altered).unwrap();
    let err = AuditLog::open(&path).unwrap().verify().unwrap_err();
    assert!(err.contains("altered"), "{err}");
    let mut lines: Vec<&str> = raw.lines().collect();
    lines.remove(2);
    std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    let err = AuditLog::open(&path).unwrap().verify().unwrap_err();
    assert!(err.contains("does not follow"), "{err}");
}

#[tokio::test]
async fn questions_are_kept_verbatim_only_when_asked() {
    let w = world("verbatim", true).await;
    let (app, admin) = (&w.app, w.admin.as_str());
    let (st, _) = call(
        app,
        "POST",
        "/retrieve",
        Some(admin),
        Some(json!({ "query": "hr-sheet about", "top_k": 3 })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (_, v) = call(app, "GET", "/audit?path=/retrieve", Some(admin), None).await;
    let e = &v["entries"][0];
    assert!(
        e["detail"].as_str().unwrap().ends_with("q=hr-sheet about"),
        "{e}"
    );
}

/// An OpenAI-compatible endpoint that answers every completion with a
/// fixed sentence naming itself.
async fn mock_openai() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = Router::new().route(
        "/v1/chat/completions",
        post(|Json(body): Json<Value>| async move {
            if body["stream"] == json!(true) {
                let sse = concat!(
                    "data: {\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"model\":\"local-llm\",",
                    "\"choices\":[{\"index\":0,\"delta\":{\"content\":\"answered on the premises\"},\"finish_reason\":null}]}

",
                    "data: {\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"model\":\"local-llm\",",
                    "\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}

",
                    "data: [DONE]

"
                );
                return (
                    [(http::header::CONTENT_TYPE, "text/event-stream")],
                    sse.to_string(),
                )
                    .into_response();
            }
            Json(json!({
                "id": "x", "object": "chat.completion", "model": "local-llm",
                "choices": [{ "index": 0, "finish_reason": "stop",
                    "message": { "role": "assistant", "content": "answered on the premises" } }],
                "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
            }))
            .into_response()
        }),
    );
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

#[tokio::test]
async fn a_question_touching_an_on_premises_domain_stays_on_the_premises() {
    let w = world("egress", false).await;
    let (app, admin) = (&w.app, w.admin.as_str());
    let payroll = json!({ "query": "payslip-march about", "top_k": 3, "lexical_weight": 1.0 });
    let public = json!({ "query": "public-sheet about", "top_k": 3, "lexical_weight": 1.0 });

    // No policy: the fallback (echo) answers everything.
    let (st, v) = call(app, "POST", "/ask", Some(admin), Some(payroll.clone())).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["model"], "echo");

    // The policy without an on-premises provider: refused, with the reason.
    let (st, v) = call(
        app,
        "PATCH",
        "/settings",
        Some(admin),
        Some(json!({ "llm": { "on_premises_domains": [" payroll ", ""] } })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["llm"]["on_premises_domains"], json!(["payroll"]));
    let (st, v) = call(app, "POST", "/ask", Some(admin), Some(payroll.clone())).await;
    assert!(!st.is_success(), "{st} {v}");
    assert!(v.to_string().contains("must not leave the premises"), "{v}");
    let (st, v) = call(app, "POST", "/ask", Some(admin), Some(public.clone())).await;
    assert_eq!(
        st,
        StatusCode::OK,
        "a question off the domain is unaffected: {v}"
    );
    assert_eq!(v["model"], "echo");

    // With a local provider: the payroll question goes there, the other
    // one keeps the fallback (no provider is active).
    let base = mock_openai().await;
    let (st, _) = call(
        app,
        "PATCH",
        "/settings",
        Some(admin),
        Some(json!({ "llm": { "on_premises_provider": "openai", "openai_api_key": "local-key", "openai_base_url": base, "openai_model": "local-llm" } })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, v) = call(app, "POST", "/ask", Some(admin), Some(payroll.clone())).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(
        v["answer"]
            .as_str()
            .unwrap()
            .contains("answered on the premises"),
        "{v}"
    );
    let (st, v) = call(app, "POST", "/ask", Some(admin), Some(public)).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["model"], "echo");
    // The streaming route takes the same decision.
    let req = Request::builder()
        .method("POST")
        .uri("/ask/stream")
        .header("authorization", format!("Bearer {admin}"))
        .header("content-type", "application/json")
        .body(Body::from(payroll.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 1 << 22).await.unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("answered on the premises"));
}
