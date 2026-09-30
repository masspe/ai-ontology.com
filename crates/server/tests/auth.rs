// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Built-in authentication (ROADMAP.md §3.8.3) and the web UI served by the
//! binary (§3.8.2): the `/auth/*` contract of the Node server, the users
//! file it wrote, the sign-up policy, administration, and the static
//! fallback that hands every non-API path to `index.html`.

mod hardening_common;

use axum::body::{to_bytes, Body};
use axum::Router;
use hardening_common::*;
use http::{Request, StatusCode};
use ontology_graph::{Ontology, OntologyGraph};
use ontology_server::auth::{UserAuth, UserStore};
use ontology_server::{build_router_with_config, JwtAuth, RouterConfig};
use ontology_storage::{MemoryStore, Store};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

const SECRET: &[u8] = b"test-secret-for-the-built-in-auth";
/// `bcryptjs.hashSync("Passw0rd!", 10)` from the Node server's dependency:
/// a users file it wrote must keep working.
const NODE_HASH: &str = "$2a$10$YSxhPGZqFhjT.ap9QPAH9uOW6dtDJB4YfxEil/fqO0JWsDqtOVUmK";

fn tempdir(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "ontology-auth-{tag}-{}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn app(users_file: PathBuf, allow_signup: bool, web_dir: Option<PathBuf>) -> Router {
    app_with(users_file, allow_signup, web_dir, None)
}

fn app_with(
    users_file: PathBuf,
    allow_signup: bool,
    web_dir: Option<PathBuf>,
    bearer_token: Option<String>,
) -> Router {
    let graph = OntologyGraph::with_arc(Ontology::new());
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    build_router_with_config(
        state_with(store, graph),
        RouterConfig {
            bearer_token,
            jwt: Some(JwtAuth::from_secret(SECRET.to_vec())),
            rate_limit: None,
            users: Some(UserAuth {
                store: Arc::new(UserStore::open(users_file).expect("users file")),
                allow_signup,
            }),
            web_dir,
            audit: None,
        },
    )
}

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value, http::HeaderMap) {
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
    let headers = resp.headers().clone();
    let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value, headers)
}

