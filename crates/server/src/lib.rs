// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! HTTP front-end for the ontology + RAG stack.
//!
//! Exposes a small JSON API that mirrors the CLI:
//!
//! | Method | Path        | Body / Returns                                      |
//! |--------|-------------|------------------------------------------------------|
//! | GET    | `/healthz`  | `"ok"`                                                |
//! | GET    | `/stats`    | counts of concepts, relations, types                  |
//! | POST   | `/concepts` | `Concept` JSON, returns the assigned `ConceptId`      |
//! | POST   | `/relations`| `Relation` JSON, returns the assigned `RelationId`    |
//! | DELETE | `/concepts/:id` | removes the concept and cascades                  |
//! | POST   | `/retrieve` | `RetrievalRequest`, returns ranked seeds + subgraph   |
//! | POST   | `/ask`      | `RetrievalRequest`, returns the full `RagAnswer`      |
//!
//! The router is constructed via [`build_router`] so callers can mount it
//! into a larger axum app or test it with `tower::ServiceExt`.

mod openapi;

use axum::{
    extract::{Multipart, Path, Query, State},
    http::{HeaderValue, Request, StatusCode},
    middleware::{self, Next},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use futures::stream::StreamExt;
use ontology_graph::{
    Action, ActionId, ActionPatch, Concept, ConceptId, ConceptPatch, Ontology, OntologyGraph,
    Path as GraphPath, Relation, RelationId, RelationPatch, Rule, RuleId, RulePatch, Subgraph,
    TraversalSpec,
};
use ontology_index::{HybridIndex, RetrievalRequest, ScoredConcept};
use ontology_io::{
    export_graph, ingest_records, CsvSource, IngestStats, JsonlSink, JsonlSource,
    TextDocumentSource, TripleSource, XlsxSource,
};
use ontology_rag::{OntologyGenError, RagAnswer, RagPipeline, RagStreamEvent};
use ontology_storage::{LogRecord, Store};
use parking_lot::{Mutex as PlMutex, RwLock as PlRwLock};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::Arc as StdArc;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tower_http::cors::{Any, CorsLayer};
use tracing::warn;

/// Shared application state passed to every handler.
#[derive(Clone)]
pub struct AppState {
    pub graph: Arc<OntologyGraph>,
    pub index: Arc<HybridIndex>,
    pub store: Arc<dyn Store>,
    pub pipeline: Arc<RagPipeline>,
    /// In-memory registry of files uploaded through `/upload`. Tracks
    /// metadata only — the raw bytes are streamed into the ingester and
    /// discarded. Resets on server restart.
    pub files: Arc<PlRwLock<FileRegistry>>,
    /// User-saved retrieval queries. In-memory; resets on restart.
    pub queries: Arc<PlRwLock<SavedQueryStore>>,
    /// Mutable user-facing settings (retrieval defaults, UI prefs).
    pub settings: Arc<PlRwLock<Settings>>,
    /// Ring buffer of recent `Stats` samples for sparklines & deltas.
    pub history: Arc<PlRwLock<StatsHistory>>,
    /// Server bind time — used by `/settings` for "uptime" display.
    pub started_at: SystemTime,
}

impl AppState {
    /// Construct a new application state with default-initialised in-memory
    /// stores for files, saved queries, settings and stats history.
    pub fn new(
        graph: Arc<OntologyGraph>,
        index: Arc<HybridIndex>,
        store: Arc<dyn Store>,
        pipeline: Arc<RagPipeline>,
    ) -> Self {
        Self {
            graph,
            index,
            store,
            pipeline,
            files: Arc::new(PlRwLock::new(FileRegistry::default())),
            queries: Arc::new(PlRwLock::new(SavedQueryStore::default())),
            settings: Arc::new(PlRwLock::new(Settings::default())),
            history: Arc::new(PlRwLock::new(StatsHistory::default())),
            started_at: SystemTime::now(),
        }
    }
}

// ---------------------------------------------------------------------------
// New in-memory state types (files / queries / settings / history)
// ---------------------------------------------------------------------------

/// A record of an uploaded file. Persisted in memory only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub id: u64,
    pub name: String,
    pub size: u64,
    pub kind: String,
    pub status: String,
    pub uploaded_at: u64,
    pub concepts: u64,
    pub relations: u64,
    pub ontology_updates: u64,
    #[serde(default)]
    pub concept_type: Option<String>,
}

#[derive(Debug, Default)]
pub struct FileRegistry {
    next_id: u64,
    records: Vec<FileRecord>,
}

impl FileRegistry {
    fn insert(&mut self, mut rec: FileRecord) -> FileRecord {
        self.next_id += 1;
        rec.id = self.next_id;
        self.records.push(rec.clone());
        rec
    }
    fn list(&self) -> Vec<FileRecord> {
        let mut v = self.records.clone();
        v.sort_by(|a, b| b.uploaded_at.cmp(&a.uploaded_at));
        v
    }
    fn get(&self, id: u64) -> Option<FileRecord> {
        self.records.iter().find(|r| r.id == id).cloned()
    }
    fn remove(&mut self, id: u64) -> bool {
        let len = self.records.len();
        self.records.retain(|r| r.id != id);
        self.records.len() != len
    }
}

/// A saved retrieval query (prompt + retrieval parameters).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedQuery {
    pub id: u64,
    pub name: String,
    pub query: String,
    #[serde(default = "default_query_top_k")]
    pub top_k: usize,
    #[serde(default = "default_query_lex_w")]
    pub lexical_weight: f32,
    #[serde(default)]
    pub concept_types: Vec<String>,
    #[serde(default = "default_query_depth")]
    pub expansion_depth: u32,
    pub created_at: u64,
    #[serde(default)]
    pub last_run_at: Option<u64>,
}

fn default_query_top_k() -> usize {
    8
}
fn default_query_lex_w() -> f32 {
    0.5
}
fn default_query_depth() -> u32 {
    2
}

/// Mutable fields a client may patch onto a saved query. All optional —
/// missing fields are left untouched.
#[derive(Debug, Default, Deserialize)]
pub struct SavedQueryPatch {
    pub name: Option<String>,
    pub query: Option<String>,
    pub top_k: Option<usize>,
    pub lexical_weight: Option<f32>,
    pub concept_types: Option<Vec<String>>,
    pub expansion_depth: Option<u32>,
}

#[derive(Debug, Default)]
pub struct SavedQueryStore {
    next_id: u64,
    records: Vec<SavedQuery>,
}

impl SavedQueryStore {
    fn insert(&mut self, mut q: SavedQuery) -> SavedQuery {
        self.next_id += 1;
        q.id = self.next_id;
        self.records.push(q.clone());
        q
    }
    fn list(&self) -> Vec<SavedQuery> {
        let mut v = self.records.clone();
        v.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        v
    }
    fn get(&self, id: u64) -> Option<SavedQuery> {
        self.records.iter().find(|q| q.id == id).cloned()
    }
    fn update(&mut self, id: u64, patch: SavedQueryPatch) -> Option<SavedQuery> {
        let rec = self.records.iter_mut().find(|q| q.id == id)?;
        if let Some(v) = patch.name {
            rec.name = v;
        }
        if let Some(v) = patch.query {
            rec.query = v;
        }
        if let Some(v) = patch.top_k {
            rec.top_k = v;
        }
        if let Some(v) = patch.lexical_weight {
            rec.lexical_weight = v;
        }
        if let Some(v) = patch.concept_types {
            rec.concept_types = v;
        }
        if let Some(v) = patch.expansion_depth {
            rec.expansion_depth = v;
        }
        Some(rec.clone())
    }
    fn touch_run(&mut self, id: u64) {
        if let Some(rec) = self.records.iter_mut().find(|q| q.id == id) {
            rec.last_run_at = Some(now_ts());
        }
    }
    fn remove(&mut self, id: u64) -> bool {
        let len = self.records.len();
        self.records.retain(|q| q.id != id);
        self.records.len() != len
    }
}

