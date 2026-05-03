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

use axum::{
    extract::{Multipart, Path, State},
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
    Concept, ConceptId, ConceptPatch, Ontology, OntologyGraph, Path as GraphPath, Relation,
    RelationId,
};
use ontology_index::{HybridIndex, RetrievalRequest, ScoredConcept};
use ontology_io::{
    ingest_records, CsvSource, IngestStats, JsonlSource, TextDocumentSource, TripleSource,
    XlsxSource,
};
use ontology_rag::{RagAnswer, RagPipeline, RagStreamEvent};
use ontology_storage::{LogRecord, Store};
use parking_lot::Mutex as PlMutex;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::Arc as StdArc;
use std::sync::Arc;
use std::time::{Duration, Instant};
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
}

/// Tunables for the optional rate limiter / request-id middleware.
#[derive(Debug, Clone)]
pub struct RouterConfig {
    /// Bearer token gate. Routes other than `/healthz` require
    /// `Authorization: Bearer <token>` when set.
    pub bearer_token: Option<String>,
    /// Per-IP request limit. `None` disables rate limiting.
    pub rate_limit: Option<RateLimit>,
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
            rate_limit: None,
        },
    )
}

/// Full-featured constructor. Adds (in this order, outer-to-inner):
/// 1. request-id injection (always on),
/// 2. optional rate-limit by client IP,
/// 3. optional bearer-token check.
pub fn build_router_with_config(state: AppState, cfg: RouterConfig) -> Router {
    build_router_inner(state, cfg.bearer_token, cfg.rate_limit)
}

fn build_router_inner(
    state: AppState,
    bearer_token: Option<String>,
    rate_limit: Option<RateLimit>,
) -> Router {
    let healthz_router = Router::new().route("/healthz", get(healthz));

    let protected = Router::new()
        .route("/stats", get(stats))
        .route("/metrics", get(metrics))
        .route("/concepts", post(create_concept))
        .route(
            "/concepts/:id",
            get(get_concept)
                .patch(update_concept)
                .delete(delete_concept),
        )
        .route("/relations", post(create_relation))
        .route("/retrieve", post(retrieve))
        .route("/ask", post(ask))
        .route("/ask/stream", post(ask_stream))
        .route("/path", post(path))
        .route("/compact", post(compact))
        .route("/upload", post(upload))
        .with_state(state);

    let protected = match bearer_token {
        Some(token) => {
            let token = StdArc::new(token);
            protected.layer(middleware::from_fn(move |req, next| {
                let token = token.clone();
                async move { require_bearer(req, next, token).await }
            }))
        }
        None => protected,
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

async fn require_bearer(
    req: Request<axum::body::Body>,
    next: Next,
    expected: StdArc<String>,
) -> Result<Response, StatusCode> {
    let header = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v: &HeaderValue| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());

    match header {
        Some(provided) => {
            // Constant-time compare to avoid leaking the token via timing.
            let a = provided.as_bytes();
            let b = expected.as_bytes();
            let eq = a.len() == b.len()
                && a.iter()
                    .zip(b.iter())
                    .fold(0u8, |acc, (x, y)| acc | (x ^ y))
                    == 0;
            if eq {
                Ok(next.run(req).await)
            } else {
                Err(StatusCode::UNAUTHORIZED)
            }
        }
        None => Err(StatusCode::UNAUTHORIZED),
    }
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
    concept_types: usize,
    relation_types: usize,
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
         ontology_relation_types {}\n",
        s.graph.concept_count(),
        s.graph.relation_count(),
        onto.concept_types.len(),
        onto.relation_types.len(),
    );
    (
        [(
            axum::http::header::CONTENT_TYPE.to_string(),
            "text/plain; version=0.0.4".to_string(),
        )],
        body,
    )
}

async fn stats(State(s): State<AppState>) -> Json<Stats> {
    let onto = s.graph.ontology();
    Json(Stats {
        concepts: s.graph.concept_count(),
        relations: s.graph.relation_count(),
        concept_types: onto.concept_types.len(),
        relation_types: onto.relation_types.len(),
    })
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

    Ok(Json(UploadResponse {
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
    ingested: IngestSummary,
}

#[derive(Serialize)]
struct IngestSummary {
    concepts: u64,
    relations: u64,
    ontology_updates: u64,
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
        };
        (status, Json(ErrorBody { error: msg })).into_response()
    }
}
