// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Sign-in through an OpenID Connect provider, against a provider spun
//! up in the test: discovery, JWKS, the token endpoint checking PKCE.
//! The happy path creates the account; every check of the flow (state,
//! nonce, issuer, audience, key, expiry, e-mail domain, verified e-mail)
//! refuses with its code; group claims decide the administrator role.

mod hardening_common;

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::Form;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use hardening_common::*;
use http::{Request, StatusCode};
use ontology_graph::{Ontology, OntologyGraph};
use ontology_server::auth::{UserAuth, UserStore};
use ontology_server::oidc::{OidcProvider, SigninSettings};
use ontology_server::{build_router_with_config, JwtAuth, RouterConfig};
use ontology_storage::{MemoryStore, Store};
use parking_lot::Mutex;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tower::ServiceExt;

const SECRET: &[u8] = b"test-secret-for-the-built-in-auth";
const DER_B64: &str = include_str!("fixtures/oidc_test_key.der.b64");
const MODULUS: &str = include_str!("fixtures/oidc_test_key.n.b64url");

fn tempdir(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "ontology-oidc-{tag}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// The provider: what it will put in the next `id_token`, and the PKCE
/// challenge it saw.
struct Mock {
    base: String,
    claims: Arc<Mutex<Value>>,
    challenge: Arc<Mutex<Option<String>>>,
}

impl Mock {
    /// A fresh id_token for `email` with our issuer and audience.
    fn will_sign(&self, email: &str, nonce: &str, extra: Value) {
        let mut c = json!({
            "iss": self.base,
            "aud": "cid",
            "sub": format!("sub-{email}"),
            "email": email,
            "name": email.split('@').next().unwrap(),
            "nonce": nonce,
            "iat": now(),
            "exp": now() + 300,
        });
        if let (Some(m), Some(e)) = (c.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                m.insert(k.clone(), v.clone());
            }
        }
        *self.claims.lock() = c;
    }
}

async fn mock_provider() -> Mock {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let claims = Arc::new(Mutex::new(json!({})));
    let challenge: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let der = base64::engine::general_purpose::STANDARD
        .decode(DER_B64.trim())
        .unwrap();

    let disc = {
        let base = base.clone();
        move || async move {
            Json(json!({
                "issuer": base,
                "authorization_endpoint": format!("{base}/authorize?tenant=t"),
                "token_endpoint": format!("{base}/token"),
                "jwks_uri": format!("{base}/jwks"),
            }))
        }
    };
    let jwks = || async {
        Json(json!({ "keys": [
            { "kty": "RSA", "kid": "k1", "use": "sig", "alg": "RS256", "n": MODULUS.trim(), "e": "AQAB" },
            { "kty": "EC", "kid": "ec", "crv": "P-256" }
        ] }))
    };
    let token = {
        let (claims, challenge) = (claims.clone(), challenge.clone());
        move |Form(f): Form<HashMap<String, String>>| async move {
            let s256 = |v: &str| {
                base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .encode(Sha256::digest(v.as_bytes()))
            };
            let ok = f.get("grant_type").map(String::as_str) == Some("authorization_code")
                && f.get("code").map(String::as_str) == Some("good-code")
                && f.get("client_id").map(String::as_str) == Some("cid")
                && f.get("client_secret").map(String::as_str) == Some("mock-secret")
                && f.get("redirect_uri").map(String::as_str)
                    == Some("http://app.test/auth/oauth/mock/callback")
                && challenge.lock().as_deref()
                    == f.get("code_verifier").map(|v| s256(v)).as_deref();
            if !ok {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": "invalid_grant" })),
                );
            }
            let c = claims.lock().clone();
            let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
            header.kid = Some(c["kid"].as_str().unwrap_or("k1").to_string());
            let id_token =
                jsonwebtoken::encode(&header, &c, &jsonwebtoken::EncodingKey::from_rsa_der(&der))
                    .unwrap();
            (
                StatusCode::OK,
                Json(json!({ "id_token": id_token, "access_token": "at", "token_type": "Bearer" })),
            )
        }
    };
    let router = Router::new()
        .route("/.well-known/openid-configuration", get(disc))
        .route("/jwks", get(jwks))
        .route("/token", post(token));
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    Mock {
        base,
        claims,
        challenge,
    }
}

