// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

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

    /// A write failed part-way (disk full, I/O error). The on-disk tail is
    /// in an unknown state, so the store refuses every further append until
    /// the process restarts and recovery truncates the torn tail. Retrying
    /// through a poisoned store would let a rejected record become durable.
    #[error("store poisoned after a failed write; restart to recover: {0}")]
    Poisoned(String),
}

/// Pluggable persistence backend.
///
/// Implementations need not be transactional across multiple records — the
/// graph is the single source of truth in memory; the store only needs to
/// guarantee that successfully-acked writes can be replayed in order.
///
/// **Write-ahead contract (`STORAGE.md` R8).** Callers append the record
/// *before* mutating the in-memory graph: `prepare_*` → `append` →
/// `apply_prepared_*`. A successful return from `append` / `append_batch`
/// means the record survives a process crash (for [`crate::FileStore`], the
/// data has been `fsync`ed).
#[async_trait]
pub trait Store: Send + Sync + 'static {
    /// Append a single log record durably.
    async fn append(&self, record: &LogRecord) -> StoreResult<()>;

    /// Append several records durably, in order, with **one** durability
    /// barrier for the whole batch (group commit — `STORAGE.md` §7.2).
    ///
    /// Atomicity is per record, not per batch: a crash mid-batch may leave a
    /// durable prefix. Callers must therefore only batch records that are
    /// individually valid on replay regardless of whether the rest of the
    /// batch made it — e.g. a concept and the cascade of relation deletions
    /// that go with it, or a run of already-validated concepts.
    ///
    /// The default implementation appends one by one; backends with a real
    /// durability barrier override it.
    async fn append_batch(&self, records: &[LogRecord]) -> StoreResult<()> {
        for r in records {
            self.append(r).await?;
        }
        Ok(())
    }

    /// Replay every persisted record into the supplied graph.
    async fn load_into(&self, graph: &Arc<OntologyGraph>) -> StoreResult<()>;

    /// Optional snapshot of the full graph for fast cold start.
    async fn snapshot(&self, _graph: &Arc<OntologyGraph>) -> StoreResult<()> {
        Ok(())
    }

    /// Atomically take a snapshot and truncate the WAL. After a successful
    /// compact, restoring from this store applies only the new snapshot —
    /// the log is empty. Implementations that don't have a separate WAL can
    /// fall back to `snapshot`.
    async fn compact(&self, graph: &Arc<OntologyGraph>) -> StoreResult<()> {
        self.snapshot(graph).await
    }
}

/// Convenience helpers used by callers that hold a graph + store together.
pub async fn persist_concept(store: &dyn Store, concept: &Concept) -> StoreResult<()> {
    store.append(&LogRecord::concept(concept.clone())).await
}

pub async fn persist_relation(store: &dyn Store, relation: &Relation) -> StoreResult<()> {
    store.append(&LogRecord::relation(relation.clone())).await
}
