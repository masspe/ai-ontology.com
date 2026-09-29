// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Built-in user authentication (ROADMAP.md §3.8.3, decided 2026-09-29):
//! the `/auth/*` routes the web UI calls, served by the binary instead of
//! the Node `auth-server`. Same contract, same `users.json` file (the
//! hashes it wrote with bcryptjs verify here), same JWT (HS256, `iss`
//! `ai-ontology`, `aud` `web`), so a store and its users move over with no
//! migration.
//!
//! Sign-up policy: the first account becomes the administrator and sign-up
//! then closes; administrators create the next accounts (`POST
//! /auth/users`). `allow_signup` reopens it for deployments that want it.
//! OAuth (Google, Microsoft) is not served here yet: the start routes
//! bounce to the login page with `error=<provider>_not_configured`, as the
//! Node server does when it has no client id (ROADMAP §3.8.3b).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{AuthContext, JwtAuth};

/// Bcrypt cost: what bcryptjs used in the Node server (12).
const BCRYPT_COST: u32 = 12;
/// Token lifetime, as the Node server's `JWT_EXPIRES_IN` default (7 days).
const TOKEN_TTL_SECS: u64 = 7 * 24 * 3600;

/// Configuration of the built-in auth: where the users live and whether
/// anyone may sign up once an administrator exists.
#[derive(Debug, Clone)]
pub struct UserAuth {
    pub users_file: PathBuf,
    pub allow_signup: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Provider {
    provider: String,
    sub: String,
}

/// One record of `users.json`, field for field what the Node server wrote
/// (`role` is new; absent means `user`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct User {
    id: String,
    email: String,
    name: String,
    #[serde(default)]
    picture: Option<String>,
    #[serde(default)]
    password_hash: Option<String>,
    #[serde(default)]
    providers: Vec<Provider>,
    created_at: String,
    #[serde(default)]
    role: Option<String>,
}

impl User {
    fn is_admin(&self) -> bool {
        self.role.as_deref() == Some("admin")
    }