fn provider(mock: &Mock) -> OidcProvider {
    OidcProvider {
        id: "mock".into(),
        label: "Mock IdP".into(),
        issuer: mock.base.clone(),
        client_id: "cid".into(),
        client_secret: "mock-secret".into(),
        allowed_domains: vec!["example.com".into()],
        admin_groups: vec!["ontology-admins".into()],
        ..Default::default()
    }
}

fn build(users_file: PathBuf, providers: Vec<OidcProvider>) -> Router {
    let graph = OntologyGraph::with_arc(Ontology::new());
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let state = state_with(store, graph);
    state.settings.write().signin = SigninSettings {
        public_url: "http://app.test".into(),
        providers,
    };
    build_router_with_config(
        state,
        RouterConfig {
            bearer_token: None,
            jwt: Some(JwtAuth::from_secret(SECRET.to_vec())),
            rate_limit: None,
            users: Some(UserAuth {
                store: Arc::new(UserStore::open(users_file).expect("users file")),
                allow_signup: false,
            }),
            web_dir: None,
            audit: None,
        },
    )
}

async fn req_get(
    app: &Router,
    uri: &str,
    cookie: Option<&str>,
    token: Option<&str>,
) -> http::Response<Body> {
    let mut req = Request::builder().method("GET").uri(uri);
    if let Some(c) = cookie {
        req = req.header("cookie", c);
    }
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    app.clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

fn location(resp: &http::Response<Body>) -> String {
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    resp.headers()["location"].to_str().unwrap().to_string()
}

fn query(url: &str) -> HashMap<String, String> {
    let q = url.split_once(['?', '#']).map(|(_, q)| q).unwrap_or("");
    q.split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_string(), percent_decode(v)))
        .collect()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

/// Start the flow: the provider's authorize URL (parsed) and the state
/// cookie to send back; the provider remembers the PKCE challenge.
async fn start(app: &Router, mock: &Mock, next: &str) -> (HashMap<String, String>, String) {
    let resp = req_get(
        app,
        &format!("/auth/oauth/mock/start?next={next}"),
        None,
        None,
    )
    .await;
    let loc = location(&resp);
    assert!(
        loc.starts_with(&format!("{}/authorize?tenant=t&", mock.base)),
        "{loc}"
    );
    let q = query(&loc);
    let cookie = resp.headers()["set-cookie"].to_str().unwrap().to_string();
    assert!(
        cookie.contains("HttpOnly") && cookie.contains("SameSite=Lax"),
        "{cookie}"
    );
    *mock.challenge.lock() = Some(q["code_challenge"].clone());
    let cookie = cookie.split(';').next().unwrap().to_string();
    (q, cookie)
}

/// Finish the flow; the bounce to the UI, parsed: `token`+`next` or `error`.
async fn callback(app: &Router, uri: &str, cookie: Option<&str>) -> HashMap<String, String> {
    let resp = req_get(app, uri, cookie, None).await;
    let loc = location(&resp);
    assert!(loc.starts_with("http://app.test/oauth/callback"), "{loc}");
    let cleared = resp.headers()["set-cookie"].to_str().unwrap().to_string();
    assert!(
        cleared.starts_with("oidc_state=;") && cleared.contains("Max-Age=0"),
        "{cleared}"
    );
    query(&loc)
}

async fn signin(app: &Router, mock: &Mock, email: &str, extra: Value) -> HashMap<String, String> {
    let (q, cookie) = start(app, mock, "%2Ffiles").await;
    mock.will_sign(email, &q["nonce"], extra);
    callback(
        app,
        &format!(
            "/auth/oauth/mock/callback?code=good-code&state={}",
            q["state"]
        ),
        Some(&cookie),
    )
    .await
}

