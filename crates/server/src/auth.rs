// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Built-in user authentication:
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
//! Node server does when it has no client id.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{oidc, AuthContext, JwtAuth, Settings};
use parking_lot::RwLock as PlRwLock;

/// Bcrypt cost: what bcryptjs used in the Node server (12).
const BCRYPT_COST: u32 = 12;
/// Token lifetime, as the Node server's `JWT_EXPIRES_IN` default (7 days).
/// An access token is short: revocation (logout, a deleted account, a
/// session ended) takes effect within this.
const TOKEN_TTL_SECS: u64 = 15 * 60;
/// A session lives this long after its last refresh.
const SESSION_TTL_SECS: u64 = 30 * 24 * 3600;
/// The refresh cookie: only sent to `/auth/*`.
pub(crate) const REFRESH_COOKIE: &str = "ontology_refresh";

/// Configuration of the built-in auth: the users store (opened by the
/// caller, so a corrupt file is an error before anything listens) and
/// whether anyone may sign up once an administrator exists.
#[derive(Clone)]
pub struct UserAuth {
    pub store: Arc<UserStore>,
    pub allow_signup: bool,
}

impl std::fmt::Debug for UserAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserAuth")
            .field("users_file", &self.store.path)
            .field("allow_signup", &self.allow_signup)
            .finish()
    }
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
pub(crate) struct User {
    pub(crate) id: String,
    pub(crate) email: String,
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
    /// Security clearances: the sheet labels this user may see.
    #[serde(default)]
    clearances: Vec<String>,
}

impl User {
    pub(crate) fn is_admin(&self) -> bool {
        self.role.as_deref() == Some("admin")
    }

    /// An account with a password: created here, not by a provider.
    pub(crate) fn is_local(&self) -> bool {
        self.password_hash.is_some()
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
            clearances: self.clearances.clone(),
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
    pub clearances: Vec<String>,
}

/// A named API key of the tenant for machine callers:
/// the secret (`ok_` + 64 hex) is shown once at creation and kept hashed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiKey {
    id: String,
    name: String,
    /// SHA-256 of the secret, hex.
    hash: String,
    /// The first characters of the secret, to tell keys apart.
    prefix: String,
    created_at: String,
    created_by: String,
}