/// User-facing settings. LLM `provider` / `model` are sourced from the
/// running pipeline and are read-only (bound at `ontology serve` start);
/// the rest may be patched at runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub retrieval: RetrievalDefaults,
    pub ui: UiPrefs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalDefaults {
    pub top_k: usize,
    pub lexical_weight: f32,
    pub expansion_depth: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiPrefs {
    pub theme: String,
    pub graph_layout: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            retrieval: RetrievalDefaults {
                top_k: 8,
                lexical_weight: 0.5,
                expansion_depth: 2,
            },
            ui: UiPrefs {
                theme: "light".into(),
                graph_layout: "dagre".into(),
            },
        }
    }
}

/// PATCH-style settings update. Every field is optional; missing fields are
/// preserved.
#[derive(Debug, Default, Deserialize)]
pub struct SettingsPatch {
    pub retrieval: Option<RetrievalDefaultsPatch>,
    pub ui: Option<UiPrefsPatch>,
}

#[derive(Debug, Default, Deserialize)]
pub struct RetrievalDefaultsPatch {
    pub top_k: Option<usize>,
    pub lexical_weight: Option<f32>,
    pub expansion_depth: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
pub struct UiPrefsPatch {
    pub theme: Option<String>,
    pub graph_layout: Option<String>,
}

/// Bounded ring buffer of `Stats` samples — capacity ~7 days at 1h.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsSample {
    pub ts: u64,
    pub concepts: usize,
    pub relations: usize,
    pub concept_types: usize,
    pub relation_types: usize,
}

#[derive(Debug, Default)]
pub struct StatsHistory {
    samples: Vec<StatsSample>,
}

impl StatsHistory {
    const CAPACITY: usize = 200;
    /// Append a sample if the last one is older than 60s (or none yet).
    /// Keeps the buffer bounded and avoids spamming on every `/stats` call.
    fn record(&mut self, s: StatsSample) {
        if let Some(last) = self.samples.last() {
            if s.ts.saturating_sub(last.ts) < 60 {
                return;
            }
        }
        self.samples.push(s);
        let len = self.samples.len();
        if len > Self::CAPACITY {
            self.samples.drain(0..len - Self::CAPACITY);
        }
    }
    fn snapshot(&self) -> Vec<StatsSample> {
        self.samples.clone()
    }
    /// Percentage delta vs the oldest sample in the buffer. Used to power
    /// the "↑12% vs last run" pills on the dashboard.
    fn deltas_pct(&self, current: &StatsSample) -> StatsDeltas {
        let baseline = self.samples.first();
        let pct = |old: usize, new: usize| -> f32 {
            if old == 0 {
                if new == 0 {
                    0.0
                } else {
                    100.0
                }
            } else {
                ((new as f32 - old as f32) / old as f32) * 100.0
            }
        };
        match baseline {
            Some(b) => StatsDeltas {
                concepts_pct: pct(b.concepts, current.concepts),
                relations_pct: pct(b.relations, current.relations),
                concept_types_pct: pct(b.concept_types, current.concept_types),
                relation_types_pct: pct(b.relation_types, current.relation_types),
            },
            None => StatsDeltas::default(),
        }
    }
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct StatsDeltas {
    pub concepts_pct: f32,
    pub relations_pct: f32,
    pub concept_types_pct: f32,
    pub relation_types_pct: f32,
}

fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Tunables for the optional rate limiter / request-id middleware.
#[derive(Debug, Clone)]
pub struct RouterConfig {
    /// Static bearer token gate. Routes other than `/healthz` require
    /// `Authorization: Bearer <token>` when set. Kept for back-compat with
    /// machine-to-machine callers; for end users prefer [`Self::jwt`].
    pub bearer_token: Option<String>,
    /// JWT verification config. When set, requests must carry a valid
    /// `Authorization: Bearer <jwt>` signed with the same secret + issuer +
    /// audience as the companion `auth-server`. Validation is HS256 with
    /// `exp` enforced and ±60 s clock skew. If both `bearer_token` and `jwt`
    /// are set, either credential is accepted.
    pub jwt: Option<JwtAuth>,
    /// Per-IP request limit. `None` disables rate limiting.
    pub rate_limit: Option<RateLimit>,
}

/// JWT verification parameters. Mirrors the Node `auth-server` defaults
/// (`iss=ai-ontology`, `aud=web`, HS256, `exp` required) so the same token
/// issued by the auth-server unlocks this Rust API.
#[derive(Debug, Clone)]
pub struct JwtAuth {
    /// HS256 shared secret. Must match `JWT_SECRET` of the auth-server.
    pub secret: Vec<u8>,
    /// Required `iss` claim, e.g. `"ai-ontology"`.
    pub issuer: Option<String>,
    /// Required `aud` claim, e.g. `"web"`.
    pub audience: Option<String>,
    /// Allowed clock skew when checking `exp` / `nbf` (default 60 s).
    pub leeway_secs: u64,
}

impl JwtAuth {
    /// Convenience constructor matching the auth-server defaults.
    pub fn from_secret(secret: impl Into<Vec<u8>>) -> Self {
        Self {
            secret: secret.into(),
            issuer: Some("ai-ontology".into()),
            audience: Some("web".into()),
            leeway_secs: 60,
        }
    }
}

/// Claims issued by the auth-server. We don't need every field — just the
/// ones we want to validate or surface to handlers via `AuthContext`.
#[derive(Debug, Clone, Deserialize)]
struct JwtClaims {
    sub: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

/// Authenticated principal attached to the request extensions when a JWT
/// (or static token) was accepted. Handlers can extract this with
/// `Extension<AuthContext>` to enforce per-user authorization.
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub subject: String,
    pub email: Option<String>,
    pub name: Option<String>,
    /// `true` when the caller authenticated with the static service token
    /// rather than a user JWT.
    pub service: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct RateLimit {
    /// Max requests allowed in `window`.
    pub max_requests: u32,
    /// Sliding window. Tokens refill linearly across this window.
    pub window: Duration,
}

impl RouterConfig {
    pub fn unprotected() -> Self {
        Self {
            bearer_token: None,
            jwt: None,
            rate_limit: None,
        }
    }
}

pub fn build_router(state: AppState) -> Router {
    build_router_with_config(state, RouterConfig::unprotected())
}

/// Convenience for the common case: bearer token only, no rate limit.
pub fn build_router_with_auth(state: AppState, bearer_token: Option<String>) -> Router {
    build_router_with_config(
        state,
        RouterConfig {
            bearer_token,
            jwt: None,
            rate_limit: None,
        },
    )
}

/// Convenience for the typical SPA setup: validate JWTs issued by the
/// companion auth-server with the shared secret.
pub fn build_router_with_jwt(state: AppState, jwt: JwtAuth) -> Router {
    build_router_with_config(
        state,
        RouterConfig {
            bearer_token: None,
            jwt: Some(jwt),
            rate_limit: None,
        },
    )
}

/// Full-featured constructor. Adds (in this order, outer-to-inner):
/// 1. request-id injection (always on),
/// 2. optional rate-limit by client IP,
/// 3. optional bearer-token / JWT check.
pub fn build_router_with_config(state: AppState, cfg: RouterConfig) -> Router {
    build_router_inner(state, cfg.bearer_token, cfg.jwt, cfg.rate_limit)
}

fn build_router_inner(
    state: AppState,
    bearer_token: Option<String>,
    jwt: Option<JwtAuth>,
    rate_limit: Option<RateLimit>,
) -> Router {
    let healthz_router = Router::new()
        .route("/healthz", get(healthz))
        .route("/openapi.json", get(openapi::openapi_spec))
        .route("/docs", get(openapi::swagger_ui));

    let protected = Router::new()
        .route("/stats", get(stats))
        .route("/stats/history", get(stats_history))
        .route("/metrics", get(metrics))
        .route("/ontology", get(get_ontology).put(put_ontology))
        .route("/ontology/generate", post(generate_ontology_handler))
        .route("/concepts", get(list_concepts).post(create_concept))
        .route(
            "/concepts/:id",
            get(get_concept)
                .patch(update_concept)
                .delete(delete_concept),
        )
        .route("/relations", get(list_relations).post(create_relation))
        .route(
            "/relations/:id",
            get(get_relation_handler)
                .patch(update_relation_handler)
                .delete(delete_relation_handler),
        )
        .route("/rules", get(list_rules).post(create_rule))
        .route(
            "/rules/:id",
            get(get_rule_handler)
                .patch(update_rule_handler)
                .delete(delete_rule_handler),
        )
        .route("/actions", get(list_actions).post(create_action))
        .route(
            "/actions/:id",
            get(get_action_handler)
                .patch(update_action_handler)
                .delete(delete_action_handler),
        )
        .route("/retrieve", post(retrieve))
        .route("/subgraph", post(subgraph_handler))
        .route("/ask", post(ask))
        .route("/ask/stream", post(ask_stream))
        .route("/path", post(path))
        .route("/compact", post(compact))
        .route("/upload", post(upload))
        .route("/export", get(export_handler))
        .route("/files", get(list_files))
        .route("/files/:id", get(get_file).delete(delete_file))
        .route("/queries", get(list_queries).post(create_query))
        .route(
            "/queries/:id",
            get(get_query).patch(update_query).delete(delete_query),
        )
        .route("/queries/:id/run", post(run_query))
        .route("/settings", get(get_settings).patch(patch_settings))
        .with_state(state);

    let protected = if bearer_token.is_some() || jwt.is_some() {
        let static_token = bearer_token.map(StdArc::new);
        let jwt_cfg = jwt.map(|j| StdArc::new(BuiltJwt::new(j)));
        protected.layer(middleware::from_fn(move |req, next| {
            let static_token = static_token.clone();
            let jwt_cfg = jwt_cfg.clone();
            async move { require_auth(req, next, static_token, jwt_cfg).await }
        }))
    } else {
        protected
    };

    let mut app = healthz_router.merge(protected);

    if let Some(rl) = rate_limit {
        let limiter = StdArc::new(RateLimiter::new(rl));
        app = app.layer(middleware::from_fn(move |req, next| {
            let limiter = limiter.clone();
            async move { rate_limit_layer(req, next, limiter).await }
        }));
    }

    // Outermost — every response gets an X-Request-Id and every span gets one.
    app = app.layer(middleware::from_fn(request_id_layer));

    // Permissive CORS — fine for the demo / local React dev server. In
    // production restrict origins via CorsLayer::new().allow_origin(...).
    app = app.layer(
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any),
    );

