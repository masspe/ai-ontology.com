// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! The HTTP surface's hardening: security headers on every response
//! (HSTS once the public URL is HTTPS), CORS restricted to the public
//! URL once set, the CSRF header on the cookie routes, body limits, and
//! a matrix of who may call what.

mod hardening_common;

use axum::body::{to_bytes, Body};
use axum::Router;
use hardening_common::*;
use http::{Request, StatusCode};
use ontology_graph::{ConceptType, Ontology, OntologyGraph};
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
        "ontology-http-{tag}-{}-{}",
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
    o
}

struct World {
    app: Router,
    settings: Arc<parking_lot::RwLock<ontology_server::Settings>>,
}

fn world(tag: &str) -> World {
    let dir = tempdir(tag);
    let graph = OntologyGraph::with_arc(ontology());
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let state = state_with(store, graph);
    let settings = state.settings.clone();
    let app = build_router_with_config(
        state,
        RouterConfig {
            bearer_token: Some("service-token".into()),
            jwt: Some(JwtAuth::from_secret(SECRET.to_vec())),
            rate_limit: None,
            users: Some(UserAuth {
                store: Arc::new(UserStore::open(dir.join("users.json")).unwrap()),
                allow_signup: true,
            }),
            web_dir: None,
            audit: Some(Arc::new(AuditLog::open(dir.join("audit.jsonl")).unwrap())),
        },
    );
    World { app, settings }
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, http::HeaderMap, Value) {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = to_bytes(resp.into_body(), 1 << 22).await.unwrap();
    let v = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (status, headers, v)
}

fn req(method: &str, uri: &str, token: Option<&str>, body: Option<Value>) -> Request<Body> {
    let mut r = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        r = r.header("authorization", format!("Bearer {t}"));
    }
    match body {
        Some(b) => r
            .header("content-type", "application/json")
            .body(Body::from(b.to_string()))
            .unwrap(),
        None => r.body(Body::empty()).unwrap(),
    }
}