impl ApiKey {
    fn public(&self) -> PublicApiKey {
        PublicApiKey {
            id: self.id.clone(),
            name: self.name.clone(),
            prefix: self.prefix.clone(),
            created_at: self.created_at.clone(),
            created_by: self.created_by.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicApiKey {
    pub id: String,
    pub name: String,
    pub prefix: String,
    pub created_at: String,
    pub created_by: String,
}

pub const API_KEY_PREFIX: &str = "ok_";

fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(s.as_bytes());
    d.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct Db {
    #[serde(default)]
    users: Vec<User>,
    #[serde(default)]
    keys: Vec<ApiKey>,
    #[serde(default)]
    sessions: Vec<SessionRec>,
}

/// One signed-in browser: the access tokens carry its id (`sid`), the
/// refresh cookie its rotating secret (kept hashed). Ended at logout, at
/// "sign out everywhere", when the account is deleted, or after
/// `SESSION_TTL_SECS` without a refresh.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionRec {
    id: String,
    user_id: String,
    /// SHA-256 of the current refresh secret, hex.
    refresh_hash: String,
    created_at: String,
    /// Unix seconds; past it the session is gone.
    expires_at: u64,
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The users file, loaded once, written whole on every change (a few
/// hundred accounts at most per tenant: the Node server did the same).
/// Every change is written to disk first and adopted in memory only when
/// the write succeeded, so memory never says yes to what disk lost.
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

/// What a `create` can refuse.
#[derive(Debug)]
enum CreateError {
    EmailTaken,
    /// Sign-up: the store already has accounts and sign-up is closed.
    Closed,
    Store(UserStoreError),
}

impl UserStore {
    /// Open (or start empty when the file does not exist yet).
    pub fn open(path: impl AsRef<FsPath>) -> Result<Self, UserStoreError> {
        let path = path.as_ref().to_path_buf();
        let db = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| UserStoreError::Parse(path.clone(), e))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Db::default(),
            Err(e) => return Err(UserStoreError::Io(path, e)),
        };
        Ok(Self {
            path,
            db: Mutex::new(db),
        })
    }

    pub fn path(&self) -> &FsPath {
        &self.path
    }

    pub fn len(&self) -> usize {
        self.db.lock().users.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `true` when `id` names an account: what `require_auth` asks so a
    /// deleted user's token stops working at once, not at its expiry.
    pub fn exists(&self, id: &str) -> bool {
        self.db.lock().users.iter().any(|u| u.id == id)
    }

    /// `true` while the session an access token names is open: what
    /// `require_auth` asks so a logout takes effect at once.
    pub fn session_alive(&self, sid: &str) -> bool {
        let now = unix_now();
        self.db
            .lock()
            .sessions
            .iter()
            .any(|s| s.id == sid && s.expires_at > now)
    }

    /// Open a session for `user_id`: its id and the refresh secret, shown
    /// once (the file keeps a hash). Expired sessions are swept here.
    fn create_session(&self, user_id: &str) -> Result<(String, String), UserStoreError> {
        use rand::RngCore;
        let mut raw = [0u8; 32];
        rand::rng().fill_bytes(&mut raw);
        let secret: String = raw.iter().map(|b| format!("{b:02x}")).collect();
        let rec = SessionRec {
            id: new_id(),
            user_id: user_id.to_string(),
            refresh_hash: sha256_hex(&secret),
            created_at: now_rfc3339(),
            expires_at: unix_now() + SESSION_TTL_SECS,
        };
        let mut db = self.db.lock();
        let mut next = db.clone();
        let now = unix_now();
        next.sessions.retain(|s| s.expires_at > now);
        next.sessions.push(rec.clone());
        self.persist(&next)?;
        *db = next;
        Ok((rec.id, secret))
    }

    /// Exchange a refresh secret for a new one (the old one is dead at
    /// once); `None` when it names no open session.
    fn rotate_session(&self, secret: &str) -> Result<Option<(String, User)>, UserStoreError> {
        use rand::RngCore;
        let h = sha256_hex(secret);
        let now = unix_now();
        let mut db = self.db.lock();
        let Some(pos) = db
            .sessions
            .iter()
            .position(|s| s.refresh_hash == h && s.expires_at > now)
        else {
            return Ok(None);
        };
        let Some(user) = db
            .users
            .iter()
            .find(|u| u.id == db.sessions[pos].user_id)
            .cloned()
        else {
            return Ok(None);
        };
        let mut raw = [0u8; 32];
        rand::rng().fill_bytes(&mut raw);
        let fresh: String = raw.iter().map(|b| format!("{b:02x}")).collect();
        let mut next = db.clone();
        next.sessions[pos].refresh_hash = sha256_hex(&fresh);
        next.sessions[pos].expires_at = now + SESSION_TTL_SECS;
        self.persist(&next)?;
        *db = next;
        Ok(Some((fresh, user)))
    }

    /// The session id a refresh secret names, if any.
    fn session_of_secret(&self, secret: &str) -> Option<String> {
        let h = sha256_hex(secret);
        self.db
            .lock()
            .sessions
            .iter()
            .find(|s| s.refresh_hash == h)
            .map(|s| s.id.clone())
    }

    /// End the sessions `keep` rejects.
    fn end_sessions(&self, keep: impl Fn(&SessionRec) -> bool) -> Result<usize, UserStoreError> {
        let mut db = self.db.lock();
        let mut next = db.clone();
        next.sessions.retain(|s| keep(s));
        let ended = db.sessions.len() - next.sessions.len();
        if ended > 0 {
            self.persist(&next)?;
            *db = next;
        }
        Ok(ended)
    }

    /// Write the whole file through a temporary neighbour and a rename, so
    /// a crash mid-write leaves the previous file intact. The file holds
    /// password hashes: owner-only on Unix.
    fn persist(&self, db: &Db) -> Result<(), UserStoreError> {
        use std::io::Write;
        let io = |e| UserStoreError::Io(self.path.clone(), e);
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(db).expect("users serialise");
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp).map_err(io)?;
        f.write_all(&bytes).map_err(io)?;
        f.sync_all().map_err(io)?;
        drop(f);
        std::fs::rename(&tmp, &self.path).map_err(io)
    }

    pub(crate) fn find_by_email(&self, email: &str) -> Option<User> {
        let lc = email.trim().to_lowercase();
        self.db.lock().users.iter().find(|u| u.email == lc).cloned()
    }

    /// The account an identity provider's subject is attached to.
    pub(crate) fn find_by_provider(&self, provider: &str, sub: &str) -> Option<User> {
        self.db
            .lock()
            .users
            .iter()
            .find(|u| {
                u.providers
                    .iter()
                    .any(|p| p.provider == provider && p.sub == sub)
            })
            .cloned()
    }

    /// Attach a provider's subject to an existing account.
    pub(crate) fn link_provider(
        &self,
        id: &str,
        provider: &str,
        sub: &str,
    ) -> Result<User, UserStoreError> {
        self.update(id, |u| {
            u.providers.push(Provider {
                provider: provider.to_string(),
                sub: sub.to_string(),
            })
        })
    }

    /// Give or take the administrator role.
    pub(crate) fn set_admin(&self, id: &str, admin: bool) -> Result<User, UserStoreError> {
        self.update(id, |u| u.role = admin.then(|| "admin".to_string()))
    }

    fn update(&self, id: &str, f: impl FnOnce(&mut User)) -> Result<User, UserStoreError> {
        let mut db = self.db.lock();
        let mut next = db.clone();
        let user = next.users.iter_mut().find(|u| u.id == id).ok_or_else(|| {
            UserStoreError::Io(self.path.clone(), std::io::Error::other("no such user"))
        })?;
        f(user);
        let user = user.clone();
        self.persist(&next)?;
        *db = next;
        Ok(user)
    }

    /// An account created by an identity provider's sign-in: no password,
    /// the provider's subject attached. The first account is the
    /// administrator, as with sign-up.
    pub(crate) fn create_external(
        &self,
        email: &str,
        name: &str,
        picture: Option<String>,
        provider: &str,
        sub: &str,
    ) -> Result<User, UserStoreError> {
        let lc = email.trim().to_lowercase();
        let mut db = self.db.lock();
        let name = name.trim();
        let user = User {
            id: new_id(),
            email: lc.clone(),
            name: if name.is_empty() {
                lc.split('@').next().unwrap_or("").to_string()
            } else {
                name.to_string()
            },
            picture,
            password_hash: None,
            providers: vec![Provider {
                provider: provider.to_string(),
                sub: sub.to_string(),
            }],
            created_at: now_rfc3339(),
            role: db.users.is_empty().then(|| "admin".to_string()),
            clearances: Vec::new(),
        };
        let mut next = db.clone();
        next.users.push(user.clone());
        self.persist(&next)?;
        *db = next;
        Ok(user)
    }

    fn find_by_id(&self, id: &str) -> Option<User> {
        self.db.lock().users.iter().find(|u| u.id == id).cloned()
    }

    /// Add an account. The sign-up policy is decided here, under the lock,
    /// so two simultaneous first sign-ups cannot both become administrator:
    /// `signup = Some(allow)` means a self sign-up (the first account is the
    /// administrator, later ones need `allow`); `None` is an administrator
    /// creating a plain user.
    fn create(
        &self,
        email: &str,
        name: &str,
        password_hash: String,
        signup: Option<bool>,
    ) -> Result<User, CreateError> {
        let lc = email.trim().to_lowercase();
        let mut db = self.db.lock();
        let role = match signup {
            Some(_) if db.users.is_empty() => Some("admin"),
            Some(false) => return Err(CreateError::Closed),
            _ => None,
        };
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
            clearances: Vec::new(),
        };
        let mut next = db.clone();
        next.users.push(user.clone());
        self.persist(&next).map_err(CreateError::Store)?;
        *db = next;
        Ok(user)
    }

    fn delete(&self, id: &str) -> Result<bool, UserStoreError> {
        let mut db = self.db.lock();
        let mut next = db.clone();
        next.users.retain(|u| u.id != id);
        if next.users.len() == db.users.len() {
            return Ok(false);
        }
        next.sessions.retain(|s| s.user_id != id);
        self.persist(&next)?;
        *db = next;
        Ok(true)
    }

    fn list(&self) -> Vec<PublicUser> {
        self.db.lock().users.iter().map(User::public).collect()
    }

    /// `true` when `id` names an administrator.
    pub fn is_admin(&self, id: &str) -> bool {
        self.find_by_id(id).map(|u| u.is_admin()).unwrap_or(false)
    }

    /// What `id` may see: `(administrator, clearances)`. An administrator
    /// sees everything; another user, the sheets whose labels are all
    /// among their clearances.
    pub fn access_of(&self, id: &str) -> Option<(bool, Vec<String>)> {
        self.find_by_id(id)
            .map(|u| (u.is_admin(), u.clearances.clone()))
    }

    /// Set the role and/or the clearances of `id` (`None` leaves one as is).
    pub(crate) fn set_access(
        &self,
        id: &str,
        role: Option<String>,
        clearances: Option<Vec<String>>,
    ) -> Result<User, UserStoreError> {
        self.update(id, |u| {
            if let Some(r) = role {
                u.role = (r == "admin").then_some(r);
            }
            if let Some(c) = clearances {
                u.clearances = c;
            }
        })
    }

    /// The key a bearer secret names, if any: what `require_auth` asks.
    pub fn key_by_secret(&self, secret: &str) -> Option<PublicApiKey> {
        if !secret.starts_with(API_KEY_PREFIX) {
            return None;
        }
        let h = sha256_hex(secret);
        self.db
            .lock()
            .keys
            .iter()
            .find(|k| k.hash == h)
            .map(ApiKey::public)
    }

    /// Mint a key: the secret is returned once, only its hash is kept.
    fn create_key(&self, name: &str, by: &str) -> Result<(PublicApiKey, String), UserStoreError> {
        use rand::RngCore;
        let mut raw = [0u8; 32];
        rand::rng().fill_bytes(&mut raw);
        let secret = format!(
            "{API_KEY_PREFIX}{}",
            raw.iter().map(|b| format!("{b:02x}")).collect::<String>()
        );
        let key = ApiKey {
            id: new_id(),
            name: name.trim().to_string(),
            hash: sha256_hex(&secret),
            prefix: secret[..API_KEY_PREFIX.len() + 8].to_string(),
            created_at: now_rfc3339(),
            created_by: by.to_string(),
        };
        let mut db = self.db.lock();
        let mut next = db.clone();
        next.keys.push(key.clone());
        self.persist(&next)?;
        *db = next;
        Ok((key.public(), secret))
    }

    fn delete_key(&self, id: &str) -> Result<bool, UserStoreError> {
        let mut db = self.db.lock();
        let mut next = db.clone();
        next.keys.retain(|k| k.id != id);
        if next.keys.len() == db.keys.len() {
            return Ok(false);
        }
        self.persist(&next)?;
        *db = next;
        Ok(true)
    }

    fn list_keys(&self) -> Vec<PublicApiKey> {
        self.db.lock().keys.iter().map(ApiKey::public).collect()
    }
}

/// Unique id: the time in nanoseconds and a hashed counter, hex. Not a
/// UUID, and nothing here needs one.
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
    format!(
        "{:016x}{:016x}",
        nanos as u64 ^ h.finish(),
        h.finish().rotate_left(17) ^ n
    )
}

