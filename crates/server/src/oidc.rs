// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Sign-in through an OpenID Connect provider (Microsoft Entra ID, Google,
//! any provider with a discovery document): authorization-code flow with
//! PKCE, `state` and `nonce` bound to a signed cookie, the `id_token`
//! verified RS256 against the provider's JWKS, then the account is found
//! by provider subject, attached by verified e-mail, or created when the
//! e-mail's domain is on the provider's allow-list. Group claims map to
//! the administrator role when the provider names admin groups.
//!
//! Routes (public, rate-limited with the rest of `/auth/*`):
//! `GET /auth/providers`, `GET /auth/oauth/:id/start?next=`,
//! `GET /auth/oauth/:id/callback?code&state` — the last one bounces to
//! the web UI's `/oauth/callback#token=…&next=…`, or `?error=<code>`.

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use base64::Engine;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::auth::{open_session, AuthState, User};
use crate::key_hint;

/// The sign-in section of the settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SigninSettings {
    /// Where browsers reach this server (`https://ontology.example.com`):
    /// the provider sends the user back to `<public_url>/auth/oauth/<id>/callback`,
    /// which must be registered at the provider. Empty: taken from the
    /// request's `Host` (development only).
    pub public_url: String,
    pub providers: Vec<OidcProvider>,
}

/// One OpenID Connect provider. `client_secret` is sealed at rest and
/// never served (`client_secret_hint` is); `issuer` is the discovery
/// base, e.g. `https://login.microsoftonline.com/<tenant-id>/v2.0` or
/// `https://accounts.google.com`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct OidcProvider {
    /// URL slug (`microsoft`, `google`, `okta`): `[a-z0-9-]{1,32}`.
    pub id: String,
    pub label: String,
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    pub client_secret_hint: String,
    /// Scopes requested; `openid` is always sent.
    pub scopes: String,
    /// E-mail domains whose users get an account on first sign-in. Empty:
    /// no account is created — only existing accounts may sign in.
    pub allowed_domains: Vec<String>,
    /// Group or role claims whose members are administrators. Empty: the
    /// provider does not decide roles.
    pub admin_groups: Vec<String>,
    pub enabled: bool,
}

impl Default for OidcProvider {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            issuer: String::new(),
            client_id: String::new(),
            client_secret: String::new(),
            client_secret_hint: String::new(),
            scopes: "openid email profile".into(),
            allowed_domains: Vec::new(),
            admin_groups: Vec::new(),
            enabled: true,
        }
    }
}

impl OidcProvider {
    fn ready(&self) -> bool {
        self.enabled && !self.issuer.trim().is_empty() && !self.client_id.trim().is_empty()
    }
}

/// `PATCH /settings { "signin": … }`: the provider list is replaced as a
/// whole; a provider sent without a secret keeps the stored one of the
/// same id.
#[derive(Debug, Default, Deserialize)]
pub struct SigninPatch {
    pub public_url: Option<String>,
    pub providers: Option<Vec<OidcProviderPatch>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct OidcProviderPatch {
    pub id: String,
    pub label: Option<String>,
    pub issuer: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub scopes: Option<String>,
    pub allowed_domains: Option<Vec<String>>,
    pub admin_groups: Option<Vec<String>>,
    pub enabled: Option<bool>,
}

pub fn valid_provider_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl SigninSettings {
    /// Apply a patch; an invalid provider id is the error text.
    pub fn apply(&mut self, patch: SigninPatch) -> Result<(), String> {
        if let Some(url) = patch.public_url {
            self.public_url = url.trim().trim_end_matches('/').to_string();
        }
        if let Some(list) = patch.providers {
            let mut next = Vec::with_capacity(list.len());
            for p in list {
                if !valid_provider_id(&p.id) {
                    return Err(format!("invalid provider id `{}`", p.id));
                }
                if next.iter().any(|x: &OidcProvider| x.id == p.id) {
                    return Err(format!("duplicate provider id `{}`", p.id));
                }
                let mut cur = self
                    .providers
                    .iter()
                    .find(|x| x.id == p.id)
                    .cloned()
                    .unwrap_or_else(|| OidcProvider {
                        id: p.id.clone(),
                        ..Default::default()
                    });
                if let Some(v) = p.label {
                    cur.label = v.trim().to_string();
                }
                if let Some(v) = p.issuer {
                    cur.issuer = v.trim().trim_end_matches('/').to_string();
                }
                if let Some(v) = p.client_id {
                    cur.client_id = v.trim().to_string();
                }
                if let Some(v) = p.client_secret.filter(|s| !s.trim().is_empty()) {
                    cur.client_secret = v.trim().to_string();
                }
                if let Some(v) = p.scopes {
                    cur.scopes = v.trim().to_string();
                }
                if let Some(v) = p.allowed_domains {
                    cur.allowed_domains = v
                        .into_iter()
                        .map(|d| d.trim().trim_start_matches('@').to_lowercase())
                        .filter(|d| !d.is_empty())
                        .collect();
                }
                if let Some(v) = p.admin_groups {
                    cur.admin_groups = v
                        .into_iter()
                        .map(|g| g.trim().to_string())
                        .filter(|g| !g.is_empty())
                        .collect();
                }
                if let Some(v) = p.enabled {
                    cur.enabled = v;
                }
                next.push(cur);
            }
            self.providers = next;
        }
        self.refresh_hints();
        Ok(())
    }

    pub fn refresh_hints(&mut self) {
        for p in &mut self.providers {
            p.client_secret_hint = key_hint(&p.client_secret);
        }
    }
}

// ------------------------------------------------------------------ flow

const STATE_COOKIE: &str = "oidc_state";
const STATE_TTL_SECS: u64 = 10 * 60;
const CACHE_TTL: Duration = Duration::from_secs(3600);
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// What the provider published: the three endpoints the flow needs.
#[derive(Debug, Clone, Deserialize)]
struct Discovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
}