async fn me(app: &Router, token: &str) -> Value {
    let resp = req_get(app, "/auth/me", None, Some(token)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    serde_json::from_slice::<Value>(&bytes).unwrap()["user"].clone()
}

#[tokio::test]
async fn a_user_of_an_allowed_domain_signs_in_and_gets_an_account() {
    let mock = mock_provider().await;
    let dir = tempdir("happy");
    let app = build(dir.join("users.json"), vec![provider(&mock)]);

    let resp = req_get(&app, "/auth/providers", None, None).await;
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap(),
        json!([{ "id": "mock", "label": "Mock IdP" }])
    );

    let (q, _) = start(&app, &mock, "%2F%2Fevil.example").await;
    assert_eq!(q["client_id"], "cid");
    assert_eq!(
        q["redirect_uri"],
        "http://app.test/auth/oauth/mock/callback"
    );
    assert_eq!(q["response_type"], "code");
    assert_eq!(q["scope"], "openid email profile");
    assert_eq!(q["code_challenge_method"], "S256");
    assert!(q["state"].len() >= 32 && q["nonce"].len() >= 32);

    let out = signin(
        &app,
        &mock,
        "Alice@Example.com",
        json!({ "picture": "http://p/a.png", "groups": ["ontology-admins"] }),
    )
    .await;
    assert_eq!(out.get("error"), None, "{out:?}");
    assert_eq!(out["next"], "/files");
    let user = me(&app, &out["token"]).await;
    assert_eq!(user["email"], "alice@example.com");
    assert_eq!(user["name"], "Alice");
    assert_eq!(user["role"], "admin", "in the admin group");
    assert_eq!(user["picture"], "http://p/a.png");
    // The same subject again, even with another e-mail: the same account.
    let again = signin(
        &app,
        &mock,
        "Alice@Example.com",
        json!({ "email": "renamed@example.com" }),
    )
    .await;
    assert_eq!(me(&app, &again["token"]).await["id"], user["id"]);
    let raw: Value =
        serde_json::from_slice(&std::fs::read(dir.join("users.json")).unwrap()).unwrap();
    assert_eq!(raw["users"].as_array().unwrap().len(), 1);
    assert_eq!(
        raw["users"][0]["providers"],
        json!([{ "provider": "mock", "sub": "sub-Alice@Example.com" }])
    );
    assert!(raw["users"][0]
        .get("passwordHash")
        .is_none_or(Value::is_null));
}

#[tokio::test]
async fn group_claims_decide_the_administrator_role() {
    let mock = mock_provider().await;
    let dir = tempdir("groups");
    let app = build(dir.join("users.json"), vec![provider(&mock)]);
    signin(&app, &mock, "first@example.com", json!({})).await;

    let out = signin(
        &app,
        &mock,
        "bob@example.com",
        json!({ "groups": ["staff"] }),
    )
    .await;
    assert_eq!(me(&app, &out["token"]).await["role"], "user");
    let out = signin(
        &app,
        &mock,
        "bob@example.com",
        json!({ "roles": ["ontology-admins"] }),
    )
    .await;
    assert_eq!(
        me(&app, &out["token"]).await["role"],
        "admin",
        "roles count as groups"
    );
    let out = signin(&app, &mock, "bob@example.com", json!({ "groups": [] })).await;
    assert_eq!(
        me(&app, &out["token"]).await["role"],
        "user",
        "taken away at the next sign-in"
    );

    // No admin groups configured: the provider does not decide.
    let mut p = provider(&mock);
    p.admin_groups.clear();
    let app = build(dir.join("users.json"), vec![p]);
    let out = signin(
        &app,
        &mock,
        "bob@example.com",
        json!({ "groups": ["ontology-admins"] }),
    )
    .await;
    assert_eq!(me(&app, &out["token"]).await["role"], "user");
}