pub(crate) fn now_rfc3339() -> String {
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
    ok(local)
        && ok(domain)
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
}

// ---------------------------------------------------------------- bcrypt

/// bcrypt at cost 12 is ~250 ms of CPU: never on a runtime worker.
async fn hash_password(password: String) -> Result<String, Refused> {
    tokio::task::spawn_blocking(move || bcrypt::hash(password, BCRYPT_COST))
        .await
        .ok()
        .and_then(Result::ok)
        .ok_or((StatusCode::INTERNAL_SERVER_ERROR, "Internal error"))
}

/// A hash to check against when there is nothing to check against, so an
/// unknown email or a password-less (OAuth) account costs as much as a
/// wrong password.
fn dummy_hash() -> &'static str {
    static DUMMY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    DUMMY.get_or_init(|| bcrypt::hash("not-a-password", BCRYPT_COST).unwrap_or_default())
}

async fn verify_password(password: String, hash: Option<String>) -> bool {
    let (hash, real) = match hash {
        Some(h) => (h, true),
        None => (dummy_hash().to_string(), false),
    };
    let ok = tokio::task::spawn_blocking(move || bcrypt::verify(password, &hash).unwrap_or(false))
        .await
        .unwrap_or(false);
    ok && real
}

// ---------------------------------------------------------------- tokens

