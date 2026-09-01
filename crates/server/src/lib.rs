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

mod ingest_review;
mod openapi;

use axum::{
    extract::{Multipart, Path, Query, State},
    http::{header, HeaderMap, HeaderValue, Request, StatusCode},
    middleware::{self, Next},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{delete, get, post},
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
use ontology_rag::{
    infomaniak_base_url, v1_api_url, AnthropicModel, GeneratedRule, LanguageModel, LlmError,
    LlmRequest, LlmResponse, LlmStream, OntologyGenError, OpenAiModel, RagAnswer, RagPipeline,
    RagStreamEvent,
};
use ontology_storage::{LogRecord, Store};
use parking_lot::{Mutex as PlMutex, RwLock as PlRwLock};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::convert::Infallible;
use std::path::PathBuf;
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
    /// User-submitted feedback (bug reports, suggestions, …). In-memory only.
    pub feedbacks: Arc<PlRwLock<FeedbackStore>>,
    /// Server bind time — used by `/settings` for "uptime" display.
    pub started_at: SystemTime,
    /// Optional path to a JSON file used to persist `settings` across
    /// restarts. When `None`, settings remain in-memory only.
    pub settings_path: Option<PathBuf>,
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
            feedbacks: Arc::new(PlRwLock::new(FeedbackStore::default())),
            started_at: SystemTime::now(),
            settings_path: None,
        }
    }

    /// Assemble the state so the pipeline can see the live settings store.
    ///
    /// `build_pipeline` receives the very handle the `/settings` handlers
    /// write to, which is what lets a [`SettingsRoutedModel`] inside the
    /// pipeline pick up an applied provider change with no restart. The
    /// settings file, when given, is loaded *before* `build_pipeline` runs, so
    /// the first request already uses the persisted provider.
    ///
    /// Prefer this over `new().with_settings_path(..)` whenever the pipeline
    /// needs the settings: it makes the only correct ordering the only
    /// available one.
    pub fn assemble(
        graph: Arc<OntologyGraph>,
        index: Arc<HybridIndex>,
        store: Arc<dyn Store>,
        settings_path: Option<PathBuf>,
        build_pipeline: impl FnOnce(Arc<PlRwLock<Settings>>) -> Arc<RagPipeline>,
    ) -> Self {
        let settings = Arc::new(PlRwLock::new(Settings::default()));
        if let Some(path) = settings_path.as_deref() {
            load_settings_into(&settings, path);
        }
        let pipeline = build_pipeline(settings.clone());
        let mut state = Self::new(graph, index, store, pipeline);
        state.settings = settings;
        state.settings_path = settings_path;
        state
    }

    /// Configure a JSON file to persist `settings` across restarts. If the
    /// file exists, its contents are loaded into the in-memory store; the
    /// path is then remembered so subsequent `PATCH /settings` calls write
    /// back to it. Failures are logged and swallowed — persistence is
    /// best-effort.
    pub fn with_settings_path(mut self, path: PathBuf) -> Self {
        load_settings_into(&self.settings, &path);
        self.settings_path = Some(path);
        self
    }
}

/// Read and migrate a settings file, or `None` when nothing usable is there.
///
/// A missing, unreadable or unparseable file is logged and swallowed: a bad
/// settings file must never stop the server (or a CLI command) from starting.
fn load_settings(path: &std::path::Path) -> Option<Settings> {
    match std::fs::read_to_string(path) {
        Ok(raw) => match serde_json::from_str::<Settings>(&raw) {
            Ok(mut loaded) => {
                loaded.migrate();
                tracing::info!(path = %path.display(), "settings loaded");
                Some(loaded)
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %path.display(),
                    "failed to parse settings file; using defaults"
                );
                None
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::info!(path = %path.display(), "no settings file yet; using defaults");
            None
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = %path.display(),
                "failed to read settings file; using defaults"
            );
            None
        }
    }
}

fn load_settings_into(settings: &Arc<PlRwLock<Settings>>, path: &std::path::Path) {
    if let Some(loaded) = load_settings(path) {
        *settings.write() = loaded;
    }
}

/// Persist the given settings to disk as JSON, secrets included — with no
/// environment variables left in the picture, this file *is* the credential
/// store. `Settings` now serializes completely and redaction happens only at
/// the HTTP boundary ([`settings_view`]), so a secret field added later can
/// no longer be silently dropped on restart.
///
/// On Unix the file is created with mode 0600. An already-existing file keeps
/// its current mode, and Windows ACLs are not touched — the file inherits the
/// data directory, so keep that directory out of shared locations.
fn persist_settings(path: &std::path::Path, s: &Settings) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(s).map_err(std::io::Error::other)?;
    write_private(path, json.as_bytes())
}

#[cfg(unix)]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

/// Field-name suffixes that mark a value as secret.
///
/// Redaction is pattern-based on purpose: a secret added to [`Settings`]
/// later is masked at the HTTP boundary without anyone having to remember to
/// update a list. Note that `max_tokens` and the `*_api_key_hint` fields do
/// not match — they are meant to reach the client.
const SECRET_FIELD_SUFFIXES: [&str; 3] = ["_api_key", "_secret", "_token"];

fn is_secret_field(name: &str) -> bool {
    SECRET_FIELD_SUFFIXES
        .iter()
        .any(|suffix| name.ends_with(suffix))
}

/// Recursively drop secret-looking fields from a JSON tree.
fn redact_secrets(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(map) => {
            map.retain(|k, _| !is_secret_field(k));
            for (_, child) in map.iter_mut() {
                redact_secrets(child);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                redact_secrets(item);
            }
        }
        _ => {}
    }
}

/// [`Settings`] as the API returns them: the whole struct minus every secret
/// field. Raw keys never leave the process; the UI works off the
/// `*_api_key_hint` previews instead.
fn settings_view(s: &Settings) -> serde_json::Value {
    let mut v = serde_json::to_value(s).unwrap_or(serde_json::Value::Null);
    redact_secrets(&mut v);
    v
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

/// User-facing settings, patchable at runtime and persisted to disk.
///
/// `#[serde(default)]` on every settings struct is load-bearing: it makes a
/// file written by an older build (missing fields added since) deserialize
/// instead of being rejected wholesale, which would drop the user back to
/// defaults and wipe their stored credentials on upgrade.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub retrieval: RetrievalDefaults,
    pub ui: UiPrefs,
    pub llm: LlmSettings,
    pub ocr: OcrSettings,
}

/// OCR engine configuration. The Google Cloud Vision API key is write-only
/// over the wire; `google_api_key_hint` exposes a masked preview.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OcrSettings {
    /// `"tesseract" | "google_vision"`.
    pub provider: String,
    /// If true, fall back to the other engine when the primary returns
    /// fewer than `min_text_threshold` characters or fails.
    pub auto_fallback: bool,
    /// Minimum number of OCR characters below which the fallback engine is
    /// triggered.
    pub min_text_threshold: u32,
    /// Tesseract language pack expression, e.g. `"fra+deu+eng"`.
    pub tesseract_languages: String,

    /// Serialized so the credential survives a restart; stripped from HTTP
    /// responses by [`settings_view`].
    pub google_api_key: String,
    pub google_api_key_hint: String,
}