    app
}

/// Token-bucket-ish rate limiter keyed by remote IP. Uses `parking_lot::Mutex`
/// because the critical section is microseconds; contention is rare.
#[derive(Debug)]
struct RateLimiter {
    cfg: RateLimit,
    state: PlMutex<ahash::AHashMap<std::net::IpAddr, BucketState>>,
}

#[derive(Debug, Clone, Copy)]
struct BucketState {
    /// Number of tokens currently in the bucket.
    tokens: f64,
    /// Last time we refilled.
    last: Instant,
}

impl RateLimiter {
    fn new(cfg: RateLimit) -> Self {
        Self {
            cfg,
            state: PlMutex::new(ahash::AHashMap::new()),
        }
    }

    fn allow(&self, ip: std::net::IpAddr) -> bool {
        let max = self.cfg.max_requests as f64;
        let refill_per_sec = max / self.cfg.window.as_secs_f64().max(0.001);
        let now = Instant::now();
        let mut buckets = self.state.lock();
        let bucket = buckets.entry(ip).or_insert(BucketState {
            tokens: max,
            last: now,
        });
        let elapsed = now.duration_since(bucket.last).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * refill_per_sec).min(max);
        bucket.last = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

async fn rate_limit_layer(
    req: Request<axum::body::Body>,
    next: Next,
    limiter: StdArc<RateLimiter>,
) -> Result<Response, StatusCode> {
    // Extract client IP from the `connect_info` extension (set by axum's
    // `IntoMakeServiceWithConnectInfo`) or fall back to a sentinel that
    // groups all anonymous callers into one bucket — fail-closed on the
    // shared bucket, not fail-open per request.
    let ip = req
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|ci| ci.0.ip())
        .unwrap_or_else(|| std::net::IpAddr::V4(std::net::Ipv4Addr::new(0, 0, 0, 0)));
    if !limiter.allow(ip) {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    Ok(next.run(req).await)
}

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);

async fn request_id_layer(mut req: Request<axum::body::Body>, next: Next) -> Response {
    // Honor an inbound X-Request-Id, otherwise mint one.
    let inbound = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let id = inbound.unwrap_or_else(|| {
        let n = REQUEST_COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(n);
        format!("req-{nanos:x}-{n:x}")
    });
    req.extensions_mut().insert(RequestId(id.clone()));
    let mut resp = next.run(req).await;
    if let Ok(value) = HeaderValue::from_str(&id) {
        resp.headers_mut().insert("x-request-id", value);
    }
    resp
}

/// Extension type holding the request id — extract via `Extension<RequestId>`
/// from a handler if you want to log or surface it.
#[derive(Debug, Clone)]
pub struct RequestId(pub String);

/// Pre-built decoding key + validation so we don't reallocate per request.
struct BuiltJwt {
    decoding: jsonwebtoken::DecodingKey,
    validation: jsonwebtoken::Validation,
}

impl std::fmt::Debug for BuiltJwt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuiltJwt").finish_non_exhaustive()
    }
}

impl BuiltJwt {
    fn new(cfg: JwtAuth) -> Self {
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.leeway = cfg.leeway_secs;
        validation.validate_exp = true;
        if let Some(iss) = cfg.issuer.as_ref() {
            validation.set_issuer(&[iss.as_str()]);
        }
        if let Some(aud) = cfg.audience.as_ref() {
            validation.set_audience(&[aud.as_str()]);
        } else {
            // jsonwebtoken validates `aud` by default; disable when not pinned.
            validation.validate_aud = false;
        }
        Self {
            decoding: jsonwebtoken::DecodingKey::from_secret(&cfg.secret),
            validation,
        }
    }

    fn verify(&self, token: &str) -> Result<JwtClaims, jsonwebtoken::errors::Error> {
        jsonwebtoken::decode::<JwtClaims>(token, &self.decoding, &self.validation)
            .map(|data| data.claims)
    }
}