#[derive(Debug, Serialize)]
struct Claims<'a> {
    sub: &'a str,
    /// The session: `require_auth` refuses the token once it is ended.
    sid: &'a str,
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

    pub(crate) fn sign(&self, user: &User, sid: &str) -> String {
        let now = unix_now();
        let claims = Claims {
            sub: &user.id,
            sid,
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
    /// The sign-in providers live in the settings.
    pub settings: Arc<PlRwLock<Settings>>,
    pub oidc: Arc<oidc::Oidc>,
}

/// `/auth/signup`, `/auth/login`, `/auth/logout` and the OAuth bounces:
/// reachable without a token.
pub fn public_routes(state: AuthState) -> Router {
    Router::new()
        .route("/auth/signup", post(signup))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/refresh", post(refresh))
        .route("/auth/providers", get(oidc::providers))
        .route("/auth/oauth/:provider/start", get(oidc::start))
        .route("/auth/oauth/:provider/callback", get(oidc::callback))
        .with_state(state)
}

/// `/auth/me` and the administrator's user management: mounted behind
/// `require_auth`, which attaches the `AuthContext` these handlers read.
pub fn protected_routes(state: AuthState) -> Router {
    Router::new()
        .route("/auth/me", get(me))
        .route("/auth/sessions", axum::routing::delete(sign_out_everywhere))
        .route("/auth/users", get(list_users).post(create_user))
        .route(
            "/auth/users/:id",
            axum::routing::delete(delete_user).patch(update_user),
        )
        .route("/auth/keys", get(list_keys).post(create_key))
        .route("/auth/keys/:id", axum::routing::delete(delete_key))
        .with_state(state)
}

#[derive(Deserialize)]
struct KeyRequest {
    #[serde(default)]
    name: String,
}

async fn list_keys(State(s): State<AuthState>, Extension(ctx): Extension<AuthContext>) -> Response {
    match admin(&s, &ctx) {
        Ok(_) => Json(serde_json::json!({ "keys": s.store.list_keys() })).into_response(),
        Err((st, m)) => error(st, m),
    }
}

async fn create_key(
    State(s): State<AuthState>,
    Extension(ctx): Extension<AuthContext>,
    Json(r): Json<KeyRequest>,
) -> Response {
    let me = match admin(&s, &ctx) {
        Ok(u) => u,
        Err((st, m)) => return error(st, m),
    };
    if r.name.trim().is_empty() || r.name.len() > 80 {
        return error(
            StatusCode::BAD_REQUEST,
            "Key name required (80 characters at most)",
        );
    }
    match s.store.create_key(&r.name, &me.email) {
        Ok((key, secret)) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "key": secret, "id": key.id, "name": key.name, "prefix": key.prefix, "createdAt": key.created_at, "createdBy": key.created_by })),
        )
            .into_response(),
        Err(e) => store_error(e),
    }
}