#[derive(Debug, Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

#[derive(Debug, Deserialize)]
struct Jwk {
    #[serde(default)]
    kid: String,
    #[serde(default)]
    kty: String,
    #[serde(default)]
    n: String,
    #[serde(default)]
    e: String,
}

/// What the state cookie carries, signed HS256 with the session secret.
#[derive(Debug, Serialize, Deserialize)]
struct StateClaims {
    /// The provider id.
    pid: String,
    /// The `state` sent to the provider.
    st: String,
    /// The `nonce` sent to the provider.
    nonce: String,
    /// The PKCE verifier.
    pkce: String,
    next: String,
    exp: u64,
}

/// The claims of an `id_token` this flow reads.
#[derive(Debug, Deserialize)]
struct IdClaims {
    #[serde(default)]
    iss: String,
    sub: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    preferred_username: Option<String>,
    #[serde(default)]
    upn: Option<String>,
    #[serde(default)]
    email_verified: Option<serde_json::Value>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    picture: Option<String>,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    tid: Option<String>,
    #[serde(default)]
    groups: Vec<String>,
    #[serde(default)]
    roles: Vec<String>,
}

/// The verified keys of one JWKS: `(kid, key)`.
type KeySet = Vec<(String, jsonwebtoken::DecodingKey)>;

/// The HTTP client, the caches and the state-cookie keys: one per server.
pub struct Oidc {
    http: reqwest::Client,
    state_enc: jsonwebtoken::EncodingKey,
    state_dec: jsonwebtoken::DecodingKey,
    discovery: Mutex<HashMap<String, (Instant, Discovery)>>,
    jwks: Mutex<HashMap<String, (Instant, KeySet)>>,
}

impl std::fmt::Debug for Oidc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Oidc(..)")
    }
}