fn extract_bearer(req: &Request<axum::body::Body>) -> Option<String> {
    req.headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v: &HeaderValue| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer ").or_else(|| s.strip_prefix("bearer ")))
        .map(|s| s.trim().to_string())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

async fn require_auth(
    mut req: Request<axum::body::Body>,
    next: Next,
    static_token: Option<StdArc<String>>,
    jwt: Option<StdArc<BuiltJwt>>,
) -> Result<Response, StatusCode> {
    let provided = extract_bearer(&req).ok_or(StatusCode::UNAUTHORIZED)?;

    // Try JWT first (user credentials), then fall back to the static
    // service token. Both paths attach an `AuthContext` extension so
    // downstream handlers can identify the caller.
    if let Some(jwt) = jwt.as_ref() {
        match jwt.verify(&provided) {
            Ok(claims) => {
                req.extensions_mut().insert(AuthContext {
                    subject: claims.sub,
                    email: claims.email,
                    name: claims.name,
                    service: false,
                });
                return Ok(next.run(req).await);
            }
            Err(err) => {
                tracing::debug!(?err, "jwt verification failed");
                // fall through to static-token check
            }
        }
    }

    if let Some(expected) = static_token.as_ref() {
        if constant_time_eq(provided.as_bytes(), expected.as_bytes()) {
            req.extensions_mut().insert(AuthContext {
                subject: "service".into(),
                email: None,
                name: None,
                service: true,
            });
            return Ok(next.run(req).await);
        }
    }

    Err(StatusCode::UNAUTHORIZED)
}

#[derive(Deserialize)]
struct PathRequest {
    from_type: String,
    from_name: String,
    to_type: String,
    to_name: String,
    #[serde(default = "default_path_depth")]
    max_depth: u32,
}

fn default_path_depth() -> u32 {
    6
}

#[derive(Serialize)]
struct PathResponse {
    found: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<GraphPath>,
}

async fn path(
    State(s): State<AppState>,
    Json(req): Json<PathRequest>,
) -> Result<Json<PathResponse>, ApiError> {
    let src = s
        .graph
        .find_by_name(&req.from_type, &req.from_name)
        .ok_or_else(|| {
            ApiError::BadRequest(format!("no concept ({}) {}", req.from_type, req.from_name,))
        })?;
    let tgt = s
        .graph
        .find_by_name(&req.to_type, &req.to_name)
        .ok_or_else(|| {
            ApiError::BadRequest(format!("no concept ({}) {}", req.to_type, req.to_name,))
        })?;
    let p = s.graph.shortest_path(src, tgt, req.max_depth)?;
    Ok(Json(PathResponse {
        found: p.is_some(),
        path: p,
    }))
}

async fn compact(State(s): State<AppState>) -> Result<StatusCode, ApiError> {
    s.store
        .compact(&s.graph)
        .await
        .map_err(|e| ApiError::Store(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn healthz() -> &'static str {
    "ok"
}

#[derive(Serialize)]
struct Stats {
    concepts: usize,
    relations: usize,
    rules: usize,
    actions: usize,
    concept_types: usize,
    relation_types: usize,
    rule_types: usize,
    action_types: usize,
}

/// Prometheus-compatible plain-text metrics. Plays nicely with any
/// scraper that speaks the 0.0.4 exposition format. Counts are gauges
/// (instantaneous), not counters.
async fn metrics(State(s): State<AppState>) -> ([(String, String); 1], String) {
    let onto = s.graph.ontology();
    let body = format!(
        "# HELP ontology_concepts Number of concepts in the graph.\n\
         # TYPE ontology_concepts gauge\n\
         ontology_concepts {}\n\
         # HELP ontology_relations Number of relations in the graph.\n\
         # TYPE ontology_relations gauge\n\
         ontology_relations {}\n\
         # HELP ontology_concept_types Number of concept types in the ontology.\n\
         # TYPE ontology_concept_types gauge\n\
         ontology_concept_types {}\n\
         # HELP ontology_relation_types Number of relation types in the ontology.\n\
         # TYPE ontology_relation_types gauge\n\
         ontology_relation_types {}\n\
         # HELP ontology_rule_types Number of rule types declared in the ontology.\n\
         # TYPE ontology_rule_types gauge\n\
         ontology_rule_types {}\n\
         # HELP ontology_action_types Number of action types declared in the ontology.\n\
         # TYPE ontology_action_types gauge\n\
         ontology_action_types {}\n\
         # HELP ontology_rules Number of rule instances in the graph.\n\
         # TYPE ontology_rules gauge\n\
         ontology_rules {}\n\
         # HELP ontology_actions Number of action instances in the graph.\n\
         # TYPE ontology_actions gauge\n\
         ontology_actions {}\n",
        s.graph.concept_count(),
        s.graph.relation_count(),
        onto.concept_types.len(),
        onto.relation_types.len(),
        onto.rule_types.len(),
        onto.action_types.len(),
        s.graph.rule_count(),
        s.graph.action_count(),
    );
    (
        [(
            axum::http::header::CONTENT_TYPE.to_string(),
            "text/plain; version=0.0.4".to_string(),
        )],
        body,
    )
}

async fn stats(State(s): State<AppState>) -> Json<StatsResponse> {
    let onto = s.graph.ontology();
    let core = Stats {
        concepts: s.graph.concept_count(),
        relations: s.graph.relation_count(),
        rules: s.graph.rule_count(),
        actions: s.graph.action_count(),
        concept_types: onto.concept_types.len(),
        relation_types: onto.relation_types.len(),
        rule_types: onto.rule_types.len(),
        action_types: onto.action_types.len(),
    };
    let sample = StatsSample {
        ts: now_ts(),
        concepts: core.concepts,
        relations: core.relations,
        concept_types: core.concept_types,
        relation_types: core.relation_types,
    };
    let deltas = {
        let mut h = s.history.write();
        let d = h.deltas_pct(&sample);
        h.record(sample);
        d
    };
    Json(StatsResponse { core, deltas })
}

#[derive(Serialize)]
struct StatsResponse {
    #[serde(flatten)]
    core: Stats,
    deltas: StatsDeltas,
}

#[derive(Serialize)]
struct CreatedConcept {
    id: ConceptId,
}

async fn create_concept(
    State(s): State<AppState>,
    Json(mut concept): Json<Concept>,
) -> Result<Json<CreatedConcept>, ApiError> {
    let id = s.graph.upsert_concept(concept.clone())?;
    concept.id = id;
    if let Err(e) = s.store.append(&LogRecord::concept(concept.clone())).await {
        warn!(error=%e, "wal append failed");
        return Err(ApiError::Store(e.to_string()));
    }
    s.index.index_concept(id)?;
    Ok(Json(CreatedConcept { id }))
}

async fn get_concept(
    State(s): State<AppState>,
    Path(id): Path<u64>,
) -> Result<Json<Concept>, ApiError> {
    let c = s.graph.get_concept(ConceptId(id))?;
    Ok(Json(c))
}

#[derive(Deserialize, Default)]
struct ListConceptsQuery {
    /// Filter by concept type (exact match).
    #[serde(rename = "type")]
    concept_type: Option<String>,
    /// Case-insensitive substring match on the concept name.
    q: Option<String>,
    /// Maximum number of concepts to return. Defaults to 200, capped at 5_000
    /// to keep the JSON payload bounded.
    limit: Option<usize>,
    /// Number of matching concepts to skip before returning results.
    #[serde(default)]
    offset: usize,
}

#[derive(Serialize)]
struct ListConceptsResponse {
    /// Total number of concepts matching the filter (before `limit`/`offset`).
    total: usize,
    concepts: Vec<Concept>,
}

/// `GET /concepts?type=&q=&limit=&offset=` — paginated browse of every node
/// in the graph. Sorted by `(concept_type, name)` so the response is stable
/// across calls.
async fn list_concepts(
    State(s): State<AppState>,
    Query(q): Query<ListConceptsQuery>,
) -> Json<ListConceptsResponse> {
    let needle = q.q.as_ref().map(|s| s.to_lowercase());
    let mut all: Vec<Concept> = s
        .graph
        .all_concepts()
        .into_iter()
        .filter(|c| {
            q.concept_type
                .as_ref()
                .map(|t| c.concept_type == *t)
                .unwrap_or(true)
        })
        .filter(|c| {
            needle
                .as_ref()
                .map(|n| c.name.to_lowercase().contains(n))
                .unwrap_or(true)
        })
        .collect();
    all.sort_by(|a, b| {
        a.concept_type
            .cmp(&b.concept_type)
            .then_with(|| a.name.cmp(&b.name))
    });
    let total = all.len();
    let limit = q.limit.unwrap_or(200).min(5_000);
    let concepts = all.into_iter().skip(q.offset).take(limit).collect();
    Json(ListConceptsResponse { total, concepts })
}

/// `GET /ontology` — the concept-type and relation-type schema, served
/// verbatim. Useful for clients that want to render type-aware UIs.
async fn get_ontology(State(s): State<AppState>) -> Json<Ontology> {
    Json(s.graph.ontology())
}

async fn update_concept(
    State(s): State<AppState>,
    Path(id): Path<u64>,
    Json(patch): Json<ConceptPatch>,
) -> Result<Json<Concept>, ApiError> {
    let updated = s.graph.update_concept(ConceptId(id), patch)?;
    s.store
        .append(&LogRecord::update_concept(updated.clone()))
        .await
        .map_err(|e| ApiError::Store(e.to_string()))?;
    s.index.index_concept(ConceptId(id))?;
    Ok(Json(updated))
}

async fn delete_concept(
    State(s): State<AppState>,
    Path(id): Path<u64>,
) -> Result<StatusCode, ApiError> {
    let cid = ConceptId(id);
    let removed = s.graph.remove_concept(cid)?;
    s.index.forget(cid);
    s.store
        .append(&LogRecord::delete_concept(cid))
        .await
        .map_err(|e| ApiError::Store(e.to_string()))?;
    for rid in removed {
        s.store
            .append(&LogRecord::delete_relation(rid))
            .await
            .map_err(|e| ApiError::Store(e.to_string()))?;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct CreatedRelation {
    id: RelationId,
}

async fn create_relation(
    State(s): State<AppState>,
    Json(mut rel): Json<Relation>,
) -> Result<Json<CreatedRelation>, ApiError> {
    let id = s.graph.add_relation(rel.clone())?;
    rel.id = id;
    s.store
        .append(&LogRecord::relation(rel))
        .await
        .map_err(|e| ApiError::Store(e.to_string()))?;
    Ok(Json(CreatedRelation { id }))
}

#[derive(Deserialize, Default)]
struct ListRelationsQuery {
    #[serde(default)]
    source: Option<u64>,
    #[serde(default)]
    target: Option<u64>,
    #[serde(default, rename = "type")]
    relation_type: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
}

#[derive(Serialize)]
struct ListRelationsResponse {
    total: usize,
    relations: Vec<Relation>,
}

async fn list_relations(
    State(s): State<AppState>,
    Query(q): Query<ListRelationsQuery>,
) -> Json<ListRelationsResponse> {
    let mut all = s.graph.all_relations();
    if let Some(src) = q.source {
        all.retain(|r| r.source.0 == src);
    }
    if let Some(tgt) = q.target {
        all.retain(|r| r.target.0 == tgt);
    }
    if let Some(t) = q.relation_type.as_deref() {
        all.retain(|r| r.relation_type == t);
    }
    all.sort_by(|a, b| a.id.0.cmp(&b.id.0));
    let total = all.len();
    let offset = q.offset.unwrap_or(0);
    let limit = q.limit.unwrap_or(100).min(1000);
    let relations = all.into_iter().skip(offset).take(limit).collect();
    Json(ListRelationsResponse { total, relations })
}

async fn get_relation_handler(
    State(s): State<AppState>,
    Path(id): Path<u64>,
) -> Result<Json<Relation>, ApiError> {
    Ok(Json(s.graph.get_relation(RelationId(id))?))
}

async fn update_relation_handler(
    State(s): State<AppState>,
    Path(id): Path<u64>,
    Json(patch): Json<RelationPatch>,
) -> Result<Json<Relation>, ApiError> {
    let updated = s.graph.update_relation(RelationId(id), patch)?;
    s.store
        .append(&LogRecord::update_relation(updated.clone()))
        .await
        .map_err(|e| ApiError::Store(e.to_string()))?;
    Ok(Json(updated))
}

async fn delete_relation_handler(
    State(s): State<AppState>,
    Path(id): Path<u64>,
) -> Result<StatusCode, ApiError> {
    let rid = RelationId(id);
    s.graph.remove_relation(rid)?;
    s.store
        .append(&LogRecord::delete_relation(rid))
        .await
        .map_err(|e| ApiError::Store(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct CreatedRule {
    id: RuleId,
}

async fn list_rules(State(s): State<AppState>) -> Json<Vec<Rule>> {
    let mut all = s.graph.all_rules();
    all.sort_by(|a, b| a.rule_type.cmp(&b.rule_type).then_with(|| a.name.cmp(&b.name)));
    Json(all)
}

async fn create_rule(
    State(s): State<AppState>,
    Json(mut rule): Json<Rule>,
) -> Result<Json<CreatedRule>, ApiError> {
    let id = s.graph.upsert_rule(rule.clone())?;
    rule.id = id;
    s.store
        .append(&LogRecord::rule(rule))
        .await
        .map_err(|e| ApiError::Store(e.to_string()))?;
    Ok(Json(CreatedRule { id }))
}

async fn get_rule_handler(
    State(s): State<AppState>,
    Path(id): Path<u64>,
) -> Result<Json<Rule>, ApiError> {
    let r = s.graph.get_rule(RuleId(id))?;
    Ok(Json(r))
}

async fn delete_rule_handler(
    State(s): State<AppState>,
    Path(id): Path<u64>,
) -> Result<StatusCode, ApiError> {
    let rid = RuleId(id);
    s.graph.remove_rule(rid)?;
    s.store
        .append(&LogRecord::delete_rule(rid))
        .await
        .map_err(|e| ApiError::Store(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn update_rule_handler(
    State(s): State<AppState>,
    Path(id): Path<u64>,
    Json(patch): Json<RulePatch>,
) -> Result<Json<Rule>, ApiError> {
    let updated = s.graph.update_rule(RuleId(id), patch)?;
    s.store
        .append(&LogRecord::rule(updated.clone()))
        .await
        .map_err(|e| ApiError::Store(e.to_string()))?;
    Ok(Json(updated))
}

#[derive(Serialize)]
struct CreatedAction {
    id: ActionId,
}

async fn list_actions(State(s): State<AppState>) -> Json<Vec<Action>> {
    let mut all = s.graph.all_actions();
    all.sort_by(|a, b| {
        a.action_type
            .cmp(&b.action_type)
            .then_with(|| a.name.cmp(&b.name))
    });
    Json(all)
}

async fn create_action(
    State(s): State<AppState>,
    Json(mut action): Json<Action>,
) -> Result<Json<CreatedAction>, ApiError> {
    let id = s.graph.upsert_action(action.clone())?;
    action.id = id;
    s.store
        .append(&LogRecord::action(action))
        .await
        .map_err(|e| ApiError::Store(e.to_string()))?;
    Ok(Json(CreatedAction { id }))
}

async fn get_action_handler(
    State(s): State<AppState>,
    Path(id): Path<u64>,
) -> Result<Json<Action>, ApiError> {
    let a = s.graph.get_action(ActionId(id))?;
    Ok(Json(a))
}

async fn delete_action_handler(
    State(s): State<AppState>,
    Path(id): Path<u64>,
) -> Result<StatusCode, ApiError> {
    let aid = ActionId(id);
    s.graph.remove_action(aid)?;
    s.store
        .append(&LogRecord::delete_action(aid))
        .await
        .map_err(|e| ApiError::Store(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn update_action_handler(
    State(s): State<AppState>,
    Path(id): Path<u64>,
    Json(patch): Json<ActionPatch>,
) -> Result<Json<Action>, ApiError> {
    let updated = s.graph.update_action(ActionId(id), patch)?;
    s.store
        .append(&LogRecord::action(updated.clone()))
        .await
        .map_err(|e| ApiError::Store(e.to_string()))?;
    Ok(Json(updated))
}

#[derive(Serialize)]
struct RetrieveResponse {
    scored: Vec<ScoredConcept>,
    subgraph: ontology_graph::Subgraph,
}

async fn retrieve(
    State(s): State<AppState>,
    Json(req): Json<RetrievalRequest>,
) -> Json<RetrieveResponse> {
    let (scored, subgraph) = s.index.retrieve(&req);
    Json(RetrieveResponse { scored, subgraph })
}

async fn ask(
    State(s): State<AppState>,
    Json(req): Json<RetrievalRequest>,
) -> Result<Json<RagAnswer>, ApiError> {
    let ans = s
        .pipeline
        .answer_with(req)
        .await
        .map_err(|e| ApiError::Llm(e.to_string()))?;
    Ok(Json(ans))
}

/// Server-Sent Events flavor of `/ask`. Each event is one
/// [`RagStreamEvent`] serialized as JSON. Order:
///
/// 1. `event: retrieved` — grounding subgraph.
/// 2. `event: token`     — zero or more text deltas.
/// 3. `event: end`       — final usage/stop reason. Stream closes after.
/// 4. `event: error`     — any LLM error; stream closes after.
async fn ask_stream(
    State(s): State<AppState>,
    Json(req): Json<RetrievalRequest>,
) -> Result<Sse<futures::stream::BoxStream<'static, Result<Event, Infallible>>>, ApiError> {
    let inner = s
        .pipeline
        .answer_stream(req)
        .await
        .map_err(|e| ApiError::Llm(e.to_string()))?;

    let events = inner.map(|item| {
        let event = match &item {
            Ok(RagStreamEvent::Retrieved { .. }) => Event::default().event("retrieved"),
            Ok(RagStreamEvent::Token { .. }) => Event::default().event("token"),
            Ok(RagStreamEvent::End { .. }) => Event::default().event("end"),
            Err(_) => Event::default().event("error"),
        };
        let payload = match item {
            Ok(ev) => serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into()),
            Err(e) => serde_json::to_string(&serde_json::json!({"message": e.to_string()}))
                .unwrap_or_else(|_| "{}".into()),
        };
        Ok::<_, Infallible>(event.data(payload))
    });

    Ok(Sse::new(events.boxed()).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

/// Multipart-upload ingest. The form must carry exactly one `file` part
/// (the bytes) and a `kind` field selecting the adapter:
///
/// | `kind`      | semantics                                                         |
/// |-------------|-------------------------------------------------------------------|
/// | `ontology`  | JSON `Ontology` definition. Replaces the current schema in place. |
/// | `jsonl`     | Tagged Records (Concept / Relation / Ontology / NamedRelation).   |
/// | `triples`   | `Type:Name predicate Type:Name` lines.                            |
/// | `csv`       | One concept per row; needs a `concept_type` form field.           |
/// | `xlsx`      | Same as CSV but for spreadsheets; needs `concept_type`.           |
/// | `text`      | The whole upload becomes one Concept whose description is the     |
/// |             | text body; needs `concept_type` (and uses the `name` form field   |
/// |             | if present, otherwise the uploaded filename's stem).              |
///
/// Files are buffered to a tempfile so the existing path-based adapters
/// (`CsvSource`, `XlsxSource`, ...) work unchanged. Returns
/// `{ ingested: { concepts, relations, ontology_updates } }`.
async fn upload(
    State(s): State<AppState>,
    mut form: Multipart,
) -> Result<Json<UploadResponse>, ApiError> {
    let mut kind: Option<String> = None;
    let mut concept_type: Option<String> = None;
    let mut name_override: Option<String> = None;
    let mut filename: Option<String> = None;
    let mut bytes: Option<Vec<u8>> = None;

    while let Some(field) = form
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("multipart: {e}")))?
    {
        match field.name().unwrap_or("") {
            "kind" => {
                kind = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| ApiError::BadRequest(e.to_string()))?,
                );
            }
            "concept_type" => {
                concept_type = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| ApiError::BadRequest(e.to_string()))?,
                );
            }
            "name" => {
                name_override = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| ApiError::BadRequest(e.to_string()))?,
                );
            }
            "file" => {
                filename = field.file_name().map(|s| s.to_string());
                bytes = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| ApiError::BadRequest(e.to_string()))?
                        .to_vec(),
                );
            }
            _ => {} // ignore unknown form fields
        }
    }

    let kind = kind.ok_or_else(|| ApiError::BadRequest("missing `kind`".into()))?;
    let bytes = bytes.ok_or_else(|| ApiError::BadRequest("missing `file`".into()))?;
    // Snapshot for the file registry — the match below consumes `concept_type`.
    let concept_type_for_record = concept_type.clone();

    let stats = match kind.as_str() {
        "ontology" => {
            let onto: Ontology = serde_json::from_slice(&bytes)
                .map_err(|e| ApiError::BadRequest(format!("ontology: {e}")))?;
            s.graph.extend_ontology(|target| {
                *target = onto.clone();
                Ok(())
            })?;
            s.store
                .append(&LogRecord::ontology(onto))
                .await
                .map_err(|e| ApiError::Store(e.to_string()))?;
            IngestStats {
                ontology_updates: 1,
                ..Default::default()
            }
        }
        "jsonl" | "ndjson" => {
            let tmp = persist_temp(&bytes, "jsonl").await?;
            let mut src = JsonlSource::open(tmp.path())
                .await
                .map_err(|e| ApiError::BadRequest(e.to_string()))?;
            ingest_records(&mut src, &s.graph, Some(s.store.as_ref()))
                .await
                .map_err(|e| ApiError::BadRequest(e.to_string()))?
        }
        "triples" => {
            let tmp = persist_temp(&bytes, "triples").await?;
            let mut src = TripleSource::open(tmp.path())
                .await
                .map_err(|e| ApiError::BadRequest(e.to_string()))?;
            ingest_records(&mut src, &s.graph, Some(s.store.as_ref()))
                .await
                .map_err(|e| ApiError::BadRequest(e.to_string()))?
        }
        "csv" => {
            let ty = concept_type
                .ok_or_else(|| ApiError::BadRequest("csv requires concept_type".into()))?;
            let tmp = persist_temp(&bytes, "csv").await?;
            let mut src = CsvSource::open(tmp.path(), ty)
                .await
                .map_err(|e| ApiError::BadRequest(e.to_string()))?;
            ingest_records(&mut src, &s.graph, Some(s.store.as_ref()))
                .await
                .map_err(|e| ApiError::BadRequest(e.to_string()))?
        }
        "xlsx" => {
            let ty = concept_type
                .ok_or_else(|| ApiError::BadRequest("xlsx requires concept_type".into()))?;
            let tmp = persist_temp(&bytes, "xlsx").await?;
            let mut src = XlsxSource::open(tmp.path(), ty)
                .map_err(|e| ApiError::BadRequest(e.to_string()))?;
            ingest_records(&mut src, &s.graph, Some(s.store.as_ref()))
                .await
                .map_err(|e| ApiError::BadRequest(e.to_string()))?
        }
        "text" => {
            let ty = concept_type
                .ok_or_else(|| ApiError::BadRequest("text requires concept_type".into()))?;
            let stem = name_override.unwrap_or_else(|| {
                filename
                    .as_deref()
                    .map(std::path::Path::new)
                    .and_then(|p| p.file_stem().and_then(|s| s.to_str()))
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "uploaded".into())
            });
            // Buffer to a tempfile + reuse TextDocumentSource so we go through
            // exactly the same code path as the CLI.
            let dir = tempfile::tempdir().map_err(|e| ApiError::Store(e.to_string()))?;
            let path = dir.path().join(format!("{stem}.txt"));
            tokio::fs::write(&path, &bytes)
                .await
                .map_err(|e| ApiError::Store(e.to_string()))?;
            let mut src = TextDocumentSource::from_files(ty, [path]);
            ingest_records(&mut src, &s.graph, Some(s.store.as_ref()))
                .await
                .map_err(|e| ApiError::BadRequest(e.to_string()))?
        }
        other => return Err(ApiError::BadRequest(format!("unknown kind: {other}"))),
    };

    s.index.reindex_all();

    let display_name = filename.clone().unwrap_or_else(|| format!("upload.{kind}"));
    let size = bytes.len() as u64;
    let rec = FileRecord {
        id: 0,
        name: display_name,
        size,
        kind: kind.clone(),
        status: "processed".into(),
        uploaded_at: now_ts(),
        concepts: stats.concepts,
        relations: stats.relations,
        ontology_updates: stats.ontology_updates,
        concept_type: concept_type_for_record,
    };
    let file = s.files.write().insert(rec);

    Ok(Json(UploadResponse {
        file_id: file.id,
        ingested: IngestSummary {
            concepts: stats.concepts,
            relations: stats.relations,
            ontology_updates: stats.ontology_updates,
        },
    }))
}

async fn persist_temp(bytes: &[u8], ext: &str) -> Result<tempfile::NamedTempFile, ApiError> {
    let tmp = tempfile::Builder::new()
        .suffix(&format!(".{ext}"))
        .tempfile()
        .map_err(|e| ApiError::Store(e.to_string()))?;
    tokio::fs::write(tmp.path(), bytes)
        .await
        .map_err(|e| ApiError::Store(e.to_string()))?;
    Ok(tmp)
}

#[derive(Serialize)]
struct UploadResponse {
    file_id: u64,
    ingested: IngestSummary,
}

#[derive(Serialize)]
struct IngestSummary {
    concepts: u64,
    relations: u64,
    ontology_updates: u64,
}

// ---------------------------------------------------------------------------
// New handlers: history / ontology gen / subgraph / export / files / queries
// / settings
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct StatsHistoryResponse {
    samples: Vec<StatsSample>,
}

async fn stats_history(State(s): State<AppState>) -> Json<StatsHistoryResponse> {
    Json(StatsHistoryResponse {
        samples: s.history.read().snapshot(),
    })
}

#[derive(Deserialize)]
struct GenerateOntologyRequest {
    description: String,
}

#[derive(Serialize)]
struct GenerateOntologyResponse {
    ontology: Ontology,
    model: String,
}

/// `POST /ontology/generate` — natural-language → ontology schema. The body
/// is `{description: "…"}`. The configured LLM (set at server start via
/// `--anthropic` / `--openai` / `--deepseek`) renders a strict JSON
/// document; this handler parses it. Malformed JSON surfaces a 422 with
/// the raw response so the UI can show it to the user.
async fn generate_ontology_handler(
    State(s): State<AppState>,
    Json(req): Json<GenerateOntologyRequest>,
) -> Result<Json<GenerateOntologyResponse>, ApiError> {
    if req.description.trim().is_empty() {
        return Err(ApiError::BadRequest("description must not be empty".into()));
    }
    let onto = s
        .pipeline
        .generate_ontology(&req.description)
        .await
        .map_err(|e| match e {
            OntologyGenError::Llm(e) => ApiError::Llm(e.to_string()),
            OntologyGenError::Parse { raw, error } => {
                ApiError::Unprocessable(format!("ontology JSON parse failed: {error}\nraw:\n{raw}"))
            }
        })?;
    Ok(Json(GenerateOntologyResponse {
        ontology: onto,
        model: "configured-llm".into(),
    }))
}

/// `PUT /ontology` — replace the ontology schema in place. The request body
/// is the full [`Ontology`] JSON. Useful after `/ontology/generate` accepts
/// the LLM output. Concepts and relations already in the graph are *not*
/// modified — validation will trip on any future edges incompatible with
/// the new schema.
async fn put_ontology(
    State(s): State<AppState>,
    Json(onto): Json<Ontology>,
) -> Result<Json<Ontology>, ApiError> {
    s.graph.extend_ontology(|target| {
        *target = onto.clone();
        Ok(())
    })?;
    s.store
        .append(&LogRecord::ontology(onto.clone()))
        .await
        .map_err(|e| ApiError::Store(e.to_string()))?;
    Ok(Json(onto))
}

#[derive(Deserialize, Default)]
struct SubgraphRequest {
    #[serde(default)]
    seed_concept_ids: Vec<u64>,
    #[serde(default)]
    seed_query: Option<String>,
    #[serde(default)]
    seed_concept_types: Vec<String>,
    #[serde(default = "default_subgraph_limit")]
    limit: usize,
    #[serde(default = "default_query_depth")]
    expansion_depth: u32,
}

fn default_subgraph_limit() -> usize {
    200
}

#[derive(Serialize)]
struct SubgraphResponse {
    subgraph: Subgraph,
}

/// `POST /subgraph` — fetch a bounded subgraph for the Graph View page.
///
/// Seeds can be supplied three ways (any combination):
/// * `seed_concept_ids` — explicit ConceptId list,
/// * `seed_query` — runs hybrid retrieval to find seeds (top-k = 8),
/// * `seed_concept_types` — pulls every concept of the given types
///   (capped at `limit`).
///
/// If no seeds are supplied, returns the first `limit` concepts in the
/// graph so the Graph View has something to render on first load.
async fn subgraph_handler(
    State(s): State<AppState>,
    Json(req): Json<SubgraphRequest>,
) -> Json<SubgraphResponse> {
    let limit = req.limit.clamp(1, 2_000);

    // 1. Collect seed concept ids.
    let mut seeds: Vec<ConceptId> = req
        .seed_concept_ids
        .into_iter()
        .map(ConceptId)
        .collect();

    if let Some(q) = req.seed_query.as_ref().filter(|q| !q.trim().is_empty()) {
        let req = RetrievalRequest {
            query: q.clone(),
            top_k: 8,
            lexical_weight: 0.5,
            concept_types: req.seed_concept_types.clone(),
            expansion: TraversalSpec {
                max_depth: 0,
                max_nodes: 8,
                ..Default::default()
            },
        };
        let (scored, _) = s.index.retrieve(&req);
        for sc in scored {
            if !seeds.contains(&sc.id) {
                seeds.push(sc.id);
            }
        }
    }

    if seeds.is_empty() {
        for c in s.graph.all_concepts() {
            if !req.seed_concept_types.is_empty()
                && !req.seed_concept_types.iter().any(|t| t == &c.concept_type)
            {
                continue;
            }
            seeds.push(c.id);
            if seeds.len() >= limit {
                break;
            }
        }
    }

    let spec = TraversalSpec {
        max_depth: req.expansion_depth,
        concept_types: req.seed_concept_types,
        max_nodes: limit,
        ..Default::default()
    };
    let subgraph = s.graph.expand(&seeds, &spec);
    Json(SubgraphResponse { subgraph })
}

/// `GET /export?format=jsonl` — stream the entire graph as newline-
/// delimited JSON records (`Ontology`, then every `Concept`, then every
/// `Relation`). Round-trips through `/upload kind=jsonl`. The response is
/// returned as `application/x-ndjson` so curl / fetch can dump it
/// straight to a file.
#[derive(Deserialize)]
struct ExportQuery {
    #[serde(default = "default_export_format")]
    format: String,
}

fn default_export_format() -> String {
    "jsonl".into()
}

async fn export_handler(
    State(s): State<AppState>,
    Query(q): Query<ExportQuery>,
) -> Result<Response, ApiError> {
    match q.format.as_str() {
        "jsonl" | "ndjson" => {
            // Write through a tempfile so we reuse the existing `JsonlSink`
            // adapter without duplicating its formatting logic.
            let tmp = tempfile::Builder::new()
                .suffix(".jsonl")
                .tempfile()
                .map_err(|e| ApiError::Store(e.to_string()))?;
            let mut sink = JsonlSink::create(tmp.path())
                .await
                .map_err(|e| ApiError::Store(e.to_string()))?;
            export_graph(&s.graph, &mut sink)
                .await
                .map_err(|e| ApiError::Store(e.to_string()))?;
            let bytes = tokio::fs::read(tmp.path())
                .await
                .map_err(|e| ApiError::Store(e.to_string()))?;
            let mut resp = (
                StatusCode::OK,
                [(
                    axum::http::header::CONTENT_TYPE.to_string(),
                    "application/x-ndjson".to_string(),
                )],
                bytes,
            )
                .into_response();
            resp.headers_mut().insert(
                axum::http::header::CONTENT_DISPOSITION,
                HeaderValue::from_static("attachment; filename=\"ontology.jsonl\""),
            );
            Ok(resp)
        }
        "json" => {
            // Compact JSON snapshot: ontology + concepts + relations.
            let body = serde_json::json!({
                "ontology": s.graph.ontology(),
                "concepts": s.graph.all_concepts(),
                "relations": s
                    .graph
                    .all_concepts()
                    .iter()
                    .flat_map(|c| s.graph.outgoing(c.id))
                    .collect::<Vec<_>>(),
            });
            Ok(Json(body).into_response())
        }
        other => Err(ApiError::BadRequest(format!("unknown format: {other}"))),
    }
}

// ---- Files registry --------------------------------------------------------

#[derive(Serialize)]
struct ListFilesResponse {
    files: Vec<FileRecord>,
}

async fn list_files(State(s): State<AppState>) -> Json<ListFilesResponse> {
    Json(ListFilesResponse {
        files: s.files.read().list(),
    })
}

async fn get_file(
    State(s): State<AppState>,
    Path(id): Path<u64>,
) -> Result<Json<FileRecord>, ApiError> {
    s.files
        .read()
        .get(id)
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("file {id}")))
}

async fn delete_file(
    State(s): State<AppState>,
    Path(id): Path<u64>,
) -> Result<StatusCode, ApiError> {
    if s.files.write().remove(id) {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound(format!("file {id}")))
    }
}