async fn delete_key(
    State(s): State<AuthState>,
    Extension(ctx): Extension<AuthContext>,
    Path(id): Path<String>,
) -> Response {
    if let Err((st, m)) = admin(&s, &ctx) {
        return error(st, m);
    }
    match s.store.delete_key(&id) {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error(StatusCode::NOT_FOUND, "Key not found"),
        Err(e) => store_error(e),
    }
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

/// Errors are `(status, message)` pairs, turned into a response at the
/// edge (a `Response` in an `Err` is what clippy calls a large error).
type Refused = (StatusCode, &'static str);

fn error(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

/// A signed-in response: the access token in the body, the refresh
/// secret in an HttpOnly cookie scoped to `/auth`.
fn session(s: &AuthState, user: &User, status: StatusCode) -> Response {
    let (sid, secret) = match s.store.create_session(&user.id) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    let mut resp = (
        status,
        Json(Session {
            token: s.signer.sign(user, &sid),
            user: user.public(),
        }),
    )
        .into_response();
    resp.headers_mut().append(
        axum::http::header::SET_COOKIE,
        refresh_cookie(&secret, SESSION_TTL_SECS, s.secure_cookies()),
    );
    resp
}

/// Open a session for `user` and sign its first access token; the
/// cookie to set comes back with it (the OIDC bounce sets it itself).
pub(crate) fn open_session(
    s: &AuthState,
    user: &User,
) -> Result<(String, axum::http::HeaderValue), UserStoreError> {
    let (sid, secret) = s.store.create_session(&user.id)?;
    Ok((
        s.signer.sign(user, &sid),
        refresh_cookie(&secret, SESSION_TTL_SECS, s.secure_cookies()),
    ))
}

pub(crate) fn refresh_cookie(value: &str, max_age: u64, secure: bool) -> axum::http::HeaderValue {
    let secure = if secure { "; Secure" } else { "" };
    axum::http::HeaderValue::from_str(&format!(
        "{REFRESH_COOKIE}={value}; Path=/auth; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}"
    ))
    .expect("ascii cookie")
}

fn refresh_secret(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .flat_map(|h| h.split(';'))
        .map(str::trim)
        .find_map(|kv| {
            kv.strip_prefix(REFRESH_COOKIE)?
                .strip_prefix('=')
                .map(str::to_string)
        })
        .filter(|v| !v.is_empty())
}

impl AuthState {
    /// `Secure` on the cookies when the server is reached over HTTPS
    /// (the configured public URL says so).
    fn secure_cookies(&self) -> bool {
        self.settings
            .read()
            .signin
            .public_url
            .starts_with("https://")
    }
}

/// `POST /auth/refresh`: the refresh cookie for a new access token and a
/// new cookie; the old secret is dead at once.
async fn refresh(State(s): State<AuthState>, headers: axum::http::HeaderMap) -> Response {
    let Some(secret) = refresh_secret(&headers) else {
        return error(StatusCode::UNAUTHORIZED, "No session");
    };
    match s.store.rotate_session(&secret) {
        Ok(Some((fresh, user))) => {
            let sid = s.store.session_of_secret(&fresh).unwrap_or_default();
            let mut resp = (
                StatusCode::OK,
                Json(Session {
                    token: s.signer.sign(&user, &sid),
                    user: user.public(),
                }),
            )
                .into_response();
            resp.headers_mut().append(
                axum::http::header::SET_COOKIE,
                refresh_cookie(&fresh, SESSION_TTL_SECS, s.secure_cookies()),
            );
            resp
        }
        Ok(None) => {
            let mut resp = error(StatusCode::UNAUTHORIZED, "Session ended");
            resp.headers_mut().append(
                axum::http::header::SET_COOKIE,
                refresh_cookie("", 0, s.secure_cookies()),
            );
            resp
        }
        Err(e) => store_error(e),
    }
}

/// `DELETE /auth/sessions`: sign the current user out everywhere.
async fn sign_out_everywhere(
    State(s): State<AuthState>,
    Extension(ctx): Extension<AuthContext>,
) -> Response {
    let user = match current(&s, &ctx) {
        Ok(u) => u,
        Err((st, m)) => return error(st, m),
    };
    match s.store.end_sessions(|rec| rec.user_id != user.id) {
        Ok(ended) => {
            let mut resp = Json(serde_json::json!({ "ok": true, "ended": ended })).into_response();
            resp.headers_mut().append(
                axum::http::header::SET_COOKIE,
                refresh_cookie("", 0, s.secure_cookies()),
            );
            resp
        }
        Err(e) => store_error(e),
    }
}

fn validate(c: &Credentials, need_name: bool) -> Result<(), Refused> {
    if !valid_email(&c.email) {
        return Err((StatusCode::BAD_REQUEST, "Invalid email"));
    }
    if !strong_enough(&c.password) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Weak password (min 8 chars, mixed classes)",
        ));
    }
    if need_name && c.name.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Name required"));
    }
    Ok(())
}