    /// What the API returns: never the hash.
    fn public(&self) -> PublicUser {
        PublicUser {
            id: self.id.clone(),
            email: self.email.clone(),
            name: self.name.clone(),
            picture: self.picture.clone(),
            providers: self.providers.clone(),
            created_at: self.created_at.clone(),
            role: self.role.clone().unwrap_or_else(|| "user".into()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicUser {
    pub id: String,
    pub email: String,
    pub name: String,
    pub picture: Option<String>,
    providers: Vec<Provider>,
    pub created_at: String,
    pub role: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Db {
    #[serde(default)]
    users: Vec<User>,
}

/// The users file, loaded once, written whole on every change (a few
/// hundred accounts at most per tenant: the Node server did the same).
pub struct UserStore {
    path: PathBuf,
    db: Mutex<Db>,
}

#[derive(Debug, thiserror::Error)]
pub enum UserStoreError {
    #[error("users file {0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("users file {0} is not valid JSON: {1}")]
    Parse(PathBuf, serde_json::Error),
}

impl UserStore {
    /// Open (or start empty when the file does not exist yet).
    pub fn open(path: impl AsRef<FsPath>) -> Result<Self, UserStoreError> {
        let path = path.as_ref().to_path_buf();
        let db = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| UserStoreError::Parse(path.clone(), e))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Db::default(),
            Err(e) => return Err(UserStoreError::Io(path, e)),
        };
        Ok(Self {
            path,
            db: Mutex::new(db),
        })
    }

    pub fn len(&self) -> usize {
        self.db.lock().users.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Write the whole file through a temporary neighbour and a rename, so
    /// a crash mid-write leaves the previous file intact.
    fn persist(&self, db: &Db) -> Result<(), UserStoreError> {
        let io = |e| UserStoreError::Io(self.path.clone(), e);
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(db).expect("users serialise");
        std::fs::write(&tmp, bytes).map_err(io)?;
        std::fs::rename(&tmp, &self.path).map_err(io)
    }

    fn find_by_email(&self, email: &str) -> Option<User> {
        let lc = email.trim().to_lowercase();
        self.db.lock().users.iter().find(|u| u.email == lc).cloned()
    }

    fn find_by_id(&self, id: &str) -> Option<User> {
        self.db.lock().users.iter().find(|u| u.id == id).cloned()
    }

    fn create(
        &self,
        email: &str,
        name: &str,
        password_hash: String,
        role: Option<&str>,
    ) -> Result<User, CreateError> {
        let lc = email.trim().to_lowercase();
        let mut db = self.db.lock();
        if db.users.iter().any(|u| u.email == lc) {
            return Err(CreateError::EmailTaken);
        }
        let name = name.trim();
        let user = User {
            id: new_id(),
            email: lc.clone(),
            name: if name.is_empty() {
                lc.split('@').next().unwrap_or("").to_string()
            } else {
                name.to_string()
            },
            picture: None,
            password_hash: Some(password_hash),
            providers: Vec::new(),
            created_at: now_rfc3339(),
            role: role.map(str::to_string),
        };
        db.users.push(user.clone());
        self.persist(&db).map_err(CreateError::Store)?;
        Ok(user)
    }

    fn delete(&self, id: &str) -> Result<bool, UserStoreError> {
        let mut db = self.db.lock();
        let before = db.users.len();
        db.users.retain(|u| u.id != id);
        if db.users.len() == before {
            return Ok(false);
        }
        self.persist(&db)?;
        Ok(true)
    }

    fn list(&self) -> Vec<PublicUser> {
        self.db.lock().users.iter().map(User::public).collect()
    }
}

#[derive(Debug)]
enum CreateError {
    EmailTaken,
    Store(UserStoreError),
}

/// Random-enough, unique id: the time in nanoseconds and a hashed counter,
/// hex. Not a UUID, and nothing here needs one.
fn new_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut h = ahash::RandomState::new().build_hasher();
    h.write_u128(nanos);
    h.write_u64(n);
    format!("{:016x}{:016x}", nanos as u64 ^ h.finish(), h.finish().rotate_left(17) ^ n)
}

fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Civil date from days since the epoch (Howard Hinnant's algorithm).
    let days = (secs / 86_400) as i64;
    let (h, m, s) = ((secs % 86_400) / 3600, (secs % 3600) / 60, secs % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}.000Z")
}

/// `min 8 chars, mixed classes`: at least three of lower, upper, digit,
/// other — the Node server's rule.
fn strong_enough(pw: &str) -> bool {
    if pw.chars().count() < 8 {
        return false;
    }
    let classes = [
        pw.chars().any(|c| c.is_ascii_uppercase()),
        pw.chars().any(|c| c.is_ascii_lowercase()),
        pw.chars().any(|c| c.is_ascii_digit()),
        pw.chars().any(|c| !c.is_ascii_alphanumeric()),
    ];
    classes.iter().filter(|&&b| b).count() >= 3
}

/// `^[^\s@]+@[^\s@]+\.[^\s@]+$`, the Node server's check.
fn valid_email(email: &str) -> bool {
    let mut parts = email.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let ok = |s: &str| !s.is_empty() && !s.chars().any(|c| c.is_whitespace() || c == '@');
    ok(local) && ok(domain) && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
}

// ---------------------------------------------------------------- tokens

#[derive(Debug, Serialize)]
struct Claims<'a> {
    sub: &'a str,
    email: &'a str,
    name: &'a str,
    iss: &'a str,
    aud: &'a str,
    iat: u64,
    exp: u64,
}

/// Signs the tokens `require_auth` verifies: same secret, issuer, audience.
pub struct Signer {
    key: jsonwebtoken::EncodingKey,
    issuer: String,
    audience: String,
}

impl Signer {
    pub fn new(jwt: &JwtAuth) -> Self {
        Self {
            key: jsonwebtoken::EncodingKey::from_secret(&jwt.secret),
            issuer: jwt.issuer.clone().unwrap_or_else(|| "ai-ontology".into()),
            audience: jwt.audience.clone().unwrap_or_else(|| "web".into()),
        }
    }

    fn sign(&self, user: &User) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let claims = Claims {
            sub: &user.id,
            email: &user.email,
            name: &user.name,
            iss: &self.issuer,
            aud: &self.audience,
            iat: now,
            exp: now + TOKEN_TTL_SECS,
        };
        jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
            &claims,
            &self.key,
        )
        .expect("HS256 signing cannot fail")
    }
}