impl Oidc {
    /// `secret` signs the state cookie: the session secret does.
    pub fn new(secret: &[u8]) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(HTTP_TIMEOUT)
                .build()
                .expect("reqwest client"),
            state_enc: jsonwebtoken::EncodingKey::from_secret(secret),
            state_dec: jsonwebtoken::DecodingKey::from_secret(secret),
            discovery: Mutex::new(HashMap::new()),
            jwks: Mutex::new(HashMap::new()),
        }
    }

    async fn discover(&self, issuer: &str) -> Result<Discovery, Flow> {
        if let Some((at, d)) = self.discovery.lock().get(issuer) {
            if at.elapsed() < CACHE_TTL {
                return Ok(d.clone());
            }
        }
        let url = format!(
            "{}/.well-known/openid-configuration",
            issuer.trim_end_matches('/')
        );
        let d: Discovery = self
            .http
            .get(&url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| Flow::fail("discovery_failed", e))?
            .json()
            .await
            .map_err(|e| Flow::fail("discovery_failed", e))?;
        self.discovery
            .lock()
            .insert(issuer.to_string(), (Instant::now(), d.clone()));
        Ok(d)
    }

    /// The RSA key `kid` of `jwks_uri`; the set is fetched again once when
    /// the kid is unknown (key rotation).
    async fn key(&self, jwks_uri: &str, kid: &str) -> Result<jsonwebtoken::DecodingKey, Flow> {
        let cached = self.jwks.lock().get(jwks_uri).and_then(|(at, keys)| {
            (at.elapsed() < CACHE_TTL)
                .then(|| keys.iter().find(|(k, _)| k == kid).map(|(_, d)| d.clone()))
        });
        if let Some(Some(k)) = cached {
            return Ok(k);
        }
        let set: Jwks = self
            .http
            .get(jwks_uri)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| Flow::fail("jwks_failed", e))?
            .json()
            .await
            .map_err(|e| Flow::fail("jwks_failed", e))?;
        let keys: KeySet = set
            .keys
            .into_iter()
            .filter(|k| k.kty == "RSA")
            .filter_map(|k| {
                jsonwebtoken::DecodingKey::from_rsa_components(&k.n, &k.e)
                    .ok()
                    .map(|d| (k.kid, d))
            })
            .collect();
        let found = keys.iter().find(|(k, _)| k == kid).map(|(_, d)| d.clone());
        self.jwks
            .lock()
            .insert(jwks_uri.to_string(), (Instant::now(), keys));
        found.ok_or_else(|| Flow::new("id_token_invalid", "unknown signing key"))
    }

    fn sign_state(&self, claims: &StateClaims) -> String {
        jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
            claims,
            &self.state_enc,
        )
        .expect("HS256 signing cannot fail")
    }

    fn verify_state(&self, token: &str) -> Option<StateClaims> {
        let mut v = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
        v.leeway = 0;
        v.validate_aud = false;
        jsonwebtoken::decode::<StateClaims>(token, &self.state_dec, &v)
            .ok()
            .map(|d| d.claims)
    }
}

/// A failed step: the code the web UI shows, the detail only the log gets.
#[derive(Debug)]
struct Flow {
    code: &'static str,
    detail: String,
}

impl Flow {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
    fn fail(code: &'static str, e: impl std::fmt::Display) -> Self {
        Self::new(code, e.to_string())
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn random_token() -> String {
    use rand::RngCore;
    let mut raw = [0u8; 32];
    rand::rng().fill_bytes(&mut raw);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)
}

fn s256(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Only a same-origin path: never an absolute or protocol-relative URL.
pub fn safe_next(raw: Option<&str>) -> String {
    match raw {
        Some(p) if p.starts_with('/') && !p.starts_with("//") && !p.contains('\\') => p.to_string(),
        _ => "/".to_string(),
    }
}

/// Where this server is reached: the configured public URL, else the
/// request's `Host` with the forwarded scheme (development).
fn public_base(settings: &SigninSettings, headers: &header::HeaderMap) -> String {
    if !settings.public_url.is_empty() {
        return settings.public_url.clone();
    }
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
        .filter(|s| *s == "https")
        .unwrap_or("http");
    format!("{scheme}://{host}")
}

fn cookie_value(headers: &header::HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .flat_map(|h| h.split(';'))
        .map(str::trim)
        .find_map(|kv| kv.strip_prefix(name)?.strip_prefix('=').map(str::to_string))
}

fn state_cookie(value: &str, max_age: u64, secure: bool) -> HeaderValue {
    let secure = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{STATE_COOKIE}={value}; Path=/auth/oauth; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}"
    ))
    .expect("ascii cookie")
}