// ---- Saved queries ---------------------------------------------------------

#[derive(Deserialize)]
struct CreateQueryRequest {
    name: String,
    query: String,
    #[serde(default = "default_query_top_k")]
    top_k: usize,
    #[serde(default = "default_query_lex_w")]
    lexical_weight: f32,
    #[serde(default)]
    concept_types: Vec<String>,
    #[serde(default = "default_query_depth")]
    expansion_depth: u32,
}

#[derive(Serialize)]
struct ListQueriesResponse {
    queries: Vec<SavedQuery>,
}

async fn list_queries(State(s): State<AppState>) -> Json<ListQueriesResponse> {
    Json(ListQueriesResponse {
        queries: s.queries.read().list(),
    })
}

async fn create_query(
    State(s): State<AppState>,
    Json(req): Json<CreateQueryRequest>,
) -> Result<Json<SavedQuery>, ApiError> {
    if req.name.trim().is_empty() {
        return Err(ApiError::BadRequest("query name is required".into()));
    }
    let q = SavedQuery {
        id: 0,
        name: req.name,
        query: req.query,
        top_k: req.top_k,
        lexical_weight: req.lexical_weight,
        concept_types: req.concept_types,
        expansion_depth: req.expansion_depth,
        created_at: now_ts(),
        last_run_at: None,
    };
    Ok(Json(s.queries.write().insert(q)))
}