fn store_error(e: UserStoreError) -> Response {
    tracing::error!(%e, "users file");
    error(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
}

async fn signup(State(s): State<AuthState>, Json(c): Json<Credentials>) -> Response {
    if let Err((st, m)) = validate(&c, true) {
        return error(st, m);
    }
    // Refuse before the hash when sign-up is visibly closed; `create`
    // decides again under its lock, so a race cannot mint two admins.
    if !s.allow_signup && !s.store.is_empty() {
        return error(
            StatusCode::FORBIDDEN,
            "Sign-up is closed: ask an administrator for an account",
        );
    }
    let hash = match hash_password(c.password).await {
        Ok(h) => h,
        Err((st, m)) => return error(st, m),
    };
    match s
        .store
        .create(&c.email, &c.name, hash, Some(s.allow_signup))
    {
        Ok(user) => {
            tracing::info!(email = %user.email, admin = user.is_admin(), "user signed up");
            session(&s, &user, StatusCode::CREATED)
        }
        Err(CreateError::Closed) => error(
            StatusCode::FORBIDDEN,
            "Sign-up is closed: ask an administrator for an account",
        ),
        Err(CreateError::EmailTaken) => error(StatusCode::CONFLICT, "Email already registered"),
        Err(CreateError::Store(e)) => store_error(e),
    }
}

async fn login(State(s): State<AuthState>, Json(c): Json<Credentials>) -> Response {
    if c.email.trim().is_empty() || c.password.is_empty() {
        return error(StatusCode::BAD_REQUEST, "Invalid credentials");
    }
    let user = s.store.find_by_email(&c.email);
    // Same cost whether the email exists, has a password or not.
    let ok = verify_password(
        c.password,
        user.as_ref().and_then(|u| u.password_hash.clone()),
    )
    .await;
    match user {
        Some(user) if ok => session(&s, &user, StatusCode::OK),
        _ => error(StatusCode::UNAUTHORIZED, "Invalid credentials"),
    }
}

/// `POST /auth/logout`: ends the session of the refresh cookie (the
/// access tokens of that session are refused from now on) and clears it.
/// Without a cookie there is nothing to end; still `ok`.
async fn logout(State(s): State<AuthState>, headers: axum::http::HeaderMap) -> Response {
    if let Some(sid) = refresh_secret(&headers).and_then(|v| s.store.session_of_secret(&v)) {
        if let Err(e) = s.store.end_sessions(|rec| rec.id != sid) {
            return store_error(e);
        }
    }
    let mut resp = Json(serde_json::json!({ "ok": true })).into_response();
    resp.headers_mut().append(
        axum::http::header::SET_COOKIE,
        refresh_cookie("", 0, s.secure_cookies()),
    );
    resp
}

fn current(s: &AuthState, ctx: &AuthContext) -> Result<User, Refused> {
    if ctx.service {
        return Err((StatusCode::UNAUTHORIZED, "User not found"));
    }
    s.store
        .find_by_id(&ctx.subject)
        .ok_or((StatusCode::UNAUTHORIZED, "User not found"))
}

fn admin(s: &AuthState, ctx: &AuthContext) -> Result<User, Refused> {
    let user = current(s, ctx)?;
    if !user.is_admin() {
        return Err((StatusCode::FORBIDDEN, "Administrator only"));
    }
    Ok(user)
}

async fn me(State(s): State<AuthState>, Extension(ctx): Extension<AuthContext>) -> Response {
    match current(&s, &ctx) {
        Ok(user) => Json(serde_json::json!({ "user": user.public() })).into_response(),
        Err((st, m)) => error(st, m),
    }
}

async fn list_users(
    State(s): State<AuthState>,
    Extension(ctx): Extension<AuthContext>,
) -> Response {
    match admin(&s, &ctx) {
        Ok(_) => Json(serde_json::json!({ "users": s.store.list() })).into_response(),
        Err((st, m)) => error(st, m),
    }
}

async fn create_user(
    State(s): State<AuthState>,
    Extension(ctx): Extension<AuthContext>,
    Json(c): Json<Credentials>,
) -> Response {
    if let Err((st, m)) = admin(&s, &ctx) {
        return error(st, m);
    }
    if let Err((st, m)) = validate(&c, false) {
        return error(st, m);
    }
    let hash = match hash_password(c.password).await {
        Ok(h) => h,
        Err((st, m)) => return error(st, m),
    };
    match s.store.create(&c.email, &c.name, hash, None) {
        Ok(user) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "user": user.public() })),
        )
            .into_response(),
        Err(CreateError::EmailTaken) => error(StatusCode::CONFLICT, "Email already registered"),
        Err(CreateError::Closed) => unreachable!("an administrator's create is never a sign-up"),
        Err(CreateError::Store(e)) => store_error(e),
    }
}