impl Default for OcrSettings {
    fn default() -> Self {
        Self {
            provider: "tesseract".into(),
            auto_fallback: true,
            min_text_threshold: 50,
            tesseract_languages: "fra+deu+eng".into(),
            google_api_key: String::new(),
            google_api_key_hint: String::new(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct OcrSettingsPatch {
    pub provider: Option<String>,
    pub auto_fallback: Option<bool>,
    pub min_text_threshold: Option<u32>,
    pub tesseract_languages: Option<String>,
    pub google_api_key: Option<String>,
}

/// LLM provider configuration — the single source of truth for provider
/// credentials, entered through `PATCH /settings` and persisted to disk. No
/// environment variable feeds these fields.
///
/// Keys are write-only over the wire: they serialize (so they survive a
/// restart) but [`settings_view`] strips them from every HTTP response. The
/// matching `*_api_key_hint` exposes the first 3 + last 4 chars so the UI can
/// show "sk-...1XAA" without round-tripping the secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmSettings {
    /// `"default" | "openai" | "anthropic" | "infomaniak"`.
    pub active_provider: String,

    pub openai_api_key: String,
    pub openai_api_key_hint: String,
    pub openai_base_url: String,
    pub openai_model: String,

    pub anthropic_api_key: String,
    pub anthropic_api_key_hint: String,
    pub anthropic_base_url: String,
    pub anthropic_model: String,

    pub infomaniak_api_key: String,
    pub infomaniak_api_key_hint: String,
    /// AI Tools product id, from `GET https://api.infomaniak.com/1/ai`. The
    /// OpenAI-compatible base URL is derived from it, so this is the only
    /// Infomaniak-specific value a user supplies beyond the token.
    pub infomaniak_product_id: String,
    /// Advanced override for the derived base URL. Empty means "derive from
    /// `infomaniak_product_id`", which is what a normal setup wants.
    pub infomaniak_base_url: String,
    pub infomaniak_model: String,

    pub temperature: f32,
    pub max_tokens: u32,
}

impl Default for LlmSettings {
    fn default() -> Self {
        Self {
            active_provider: "default".into(),
            openai_api_key: String::new(),
            openai_api_key_hint: String::new(),
            openai_base_url: String::new(),
            openai_model: "gpt-4o".into(),
            anthropic_api_key: String::new(),
            anthropic_api_key_hint: String::new(),
            anthropic_base_url: String::new(),
            anthropic_model: "claude-sonnet-4-6".into(),
            infomaniak_api_key: String::new(),
            infomaniak_api_key_hint: String::new(),
            infomaniak_product_id: String::new(),
            infomaniak_base_url: String::new(),
            infomaniak_model: String::new(),
            temperature: 0.3,
            max_tokens: 1000,
        }
    }
}

impl LlmSettings {
    /// Recompute the masked hints from the live keys. Run after loading a
    /// settings file so a hand-edited key still reads as "configured" in the
    /// UI instead of showing a stale or blank hint.
    fn refresh_key_hints(&mut self) {
        self.openai_api_key_hint = key_hint(&self.openai_api_key);
        self.anthropic_api_key_hint = key_hint(&self.anthropic_api_key);
        self.infomaniak_api_key_hint = key_hint(&self.infomaniak_api_key);
    }
}

impl OcrSettings {
    fn refresh_key_hints(&mut self) {
        self.google_api_key_hint = key_hint(&self.google_api_key);
    }
}

impl Settings {
    /// Bring a settings file written by an older build up to date.
    ///
    /// Builds before Infomaniak became product-scoped stored
    /// `infomaniak_base_url = "https://api.infomaniak.com/1/ai"`. That URL
    /// cannot serve chat completions — the real root is
    /// `/2/ai/{product_id}/openai/v1` — and, being an explicit override, it
    /// would keep shadowing the derived URL forever. Clearing it hands
    /// control back to `infomaniak_product_id`. Only that exact value is
    /// touched: anything a user typed themselves is left alone.
    pub fn migrate(&mut self) {
        const DEAD_INFOMANIAK_BASE: &str = "https://api.infomaniak.com/1/ai";
        let base = self.llm.infomaniak_base_url.trim().trim_end_matches('/');
        if base == DEAD_INFOMANIAK_BASE {
            tracing::info!(
                "clearing obsolete infomaniak_base_url; it is now derived from the product id"
            );
            self.llm.infomaniak_base_url.clear();
        }
        self.llm.refresh_key_hints();
        self.ocr.refresh_key_hints();
    }

    /// Read a settings file, falling back to defaults when it is absent or
    /// unusable — the same tolerance the server applies at startup, so a CLI
    /// command and the server always agree on what the stored config is.
    pub fn load_or_default(path: &std::path::Path) -> Self {
        load_settings(path).unwrap_or_default()
    }
}

fn key_hint(key: &str) -> String {
    let k = key.trim();
    if k.is_empty() {
        return String::new();
    }
    let prefix = k.chars().take(3).collect::<String>();
    let suffix: String = k.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("{prefix}...{suffix}")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RetrievalDefaults {
    pub top_k: usize,
    pub lexical_weight: f32,
    pub expansion_depth: u32,
}

impl Default for RetrievalDefaults {
    fn default() -> Self {
        Self {
            top_k: 8,
            lexical_weight: 0.5,
            expansion_depth: 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiPrefs {
    pub theme: String,
    pub graph_layout: String,
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            theme: "light".into(),
            graph_layout: "dagre".into(),
        }
    }
}

/// PATCH-style settings update. Every field is optional; missing fields are
/// preserved.
#[derive(Debug, Default, Deserialize)]
pub struct SettingsPatch {
    pub retrieval: Option<RetrievalDefaultsPatch>,
    pub ui: Option<UiPrefsPatch>,
    pub llm: Option<LlmSettingsPatch>,
    pub ocr: Option<OcrSettingsPatch>,
}

#[derive(Debug, Default, Deserialize)]
pub struct LlmSettingsPatch {
    pub active_provider: Option<String>,
    pub openai_api_key: Option<String>,
    pub openai_base_url: Option<String>,
    pub openai_model: Option<String>,
    pub anthropic_api_key: Option<String>,
    pub anthropic_base_url: Option<String>,
    pub anthropic_model: Option<String>,
    pub infomaniak_api_key: Option<String>,
    pub infomaniak_product_id: Option<String>,
    pub infomaniak_base_url: Option<String>,
    pub infomaniak_model: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
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

// ---------------------------------------------------------------------------
// Feedback (in-memory store)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Feedback {
    pub id: u64,
    pub created_at: u64,
    /// `"bug" | "error" | "evolution" | "improvement"`.
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// Optional screenshot as a `data:image/png;base64,…` URL.
    #[serde(default)]
    pub screenshot: Option<String>,
    /// Browser-side console log dump (newest first), already truncated client-side.
    #[serde(default)]
    pub frontend_logs: String,
    /// Server-side log tail at submission time.
    #[serde(default)]
    pub backend_logs: String,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub reporter_email: Option<String>,
}

#[derive(Debug, Default)]
pub struct FeedbackStore {
    next_id: u64,
    records: Vec<Feedback>,
}

impl FeedbackStore {
    fn insert(&mut self, mut f: Feedback) -> Feedback {
        self.next_id += 1;
        f.id = self.next_id;
        f.created_at = now_ts();
        self.records.push(f.clone());
        f
    }
    fn list(&self) -> Vec<Feedback> {
        let mut v = self.records.clone();
        v.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        v
    }
    fn remove(&mut self, id: u64) -> bool {
        let len = self.records.len();
        self.records.retain(|f| f.id != id);
        self.records.len() != len
    }
}

// ---------------------------------------------------------------------------
// Recent-logs ring buffer (used by /logs/tail for feedback bundles)
// ---------------------------------------------------------------------------

const RECENT_LOGS_CAPACITY: usize = 500;
static RECENT_LOGS: PlMutex<VecDeque<String>> = PlMutex::new(VecDeque::new());

/// Append a one-line log entry to the recent-logs ring buffer. Called from
/// the access-log middleware so `/logs/tail` always has request history; can
/// also be called by external tracing layers.
pub fn push_recent_log(line: impl Into<String>) {
    let mut q = RECENT_LOGS.lock();
    if q.len() >= RECENT_LOGS_CAPACITY {
        q.pop_front();
    }
    q.push_back(line.into());
}

fn snapshot_recent_logs(limit: usize) -> Vec<String> {
    let q = RECENT_LOGS.lock();
    let n = q.len().min(limit);
    q.iter().skip(q.len() - n).cloned().collect()
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
        .route("/rules/generate", post(generate_rule_handler))
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
        .route("/ingest/analyze", post(ingest_review::analyze))
        .route("/ingest/apply", post(ingest_review::apply))
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
        .route("/settings/llm/test", post(test_llm_connection))
        .route(
            "/settings/llm/models",
            get(list_llm_models).post(list_llm_models_with_overrides),
        )
        .route(
            "/settings/llm/infomaniak/products",
            post(list_infomaniak_products),
        )
        .route("/settings/ocr/status", get(get_ocr_status))
        .route("/feedbacks", get(list_feedbacks).post(create_feedback))
        .route("/feedbacks/:id", delete(delete_feedback))
        .route("/logs/tail", get(logs_tail))
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

    // Access log — captured before the body is consumed downstream.
    let method = req.method().clone();
    let uri = req.uri().clone();
    let path = uri.path().to_string();
    let query = uri.query().map(|q| q.to_string());
    let started = std::time::Instant::now();
    // Healthz and metrics scrapes are noisy — log them at trace level only.
    let noisy = matches!(path.as_str(), "/healthz" | "/metrics");
    if !noisy {
        match &query {
            Some(q) => tracing::info!(
                target: "http",
                req_id = %id,
                method = %method,
                path = %path,
                query = %q,
                "→ request"
            ),
            None => tracing::info!(
                target: "http",
                req_id = %id,
                method = %method,
                path = %path,
                "→ request"
            ),
        }
    }

    let mut resp = next.run(req).await;
    if let Ok(value) = HeaderValue::from_str(&id) {
        resp.headers_mut().insert("x-request-id", value);
    }

    let status = resp.status();
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let code = status.as_u16();
    if !noisy {
        let q = query.as_deref().map(|s| format!("?{s}")).unwrap_or_default();
        push_recent_log(format!(
            "{} {} {}{} {} {}ms",
            now_ts(),
            method,
            path,
            q,
            code,
            elapsed_ms
        ));
    }
    if code >= 500 {
        tracing::error!(
            target: "http",
            req_id = %id,
            method = %method,
            path = %path,
            status = code,
            elapsed_ms = elapsed_ms,
            "← response (server error)"
        );
    } else if code >= 400 {
        tracing::warn!(
            target: "http",
            req_id = %id,
            method = %method,
            path = %path,
            status = code,
            elapsed_ms = elapsed_ms,
            "← response (client error)"
        );
    } else if !noisy {
        tracing::info!(
            target: "http",
            req_id = %id,
            method = %method,
            path = %path,
            status = code,
            elapsed_ms = elapsed_ms,
            "← response"
        );
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
    /// When false, stop scanning as soon as the page is full and report
    /// `total = offset + page.len()` (a lower bound). Default true to keep
    /// the legacy exact-count behavior.
    #[serde(default = "default_true")]
    track_total: bool,
    /// When true, a `type=X` filter also returns instances of subtypes of `X`.
    #[serde(default = "default_true")]
    include_subtypes: bool,
}

fn default_true() -> bool {
    true
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
    headers: HeaderMap,
    Query(q): Query<ListConceptsQuery>,
) -> Response {
    // ETag = generation tag. The cache key for the client is the full URL
    // (query string included), so the ETag only needs to vary on the
    // underlying data version: as long as no concept has been written, the
    // representation the client already has for this exact URL is still
    // valid.
    let gen = s.graph.concepts_generation();
    let etag = format!("W/\"c{gen}\"");
    if let Some(if_match) = headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) {
        if if_match == etag {
            return StatusCode::NOT_MODIFIED.into_response();
        }
    }

    let needle = q.q.as_ref().map(|s| s.to_lowercase());
    let limit = q.limit.unwrap_or(200).min(5_000);
    let (total, concepts) = s.graph.list_concepts_page(
        q.concept_type.as_deref(),
        needle.as_deref(),
        q.offset,
        limit,
        q.track_total,
        q.include_subtypes,
    );
    let mut resp = Json(ListConceptsResponse { total, concepts }).into_response();
    if let Ok(v) = HeaderValue::from_str(&etag) {
        resp.headers_mut().insert(header::ETAG, v);
    }
    resp
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
    #[serde(default = "default_true")]
    track_total: bool,
}

#[derive(Serialize)]
struct ListRelationsResponse {
    total: usize,
    relations: Vec<Relation>,
}

async fn list_relations(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ListRelationsQuery>,
) -> Response {
    let gen = s.graph.relations_generation();
    let etag = format!("W/\"r{gen}\"");
    if let Some(if_match) = headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) {
        if if_match == etag {
            return StatusCode::NOT_MODIFIED.into_response();
        }
    }

    let offset = q.offset.unwrap_or(0);
    let limit = q.limit.unwrap_or(100).min(1000);
    let (total, relations) = s.graph.list_relations_page(
        q.source.map(ConceptId),
        q.target.map(ConceptId),
        q.relation_type.as_deref(),
        offset,
        limit,
        q.track_total,
    );
    let mut resp = Json(ListRelationsResponse { total, relations }).into_response();
    if let Ok(v) = HeaderValue::from_str(&etag) {
        resp.headers_mut().insert(header::ETAG, v);
    }
    resp
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
            // Preserve the original extension so binary formats like .docx
            // are routed through their dedicated extractor in
            // TextDocumentSource instead of being decoded as raw bytes.
            let ext = filename
                .as_deref()
                .map(std::path::Path::new)
                .and_then(|p| p.extension().and_then(|e| e.to_str()))
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_else(|| "txt".into());
            let path = dir.path().join(format!("{stem}.{ext}"));
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
/// is `{description: "…"}`. The LLM selected in the settings store renders a
/// strict JSON document; this handler parses it. Malformed JSON surfaces a
/// 422 with the raw response so the UI can show it to the user.
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

#[derive(Deserialize)]
struct GenerateRuleRequest {
    description: String,
    rule_type: String,
    #[serde(default)]
    applies_to: Vec<u64>,
}

#[derive(Serialize)]
struct GenerateRuleResponse {
    name: String,
    when: String,
    then: String,
    description: String,
    strict: bool,
}

/// `POST /rules/generate` — natural-language → rule fields. The caller
/// supplies the prompt, the rule type and the concept ids the rule will
/// scope to. The handler resolves those ids to concept names, invokes the
/// configured LLM, and returns the generated `name`/`when`/`then`/
/// `description`/`strict` fields. Malformed JSON surfaces a 422 with the
/// raw response so the UI can show it to the user.
async fn generate_rule_handler(
    State(s): State<AppState>,
    Json(req): Json<GenerateRuleRequest>,
) -> Result<Json<GenerateRuleResponse>, ApiError> {
    if req.description.trim().is_empty() {
        return Err(ApiError::BadRequest("description must not be empty".into()));
    }
    if req.rule_type.trim().is_empty() {
        return Err(ApiError::BadRequest("rule_type must not be empty".into()));
    }
    if req.applies_to.is_empty() {
        return Err(ApiError::BadRequest(
            "applies_to must contain at least one concept id".into(),
        ));
    }
    let mut concept_names = Vec::with_capacity(req.applies_to.len());
    for cid in &req.applies_to {
        let c = s.graph.get_concept(ConceptId(*cid))?;
        concept_names.push(c.name);
    }
    let generated: GeneratedRule = s
        .pipeline
        .generate_rule(&req.description, &req.rule_type, &concept_names)
        .await
        .map_err(|e| match e {
            OntologyGenError::Llm(e) => ApiError::Llm(e.to_string()),
            OntologyGenError::Parse { raw, error } => {
                ApiError::Unprocessable(format!("rule JSON parse failed: {error}\nraw:\n{raw}"))
            }
        })?;
    Ok(Json(GenerateRuleResponse {
        name: generated.name,
        when: generated.when,
        then: generated.then,
        description: generated.description,
        strict: generated.strict,
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
        // Use the per-type label index when types are constrained, otherwise
        // pull the first `limit` concepts from the global ordered index. In
        // both cases we never materialize the whole graph.
        if req.seed_concept_types.is_empty() {
            let (_, page) = s
                .graph
                .list_concepts_page(None, None, 0, limit, false, true);
            for c in page {
                seeds.push(c.id);
            }
        } else {
            'outer: for t in &req.seed_concept_types {
                let (_, page) = s
                    .graph
                    .list_concepts_page(Some(t), None, 0, limit - seeds.len(), false, true);
                for c in page {
                    seeds.push(c.id);
                    if seeds.len() >= limit {
                        break 'outer;
                    }
                }
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
                "relations": s.graph.all_relations(),
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

async fn get_settings(State(s): State<AppState>) -> Json<serde_json::Value> {
    Json(settings_view(&s.settings.read()))
}

async fn patch_settings(
    State(s): State<AppState>,
    Json(patch): Json<SettingsPatch>,
) -> Json<serde_json::Value> {
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
    if let Some(l) = patch.llm {
        let llm = &mut current.llm;
        if let Some(v) = l.active_provider {
            llm.active_provider = v;
        }
        if let Some(v) = l.openai_api_key {
            llm.openai_api_key_hint = key_hint(&v);
            llm.openai_api_key = v;
        }
        if let Some(v) = l.openai_base_url {
            llm.openai_base_url = v;
        }
        if let Some(v) = l.openai_model {
            llm.openai_model = v;
        }
        if let Some(v) = l.anthropic_api_key {
            llm.anthropic_api_key_hint = key_hint(&v);
            llm.anthropic_api_key = v;
        }
        if let Some(v) = l.anthropic_base_url {
            llm.anthropic_base_url = v;
        }
        if let Some(v) = l.anthropic_model {
            llm.anthropic_model = v;
        }
        if let Some(v) = l.infomaniak_api_key {
            llm.infomaniak_api_key_hint = key_hint(&v);
            llm.infomaniak_api_key = v;
        }
        if let Some(v) = l.infomaniak_product_id {
            llm.infomaniak_product_id = v.trim().to_string();
        }
        if let Some(v) = l.infomaniak_base_url {
            llm.infomaniak_base_url = v.trim().to_string();
        }
        if let Some(v) = l.infomaniak_model {
            llm.infomaniak_model = v;
        }
        if let Some(v) = l.temperature {
            llm.temperature = v;
        }
        if let Some(v) = l.max_tokens {
            llm.max_tokens = v;
        }
    }
    if let Some(o) = patch.ocr {
        let ocr = &mut current.ocr;
        if let Some(v) = o.provider {
            ocr.provider = v;
        }
        if let Some(v) = o.auto_fallback {
            ocr.auto_fallback = v;
        }
        if let Some(v) = o.min_text_threshold {
            ocr.min_text_threshold = v;
        }
        if let Some(v) = o.tesseract_languages {
            ocr.tesseract_languages = v;
        }
        if let Some(v) = o.google_api_key {
            ocr.google_api_key_hint = key_hint(&v);
            ocr.google_api_key = v;
        }
    }
    let snapshot = current.clone();
    drop(current);
    if let Some(path) = s.settings_path.as_ref() {
        if let Err(e) = persist_settings(path, &snapshot) {
            tracing::warn!(error = %e, path = %path.display(), "failed to persist settings");
        }
    }
    Json(settings_view(&snapshot))
}

// ---- OCR engine status ----------------------------------------------------

#[derive(Debug, Serialize)]
struct OcrEngineProbe {
    available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    ocrmypdf_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tesseract_version: Option<String>,
    ghostscript_available: bool,
}

#[derive(Debug, Serialize)]
struct GoogleVisionProbe {
    configured: bool,
    auth: &'static str,
}

#[derive(Debug, Serialize)]
struct OcrStatusResponse {
    tesseract: OcrEngineProbe,
    google_vision: GoogleVisionProbe,
}

fn probe_version(cmd: &str, arg: &str) -> Option<String> {
    let out = std::process::Command::new(cmd).arg(arg).output().ok()?;
    if !out.status.success() && out.stdout.is_empty() && out.stderr.is_empty() {
        return None;
    }
    let combined = if !out.stdout.is_empty() {
        String::from_utf8_lossy(&out.stdout).to_string()
    } else {
        String::from_utf8_lossy(&out.stderr).to_string()
    };
    let first = combined.lines().next().unwrap_or("").trim().to_string();
    // Strip a leading binary name to keep just the version token.
    let v = first
        .split_whitespace()
        .find(|t| t.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false))
        .map(|s| s.to_string())
        .unwrap_or(first);
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

fn probe_ghostscript() -> bool {
    let candidates = if cfg!(windows) {
        &["gswin64c", "gswin32c", "gs"][..]
    } else {
        &["gs"][..]
    };
    candidates
        .iter()
        .any(|c| probe_version(c, "--version").is_some())
}

async fn get_ocr_status(State(s): State<AppState>) -> Json<OcrStatusResponse> {
    let ocrmypdf_version = probe_version("ocrmypdf", "--version");
    let tesseract_version = probe_version("tesseract", "--version");
    let ghostscript_available = probe_ghostscript();
    let tesseract_available = tesseract_version.is_some();
    let google_configured = !s.settings.read().ocr.google_api_key.is_empty();
    Json(OcrStatusResponse {
        tesseract: OcrEngineProbe {
            available: tesseract_available,
            ocrmypdf_version,
            tesseract_version,
            ghostscript_available,
        },
        google_vision: GoogleVisionProbe {
            configured: google_configured,
            auth: "API key",
        },
    })
}

// ---- LLM provider resolution, connection test & model discovery ----------

/// Wire dialect spoken by a provider endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderKind {
    /// OpenAI `chat/completions` shape — OpenAI, Infomaniak AI Tools, and any
    /// other OpenAI-compatible gateway.
    OpenAiCompatible,
    /// Anthropic Messages API.
    Anthropic,
}

/// Everything needed to reach one provider, after the stored settings and the
/// per-request overrides have been merged.
#[derive(Debug, Clone)]
struct ProviderCreds {
    api_key: String,
    /// API root, already product-scoped for Infomaniak. Combine with
    /// [`v1_api_url`] to build a concrete endpoint.
    base_url: String,
    /// Selected model. Empty is legal here — listing the catalogue is how a
    /// user finds a model to select — but not in [`model_for`].
    model: String,
    kind: ProviderKind,
    /// Resolved provider name, for responses and error messages.
    provider: String,
}

impl ProviderCreds {
    /// Hash of the fields that shape a client, so [`SettingsRoutedModel`] can
    /// tell "same configuration" from "reconfigured" without keeping its own
    /// copy of the API key around.
    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.provider.hash(&mut h);
        self.base_url.hash(&mut h);
        self.model.hash(&mut h);
        self.api_key.hash(&mut h);
        h.finish()
    }
}

/// Per-request provider overrides, merged over the stored settings and
/// **never persisted**.
///
/// This is what lets the UI test a key, discover a product id or list models
/// before the user commits to saving anything; without it the frontend had to
/// PATCH a half-filled form first, quietly making unvalidated config live.
#[derive(Debug, Default, Deserialize)]
pub struct LlmOverrides {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub product_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

/// Provider actually in force: an explicit override wins over
/// `settings.llm.active_provider`, and both fall back to `"default"` — the
/// LLM the server booted with.
pub(crate) fn effective_provider(llm: &LlmSettings, ov: &LlmOverrides) -> String {
    let explicit = ov.provider.as_deref().unwrap_or_default().trim();
    if !explicit.is_empty() && explicit != "default" {
        return explicit.to_string();
    }
    let active = llm.active_provider.trim();
    if active.is_empty() {
        "default".to_string()
    } else {
        active.to_string()
    }
}

/// Take the override when it carries a value, else the stored setting. Both
/// sides are trimmed, so a field holding only spaces counts as unset.
fn pick(stored: &str, override_value: &Option<String>) -> String {
    override_value
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(stored.trim())
        .to_string()
}

fn resolve_provider_creds(llm: &LlmSettings, ov: &LlmOverrides) -> Result<ProviderCreds, String> {
    let provider = effective_provider(llm, ov);
    match provider.as_str() {
        "openai" => {
            let api_key = pick(&llm.openai_api_key, &ov.api_key);
            if api_key.is_empty() {
                return Err("Clé API OpenAI manquante — saisissez-la dans Settings.".into());
            }
            let mut base_url = pick(&llm.openai_base_url, &ov.base_url);
            if base_url.is_empty() {
                base_url = "https://api.openai.com".to_string();
            }
            Ok(ProviderCreds {
                api_key,
                base_url,
                model: pick(&llm.openai_model, &ov.model),
                kind: ProviderKind::OpenAiCompatible,
                provider,
            })
        }
        "anthropic" => {
            let api_key = pick(&llm.anthropic_api_key, &ov.api_key);
            if api_key.is_empty() {
                return Err("Clé API Anthropic manquante — saisissez-la dans Settings.".into());
            }
            let mut base_url = pick(&llm.anthropic_base_url, &ov.base_url);
            if base_url.is_empty() {
                base_url = "https://api.anthropic.com".to_string();
            }
            Ok(ProviderCreds {
                api_key,
                base_url,
                model: pick(&llm.anthropic_model, &ov.model),
                kind: ProviderKind::Anthropic,
                provider,
            })
        }
        "infomaniak" => {
            let api_key = pick(&llm.infomaniak_api_key, &ov.api_key);
            if api_key.is_empty() {
                return Err("Clé API Infomaniak manquante — saisissez-la dans Settings.".into());
            }
            // The root is derived from the product id. An explicit base URL
            // stays available as an escape hatch (staging hosts, proxies).
            let mut base_url = pick(&llm.infomaniak_base_url, &ov.base_url);
            if base_url.is_empty() {
                let product_id = pick(&llm.infomaniak_product_id, &ov.product_id);
                if product_id.is_empty() {
                    return Err(
                        "Product ID Infomaniak manquant — utilisez « Détecter » pour le récupérer."
                            .into(),
                    );
                }
                base_url = infomaniak_base_url(&product_id);
            }
            Ok(ProviderCreds {
                api_key,
                base_url,
                model: pick(&llm.infomaniak_model, &ov.model),
                kind: ProviderKind::OpenAiCompatible,
                provider,
            })
        }
        "default" => Err(
            "Aucun fournisseur LLM actif — choisissez-en un dans Settings puis appliquez.".into(),
        ),
        other => Err(format!("Fournisseur inconnu : {other}")),
    }
}

/// Build a live LLM client from the stored settings plus per-request
/// overrides.
///
/// The only place that turns configuration into a client: `/ask`,
/// `/ask/stream` and `/ingest/analyze` all route through it, so applying a
/// provider/model in the UI takes effect everywhere at once, without a
/// restart.
pub(crate) fn model_for(
    llm: &LlmSettings,
    ov: &LlmOverrides,
) -> Result<Arc<dyn LanguageModel>, String> {
    client_from_creds(resolve_provider_creds(llm, ov)?)
}

/// Instantiate the client for already-resolved credentials.
///
/// A model name is mandatory here even though it is optional in
/// [`resolve_provider_creds`]: listing a catalogue legitimately happens before
/// a model is chosen, but generating without one would send `"model": ""`
/// upstream and come back as an opaque provider error.
fn client_from_creds(creds: ProviderCreds) -> Result<Arc<dyn LanguageModel>, String> {
    if creds.model.is_empty() {
        return Err(format!(
            "Aucun modèle sélectionné pour {} — chargez la liste des modèles et choisissez-en un.",
            creds.provider
        ));
    }
    match creds.kind {
        ProviderKind::Anthropic => Ok(Arc::new(
            AnthropicModel::new(creds.api_key)
                .with_base_url(creds.base_url)
                .with_model(creds.model),
        )),
        ProviderKind::OpenAiCompatible => Ok(Arc::new(
            OpenAiModel::new(creds.api_key)
                .with_base_url(creds.base_url)
                .with_model(creds.model),
        )),
    }
}

/// A [`LanguageModel`] that resolves the provider from the live settings on
/// **every call**, falling back to `fallback` while the active provider is
/// `"default"`.
///
/// This is what makes "Apply" in the UI reach `/ask` and `/ask/stream`:
/// [`RagPipeline`] holds one `Arc<dyn LanguageModel>` bound at startup, so
/// without this indirection a provider or model change stayed invisible until
/// the process restarted.
///
/// The built client is cached behind its config fingerprint. Rebuilding per
/// call would hand every question a fresh `reqwest` connection pool — a new
/// TLS handshake each time — while never rebuilding would defeat the point.
pub struct SettingsRoutedModel {
    settings: Arc<PlRwLock<Settings>>,
    fallback: Arc<dyn LanguageModel>,
    cached: PlMutex<Option<(u64, Arc<dyn LanguageModel>)>>,
}

impl SettingsRoutedModel {
    /// `settings` must be the same handle the `/settings` handlers write to —
    /// [`AppState::assemble`] hands it over for exactly this.
    ///
    /// `fallback` serves requests while no provider is configured; `ontology
    /// serve` passes `EchoModel` so an unconfigured server still answers
    /// deterministically instead of erroring.
    pub fn new(settings: Arc<PlRwLock<Settings>>, fallback: Arc<dyn LanguageModel>) -> Self {
        Self {
            settings,
            fallback,
            cached: PlMutex::new(None),
        }
    }

    /// Client for the configuration in force right now.
    fn current(&self) -> Result<Arc<dyn LanguageModel>, LlmError> {
        let llm = self.settings.read().llm.clone();
        let ov = LlmOverrides::default();
        if effective_provider(&llm, &ov) == "default" {
            return Ok(self.fallback.clone());
        }
        let creds = resolve_provider_creds(&llm, &ov).map_err(LlmError::Config)?;
        let fingerprint = creds.fingerprint();

        let mut cached = self.cached.lock();
        if let Some((known, model)) = cached.as_ref() {
            if *known == fingerprint {
                return Ok(model.clone());
            }
        }
        let model = client_from_creds(creds).map_err(LlmError::Config)?;
        *cached = Some((fingerprint, model.clone()));
        Ok(model)
    }
}

#[async_trait::async_trait]
impl LanguageModel for SettingsRoutedModel {
    async fn generate(&self, req: &LlmRequest) -> Result<LlmResponse, LlmError> {
        self.current()?.generate(req).await
    }

    async fn generate_stream(&self, req: &LlmRequest) -> Result<LlmStream, LlmError> {
        self.current()?.generate_stream(req).await
    }
}

/// Outcome of resolving the persisted provider configuration.
///
/// Three-way on purpose: "nothing configured" is a normal state whose answer
/// is caller-specific — the server keeps its boot-time fallback, the CLI uses
/// `EchoModel` — whereas "configured but unusable" is a user error worth
/// reporting instead of silently downgrading to an echo.
pub enum ConfiguredModel {
    /// No provider selected; the caller's own fallback applies.
    NotConfigured,
    /// Client built from the stored credentials.
    Ready(Arc<dyn LanguageModel>),
    /// A provider is selected but its configuration is incomplete. The
    /// payload is a message meant for the user.
    Invalid(String),
}

/// Resolve the LLM the given settings select, for callers outside the HTTP
/// layer. `ov` applies per-invocation overrides (the CLI's `--model`) and is
/// never persisted.
pub fn configured_model(settings: &Settings, ov: &LlmOverrides) -> ConfiguredModel {
    if effective_provider(&settings.llm, ov) == "default" {
        return ConfiguredModel::NotConfigured;
    }
    match model_for(&settings.llm, ov) {
        Ok(model) => ConfiguredModel::Ready(model),
        Err(message) => ConfiguredModel::Invalid(message),
    }
}

/// Short-timeout client for the control-plane calls below (model catalogue,
/// connection test, product discovery). Completions go through the clients in
/// `ontology-rag`, which carry their own retry policy.
fn control_plane_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// GET `url` with the auth headers `kind` expects, returning decoded JSON or
/// a message fit to show a user.
async fn get_provider_json(
    url: &str,
    api_key: &str,
    kind: ProviderKind,
) -> Result<serde_json::Value, String> {
    let client = control_plane_client();
    let req = match kind {
        ProviderKind::Anthropic => client
            .get(url)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01"),
        ProviderKind::OpenAiCompatible => client.get(url).bearer_auth(api_key),
    };
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "HTTP {status}: {}",
            body.chars().take(200).collect::<String>()
        ));
    }
    resp.json().await.map_err(|e| e.to_string())
}

#[derive(Debug, Serialize)]
struct LlmTestResponse {
    ok: bool,
    /// Provider actually tested, once overrides and settings are resolved.
    provider: String,
    /// Endpoint that was called. Echoed back so a 404 or 401 is diagnosable
    /// from the UI alone; it carries no secret.
    #[serde(skip_serializing_if = "Option::is_none")]
    endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// `POST /settings/llm/test` — probe the provider's model catalogue with the
/// merged config. Credentials in the body are used as-is and never persisted.
async fn test_llm_connection(
    State(s): State<AppState>,
    Json(ov): Json<LlmOverrides>,
) -> Json<LlmTestResponse> {
    let llm = s.settings.read().llm.clone();
    let creds = match resolve_provider_creds(&llm, &ov) {
        Ok(c) => c,
        Err(error) => {
            return Json(LlmTestResponse {
                ok: false,
                provider: effective_provider(&llm, &ov),
                endpoint: None,
                model: None,
                error: Some(error),
            })
        }
    };
    let endpoint = v1_api_url(&creds.base_url, "models");
    match get_provider_json(&endpoint, &creds.api_key, creds.kind).await {
        Ok(_) => Json(LlmTestResponse {
            ok: true,
            provider: creds.provider,
            endpoint: Some(endpoint),
            model: if creds.model.is_empty() {
                None
            } else {
                Some(creds.model)
            },
            error: None,
        }),
        Err(error) => Json(LlmTestResponse {
            ok: false,
            provider: creds.provider,
            endpoint: Some(endpoint),
            model: None,
            error: Some(error),
        }),
    }
}

/// One entry of a provider's model catalogue.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LlmModelInfo {
    /// Value to send as `model` in a completion request.
    pub id: String,
    /// Display label; equals `id` unless the provider ships a nicer name.
    pub label: String,
    /// Context window advertised by the provider, when it advertises one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u32>,
    /// Provider-declared family, passed through verbatim — Infomaniak uses it
    /// to separate `llm` from image/audio products. Deliberately not filtered
    /// server-side: a kind we have never seen must not silently vanish.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Provider flagged the model as beta.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub beta: bool,
}

/// Parse a model catalogue from either wire shape.
///
/// * OpenAI-compatible (`GET {base}/v1/models`):
///   `{"data":[{"id":"gpt-4o","object":"model"}]}` — `id` is the string a
///   completion request expects.
/// * Infomaniak's account catalogue (`GET /1/ai/models`):
///   `{"result":"success","data":[{"id":57064,"name":"mixtral","type":"llm",
///   "max_token_input":32000,"meta":{"is_beta":false}}]}` — there `id` is a
///   numeric database key and **`name`** is the value to send, so reading
///   `id` as a string (what this endpoint used to do) returns an empty list.
///
/// Rows with no usable name are skipped, duplicate ids collapse, and the
/// result is sorted by label so the picker order is stable across calls.
fn parse_model_catalogue(v: &serde_json::Value) -> Vec<LlmModelInfo> {
    let rows = match v.get("data").and_then(|d| d.as_array()) {
        Some(rows) => rows,
        None => return Vec::new(),
    };
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for row in rows {
        let id = match row
            .get("id")
            .and_then(|i| i.as_str())
            .or_else(|| row.get("name").and_then(|n| n.as_str()))
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(id) => id.to_string(),
            None => continue,
        };
        if !seen.insert(id.clone()) {
            continue;
        }
        let label = row
            .get("name")
            .and_then(|n| n.as_str())
            .map(str::trim)
            .filter(|n| !n.is_empty() && *n != id.as_str())
            .map(|n| format!("{n} ({id})"))
            .unwrap_or_else(|| id.clone());
        let max_input_tokens = ["max_token_input", "max_input_tokens", "context_length"]
            .iter()
            .find_map(|k| row.get(*k).and_then(|v| v.as_u64()))
            .and_then(|v| u32::try_from(v).ok());
        let kind = row
            .get("type")
            .and_then(|t| t.as_str())
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string);
        let beta = row
            .get("meta")
            .and_then(|m| m.get("is_beta"))
            .and_then(|b| b.as_bool())
            .unwrap_or(false);
        out.push(LlmModelInfo {
            id,
            label,
            max_input_tokens,
            kind,
            beta,
        });
    }
    out.sort_by_key(|m| m.label.to_lowercase());
    out
}

#[derive(Debug, Deserialize)]
struct LlmModelsQuery {
    #[serde(default)]
    provider: Option<String>,
}

#[derive(Debug, Serialize)]
struct LlmModelsResponse {
    models: Vec<LlmModelInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// `GET /settings/llm/models?provider=…` — catalogue for the stored config.
async fn list_llm_models(
    State(s): State<AppState>,
    Query(q): Query<LlmModelsQuery>,
) -> Json<LlmModelsResponse> {
    let ov = LlmOverrides {
        provider: q.provider,
        ..Default::default()
    };
    Json(fetch_models(&s, &ov).await)
}

/// `POST /settings/llm/models` — same, but with unsaved credentials in the
/// body so the picker can be populated before anything is stored.
async fn list_llm_models_with_overrides(
    State(s): State<AppState>,
    Json(ov): Json<LlmOverrides>,
) -> Json<LlmModelsResponse> {
    Json(fetch_models(&s, &ov).await)
}

async fn fetch_models(s: &AppState, ov: &LlmOverrides) -> LlmModelsResponse {
    let llm = s.settings.read().llm.clone();
    let creds = match resolve_provider_creds(&llm, ov) {
        Ok(c) => c,
        Err(error) => {
            return LlmModelsResponse {
                models: Vec::new(),
                endpoint: None,
                error: Some(error),
            }
        }
    };
    let endpoint = v1_api_url(&creds.base_url, "models");
    match get_provider_json(&endpoint, &creds.api_key, creds.kind).await {
        Ok(v) => LlmModelsResponse {
            models: parse_model_catalogue(&v),
            endpoint: Some(endpoint),
            error: None,
        },
        Err(error) => LlmModelsResponse {
            models: Vec::new(),
            endpoint: Some(endpoint),
            error: Some(error),
        },
    }
}

/// Account-level AI Tools endpoint listing the products a token can reach.
/// Not product-scoped, so it cannot be derived from the provider base URL.
const INFOMANIAK_PRODUCTS_URL: &str = "https://api.infomaniak.com/1/ai";

#[derive(Debug, Default, Deserialize)]
pub struct InfomaniakProductsRequest {
    /// Token to probe with; falls back to the stored Infomaniak key. Carried
    /// in the body, never in a query string, so it stays out of access logs.
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InfomaniakProduct {
    pub product_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Serialize)]
struct InfomaniakProductsResponse {
    products: Vec<InfomaniakProduct>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Parse `GET /1/ai`. Rows carry the id under `product_id` or `id`, as a
/// number or a string depending on the endpoint version, so both are read.
fn parse_infomaniak_products(v: &serde_json::Value) -> Vec<InfomaniakProduct> {
    let rows = match v.get("data").and_then(|d| d.as_array()) {
        Some(rows) => rows,
        None => return Vec::new(),
    };
    rows.iter()
        .filter_map(|row| {
            let product_id = ["product_id", "id"].iter().find_map(|k| match row.get(*k) {
                Some(serde_json::Value::String(s)) if !s.trim().is_empty() => {
                    Some(s.trim().to_string())
                }
                Some(serde_json::Value::Number(n)) => Some(n.to_string()),
                _ => None,
            })?;
            let name = ["name", "customer_name", "label"]
                .iter()
                .find_map(|k| row.get(*k).and_then(|n| n.as_str()))
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_string);
            Some(InfomaniakProduct { product_id, name })
        })
        .collect()
}

/// `POST /settings/llm/infomaniak/products` — resolve the AI Tools product
/// id(s) a token can reach, so nobody has to hunt for it in the Infomaniak
/// manager and paste it by hand.
async fn list_infomaniak_products(
    State(s): State<AppState>,
    Json(req): Json<InfomaniakProductsRequest>,
) -> Json<InfomaniakProductsResponse> {
    let stored = s.settings.read().llm.infomaniak_api_key.clone();
    let api_key = pick(&stored, &req.api_key);
    if api_key.is_empty() {
        return Json(InfomaniakProductsResponse {
            products: Vec::new(),
            error: Some("Clé API Infomaniak manquante — saisissez-la d'abord.".into()),
        });
    }
    match get_provider_json(
        INFOMANIAK_PRODUCTS_URL,
        &api_key,
        ProviderKind::OpenAiCompatible,
    )
    .await
    {
        Ok(v) => {
            let products = parse_infomaniak_products(&v);
            let error = if products.is_empty() {
                Some(
                    "Aucun produit AI Tools sur ce compte — vérifiez que le token porte le scope « ai-tools »."
                        .into(),
                )
            } else {
                None
            };
            Json(InfomaniakProductsResponse { products, error })
        }
        Err(error) => Json(InfomaniakProductsResponse {
            products: Vec::new(),
            error: Some(error),
        }),
    }
}

// ---------------------------------------------------------------------------
// Feedback handlers
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CreateFeedbackRequest {
    #[serde(default = "default_feedback_kind")]
    kind: String,
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    screenshot: Option<String>,
    #[serde(default)]
    frontend_logs: String,
    #[serde(default)]
    user_agent: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    reporter_email: Option<String>,
}

fn default_feedback_kind() -> String {
    "bug".into()
}

async fn create_feedback(
    State(s): State<AppState>,
    Json(req): Json<CreateFeedbackRequest>,
) -> Result<Json<Feedback>, ApiError> {
    if req.title.trim().is_empty() {
        return Err(ApiError::BadRequest("title must not be empty".into()));
    }
    let backend_logs = snapshot_recent_logs(200).join("\n");
    let fb = Feedback {
        id: 0,
        created_at: 0,
        kind: req.kind,
        title: req.title,
        description: req.description,
        screenshot: req.screenshot,
        frontend_logs: req.frontend_logs,
        backend_logs,
        user_agent: req.user_agent,
        url: req.url,
        reporter_email: req.reporter_email,
    };
    let stored = s.feedbacks.write().insert(fb);
    Ok(Json(stored))
}

async fn list_feedbacks(State(s): State<AppState>) -> Json<Vec<Feedback>> {
    Json(s.feedbacks.read().list())
}

async fn delete_feedback(
    State(s): State<AppState>,
    Path(id): Path<u64>,
) -> Result<StatusCode, ApiError> {
    if s.feedbacks.write().remove(id) {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound(format!("feedback {id}")))
    }
}

#[derive(Debug, Deserialize)]
struct LogsTailQuery {
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
struct LogsTailResponse {
    lines: Vec<String>,
}

async fn logs_tail(Query(q): Query<LogsTailQuery>) -> Json<LogsTailResponse> {
    let limit = q.limit.unwrap_or(200).min(RECENT_LOGS_CAPACITY);
    Json(LogsTailResponse {
        lines: snapshot_recent_logs(limit),
    })
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
            ApiError::Graph(ontology_graph::GraphError::MissingRequiredProperty { .. })
            | ApiError::Graph(ontology_graph::GraphError::DisjointTypeViolation { .. })
            | ApiError::Graph(ontology_graph::GraphError::CardinalityViolation { .. }) => {
                (StatusCode::UNPROCESSABLE_ENTITY, self.to_string())
            }
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

// ---------------------------------------------------------------------------
// Unit tests: provider resolution, catalogue parsing, redaction, migration
// ---------------------------------------------------------------------------

#[cfg(test)]
mod provider_tests {
    use super::*;
    use serde_json::json;

    fn infomaniak_settings() -> LlmSettings {
        LlmSettings {
            active_provider: "infomaniak".into(),
            infomaniak_api_key: "tok-abcd".into(),
            infomaniak_product_id: "101112".into(),
            infomaniak_model: "mixtral".into(),
            ..Default::default()
        }
    }

    // -- effective_provider -------------------------------------------------

    #[test]
    fn override_provider_beats_active_provider() {
        let llm = infomaniak_settings();
        let ov = LlmOverrides {
            provider: Some("openai".into()),
            ..Default::default()
        };
        assert_eq!(effective_provider(&llm, &ov), "openai");
    }

    #[test]
    fn blank_or_default_override_falls_back_to_active_provider() {
        let llm = infomaniak_settings();
        for probe in [None, Some("".to_string()), Some("  ".to_string()), Some("default".to_string())] {
            let ov = LlmOverrides {
                provider: probe.clone(),
                ..Default::default()
            };
            assert_eq!(
                effective_provider(&llm, &ov),
                "infomaniak",
                "probe {probe:?} should defer to active_provider"
            );
        }
    }

    #[test]
    fn empty_active_provider_reads_as_default() {
        let llm = LlmSettings {
            active_provider: String::new(),
            ..Default::default()
        };
        assert_eq!(
            effective_provider(&llm, &LlmOverrides::default()),
            "default"
        );
    }

    // -- resolve_provider_creds --------------------------------------------

    #[test]
    fn infomaniak_base_url_is_derived_from_the_product_id() {
        let creds =
            resolve_provider_creds(&infomaniak_settings(), &LlmOverrides::default()).unwrap();
        assert_eq!(
            creds.base_url,
            "https://api.infomaniak.com/2/ai/101112/openai/v1"
        );
        assert_eq!(creds.kind, ProviderKind::OpenAiCompatible);
        assert_eq!(creds.model, "mixtral");
        // And the endpoint the control plane derives from it:
        assert_eq!(
            v1_api_url(&creds.base_url, "models"),
            "https://api.infomaniak.com/2/ai/101112/openai/v1/models"
        );
    }

    #[test]
    fn infomaniak_without_a_product_id_is_a_clear_error() {
        let llm = LlmSettings {
            infomaniak_product_id: String::new(),
            ..infomaniak_settings()
        };
        let err = resolve_provider_creds(&llm, &LlmOverrides::default()).unwrap_err();
        assert!(err.contains("Product ID"), "unhelpful error: {err}");
    }

    #[test]
    fn infomaniak_without_a_key_is_a_clear_error() {
        let llm = LlmSettings {
            infomaniak_api_key: String::new(),
            ..infomaniak_settings()
        };
        let err = resolve_provider_creds(&llm, &LlmOverrides::default()).unwrap_err();
        assert!(err.contains("Infomaniak"), "unhelpful error: {err}");
    }

    #[test]
    fn explicit_base_url_overrides_the_derived_one() {
        let llm = LlmSettings {
            infomaniak_base_url: "https://staging.example.test/openai/v1".into(),
            ..infomaniak_settings()
        };
        let creds = resolve_provider_creds(&llm, &LlmOverrides::default()).unwrap();
        assert_eq!(creds.base_url, "https://staging.example.test/openai/v1");
    }

    #[test]
    fn request_overrides_win_over_stored_settings() {
        let ov = LlmOverrides {
            api_key: Some("tok-fresh".into()),
            product_id: Some("999".into()),
            model: Some("granite".into()),
            ..Default::default()
        };
        let llm = LlmSettings {
            infomaniak_product_id: String::new(),
            ..infomaniak_settings()
        };
        let creds = resolve_provider_creds(&llm, &ov).unwrap();
        assert_eq!(creds.api_key, "tok-fresh");
        assert_eq!(creds.model, "granite");
        assert_eq!(
            creds.base_url,
            "https://api.infomaniak.com/2/ai/999/openai/v1"
        );
    }

    #[test]
    fn whitespace_only_override_does_not_shadow_a_stored_value() {
        let ov = LlmOverrides {
            api_key: Some("   ".into()),
            ..Default::default()
        };
        let creds = resolve_provider_creds(&infomaniak_settings(), &ov).unwrap();
        assert_eq!(creds.api_key, "tok-abcd");
    }

    #[test]
    fn openai_and_anthropic_defaults_carry_no_duplicate_v1() {
        let llm = LlmSettings {
            active_provider: "openai".into(),
            openai_api_key: "sk-x".into(),
            ..Default::default()
        };
        let creds = resolve_provider_creds(&llm, &LlmOverrides::default()).unwrap();
        assert_eq!(
            v1_api_url(&creds.base_url, "models"),
            "https://api.openai.com/v1/models"
        );

        let llm = LlmSettings {
            active_provider: "anthropic".into(),
            anthropic_api_key: "sk-ant".into(),
            // A user who pasted the documented "with /v1" spelling must not
            // end up on /v1/v1/models.
            anthropic_base_url: "https://api.anthropic.com/v1".into(),
            ..Default::default()
        };
        let creds = resolve_provider_creds(&llm, &LlmOverrides::default()).unwrap();
        assert_eq!(creds.kind, ProviderKind::Anthropic);
        assert_eq!(
            v1_api_url(&creds.base_url, "models"),
            "https://api.anthropic.com/v1/models"
        );
    }

    #[test]
    fn default_provider_and_unknown_provider_are_distinct_errors() {
        let llm = LlmSettings::default();
        let err = resolve_provider_creds(&llm, &LlmOverrides::default()).unwrap_err();
        assert!(err.contains("Aucun fournisseur"), "got: {err}");

        let ov = LlmOverrides {
            provider: Some("mistral".into()),
            ..Default::default()
        };
        let err = resolve_provider_creds(&llm, &ov).unwrap_err();
        assert!(err.contains("mistral"), "got: {err}");
    }

    // -- model_for ---------------------------------------------------------

    #[test]
    fn model_for_refuses_to_build_a_client_without_a_model() {
        let llm = LlmSettings {
            infomaniak_model: String::new(),
            ..infomaniak_settings()
        };
        // Arc<dyn LanguageModel> is not Debug, so unwrap_err() is unavailable.
        let err = match model_for(&llm, &LlmOverrides::default()) {
            Err(e) => e,
            Ok(_) => panic!("expected a missing-model error, got a client"),
        };
        assert!(err.contains("Aucun modèle"), "got: {err}");
    }

    #[test]
    fn model_for_builds_a_client_for_a_complete_config() {
        assert!(model_for(&infomaniak_settings(), &LlmOverrides::default()).is_ok());
    }

    // -- parse_model_catalogue --------------------------------------------

    #[test]
    fn parses_the_openai_catalogue_shape() {
        let v = json!({
            "object": "list",
            "data": [
                { "id": "gpt-4o", "object": "model" },
                { "id": "gpt-4o-mini", "object": "model" }
            ]
        });
        let models = parse_model_catalogue(&v);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gpt-4o");
        assert_eq!(models[0].label, "gpt-4o");
        assert_eq!(models[0].max_input_tokens, None);
        assert!(!models[0].beta);
    }

    #[test]
    fn parses_the_infomaniak_catalogue_where_id_is_numeric() {
        // The regression this pins: reading `id` as a string here yields an
        // empty list, because the model name lives in `name`.
        let v = json!({
            "result": "success",
            "data": [
                {
                    "id": 57064,
                    "name": "mixtral",
                    "type": "llm",
                    "max_token_input": 32000,
                    "meta": { "is_beta": false, "is_coder": false }
                },
                {
                    "id": 57065,
                    "name": "granite",
                    "type": "llm",
                    "max_token_input": 8192,
                    "meta": { "is_beta": true }
                }
            ]
        });
        let models = parse_model_catalogue(&v);
        assert_eq!(models.len(), 2);

        let granite = &models[0];
        assert_eq!(granite.id, "granite");
        assert_eq!(granite.label, "granite");
        assert_eq!(granite.max_input_tokens, Some(8192));
        assert_eq!(granite.kind.as_deref(), Some("llm"));
        assert!(granite.beta);

        let mixtral = &models[1];
        assert_eq!(mixtral.id, "mixtral");
        assert_eq!(mixtral.max_input_tokens, Some(32000));
        assert!(!mixtral.beta);
    }

    #[test]
    fn catalogue_keeps_non_llm_kinds_visible_rather_than_dropping_them() {
        let v = json!({ "data": [ { "id": 1, "name": "whisper", "type": "transcription" } ] });
        let models = parse_model_catalogue(&v);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].kind.as_deref(), Some("transcription"));
    }

    #[test]
    fn catalogue_skips_unusable_rows_and_dedups() {
        let v = json!({
            "data": [
                { "object": "model" },
                { "id": "  " },
                { "id": "gpt-4o" },
                { "id": "gpt-4o" }
            ]
        });
        let models = parse_model_catalogue(&v);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "gpt-4o");
    }

    #[test]
    fn catalogue_labels_a_distinct_display_name_with_its_id() {
        let v = json!({ "data": [ { "id": "llama-3.3-70b", "name": "Llama 3.3 70B" } ] });
        let models = parse_model_catalogue(&v);
        assert_eq!(models[0].id, "llama-3.3-70b");
        assert_eq!(models[0].label, "Llama 3.3 70B (llama-3.3-70b)");
    }

    #[test]
    fn catalogue_is_empty_when_data_is_missing_or_not_an_array() {
        assert!(parse_model_catalogue(&json!({})).is_empty());
        assert!(parse_model_catalogue(&json!({ "data": "nope" })).is_empty());
    }

    #[test]
    fn catalogue_is_sorted_case_insensitively_by_label() {
        let v = json!({ "data": [ { "id": "zeta" }, { "id": "Alpha" }, { "id": "beta" } ] });
        let labels: Vec<_> = parse_model_catalogue(&v)
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(labels, vec!["Alpha", "beta", "zeta"]);
    }

    // -- parse_infomaniak_products ----------------------------------------

    #[test]
    fn parses_products_with_numeric_and_string_ids() {
        let v = json!({
            "result": "success",
            "data": [
                { "product_id": 101112, "name": "AI Tools" },
                { "id": "202122" }
            ]
        });
        let products = parse_infomaniak_products(&v);
        assert_eq!(
            products,
            vec![
                InfomaniakProduct {
                    product_id: "101112".into(),
                    name: Some("AI Tools".into())
                },
                InfomaniakProduct {
                    product_id: "202122".into(),
                    name: None
                },
            ]
        );
    }

    #[test]
    fn products_skip_rows_without_an_id() {
        let v = json!({ "data": [ { "name": "orphan" } ] });
        assert!(parse_infomaniak_products(&v).is_empty());
    }

    // -- redaction ---------------------------------------------------------

    #[test]
    fn settings_view_strips_every_secret_but_keeps_hints_and_limits() {
        let mut settings = Settings::default();
        settings.llm.openai_api_key = "sk-openai-1234".into();
        settings.llm.anthropic_api_key = "sk-ant-5678".into();
        settings.llm.infomaniak_api_key = "tok-info-9012".into();
        settings.ocr.google_api_key = "goog-3456".into();
        settings.llm.refresh_key_hints();
        settings.ocr.refresh_key_hints();

        let view = settings_view(&settings);
        let flat = view.to_string();
        for secret in [
            "sk-openai-1234",
            "sk-ant-5678",
            "tok-info-9012",
            "goog-3456",
        ] {
            assert!(!flat.contains(secret), "secret leaked in {flat}");
        }
        assert!(view["llm"].get("openai_api_key").is_none());
        assert_eq!(view["llm"]["openai_api_key_hint"], "sk-...1234");
        assert_eq!(view["llm"]["infomaniak_api_key_hint"], "tok...9012");
        assert_eq!(view["ocr"]["google_api_key_hint"], "goo...3456");
        // Non-secret fields whose names flirt with the suffix list survive.
        assert_eq!(view["llm"]["max_tokens"], 1000);
    }

    #[test]
    fn redaction_reaches_nested_objects_and_arrays() {
        let mut v = json!({
            "outer": { "some_api_key": "x", "keep": 1 },
            "list": [ { "auth_token": "y", "keep": 2 } ]
        });
        redact_secrets(&mut v);
        assert!(v["outer"].get("some_api_key").is_none());
        assert_eq!(v["outer"]["keep"], 1);
        assert!(v["list"][0].get("auth_token").is_none());
        assert_eq!(v["list"][0]["keep"], 2);
    }

    // -- persistence round-trip & migration -------------------------------

    #[test]
    fn secrets_survive_a_serialize_deserialize_round_trip() {
        let mut settings = Settings::default();
        settings.llm.infomaniak_api_key = "tok-info-9012".into();
        settings.ocr.google_api_key = "goog-3456".into();

        let json = serde_json::to_string(&settings).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.llm.infomaniak_api_key, "tok-info-9012");
        assert_eq!(back.ocr.google_api_key, "goog-3456");
    }

    #[test]
    fn a_settings_file_from_an_older_build_still_loads() {
        // No `infomaniak_product_id`, no `ocr` block, no `ui` block: every
        // absent field must fall back to its default instead of failing the
        // whole parse and wiping the stored credentials.
        let raw = r#"{
            "retrieval": { "top_k": 12 },
            "llm": { "active_provider": "infomaniak", "infomaniak_api_key": "tok-old" }
        }"#;
        let loaded: Settings = serde_json::from_str(raw).unwrap();
        assert_eq!(loaded.retrieval.top_k, 12);
        assert_eq!(loaded.retrieval.lexical_weight, 0.5);
        assert_eq!(loaded.ui.theme, "light");
        assert_eq!(loaded.llm.infomaniak_api_key, "tok-old");
        assert_eq!(loaded.llm.infomaniak_product_id, "");
        assert_eq!(loaded.ocr.provider, "tesseract");
    }

    #[test]
    fn migrate_clears_the_dead_infomaniak_base_url_and_rebuilds_hints() {
        let mut settings = Settings::default();
        settings.llm.infomaniak_base_url = "https://api.infomaniak.com/1/ai/".into();
        settings.llm.infomaniak_api_key = "tok-info-9012".into();
        settings.llm.infomaniak_api_key_hint.clear();

        settings.migrate();

        assert_eq!(settings.llm.infomaniak_base_url, "");
        assert_eq!(settings.llm.infomaniak_api_key_hint, "tok...9012");
    }

    #[test]
    fn migrate_leaves_a_user_supplied_base_url_alone() {
        let mut settings = Settings::default();
        settings.llm.infomaniak_base_url = "https://staging.example.test/openai/v1".into();
        settings.migrate();
        assert_eq!(
            settings.llm.infomaniak_base_url,
            "https://staging.example.test/openai/v1"
        );
    }

    // -- SettingsRoutedModel ----------------------------------------------
    //
    // No test here reaches the network, and none needs to: routing is checked
    // by identity against the fallback client, and the one case that does call
    // generate() fails on validation before a request is dispatched. The
    // 127.0.0.1:1 base URL is a tripwire — if resolution ever started
    // dispatching where it should not, the test would hang on a refused
    // connection instead of passing.

    fn routed(settings: Settings) -> (Arc<PlRwLock<Settings>>, SettingsRoutedModel) {
        let shared = Arc::new(PlRwLock::new(settings));
        let model = SettingsRoutedModel::new(shared.clone(), Arc::new(ontology_rag::EchoModel));
        (shared, model)
    }

    /// Provider fully configured against a port nothing listens on.
    fn unreachable_infomaniak() -> Settings {
        let mut settings = Settings::default();
        settings.llm.active_provider = "infomaniak".into();
        settings.llm.infomaniak_api_key = "tok-abcd".into();
        settings.llm.infomaniak_base_url = "http://127.0.0.1:1/v1".into();
        settings.llm.infomaniak_model = "mixtral".into();
        settings
    }

    fn ping() -> LlmRequest {
        LlmRequest {
            messages: vec![ontology_rag::Message::user("hi")],
            max_tokens: 8,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn routed_model_delegates_to_the_fallback_while_no_provider_is_configured() {
        let (_shared, model) = routed(Settings::default());
        let resp = model.generate(&ping()).await.unwrap();
        assert_eq!(resp.content, "[echo] hi");
    }

    #[test]
    fn routed_model_picks_up_an_applied_provider_without_a_restart() {
        let (shared, model) = routed(Settings::default());
        // Unconfigured: the fallback itself, not a copy of it.
        let before = model.current().unwrap();
        assert!(
            Arc::ptr_eq(&before, &model.fallback),
            "an unconfigured server should answer from the fallback"
        );

        // Exactly what PATCH /settings does to the shared store.
        *shared.write() = unreachable_infomaniak();

        let after = model.current().unwrap();
        assert!(
            !Arc::ptr_eq(&after, &model.fallback),
            "provider change ignored: still routing to the fallback"
        );
    }

    #[tokio::test]
    async fn routed_model_reports_a_misconfiguration_as_config_not_upstream() {
        let mut settings = unreachable_infomaniak();
        settings.llm.infomaniak_model.clear();
        let (_shared, model) = routed(settings);

        let err = match model.generate(&ping()).await {
            Err(e) => e,
            Ok(r) => panic!("expected a config error, got {r:?}"),
        };
        assert!(matches!(err, LlmError::Config(_)), "got {err:?}");
    }

    #[test]
    fn routed_model_reuses_its_client_until_the_config_changes() {
        let (shared, model) = routed(unreachable_infomaniak());

        let first = model.current().unwrap();
        let second = model.current().unwrap();
        assert!(
            Arc::ptr_eq(&first, &second),
            "client rebuilt for an unchanged configuration"
        );

        shared.write().llm.infomaniak_model = "granite".into();
        let third = model.current().unwrap();
        assert!(
            !Arc::ptr_eq(&first, &third),
            "client reused across a model change"
        );
    }

    #[test]
    fn fingerprint_changes_with_every_field_that_shapes_a_client() {
        let base = resolve_provider_creds(&infomaniak_settings(), &LlmOverrides::default()).unwrap();
        let reference = base.fingerprint();

        for mutate in [
            (|c: &mut ProviderCreds| c.api_key = "other".into()) as fn(&mut ProviderCreds),
            |c: &mut ProviderCreds| c.base_url = "http://other.test/v1".into(),
            |c: &mut ProviderCreds| c.model = "other".into(),
            |c: &mut ProviderCreds| c.provider = "openai".into(),
        ] {
            let mut altered = base.clone();
            mutate(&mut altered);
            assert_ne!(
                altered.fingerprint(),
                reference,
                "fingerprint blind to a change that shapes the client"
            );
        }
    }
}