async fn get_query(
    State(s): State<AppState>,
    Path(id): Path<u64>,
) -> Result<Json<SavedQuery>, ApiError> {
    s.queries
        .read()
        .get(id)
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("query {id}")))
}

async fn update_query(
    State(s): State<AppState>,
    Path(id): Path<u64>,
    Json(patch): Json<SavedQueryPatch>,
) -> Result<Json<SavedQuery>, ApiError> {
    s.queries
        .write()
        .update(id, patch)
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("query {id}")))
}

async fn delete_query(
    State(s): State<AppState>,
    Path(id): Path<u64>,
) -> Result<StatusCode, ApiError> {
    if s.queries.write().remove(id) {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound(format!("query {id}")))
    }
}

/// `POST /queries/:id/run` — execute the saved query through the same
/// RAG pipeline `/ask` uses and return the full answer. Updates the
/// `last_run_at` timestamp as a side effect.
async fn run_query(
    State(s): State<AppState>,
    Path(id): Path<u64>,
) -> Result<Json<RagAnswer>, ApiError> {
    let q = s
        .queries
        .read()
        .get(id)
        .ok_or_else(|| ApiError::NotFound(format!("query {id}")))?;
    let req = RetrievalRequest {
        query: q.query.clone(),
        top_k: q.top_k,
        lexical_weight: q.lexical_weight,
        concept_types: q.concept_types.clone(),
        expansion: TraversalSpec {
            max_depth: q.expansion_depth,
            ..Default::default()
        },
    };
    let ans = s
        .pipeline
        .answer_with(req)
        .await
        .map_err(|e| ApiError::Llm(e.to_string()))?;
    s.queries.write().touch_run(id);
    Ok(Json(ans))
}