async fn text(app: &Router, uri: &str) -> (StatusCode, String) {
    let resp = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// A browser navigation: `Accept: text/html`.
async fn browse(app: &Router, uri: &str) -> (StatusCode, String) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("accept", "text/html,application/xhtml+xml,*/*;q=0.8")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn creds(email: &str, password: &str, name: &str) -> Value {
    json!({ "email": email, "password": password, "name": name })
}

#[tokio::test]
async fn first_account_is_the_administrator_and_signup_then_closes() {
    let dir = tempdir("policy");
    let users = dir.join("users.json");
    let app = app(users.clone(), false, None);

    // Validation, as the Node server: email, password strength, name.
    let (st, v, _) = call(
        &app,
        "POST",
        "/auth/signup",
        None,
        Some(creds("nope", "Passw0rd!", "A")),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert_eq!(v["error"], "Invalid email");
    let (st, v, _) = call(
        &app,
        "POST",
        "/auth/signup",
        None,
        Some(creds("a@b.co", "weak", "A")),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().unwrap().starts_with("Weak password"));
    let (st, v, _) = call(
        &app,
        "POST",
        "/auth/signup",
        None,
        Some(creds("a@b.co", "Passw0rd!", " ")),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert_eq!(v["error"], "Name required");

    // First account: created, administrator, token issued.
    let (st, v, _) = call(
        &app,
        "POST",
        "/auth/signup",
        None,
        Some(creds("Admin@Example.com", "Passw0rd!", "Ada")),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    assert_eq!(v["user"]["email"], "admin@example.com", "lower-cased");
    assert_eq!(v["user"]["name"], "Ada");
    assert_eq!(v["user"]["role"], "admin");
    assert!(v["user"].get("passwordHash").is_none(), "never exposed");
    let admin_token = v["token"].as_str().unwrap().to_string();

    // The users file exists, in the Node server's shape.
    let raw: Value = serde_json::from_slice(&std::fs::read(&users).unwrap()).unwrap();
    assert_eq!(raw["users"].as_array().unwrap().len(), 1);
    assert!(raw["users"][0]["passwordHash"]
        .as_str()
        .unwrap()
        .starts_with("$2"));
    assert_eq!(raw["users"][0]["providers"], json!([]));
    assert!(raw["users"][0]["createdAt"]
        .as_str()
        .unwrap()
        .ends_with('Z'));

    // Second sign-up: closed.
    let (st, v, _) = call(
        &app,
        "POST",
        "/auth/signup",
        None,
        Some(creds("bob@example.com", "Passw0rd!", "Bob")),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{v}");

    // The token opens the API and /auth/me.
    let (st, v, _) = call(&app, "GET", "/auth/me", Some(&admin_token), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["user"]["role"], "admin");
    let (st, _, _) = call(&app, "GET", "/stats", Some(&admin_token), None).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _, _) = call(&app, "GET", "/auth/me", None, None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);

    // The administrator creates the next account; that user is not an admin.
    let (st, v, _) = call(
        &app,
        "POST",
        "/auth/users",
        Some(&admin_token),
        Some(creds("bob@example.com", "Passw0rd!", "Bob")),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    assert_eq!(v["user"]["role"], "user");
    let bob_id = v["user"]["id"].as_str().unwrap().to_string();
    let (st, v, _) = call(
        &app,
        "POST",
        "/auth/users",
        Some(&admin_token),
        Some(creds("bob@example.com", "Passw0rd!", "Bob")),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");

    // Bob logs in, cannot administer, sees himself.
    let (st, v, _) = call(
        &app,
        "POST",
        "/auth/login",
        None,
        Some(json!({ "email": "BOB@example.com", "password": "Passw0rd!" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let bob_token = v["token"].as_str().unwrap().to_string();
    let (st, _, _) = call(&app, "GET", "/auth/users", Some(&bob_token), None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, _, _) = call(
        &app,
        "POST",
        "/auth/users",
        Some(&bob_token),
        Some(creds("c@example.com", "Passw0rd!", "C")),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, v, _) = call(&app, "GET", "/auth/me", Some(&bob_token), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["user"]["email"], "bob@example.com");

    // Wrong password, unknown email, empty body: 401 / 401 / 400.
    let (st, _, _) = call(
        &app,
        "POST",
        "/auth/login",
        None,
        Some(json!({ "email": "bob@example.com", "password": "nope" })),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _, _) = call(
        &app,
        "POST",
        "/auth/login",
        None,
        Some(json!({ "email": "ghost@example.com", "password": "Passw0rd!" })),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _, _) = call(&app, "POST", "/auth/login", None, Some(json!({}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    // Listing and deletion are the administrator's; not their own account.
    let (st, v, _) = call(&app, "GET", "/auth/users", Some(&admin_token), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["users"].as_array().unwrap().len(), 2);
    let admin_id = v["users"][0]["id"].as_str().unwrap().to_string();
    let (st, _, _) = call(
        &app,
        "DELETE",
        &format!("/auth/users/{admin_id}"),
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _, _) = call(
        &app,
        "DELETE",
        &format!("/auth/users/{bob_id}"),
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, _, _) = call(
        &app,
        "DELETE",
        &format!("/auth/users/{bob_id}"),
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    // Bob's token names an account that no longer exists: refused
    // everywhere, not only on /auth/me, until it would have expired.
    let (st, _, _) = call(&app, "GET", "/auth/me", Some(&bob_token), None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _, _) = call(&app, "GET", "/stats", Some(&bob_token), None).await;
    assert_eq!(
        st,
        StatusCode::UNAUTHORIZED,
        "deleted user's token on the API"
    );

    // Logout is stateless.
    let (st, v, _) = call(&app, "POST", "/auth/logout", None, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["ok"], true);

    // A fresh router over the same file sees the same accounts.
    let again = self::app(users, false, None);
    let (st, v, _) = call(
        &again,
        "POST",
        "/auth/login",
        None,
        Some(json!({ "email": "admin@example.com", "password": "Passw0rd!" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["user"]["role"], "admin");
}

#[tokio::test]
async fn a_users_file_written_by_the_node_server_still_logs_in() {
    let dir = tempdir("node");
    let users = dir.join("users.json");
    std::fs::write(
        &users,
        json!({ "users": [{
            "id": "8b7f3a2c-0000-4000-8000-000000000001",
            "email": "legacy@example.com",
            "name": "Legacy",
            "picture": null,
            "passwordHash": NODE_HASH,
            "providers": [{ "provider": "google", "sub": "123" }],
            "createdAt": "2026-09-01T10:00:00.000Z"
        }] })
        .to_string(),
    )
    .unwrap();
    let app = app(users, false, None);
    let (st, v, _) = call(
        &app,
        "POST",
        "/auth/login",
        None,
        Some(json!({ "email": "legacy@example.com", "password": "Passw0rd!" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(
        v["user"]["role"], "user",
        "no role in the file means a plain user"
    );
    assert_eq!(v["user"]["providers"][0]["provider"], "google");
    // The store is not empty, so sign-up is closed even though nobody is admin.
    let (st, _, _) = call(
        &app,
        "POST",
        "/auth/signup",
        None,
        Some(creds("new@example.com", "Passw0rd!", "New")),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let token = v["token"].as_str().unwrap();
    let (st, v, _) = call(&app, "GET", "/auth/me", Some(token), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
}

#[tokio::test]
async fn a_service_token_opens_the_api_but_is_nobody_for_auth_me() {
    let dir = tempdir("service");
    let app = app_with(
        dir.join("users.json"),
        false,
        None,
        Some("svc-token".into()),
    );
    let (st, _, _) = call(&app, "GET", "/stats", Some("svc-token"), None).await;
    assert_eq!(st, StatusCode::OK);
    let (st, v, _) = call(&app, "GET", "/auth/me", Some("svc-token"), None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED, "{v}");
    let (st, _, _) = call(&app, "GET", "/auth/users", Some("svc-token"), None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn login_attempts_are_rate_limited() {
    // The bucket is 30 a minute per client; bcrypt makes a login slow
    // enough to refill it, so the cheap route under the same limit shows
    // the cut-off: 30 pass, the 31st is refused.
    let dir = tempdir("rate");
    let app = app(dir.join("users.json"), false, None);
    let mut refused_at = None;
    for i in 1..=40 {
        let (st, _, _) = call(&app, "POST", "/auth/logout", None, None).await;
        if st == StatusCode::TOO_MANY_REQUESTS {
            refused_at = Some(i);
            break;
        }
        assert_eq!(st, StatusCode::OK, "attempt {i}");
    }
    assert_eq!(refused_at, Some(31), "30 a minute, then 429");
    // The limit guards /auth/* only: the rest is untouched.
    let (st, _) = text(&app, "/healthz").await;
    assert_eq!(st, StatusCode::OK);
}

#[tokio::test]
async fn open_signup_keeps_creating_plain_users() {
    let dir = tempdir("open");
    let app = app(dir.join("users.json"), true, None);
    let (st, v, _) = call(
        &app,
        "POST",
        "/auth/signup",
        None,
        Some(creds("one@example.com", "Passw0rd!", "One")),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    assert_eq!(v["user"]["role"], "admin");
    let (st, v, _) = call(
        &app,
        "POST",
        "/auth/signup",
        None,
        Some(creds("two@example.com", "Passw0rd!", "Two")),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    assert_eq!(v["user"]["role"], "user");
    let (st, v, _) = call(
        &app,
        "POST",
        "/auth/signup",
        None,
        Some(creds("two@example.com", "Passw0rd!", "Two")),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    // A name left empty falls back to the local part, as the Node server did.
    let (st, v, _) = call(
        &app,
        "POST",
        "/auth/signup",
        None,
        Some(creds("three.four@example.com", "Passw0rd!", "")),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    let admin = call(
        &app,
        "POST",
        "/auth/login",
        None,
        Some(json!({ "email": "one@example.com", "password": "Passw0rd!" })),
    )
    .await
    .1;
    let (st, v, _) = call(
        &app,
        "POST",
        "/auth/users",
        admin["token"].as_str(),
        Some(creds("three.four@example.com", "Passw0rd!", "")),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    assert_eq!(v["user"]["name"], "three.four");
}

#[tokio::test]
async fn oauth_routes_bounce_to_the_login_page_until_configured() {
    let dir = tempdir("oauth");
    let app = app(dir.join("users.json"), false, None);
    for uri in [
        "/auth/oauth/google/start?next=%2F",
        "/auth/oauth/microsoft/callback?code=x",
    ] {
        let (st, _, headers) = call(&app, "GET", uri, None, None).await;
        assert_eq!(st, StatusCode::SEE_OTHER, "{uri}");
        let loc = headers.get("location").unwrap().to_str().unwrap();
        assert!(loc.starts_with("/login?error="), "{loc}");
        assert!(loc.ends_with("_not_configured"), "{loc}");
    }
    let (_, _, headers) = call(&app, "GET", "/auth/oauth/ev%22il/start", None, None).await;
    let loc = headers.get("location").unwrap().to_str().unwrap();
    assert_eq!(
        loc, "/login?error=evil_not_configured",
        "provider name sanitised"
    );
}

#[tokio::test]
async fn a_corrupt_users_file_refuses_to_start() {
    let dir = tempdir("corrupt");
    let users = dir.join("users.json");
    std::fs::write(&users, b"{ not json").unwrap();
    let err = ontology_server::auth::UserStore::open(&users)
        .err()
        .expect("must fail");
    assert!(err.to_string().contains("not valid JSON"), "{err}");
    let store = ontology_server::auth::UserStore::open(dir.join("absent.json")).unwrap();
    assert!(store.is_empty());
    assert_eq!(store.len(), 0);
}

#[tokio::test]
async fn the_web_ui_is_served_for_every_non_api_path() {
    let dir = tempdir("web");
    let web = dir.join("dist");
    std::fs::create_dir_all(web.join("assets")).unwrap();
    std::fs::write(web.join("index.html"), "<!doctype html><title>UI</title>").unwrap();
    std::fs::write(web.join("assets").join("app.js"), "console.log('ui')").unwrap();
    let app = app(dir.join("users.json"), false, Some(web));

    let (st, body) = text(&app, "/").await;
    assert_eq!(st, StatusCode::OK);
    assert!(body.contains("<title>UI</title>"), "{body}");
    let (st, body) = text(&app, "/assets/app.js").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body, "console.log('ui')");
    // Client-side routes get index.html, so a reload on /concepts works.
    let (st, body) = text(&app, "/concepts/edit/42").await;
    assert_eq!(st, StatusCode::OK);
    assert!(body.contains("<title>UI</title>"));
    let (st, body) = text(&app, "/login?next=%2Fgraph").await;
    assert_eq!(st, StatusCode::OK);
    assert!(body.contains("<title>UI</title>"));
    // API routes win over the fallback: /concepts is the API, protected …
    let (st, _) = text(&app, "/concepts").await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    // … unless a browser navigates there (a reload of the Concepts page):
    // then it is the UI, and the same for the other pages sharing a path.
    for page in [
        "/concepts",
        "/files",
        "/rules",
        "/queries",
        "/actions",
        "/settings",
    ] {
        let (st, body) = browse(&app, page).await;
        assert_eq!(st, StatusCode::OK, "{page}");
        assert!(body.contains("<title>UI</title>"), "{page}: {body}");
    }
    // A browser navigating to an API-only path still gets the API.
    let (st, body) = browse(&app, "/healthz").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body, "ok");
    let (st, _) = browse(&app, "/auth/me").await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, body) = text(&app, "/healthz").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body, "ok");
}

#[tokio::test]
async fn without_a_web_dir_unknown_paths_are_refused_by_the_auth_layer() {
    // No web dir: the protected router's fallback answers, behind its
    // auth layer, as it always did.
    let dir = tempdir("noweb");
    let app = app(dir.join("users.json"), false, None);
    let (st, _) = text(&app, "/anything").await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

/// A router with the built-in login and an audit log at `audit`.
fn app_audited(users_file: PathBuf, audit: PathBuf) -> Router {
    let graph = OntologyGraph::with_arc(Ontology::new());
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    build_router_with_config(
        state_with(store, graph),
        RouterConfig {
            bearer_token: None,
            jwt: Some(JwtAuth::from_secret(SECRET.to_vec())),
            rate_limit: None,
            users: Some(UserAuth {
                store: Arc::new(UserStore::open(users_file).expect("users file")),
                allow_signup: false,
            }),
            web_dir: None,
            audit: Some(Arc::new(
                ontology_server::audit::AuditLog::open(audit).expect("audit log"),
            )),
        },
    )
}

async fn admin_token(app: &Router) -> String {
    let (st, v, _) = call(
        app,
        "POST",
        "/auth/signup",
        None,
        Some(creds("admin@example.com", "Passw0rd!", "Admin")),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    v["token"].as_str().unwrap().to_string()
}

/// Named API keys (ROADMAP §3.8.6): minted by an administrator, shown
/// once, stored hashed, usable as a bearer for the API but nobody for
/// `/auth/me`, revoked at once.
#[tokio::test]
async fn api_keys_are_minted_once_stored_hashed_and_revoked_at_once() {
    let dir = tempdir("keys");
    let users = dir.join("users.json");
    let app = app(users.clone(), false, None);
    let admin = admin_token(&app).await;

    // Validation, then a key.
    let (st, v, _) = call(
        &app,
        "POST",
        "/auth/keys",
        Some(&admin),
        Some(json!({ "name": "  " })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    let (st, v, _) = call(
        &app,
        "POST",
        "/auth/keys",
        Some(&admin),
        Some(json!({ "name": "ERP integration" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let secret = v["key"].as_str().unwrap().to_string();
    assert!(
        secret.starts_with("ok_") && secret.len() == 3 + 64,
        "{secret}"
    );
    assert_eq!(v["name"], "ERP integration");
    assert_eq!(v["prefix"], &secret[..11]);
    assert_eq!(v["createdBy"], "admin@example.com");
    let id = v["id"].as_str().unwrap().to_string();

    // Listed without the secret; the file holds a hash, never the secret.
    let (st, v, _) = call(&app, "GET", "/auth/keys", Some(&admin), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["keys"].as_array().unwrap().len(), 1);
    assert!(
        v["keys"][0].get("key").is_none() && v["keys"][0].get("hash").is_none(),
        "{v}"
    );
    let file = std::fs::read_to_string(&users).unwrap();
    assert!(
        !file.contains(&secret) && file.contains("\"hash\""),
        "{file}"
    );

    // The key opens the API, is a service caller for /auth/me, cannot
    // manage keys or users.
    let (st, v, _) = call(&app, "GET", "/stats", Some(&secret), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (st, _, _) = call(&app, "GET", "/auth/me", Some(&secret), None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _, _) = call(&app, "GET", "/auth/keys", Some(&secret), None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    // A key writes data but never wipes the tenant nor runs its backup.
    let (st, v, _) = call(&app, "POST", "/reset", Some(&secret), None).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{v}");
    let (st, v, _) = call(&app, "POST", "/backup", Some(&secret), None).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{v}");
    let (st, v, _) = call(&app, "POST", "/backup", Some(&admin), None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}"); // not configured, but allowed
                                                    // An unknown key, or a key-shaped garbage, is refused.
    let (st, _, _) = call(&app, "GET", "/stats", Some("ok_nope"), None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);

    // A plain user cannot mint keys.
    let (st, v, _) = call(
        &app,
        "POST",
        "/auth/users",
        Some(&admin),
        Some(creds("u@example.com", "Passw0rd!", "U")),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let (st, v, _) = call(
        &app,
        "POST",
        "/auth/login",
        None,
        Some(creds("u@example.com", "Passw0rd!", "")),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let user = v["token"].as_str().unwrap().to_string();
    let (st, _, _) = call(
        &app,
        "POST",
        "/auth/keys",
        Some(&user),
        Some(json!({ "name": "x" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // Revoked: gone at once, twice is not found.
    let (st, _, _) = call(
        &app,
        "DELETE",
        &format!("/auth/keys/{id}"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, _, _) = call(&app, "GET", "/stats", Some(&secret), None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _, _) = call(
        &app,
        "DELETE",
        &format!("/auth/keys/{id}"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // The keys survive a reopen of the file.
    let (st, v, _) = call(
        &app,
        "POST",
        "/auth/keys",
        Some(&admin),
        Some(json!({ "name": "again" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let secret2 = v["key"].as_str().unwrap().to_string();
    let reopened = self::app(users.clone(), false, None);
    let (st, _, _) = call(&reopened, "GET", "/stats", Some(&secret2), None).await;
    assert_eq!(st, StatusCode::OK);
}

/// The audit log (ROADMAP §3.8.6): every successful write with its caller
/// (a user's email, a key's name), nothing for reads or refused writes,
/// read back by administrators only.
#[tokio::test]
async fn writes_are_audited_with_their_caller_and_read_by_administrators() {
    let dir = tempdir("audit");
    let audit = dir.join("logs").join("audit.jsonl");
    let app = app_audited(dir.join("users.json"), audit.clone());
    let admin = admin_token(&app).await;

    // A write by the administrator, a read, a refused write, a write by a key.
    let onto = json!({ "concept_types": { "Topic": { "name": "Topic" } }, "relation_types": {} });
    let (st, v, _) = call(&app, "PUT", "/ontology", Some(&admin), Some(onto)).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (st, _, _) = call(&app, "GET", "/stats", Some(&admin), None).await;
    assert_eq!(st, StatusCode::OK);
    // A POST that reads (a retrieval) is not a write.
    let (st, _, _) = call(
        &app,
        "POST",
        "/retrieve",
        Some(&admin),
        Some(json!({ "query": "x" })),
    )
    .await;
    assert!(st.is_success(), "{st}");
    let (st, _, _) = call(
        &app,
        "POST",
        "/concepts",
        Some(&admin),
        Some(json!({ "id": 0, "concept_type": "Nope", "name": "x" })),
    )
    .await;
    assert!(st.is_client_error(), "{st}");
    let (_, v, _) = call(
        &app,
        "POST",
        "/auth/keys",
        Some(&admin),
        Some(json!({ "name": "ERP" })),
    )
    .await;
    let key = v["key"].as_str().unwrap().to_string();
    let (st, v, headers) = call(
        &app,
        "POST",
        "/concepts",
        Some(&key),
        Some(json!({ "id": 0, "concept_type": "Topic", "name": "A" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let rid = headers
        .get("x-request-id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    let (st, v, _) = call(&app, "GET", "/audit", Some(&admin), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let entries = v["entries"].as_array().unwrap();
    let rows: Vec<(String, String, String, u64)> = entries
        .iter()
        .map(|e| {
            (
                e["who"].as_str().unwrap().to_string(),
                e["method"].as_str().unwrap().to_string(),
                e["path"].as_str().unwrap().to_string(),
                e["status"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            (
                "admin@example.com".to_string(),
                "PUT".to_string(),
                "/ontology".to_string(),
                200
            ),
            (
                "admin@example.com".to_string(),
                "POST".to_string(),
                "/auth/keys".to_string(),
                201
            ),
            (
                "ERP".to_string(),
                "POST".to_string(),
                "/concepts".to_string(),
                200
            ),
        ],
        "{v}"
    );
    assert_eq!(entries[2]["request_id"], rid);
    assert!(entries[0]["ts"].as_str().unwrap().ends_with('Z'));
    // The file is what the API reads.
    assert_eq!(std::fs::read_to_string(&audit).unwrap().lines().count(), 3);

    // `limit` keeps the last ones; a plain user and a key are refused.
    let (_, v, _) = call(&app, "GET", "/audit?limit=1", Some(&admin), None).await;
    assert_eq!(v["entries"].as_array().unwrap().len(), 1);
    assert_eq!(v["entries"][0]["path"], "/concepts");
    let (st, _, _) = call(&app, "GET", "/audit", Some(&key), None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (_, _, _) = call(
        &app,
        "POST",
        "/auth/users",
        Some(&admin),
        Some(creds("u@example.com", "Passw0rd!", "U")),
    )
    .await;
    let (_, v, _) = call(
        &app,
        "POST",
        "/auth/login",
        None,
        Some(creds("u@example.com", "Passw0rd!", "")),
    )
    .await;
    let user = v["token"].as_str().unwrap().to_string();
    let (st, _, _) = call(&app, "GET", "/audit", Some(&user), None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

/// Without the built-in login (a service token only), the audit log names
/// the service and anyone with the token may read it.
#[tokio::test]
async fn without_the_login_the_audit_names_the_service_token() {
    let dir = tempdir("audit-service");
    let graph = OntologyGraph::with_arc(Ontology::new());
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let app = build_router_with_config(
        state_with(store, graph),
        RouterConfig {
            bearer_token: Some("svc".into()),
            jwt: None,
            rate_limit: None,
            users: None,
            web_dir: None,
            audit: Some(Arc::new(
                ontology_server::audit::AuditLog::open(dir.join("audit.jsonl")).unwrap(),
            )),
        },
    );
    let onto = json!({ "concept_types": { "Topic": { "name": "Topic" } }, "relation_types": {} });
    let (st, _, _) = call(&app, "PUT", "/ontology", Some("svc"), Some(onto)).await;
    assert_eq!(st, StatusCode::OK);
    let (st, v, _) = call(&app, "GET", "/audit", Some("svc"), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["entries"][0]["who"], "service");
    let (st, _, _) = call(&app, "GET", "/audit", None, None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}
