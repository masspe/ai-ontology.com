// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Test doubles for the [`Store`] trait, shared by the integration tests of
//! every crate that writes to a store. Not behind a feature flag: the code
//! is tiny and having one definition keeps the tests honest about the
//! contract they exercise.

use async_trait::async_trait;
use ontology_graph::OntologyGraph;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use crate::log::LogRecord;
use crate::memory::MemoryStore;
use crate::store::{Store, StoreError, StoreResult};

/// A [`MemoryStore`] that can be told to fail every append, and that counts
/// how many `append` / `append_batch` calls reached it.
///
/// Used to prove the write-ahead ordering (`STORAGE.md` R8): when the store
/// fails, the in-memory graph must be left untouched; when it succeeds, a
/// cascade must have cost exactly one batch call.
#[derive(Default)]
pub struct FlakyStore {
    inner: MemoryStore,
    failing: AtomicBool,
    append_calls: AtomicU64,
    batch_calls: AtomicU64,
    records_written: AtomicU64,
}

impl FlakyStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// From now on, every append fails with [`StoreError::Io`]. Nothing is
    /// recorded while failing.
    pub fn set_failing(&self, failing: bool) {
        self.failing.store(failing, Ordering::SeqCst);
    }

    /// Number of `append` calls (single-record).
    pub fn append_calls(&self) -> u64 {
        self.append_calls.load(Ordering::SeqCst)
    }

    /// Number of `append_batch` calls, whatever their size.
    pub fn batch_calls(&self) -> u64 {
        self.batch_calls.load(Ordering::SeqCst)
    }

    /// Records durably recorded so far (failed calls record nothing).
    pub fn records_written(&self) -> u64 {
        self.records_written.load(Ordering::SeqCst)
    }

    /// The records recorded so far, in order.
    pub fn records(&self) -> Vec<LogRecord> {
        self.inner.records()
    }

    fn check(&self) -> StoreResult<()> {
        if self.failing.load(Ordering::SeqCst) {
            return Err(StoreError::Io(std::io::Error::other(
                "FlakyStore: injected append failure",
            )));
        }
        Ok(())
    }
}

#[async_trait]
impl Store for FlakyStore {
    async fn append(&self, record: &LogRecord) -> StoreResult<()> {
        self.append_calls.fetch_add(1, Ordering::SeqCst);
        self.check()?;
        self.inner.append(record).await?;
        self.records_written.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn append_batch(&self, records: &[LogRecord]) -> StoreResult<()> {
        self.batch_calls.fetch_add(1, Ordering::SeqCst);
        self.check()?;
        for r in records {
            self.inner.append(r).await?;
        }
        self.records_written
            .fetch_add(records.len() as u64, Ordering::SeqCst);
        Ok(())
    }

    async fn load_into(&self, graph: &Arc<OntologyGraph>) -> StoreResult<()> {
        self.inner.load_into(graph).await
    }

    async fn reset(&self) -> StoreResult<()> {
        self.check()?;
        self.inner.reset().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ontology_graph::{Concept, ConceptId};

    fn concept(i: u64) -> LogRecord {
        LogRecord::concept(Concept::new(ConceptId(i), "T", format!("c{i}")))
    }

    /// Every failure mode is counted as a call and records nothing; a
    /// success records exactly what it was given; `reset` obeys the same
    /// switch and keeps the records when it fails.
    #[tokio::test]
    async fn failures_are_counted_and_record_nothing_successes_record_everything() {
        let store = FlakyStore::new();
        assert_eq!(
            (
                store.append_calls(),
                store.batch_calls(),
                store.records_written()
            ),
            (0, 0, 0)
        );

        store.set_failing(true);
        let err = store.append(&concept(1)).await.unwrap_err();
        assert!(matches!(err, StoreError::Io(_)), "{err}");
        let err = store
            .append_batch(&[concept(2), concept(3), concept(4)])
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::Io(_)), "{err}");
        assert_eq!(store.append_calls(), 1);
        assert_eq!(store.batch_calls(), 1);
        assert_eq!(store.records_written(), 0);
        assert!(store.records().is_empty());
        // An empty batch while failing is still a failed call.
        assert!(store.append_batch(&[]).await.is_err());
        assert_eq!(store.batch_calls(), 2);

        store.set_failing(false);
        store.append(&concept(1)).await.unwrap();
        store.append_batch(&[concept(2), concept(3)]).await.unwrap();
        store.append_batch(&[]).await.unwrap();
        assert_eq!(store.append_calls(), 2);
        assert_eq!(store.batch_calls(), 4);
        assert_eq!(store.records_written(), 3);
        let recs = store.records();
        assert_eq!(recs.len(), 3);
        assert_eq!(
            recs.iter().map(|r| r.seq).collect::<Vec<_>>(),
            vec![1, 2, 3],
            "seqs assigned by the inner store"
        );

        store.set_failing(true);
        assert!(store.reset().await.is_err());
        assert_eq!(store.records().len(), 3, "a failed reset keeps the records");
        store.set_failing(false);
        store.reset().await.unwrap();
        assert!(store.records().is_empty());
        assert_eq!(
            store.records_written(),
            3,
            "the counter is cumulative, not the current size"
        );
    }
}