// ---- Settings --------------------------------------------------------------

async fn get_settings(State(s): State<AppState>) -> Json<Settings> {
    Json(s.settings.read().clone())
}

async fn patch_settings(
    State(s): State<AppState>,
    Json(patch): Json<SettingsPatch>,
) -> Json<Settings> {
    let mut current = s.settings.write();
    if let Some(r) = patch.retrieval {
        if let Some(v) = r.top_k {
            current.retrieval.top_k = v;
        }
        if let Some(v) = r.lexical_weight {
            current.retrieval.lexical_weight = v;
        }
        if let Some(v) = r.expansion_depth {
            current.retrieval.expansion_depth = v;
        }
    }
    if let Some(u) = patch.ui {
        if let Some(v) = u.theme {
            current.ui.theme = v;
        }
        if let Some(v) = u.graph_layout {
            current.ui.graph_layout = v;
        }
    }
    Json(current.clone())
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("graph: {0}")]
    Graph(#[from] ontology_graph::GraphError),
    #[error("store: {0}")]
    Store(String),
    #[error("llm: {0}")]
    Llm(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("unprocessable: {0}")]
    Unprocessable(String),
}

#[derive(Serialize, Deserialize)]
struct ErrorBody {
    error: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self {
            ApiError::Graph(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            ApiError::Store(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            ApiError::Llm(_) => (StatusCode::BAD_GATEWAY, self.to_string()),
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            ApiError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            ApiError::Unprocessable(_) => (StatusCode::UNPROCESSABLE_ENTITY, self.to_string()),
        };
        (status, Json(ErrorBody { error: msg })).into_response()
    }
}