#[derive(Deserialize)]
struct AccessPatch {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    clearances: Option<Vec<String>>,
}

/// `PATCH /auth/users/:id {role?, clearances?}`: an administrator sets
/// what an account may see. Taking one's own administrator role away
/// is refused (the last administrator would lock everyone out).
async fn update_user(
    State(s): State<AuthState>,
    Extension(ctx): Extension<AuthContext>,
    Path(id): Path<String>,
    Json(p): Json<AccessPatch>,
) -> Response {
    let me = match admin(&s, &ctx) {
        Ok(u) => u,
        Err((st, m)) => return error(st, m),
    };
    if let Some(r) = p.role.as_deref() {
        if r != "admin" && r != "user" {
            return error(StatusCode::BAD_REQUEST, "role must be admin or user");
        }
        if me.id == id && r != "admin" {
            return error(
                StatusCode::BAD_REQUEST,
                "An administrator cannot take their own role away",
            );
        }
    }
    let clearances = p.clearances.map(|c| {
        c.into_iter()
            .map(|l| l.trim().to_lowercase())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
    });
    if let Some(bad) = clearances
        .as_ref()
        .and_then(|c| c.iter().find(|l| !crate::valid_label(l)))
    {
        return error(
            StatusCode::BAD_REQUEST,
            &format!("invalid clearance `{bad}`: lowercase letters, digits, `_ . -`, 64 at most"),
        );
    }
    if s.store.find_by_id(&id).is_none() {
        return error(StatusCode::NOT_FOUND, "User not found");
    }
    match s.store.set_access(&id, p.role, clearances) {
        Ok(user) => Json(serde_json::json!({ "user": user.public() })).into_response(),
        Err(e) => store_error(e),
    }
}