// ---------------------------------------------------------------- routes

#[derive(Clone)]
pub struct AuthState {
    pub store: Arc<UserStore>,
    pub signer: Arc<Signer>,
    pub allow_signup: bool,
}

/// `/auth/signup`, `/auth/login`, `/auth/logout` and the OAuth bounces:
/// reachable without a token.
pub fn public_routes(state: AuthState) -> Router {
    Router::new()
        .route("/auth/signup", post(signup))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/oauth/:provider/start", get(oauth_not_configured))
        .route("/auth/oauth/:provider/callback", get(oauth_not_configured))
        .with_state(state)
}

/// `/auth/me` and the administrator's user management: mounted behind
/// `require_auth`, which attaches the `AuthContext` these handlers read.
pub fn protected_routes(state: AuthState) -> Router {
    Router::new()
        .route("/auth/me", get(me))
        .route("/auth/users", get(list_users).post(create_user))
        .route("/auth/users/:id", axum::routing::delete(delete_user))
        .with_state(state)
}

#[derive(Deserialize)]
struct Credentials {
    #[serde(default)]
    email: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    name: String,
}

#[derive(Serialize)]
struct Session {
    token: String,
    user: PublicUser,
}

fn error(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

fn hash_password(password: &str) -> Result<String, Response> {
    bcrypt::hash(password, BCRYPT_COST)
        .map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "Internal error"))
}

fn created(s: &AuthState, user: &User, status: StatusCode) -> Response {
    (
        status,
        Json(Session {
            token: s.signer.sign(user),
            user: user.public(),
        }),
    )
        .into_response()
}