async fn signup(app: &Router, email: &str) -> (String, String) {
    let (st, headers, v) = send(
        app,
        req(
            "POST",
            "/auth/signup",
            None,
            Some(json!({ "email": email, "name": "x", "password": "Passw0rd!x" })),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let cookie = headers["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    (v["token"].as_str().unwrap().to_string(), cookie)
}

#[tokio::test]
async fn every_response_carries_the_security_headers_and_hsts_only_behind_https() {
    let w = world("headers");
    let (st, h, _) = send(&w.app, req("GET", "/healthz", None, None)).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(h["x-content-type-options"], "nosniff");
    assert_eq!(h["x-frame-options"], "DENY");
    assert_eq!(h["referrer-policy"], "strict-origin-when-cross-origin");
    assert!(h["content-security-policy"]
        .to_str()
        .unwrap()
        .contains("frame-ancestors 'none'"));
    assert!(h["permissions-policy"]
        .to_str()
        .unwrap()
        .contains("camera=()"));
    assert!(
        h.get("strict-transport-security").is_none(),
        "no HSTS on plain http"
    );
    // A refused request carries them too.
    let (st, h, _) = send(&w.app, req("GET", "/concepts", None, None)).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    assert_eq!(h["x-content-type-options"], "nosniff");

    w.settings.write().signin.public_url = "https://ontology.example".into();
    let (_, h, _) = send(&w.app, req("GET", "/healthz", None, None)).await;
    assert_eq!(
        h["strict-transport-security"],
        "max-age=31536000; includeSubDomains"
    );
}

#[tokio::test]
async fn cors_is_open_until_a_public_url_is_set_then_that_origin_only() {
    let w = world("cors");
    let with_origin = |origin: &str| {
        Request::builder()
            .method("GET")
            .uri("/healthz")
            .header("origin", origin)
            .body(Body::empty())
            .unwrap()
    };
    let (_, h, _) = send(&w.app, with_origin("http://localhost:5173")).await;
    assert_eq!(h["access-control-allow-origin"], "http://localhost:5173");

    w.settings.write().signin.public_url = "https://ontology.example".into();
    let (_, h, _) = send(&w.app, with_origin("https://ontology.example")).await;
    assert_eq!(h["access-control-allow-origin"], "https://ontology.example");
    let (st, h, _) = send(&w.app, with_origin("https://evil.example")).await;
    assert_eq!(st, StatusCode::OK, "the request itself is served");
    assert!(
        h.get("access-control-allow-origin").is_none(),
        "but not to that origin: {h:?}"
    );
}

#[tokio::test]
async fn the_cookie_routes_need_the_custom_header() {
    let w = world("csrf");
    let (_, cookie) = signup(&w.app, "ann@example.com").await;
    let cookie_req = |uri: &str, with_header: bool| {
        let mut r = Request::builder()
            .method("POST")
            .uri(uri)
            .header("cookie", cookie.clone());
        if with_header {
            r = r.header("x-requested-with", "ontology");
        }
        r.body(Body::empty()).unwrap()
    };
    // A cross-site form can send the cookie but not the header: refused.
    let (st, _, v) = send(&w.app, cookie_req("/auth/refresh", false)).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{v}");
    let (st, _, v) = send(&w.app, cookie_req("/auth/logout", false)).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{v}");
    // Our pages send it.
    let (st, _, v) = send(&w.app, cookie_req("/auth/refresh", true)).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    // Without a cookie there is nothing to protect: a logout is still ok.
    let (st, _, _) = send(&w.app, req("POST", "/auth/logout", None, None)).await;
    assert_eq!(st, StatusCode::OK);
}

#[tokio::test]
async fn request_bodies_are_capped() {
    let w = world("limits");
    let (admin, _) = signup(&w.app, "ann@example.com").await;
    let big = json!({ "id": 0, "concept_type": "Topic", "name": "big", "description": "x".repeat(9 * 1024 * 1024) });
    let (st, _, _) = send(&w.app, req("POST", "/concepts", Some(&admin), Some(big))).await;
    assert_eq!(st, StatusCode::PAYLOAD_TOO_LARGE);
    let fine = json!({ "id": 0, "concept_type": "Topic", "name": "fine", "description": "x".repeat(1024 * 1024) });
    let (st, _, v) = send(&w.app, req("POST", "/concepts", Some(&admin), Some(fine))).await;
    assert_eq!(st, StatusCode::OK, "{v}");
}

/// Who may call what: anonymous, a plain user, a read-only key, an
/// unscoped key, the service token, an administrator.
#[tokio::test]
async fn the_authorization_matrix_holds() {
    let w = world("matrix");
    let app = &w.app;
    let (admin, _) = signup(app, "admin@example.com").await;
    let (_, _, v) = send(
        app,
        req(
            "POST",
            "/auth/users",
            Some(&admin),
            Some(json!({ "email": "u@example.com", "name": "U", "password": "Passw0rd!x" })),
        ),
    )
    .await;
    let user_id = v["user"]["id"].as_str().unwrap().to_string();
    let (_, _, v) = send(
        app,
        req(
            "POST",
            "/auth/login",
            None,
            Some(json!({ "email": "u@example.com", "password": "Passw0rd!x" })),
        ),
    )
    .await;
    let user = v["token"].as_str().unwrap().to_string();
    let (_, _, v) = send(
        app,
        req(
            "POST",
            "/auth/keys",
            Some(&admin),
            Some(json!({ "name": "ro", "read_only": true })),
        ),
    )
    .await;
    let ro_key = v["key"].as_str().unwrap().to_string();
    let (_, _, v) = send(
        app,
        req(
            "POST",
            "/auth/keys",
            Some(&admin),
            Some(json!({ "name": "rw" })),
        ),
    )
    .await;
    let rw_key = v["key"].as_str().unwrap().to_string();
    let service = "service-token".to_string();

    // The body is rebuilt per call: a created concept must not collide
    // with the one the previous principal created.
    let concept = json!({ "id": 0, "concept_type": "Topic", "name": "fresh" });
    // (method, path, body, [anon, user, ro key, rw key, service, admin])
    let cases: Vec<(&str, String, Option<Value>, [u16; 6])> = vec![
        (
            "GET",
            "/concepts".into(),
            None,
            [401, 200, 200, 200, 200, 200],
        ),
        (
            "POST",
            "/concepts".into(),
            Some(concept),
            [401, 200, 403, 200, 200, 200],
        ),
        (
            "POST",
            "/retrieve".into(),
            Some(json!({ "query": "x" })),
            [401, 200, 200, 200, 200, 200],
        ),
        (
            "GET",
            "/export?format=json".into(),
            None,
            [401, 403, 200, 200, 200, 200],
        ),
        ("GET", "/audit".into(), None, [401, 403, 403, 403, 403, 200]),
        (
            "GET",
            "/auth/users".into(),
            None,
            [401, 403, 401, 401, 401, 200],
        ),
        (
            "PATCH",
            format!("/auth/users/{user_id}"),
            Some(json!({ "clearances": [] })),
            [401, 403, 403, 401, 401, 200],
        ),
        (
            "POST",
            "/backup".into(),
            None,
            [401, 400, 403, 403, 400, 400],
        ),
        // A read-only key is refused before the route can say it is nobody.
        (
            "DELETE",
            "/auth/sessions".into(),
            None,
            [401, 200, 403, 401, 401, 200],
        ),
    ];
    let principals: [Option<&str>; 6] = [
        None,
        Some(&user),
        Some(&ro_key),
        Some(&rw_key),
        Some(&service),
        Some(&admin),
    ];
    let names = [
        "anonymous",
        "user",
        "read-only key",
        "key",
        "service token",
        "admin",
    ];
    let mut failures = Vec::new();
    for (method, path, body, expected) in &cases {
        for (i, token) in principals.iter().enumerate() {
            let mut body = body.clone();
            if let Some(b) = body.as_mut().filter(|b| b["name"] == "fresh") {
                b["name"] = json!(format!("c{}", rand_suffix()));
            }
            let (st, _, v) = send(app, req(method, path, *token, body)).await;
            if st.as_u16() != expected[i] {
                failures.push(format!(
                    "{} as {}: {} ({}), expected {}",
                    path, names[i], st, v, expected[i]
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn rand_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}