/// The UI's callback page with the token in the fragment (never in a
/// `Referer` or an access log) or the error in the query; the refresh
/// cookie of the new session comes with the token.
fn bounce(base: &str, result: Result<Signed, Flow>) -> Response {
    let (target, cookie) = match result {
        Ok(signed) => (
            format!(
                "{base}/oauth/callback#token={}&next={}",
                url_encode(&signed.token),
                url_encode(&signed.next)
            ),
            Some(signed.cookie),
        ),
        Err(flow) => {
            tracing::warn!(code = flow.code, detail = %flow.detail, "sign-in refused");
            (format!("{base}/oauth/callback?error={}", flow.code), None)
        }
    };
    let mut resp = Redirect::to(&target).into_response();
    if let Some(c) = cookie {
        resp.headers_mut().append(header::SET_COOKIE, c);
    }
    // The state cookie has served; drop it.
    resp.headers_mut().append(
        header::SET_COOKIE,
        state_cookie("", 0, base.starts_with("https://")),
    );
    resp
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// A finished sign-in: the access token, the refresh cookie, where to go.
struct Signed {
    token: String,
    cookie: HeaderValue,
    next: String,
}

#[derive(Serialize)]
pub struct PublicProvider {
    pub id: String,
    pub label: String,
}

/// `GET /auth/providers`: the sign-in buttons to show.
pub async fn providers(State(s): State<AuthState>) -> Json<Vec<PublicProvider>> {
    let list = s
        .settings
        .read()
        .signin
        .providers
        .iter()
        .filter(|p| p.ready())
        .map(|p| PublicProvider {
            id: p.id.clone(),
            label: if p.label.is_empty() {
                p.id.clone()
            } else {
                p.label.clone()
            },
        })
        .collect();
    Json(list)
}

#[derive(Deserialize)]
pub struct StartQuery {
    #[serde(default)]
    next: Option<String>,
}

fn provider(s: &AuthState, id: &str) -> Option<OidcProvider> {
    s.settings
        .read()
        .signin
        .providers
        .iter()
        .find(|p| p.id == id && p.ready())
        .cloned()
}

/// `GET /auth/oauth/:id/start?next=`: to the provider's authorization
/// endpoint, with the state cookie set.
pub async fn start(
    State(s): State<AuthState>,
    Path(id): Path<String>,
    Query(q): Query<StartQuery>,
    headers: header::HeaderMap,
) -> Response {
    let base = public_base(&s.settings.read().signin, &headers);
    let Some(p) = provider(&s, &id) else {
        return bounce(&base, Err(Flow::new("provider_not_configured", id)));
    };
    let d = match s.oidc.discover(&p.issuer).await {
        Ok(d) => d,
        Err(e) => return bounce(&base, Err(e)),
    };
    let claims = StateClaims {
        pid: p.id.clone(),
        st: random_token(),
        nonce: random_token(),
        pkce: random_token(),
        next: safe_next(q.next.as_deref()),
        exp: now() + STATE_TTL_SECS,
    };
    let scopes = if p.scopes.split_whitespace().any(|x| x == "openid") {
        p.scopes.clone()
    } else {
        format!("openid {}", p.scopes)
    };
    let redirect_uri = format!("{base}/auth/oauth/{}/callback", p.id);
    let url = format!(
        "{}{}client_id={}&redirect_uri={}&response_type=code&scope={}&state={}&nonce={}&code_challenge={}&code_challenge_method=S256&prompt=select_account",
        d.authorization_endpoint,
        if d.authorization_endpoint.contains('?') { "&" } else { "?" },
        url_encode(&p.client_id),
        url_encode(&redirect_uri),
        url_encode(scopes.trim()),
        url_encode(&claims.st),
        url_encode(&claims.nonce),
        s256(&claims.pkce),
    );
    let cookie = state_cookie(
        &s.oidc.sign_state(&claims),
        STATE_TTL_SECS,
        base.starts_with("https://"),
    );
    let mut resp = Redirect::to(&url).into_response();
    resp.headers_mut().append(header::SET_COOKIE, cookie);
    resp
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

/// `GET /auth/oauth/:id/callback?code&state`.
pub async fn callback(
    State(s): State<AuthState>,
    Path(id): Path<String>,
    Query(q): Query<CallbackQuery>,
    headers: header::HeaderMap,
) -> Response {
    let base = public_base(&s.settings.read().signin, &headers);
    let result = finish(&s, &base, &id, q, &headers).await;
    bounce(&base, result)
}

async fn finish(
    s: &AuthState,
    base: &str,
    id: &str,
    q: CallbackQuery,
    headers: &header::HeaderMap,
) -> Result<Signed, Flow> {
    let p = provider(s, id).ok_or_else(|| Flow::new("provider_not_configured", id))?;
    let state = cookie_value(headers, STATE_COOKIE)
        .and_then(|c| s.oidc.verify_state(&c))
        .filter(|c| c.pid == p.id && c.exp > now())
        .ok_or_else(|| Flow::new("invalid_state", "no or expired state cookie"))?;
    if q.state.as_deref() != Some(state.st.as_str()) {
        return Err(Flow::new("invalid_state", "state mismatch"));
    }
    if let Some(e) = q.error {
        return Err(Flow::new("provider_refused", e));
    }
    let code = q
        .code
        .filter(|c| !c.is_empty())
        .ok_or_else(|| Flow::new("missing_code", "no code in the callback"))?;
    let d = s.oidc.discover(&p.issuer).await?;

    // 1. The code for the tokens.
    let redirect_uri = format!("{base}/auth/oauth/{}/callback", p.id);
    let form = [
        ("client_id", p.client_id.as_str()),
        ("client_secret", p.client_secret.as_str()),
        ("code", code.as_str()),
        ("grant_type", "authorization_code"),
        ("redirect_uri", redirect_uri.as_str()),
        ("code_verifier", state.pkce.as_str()),
    ];
    let tokens: serde_json::Value = s
        .oidc
        .http
        .post(&d.token_endpoint)
        .header(header::ACCEPT, "application/json")
        .form(&form)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| Flow::fail("token_exchange_failed", e))?
        .json()
        .await
        .map_err(|e| Flow::fail("token_exchange_failed", e))?;
    let id_token = tokens["id_token"]
        .as_str()
        .ok_or_else(|| Flow::new("token_exchange_failed", "no id_token"))?;

    // 2. The id_token: signed by the provider, for us, fresh, our nonce.
    let head =
        jsonwebtoken::decode_header(id_token).map_err(|e| Flow::fail("id_token_invalid", e))?;
    let key = s
        .oidc
        .key(&d.jwks_uri, head.kid.as_deref().unwrap_or(""))
        .await?;
    let mut v = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    v.set_audience(&[p.client_id.as_str()]);
    v.leeway = 60;
    // Validated below: a multi-tenant Microsoft issuer carries `{tenantid}`.
    v.validate_aud = true;
    v.iss = None;
    let claims = jsonwebtoken::decode::<IdClaims>(id_token, &key, &v)
        .map_err(|e| Flow::fail("id_token_invalid", e))?
        .claims;
    let expected_iss = match (&claims.tid, d.issuer.contains("{tenantid}")) {
        (Some(tid), true) => d.issuer.replace("{tenantid}", tid),
        _ => d.issuer.clone(),
    };
    if claims.iss != expected_iss {
        return Err(Flow::new("id_token_invalid", "issuer mismatch"));
    }
    if claims.nonce.as_deref() != Some(state.nonce.as_str()) {
        return Err(Flow::new("id_token_invalid", "nonce mismatch"));
    }

    // 3. The account.
    let email = claims
        .email
        .clone()
        .or(claims.preferred_username.clone())
        .or(claims.upn.clone())
        .map(|e| e.trim().to_lowercase())
        .filter(|e| e.contains('@'))
        .ok_or_else(|| Flow::new("email_missing", "no e-mail claim"))?;
    let verified = match &claims.email_verified {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(t)) => t == "true",
        // Providers that manage the accounts themselves (Entra) send none.
        None => true,
        _ => false,
    };
    let domain = email.rsplit('@').next().unwrap_or("");
    let mut groups = claims.groups.clone();
    groups.extend(claims.roles.clone());
    let admin =
        (!p.admin_groups.is_empty()).then(|| groups.iter().any(|g| p.admin_groups.contains(g)));

    let user = match s.store.find_by_provider(&p.id, &claims.sub) {
        Some(u) => u,
        None => match s.store.find_by_email(&email) {
            Some(existing) => {
                if !verified {
                    return Err(Flow::new("email_not_verified", email));
                }
                s.store
                    .link_provider(&existing.id, &p.id, &claims.sub)
                    .map_err(|e| Flow::fail("store_failed", e))?
            }
            None => {
                if !p.allowed_domains.iter().any(|d| d == domain) {
                    return Err(Flow::new("domain_not_allowed", email));
                }
                s.store
                    .create_external(
                        &email,
                        claims.name.as_deref().unwrap_or(""),
                        claims.picture.clone(),
                        &p.id,
                        &claims.sub,
                    )
                    .map_err(|e| Flow::fail("store_failed", e))?
            }
        },
    };
    // Groups grant the role to anyone and take it away only from accounts
    // the provider owns (no password): a local administrator is the
    // break-glass account and keeps what an administrator gave.
    let user: User = match admin {
        Some(true) if !user.is_admin() => s
            .store
            .set_admin(&user.id, true)
            .map_err(|e| Flow::fail("store_failed", e))?,
        Some(false) if user.is_admin() && !user.is_local() => s
            .store
            .set_admin(&user.id, false)
            .map_err(|e| Flow::fail("store_failed", e))?,
        _ => user,
    };
    tracing::info!(provider = %p.id, email = %user.email, admin = user.is_admin(), "user signed in");
    let (token, cookie) = open_session(s, &user).map_err(|e| Flow::fail("store_failed", e))?;
    Ok(Signed {
        token,
        cookie,
        next: state.next,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_is_a_same_origin_path_or_root() {
        assert_eq!(safe_next(Some("/files?tab=2")), "/files?tab=2");
        assert_eq!(safe_next(Some("//evil.example")), "/");
        assert_eq!(safe_next(Some("https://evil.example")), "/");
        assert_eq!(safe_next(Some("/a\\b")), "/");
        assert_eq!(safe_next(None), "/");
    }

    #[test]
    fn cookies_are_read_by_name_and_pkce_is_s256() {
        let mut h = header::HeaderMap::new();
        h.append(
            header::COOKIE,
            HeaderValue::from_static("a=1; oidc_state=xyz; b=2"),
        );
        assert_eq!(cookie_value(&h, STATE_COOKIE).as_deref(), Some("xyz"));
        assert_eq!(cookie_value(&h, "nope"), None);
        // RFC 7636 appendix B.
        assert_eq!(
            s256("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert_eq!(url_encode("a b/c~"), "a%20b%2Fc~");
    }

    #[test]
    fn the_state_cookie_round_trips_and_expires() {
        let o = Oidc::new(b"secret");
        let c = StateClaims {
            pid: "ms".into(),
            st: "s".into(),
            nonce: "n".into(),
            pkce: "p".into(),
            next: "/x".into(),
            exp: now() + 60,
        };
        let token = o.sign_state(&c);
        let back = o.verify_state(&token).unwrap();
        assert_eq!(
            (back.pid, back.st, back.nonce, back.pkce, back.next),
            ("ms".into(), "s".into(), "n".into(), "p".into(), "/x".into())
        );
        assert!(Oidc::new(b"other").verify_state(&token).is_none());
        let expired = o.sign_state(&StateClaims {
            exp: now() - 1,
            ..c
        });
        assert!(o.verify_state(&expired).is_none());
    }

    #[test]
    fn a_patch_replaces_the_list_and_keeps_secrets_it_does_not_send() {
        let mut s = SigninSettings::default();
        s.apply(SigninPatch {
            public_url: Some("https://o.example/ ".into()),
            providers: Some(vec![OidcProviderPatch {
                id: "microsoft".into(),
                issuer: Some("https://login.microsoftonline.com/t/v2.0/".into()),
                client_id: Some("cid".into()),
                client_secret: Some("sec-123456".into()),
                allowed_domains: Some(vec![" @Example.com ".into(), "".into()]),
                ..Default::default()
            }]),
        })
        .unwrap();
        assert_eq!(s.public_url, "https://o.example");
        let p = &s.providers[0];
        assert_eq!(p.issuer, "https://login.microsoftonline.com/t/v2.0");
        assert_eq!(p.allowed_domains, vec!["example.com"]);
        assert_eq!(p.client_secret_hint, "sec...3456");
        assert!(p.ready());

        s.apply(SigninPatch {
            public_url: None,
            providers: Some(vec![OidcProviderPatch {
                id: "microsoft".into(),
                client_secret: Some("  ".into()),
                enabled: Some(false),
                ..Default::default()
            }]),
        })
        .unwrap();
        assert_eq!(s.providers[0].client_secret, "sec-123456", "kept");
        assert!(!s.providers[0].ready());

        assert!(s
            .apply(SigninPatch {
                public_url: None,
                providers: Some(vec![OidcProviderPatch {
                    id: "Bad Id".into(),
                    ..Default::default()
                }]),
            })
            .is_err());
        assert!(s
            .apply(SigninPatch {
                public_url: None,
                providers: Some(vec![
                    OidcProviderPatch {
                        id: "a".into(),
                        ..Default::default()
                    },
                    OidcProviderPatch {
                        id: "a".into(),
                        ..Default::default()
                    }
                ]),
            })
            .is_err());
        s.apply(SigninPatch {
            public_url: None,
            providers: Some(vec![]),
        })
        .unwrap();
        assert!(s.providers.is_empty(), "the list is replaced as a whole");
    }

    #[test]
    fn the_public_base_is_configured_or_taken_from_the_request() {
        let mut h = header::HeaderMap::new();
        h.insert(header::HOST, HeaderValue::from_static("o.example:5000"));
        let plain = SigninSettings::default();
        assert_eq!(public_base(&plain, &h), "http://o.example:5000");
        h.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        assert_eq!(public_base(&plain, &h), "https://o.example:5000");
        let fixed = SigninSettings {
            public_url: "https://ontology.example".into(),
            ..Default::default()
        };
        assert_eq!(public_base(&fixed, &h), "https://ontology.example");
    }
}
