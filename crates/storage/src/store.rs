use async_trait::async_trait;
use ontology_graph::{Concept, OntologyGraph, Relation};
use std::sync::Arc;
use thiserror::Error;

use crate::log::LogRecord;

pub type StoreResult<T> = Result<T, StoreError>;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("graph: {0}")]
    Graph(#[from] ontology_graph::GraphError),

    #[error("encode: {0}")]
    Encode(String),

    #[error("decode: {0}")]
    Decode(String),

    #[error("corrupt log at offset {0}")]
    Corrupt(u64),
}

/// Pluggable persistence backend.
///
/// Implementations need not be transactional across multiple records — the
/// graph is the single source of truth in memory; the store only needs to
/// guarantee that successfully-acked writes can be replayed in order.
#[async_trait]
pub trait Store: Send + Sync + 'static {
    /// Append a single log record durably.
    async fn append(&self, record: &LogRecord) -> StoreResult<()>;

    /// Replay every persisted record into the supplied graph.
    async fn load_into(&self, graph: &Arc<OntologyGraph>) -> StoreResult<()>;

    /// Optional snapshot of the full graph for fast cold start.
    async fn snapshot(&self, _graph: &Arc<OntologyGraph>) -> StoreResult<()> { Ok(()) }

    /// Atomically take a snapshot and truncate the WAL. After a successful
    /// compact, restoring from this store applies only the new snapshot —
    /// the log is empty. Implementations that don't have a separate WAL can
    /// fall back to `snapshot`.
    async fn compact(&self, graph: &Arc<OntologyGraph>) -> StoreResult<()> {
        self.snapshot(graph).await
    }
}

/// Convenience helpers used by callers that hold a graph + store together.
pub async fn persist_concept(
    store: &dyn Store,
    concept: &Concept,
) -> StoreResult<()> {
    store.append(&LogRecord::concept(concept.clone())).await
}

pub async fn persist_relation(
    store: &dyn Store,
    relation: &Relation,
) -> StoreResult<()> {
    store.append(&LogRecord::relation(relation.clone())).await
}