#[tokio::test]
async fn an_existing_local_account_is_attached_by_verified_email_only() {
    let mock = mock_provider().await;
    let dir = tempdir("link");
    std::fs::write(
        dir.join("users.json"),
        json!({ "users": [{ "id": "u1", "email": "carol@other.org", "name": "Carol",
            "passwordHash": "$2a$10$x", "providers": [], "createdAt": "2026-01-01T00:00:00Z", "role": "admin" }] })
        .to_string(),
    )
    .unwrap();
    let app = build(dir.join("users.json"), vec![provider(&mock)]);

    let out = signin(
        &app,
        &mock,
        "carol@other.org",
        json!({ "email_verified": false }),
    )
    .await;
    assert_eq!(out["error"], "email_not_verified");
    let out = signin(
        &app,
        &mock,
        "carol@other.org",
        json!({ "email_verified": "true" }),
    )
    .await;
    let user = me(&app, &out["token"]).await;
    assert_eq!(user["id"], "u1", "attached, not duplicated");
    assert_eq!(
        user["role"], "admin",
        "a local administrator is not demoted by the groups"
    );
    // Entra sends `preferred_username` and no `email_verified`.
    let out = signin(
        &app,
        &mock,
        "carol@other.org",
        json!({ "email": null, "preferred_username": "Carol@Other.org" }),
    )
    .await;
    assert_eq!(me(&app, &out["token"]).await["id"], "u1");

    let out = signin(&app, &mock, "dave@other.org", json!({})).await;
    assert_eq!(
        out["error"], "domain_not_allowed",
        "no account for a domain off the list"
    );
    let out = signin(&app, &mock, "x", json!({ "email": "no-at-sign" })).await;
    assert_eq!(out["error"], "email_missing");
}

#[tokio::test]
async fn every_check_of_the_flow_refuses_with_its_code() {
    let mock = mock_provider().await;
    let dir = tempdir("refusals");
    let app = build(dir.join("users.json"), vec![provider(&mock)]);
    let bad_token = |extra: Value| {
        let (app, mock) = (app.clone(), &mock);
        async move { signin(&app, mock, "eve@example.com", extra).await["error"].clone() }
    };
    assert_eq!(
        bad_token(json!({ "nonce": "other" })).await,
        "id_token_invalid"
    );
    assert_eq!(
        bad_token(json!({ "aud": "someone-else" })).await,
        "id_token_invalid"
    );
    assert_eq!(
        bad_token(json!({ "iss": "https://evil.example" })).await,
        "id_token_invalid"
    );
    assert_eq!(
        bad_token(json!({ "kid": "unknown" })).await,
        "id_token_invalid"
    );
    assert_eq!(
        bad_token(json!({ "exp": now() - 600 })).await,
        "id_token_invalid"
    );

    let (q, cookie) = start(&app, &mock, "%2F").await;
    mock.will_sign("eve@example.com", &q["nonce"], json!({}));
    let cb = |uri: String, cookie: Option<String>| {
        let app = app.clone();
        async move { callback(&app, &uri, cookie.as_deref()).await["error"].clone() }
    };
    let good = format!(
        "/auth/oauth/mock/callback?code=good-code&state={}",
        q["state"]
    );
    assert_eq!(cb(good.clone(), None).await, "invalid_state", "no cookie");
    assert_eq!(
        cb(good.clone(), Some("oidc_state=garbage".into())).await,
        "invalid_state"
    );
    assert_eq!(
        cb(
            "/auth/oauth/mock/callback?code=good-code&state=other".to_string(),
            Some(cookie.clone())
        )
        .await,
        "invalid_state"
    );
    assert_eq!(
        cb(
            format!(
                "/auth/oauth/mock/callback?state={}&error=access_denied",
                q["state"]
            ),
            Some(cookie.clone())
        )
        .await,
        "provider_refused"
    );
    assert_eq!(
        cb(
            format!("/auth/oauth/mock/callback?state={}", q["state"]),
            Some(cookie.clone())
        )
        .await,
        "missing_code"
    );
    assert_eq!(
        cb(
            format!(
                "/auth/oauth/mock/callback?code=bad-code&state={}",
                q["state"]
            ),
            Some(cookie.clone())
        )
        .await,
        "token_exchange_failed"
    );
    // The cookie is bound to its provider and the flow still works after
    // all those refusals (the cookie was not consumed).
    let mut other = provider(&mock);
    other.id = "other".into();
    let app2 = app_with_two(dir.join("users.json"), provider(&mock), other);
    assert_eq!(
        callback(&app2, &good.replace("/mock/", "/other/"), Some(&cookie)).await["error"],
        "invalid_state"
    );
    assert_eq!(
        callback(&app2, &good, Some(&cookie)).await.get("error"),
        None
    );

    // Unknown or disabled providers.
    assert_eq!(
        callback(&app, "/auth/oauth/nope/callback?code=x", None).await["error"],
        "provider_not_configured"
    );
    let mut off = provider(&mock);
    off.enabled = false;
    let app3 = build(dir.join("users.json"), vec![off]);
    let resp = req_get(&app3, "/auth/providers", None, None).await;
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), json!([]));
    assert_eq!(
        query(&location(
            &req_get(&app3, "/auth/oauth/mock/start", None, None).await
        ))["error"],
        "provider_not_configured"
    );
    // A provider whose discovery is unreachable.
    let mut dead = provider(&mock);
    dead.issuer = "http://127.0.0.1:1".into();
    let app4 = build(dir.join("users.json"), vec![dead]);
    assert_eq!(
        query(&location(
            &req_get(&app4, "/auth/oauth/mock/start", None, None).await
        ))["error"],
        "discovery_failed"
    );
}