async fn delete_user(
    State(s): State<AuthState>,
    Extension(ctx): Extension<AuthContext>,
    Path(id): Path<String>,
) -> Response {
    let me = match admin(&s, &ctx) {
        Ok(u) => u,
        Err((st, m)) => return error(st, m),
    };
    if me.id == id {
        return error(
            StatusCode::BAD_REQUEST,
            "An administrator cannot delete their own account",
        );
    }
    match s.store.delete(&id) {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error(StatusCode::NOT_FOUND, "User not found"),
        Err(e) => store_error(e),
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

    #[test]
    fn the_policy_is_decided_under_the_lock() {
        let dir = std::env::temp_dir().join(format!("ontology-auth-unit-{}", new_id()));
        let store = UserStore::open(dir.join("users.json")).unwrap();
        let first = store
            .create("a@b.co", "A", "h".into(), Some(false))
            .unwrap();
        assert!(first.is_admin());
        assert!(matches!(
            store.create("b@b.co", "B", "h".into(), Some(false)),
            Err(CreateError::Closed)
        ));
        let second = store.create("b@b.co", "B", "h".into(), Some(true)).unwrap();
        assert!(!second.is_admin());
        assert!(matches!(
            store.create("b@b.co", "B", "h".into(), None),
            Err(CreateError::EmailTaken)
        ));
        assert!(store.exists(&first.id));
        assert!(store.delete(&second.id).unwrap());
        assert!(!store.exists(&second.id));
        assert_eq!(store.path(), dir.join("users.json"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