async fn signup(State(s): State<AuthState>, Json(c): Json<Credentials>) -> Response {
    if !valid_email(&c.email) {
        return error(StatusCode::BAD_REQUEST, "Invalid email");
    }
    if !strong_enough(&c.password) {
        return error(
            StatusCode::BAD_REQUEST,
            "Weak password (min 8 chars, mixed classes)",
        );
    }
    if c.name.trim().is_empty() {
        return error(StatusCode::BAD_REQUEST, "Name required");
    }
    // The first account administers the tenant; after it, sign-up is an
    // administrator's decision unless the deployment opened it.
    let first = s.store.is_empty();
    if !first && !s.allow_signup {
        return error(
            StatusCode::FORBIDDEN,
            "Sign-up is closed: ask an administrator for an account",
        );
    }
    let hash = match hash_password(&c.password) {
        Ok(h) => h,
        Err(r) => return r,
    };
    match s
        .store
        .create(&c.email, &c.name, hash, first.then_some("admin"))
    {
        Ok(user) => {
            tracing::info!(email = %user.email, admin = first, "user signed up");
            created(&s, &user, StatusCode::CREATED)
        }
        Err(CreateError::EmailTaken) => error(StatusCode::CONFLICT, "Email already registered"),
        Err(CreateError::Store(e)) => {
            tracing::error!(%e, "users file");
            error(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
        }
    }
}

async fn login(State(s): State<AuthState>, Json(c): Json<Credentials>) -> Response {
    if c.email.trim().is_empty() || c.password.is_empty() {
        return error(StatusCode::BAD_REQUEST, "Invalid credentials");
    }
    let Some(user) = s.store.find_by_email(&c.email) else {
        // Same cost as a real check, so an unknown email is not faster.
        static DUMMY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        let dummy = DUMMY.get_or_init(|| bcrypt::hash("not-a-password", BCRYPT_COST).unwrap_or_default());
        let _ = bcrypt::verify(&c.password, dummy);
        return error(StatusCode::UNAUTHORIZED, "Invalid credentials");
    };
    let ok = user
        .password_hash
        .as_deref()
        .map(|h| bcrypt::verify(&c.password, h).unwrap_or(false))
        .unwrap_or(false);
    if !ok {
        return error(StatusCode::UNAUTHORIZED, "Invalid credentials");
    }
    created(&s, &user, StatusCode::OK)
}

async fn logout() -> Json<serde_json::Value> {
    // Tokens are stateless; the client drops its copy.
    Json(serde_json::json!({ "ok": true }))
}

async fn oauth_not_configured(Path(provider): Path<String>) -> Redirect {
    let provider: String = provider
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(32)
        .collect();
    Redirect::to(&format!("/login?error={provider}_not_configured"))
}

fn current(s: &AuthState, ctx: &AuthContext) -> Result<User, Response> {
    if ctx.service {
        return Err(error(StatusCode::UNAUTHORIZED, "User not found"));
    }
    s.store
        .find_by_id(&ctx.subject)
        .ok_or_else(|| error(StatusCode::UNAUTHORIZED, "User not found"))
}

fn admin(s: &AuthState, ctx: &AuthContext) -> Result<User, Response> {
    let user = current(s, ctx)?;
    if !user.is_admin() {
        return Err(error(StatusCode::FORBIDDEN, "Administrator only"));
    }
    Ok(user)
}

async fn me(State(s): State<AuthState>, Extension(ctx): Extension<AuthContext>) -> Response {
    match current(&s, &ctx) {
        Ok(user) => Json(serde_json::json!({ "user": user.public() })).into_response(),
        Err(r) => r,
    }
}

async fn list_users(State(s): State<AuthState>, Extension(ctx): Extension<AuthContext>) -> Response {
    match admin(&s, &ctx) {
        Ok(_) => Json(serde_json::json!({ "users": s.store.list() })).into_response(),
        Err(r) => r,
    }
}

async fn create_user(
    State(s): State<AuthState>,
    Extension(ctx): Extension<AuthContext>,
    Json(c): Json<Credentials>,
) -> Response {
    if let Err(r) = admin(&s, &ctx) {
        return r;
    }
    if !valid_email(&c.email) {
        return error(StatusCode::BAD_REQUEST, "Invalid email");
    }
    if !strong_enough(&c.password) {
        return error(
            StatusCode::BAD_REQUEST,
            "Weak password (min 8 chars, mixed classes)",
        );
    }
    let hash = match hash_password(&c.password) {
        Ok(h) => h,
        Err(r) => return r,
    };
    match s.store.create(&c.email, &c.name, hash, None) {
        Ok(user) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "user": user.public() })),
        )
            .into_response(),
        Err(CreateError::EmailTaken) => error(StatusCode::CONFLICT, "Email already registered"),
        Err(CreateError::Store(e)) => {
            tracing::error!(%e, "users file");
            error(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
        }
    }
}

async fn delete_user(
    State(s): State<AuthState>,
    Extension(ctx): Extension<AuthContext>,
    Path(id): Path<String>,
) -> Response {
    let me = match admin(&s, &ctx) {
        Ok(u) => u,
        Err(r) => return r,
    };
    if me.id == id {
        return error(StatusCode::BAD_REQUEST, "An administrator cannot delete their own account");
    }
    match s.store.delete(&id) {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error(StatusCode::NOT_FOUND, "User not found"),
        Err(e) => {
            tracing::error!(%e, "users file");
            error(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_rule_matches_the_node_server() {
        assert!(strong_enough("Passw0rd!"));
        assert!(strong_enough("Abcdefg1"));
        assert!(!strong_enough("abcdefgh"), "one class");
        assert!(!strong_enough("Abcdefgh"), "two classes");
        assert!(!strong_enough("Ab1!"), "too short");
    }

    #[test]
    fn email_rule_matches_the_node_server() {
        assert!(valid_email("a@b.co"));
        assert!(!valid_email("a@b"));
        assert!(!valid_email("a b@c.d"));
        assert!(!valid_email("@c.d"));
        assert!(!valid_email("a@@c.d"));
        assert!(!valid_email("a@.d"));
    }

    #[test]
    fn dates_are_rfc3339_utc() {
        let s = now_rfc3339();
        assert_eq!(s.len(), "2026-09-29T10:00:00.000Z".len(), "{s}");
        assert!(s.starts_with("20"), "{s}");
        assert!(s.ends_with(".000Z"), "{s}");
    }

    #[test]
    fn ids_are_unique_hex() {
        let a = new_id();
        let b = new_id();
        assert_ne!(a, b);
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
