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
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use ontology_graph::{Concept, ConceptId, OntologyGraph, Relation, RelationId};
use ontology_index::{HybridIndex, RetrievalRequest, ScoredConcept};
use ontology_rag::{RagAnswer, RagPipeline};
use ontology_storage::{LogRecord, Store};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use tracing::warn;

/// Shared application state passed to every handler.
#[derive(Clone)]
pub struct AppState {
    pub graph: Arc<OntologyGraph>,
    pub index: Arc<HybridIndex>,
    pub store: Arc<dyn Store>,
    pub pipeline: Arc<RagPipeline>,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/stats", get(stats))
        .route("/concepts", post(create_concept))
        .route("/concepts/:id", delete(delete_concept))
        .route("/relations", post(create_relation))
        .route("/retrieve", post(retrieve))
        .route("/ask", post(ask))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn healthz() -> &'static str { "ok" }

#[derive(Serialize)]
struct Stats {
    concepts: usize,
    relations: usize,
    concept_types: usize,
    relation_types: usize,
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
struct CreatedConcept { id: ConceptId }

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

async fn delete_concept(
    State(s): State<AppState>,
    Path(id): Path<u64>,
) -> Result<StatusCode, ApiError> {
    let cid = ConceptId(id);
    let removed = s.graph.remove_concept(cid)?;
    s.index.forget(cid);
    s.store.append(&LogRecord::delete_concept(cid)).await
        .map_err(|e| ApiError::Store(e.to_string()))?;
    for rid in removed {
        s.store.append(&LogRecord::delete_relation(rid)).await
            .map_err(|e| ApiError::Store(e.to_string()))?;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct CreatedRelation { id: RelationId }

async fn create_relation(
    State(s): State<AppState>,
    Json(mut rel): Json<Relation>,
) -> Result<Json<CreatedRelation>, ApiError> {
    let id = s.graph.add_relation(rel.clone())?;
    rel.id = id;
    s.store.append(&LogRecord::relation(rel)).await
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
    let ans = s.pipeline.answer_with(req).await
        .map_err(|e| ApiError::Llm(e.to_string()))?;
    Ok(Json(ans))
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
struct ErrorBody { error: String }

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self {
            ApiError::Graph(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            ApiError::Store(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            ApiError::Llm(_)   => (StatusCode::BAD_GATEWAY, self.to_string()),
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
        };
        (status, Json(ErrorBody { error: msg })).into_response()
    }
}