fn app_with_two(users_file: PathBuf, a: OidcProvider, b: OidcProvider) -> Router {
    build(users_file, vec![a, b])
}

#[tokio::test]
async fn the_signin_settings_are_patched_and_served_without_the_secret() {
    let mock = mock_provider().await;
    let dir = tempdir("settings");
    let app = build(dir.join("users.json"), vec![]);
    let admin = {
        // The first account signs up locally and administers.
        let req = Request::builder()
            .method("POST")
            .uri("/auth/signup")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({ "email": "root@example.com", "name": "Root", "password": "Passw0rd!x" })
                    .to_string(),
            ))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        serde_json::from_slice::<Value>(&bytes).unwrap()["token"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let patch = |body: Value| {
        let (app, admin) = (app.clone(), admin.clone());
        async move {
            let req = Request::builder()
                .method("PATCH")
                .uri("/settings")
                .header("authorization", format!("Bearer {admin}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap();
            let resp = app.oneshot(req).await.unwrap();
            let status = resp.status();
            let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
                .await
                .unwrap();
            (status, serde_json::from_slice::<Value>(&bytes).unwrap())
        }
    };
    let (status, view) = patch(
        json!({ "signin": { "public_url": "http://app.test/", "providers": [
        { "id": "mock", "label": "Mock IdP", "issuer": mock.base, "client_id": "cid",
          "client_secret": "mock-secret", "allowed_domains": ["example.com"] }
    ] } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["signin"]["public_url"], "http://app.test");
    assert_eq!(
        view["signin"]["providers"][0]["client_secret_hint"],
        "moc...cret"
    );
    assert!(
        view["signin"]["providers"][0]
            .get("client_secret")
            .is_none(),
        "never served"
    );

    let (status, err) = patch(json!({ "signin": { "providers": [{ "id": "Not Valid" }] } })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(err["error"].as_str().unwrap().contains("Not Valid"));

    // The patched provider works at once, with the secret it kept.
    let out = signin(&app, &mock, "zoe@example.com", json!({})).await;
    assert_eq!(out.get("error"), None, "{out:?}");
    assert_eq!(me(&app, &out["token"]).await["role"], "user");
}
