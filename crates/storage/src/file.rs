// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

use async_trait::async_trait;
use ontology_graph::OntologyGraph;
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tracing::{debug, warn};

use crate::log::LogRecord;
use crate::memory::apply;
use crate::store::{Store, StoreError, StoreResult};

/// Append-only log file with a sibling JSON snapshot for fast cold start.
///
/// Wire format per record:
///
/// ```text
/// [u32 length BE] [JSON-encoded LogRecord]
/// ```
///
/// On `load_into`, the snapshot (if present) is applied first, then any log
/// records strictly newer than the snapshot's high-water seq are replayed.
///
/// # Durability
///
/// Every `append` / `append_batch` ends with `fdatasync` on the log file
/// (`File::sync_data`), so an acknowledged record survives an OS crash —
/// `STORAGE.md` R8 assumes this. A batch pays a single sync (§7.2 group
/// commit); [`FileStore::sync_count`] exposes how many were issued, for
/// tests and metrics.
///
/// # Recovery
///
/// Only the tail of the log can be inconsistent (a crash mid-write). On
/// `load_into`, a truncated trailing record — length prefix cut short,
/// payload shorter than announced, or undecodable JSON *in last position* —
/// is logged, the file is truncated back to the last complete record, and
/// startup proceeds. Corruption anywhere before the tail is still fatal:
/// it cannot be a torn write and must not be silently dropped.
pub struct FileStore {
    log_path: PathBuf,
    snapshot_path: PathBuf,
    writer: tokio::sync::Mutex<File>,
    seq: Mutex<u64>,
    syncs: AtomicU64,
}

/// Outcome of scanning the log tail during recovery.
enum Tail {
    /// Every byte of the file belongs to a complete, decodable record.
    Clean,
    /// Bytes from `offset` to EOF are a torn write to discard.
    Torn { offset: u64, reason: &'static str },
}

impl FileStore {
    pub async fn open(dir: impl AsRef<Path>) -> StoreResult<Self> {
        let dir = dir.as_ref();
        tokio::fs::create_dir_all(dir).await?;
        let log_path = dir.join("graph.log");
        let snapshot_path = dir.join("graph.snap");

        let writer = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .await?;

        Ok(Self {
            log_path,
            snapshot_path,
            writer: tokio::sync::Mutex::new(writer),
            seq: Mutex::new(0),
            syncs: AtomicU64::new(0),
        })
    }

    /// Number of `fdatasync` calls issued on the log so far. One per
    /// `append`, one per `append_batch` regardless of batch size.
    pub fn sync_count(&self) -> u64 {
        self.syncs.load(Ordering::Relaxed)
    }

    /// Path of the write-ahead log this store appends to.
    pub fn log_path(&self) -> &Path {
        &self.log_path
    }

    /// Encode `records`, assigning consecutive sequence numbers.
    fn encode_batch(&self, records: &[LogRecord]) -> StoreResult<Vec<u8>> {
        let mut out: Vec<u8> = Vec::new();
        let mut seq = self.seq.lock();
        for record in records {
            let mut r = record.clone();
            *seq += 1;
            r.seq = *seq;
            let bytes = serde_json::to_vec(&r).map_err(|e| StoreError::Encode(e.to_string()))?;
            let len = u32::try_from(bytes.len())
                .map_err(|_| StoreError::Encode("record too large".into()))?;
            out.extend_from_slice(&len.to_be_bytes());
            out.extend_from_slice(&bytes);
        }
        Ok(out)
    }

    /// Write an already-fsynced temp file then atomically rename it over
    /// `dest`. The directory entry is synced too where the platform allows
    /// (Unix); on Windows a rename is durable on its own once the file is.
    async fn write_atomic(dest: &Path, bytes: &[u8]) -> StoreResult<()> {
        let tmp = dest.with_extension("snap.tmp");
        {
            let mut f = File::create(&tmp).await?;
            f.write_all(bytes).await?;
            f.sync_all().await?;
        }
        tokio::fs::rename(&tmp, dest).await?;
        #[cfg(unix)]
        if let Some(dir) = dest.parent() {
            if let Ok(d) = File::open(dir).await {
                let _ = d.sync_all().await;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Store for FileStore {
    async fn append(&self, record: &LogRecord) -> StoreResult<()> {
        self.append_batch(std::slice::from_ref(record)).await
    }

    async fn append_batch(&self, records: &[LogRecord]) -> StoreResult<()> {
        if records.is_empty() {
            return Ok(());
        }
        // Serialize the whole batch under the writer lock so sequence numbers
        // and on-disk order agree, then pay a single sync (R7 says the index
        // is never synced — there is no index yet, only the data file).
        let mut w = self.writer.lock().await;
        let bytes = self.encode_batch(records)?;
        w.write_all(&bytes).await?;
        w.flush().await?;
        w.sync_data().await?;
        self.syncs.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn load_into(&self, graph: &Arc<OntologyGraph>) -> StoreResult<()> {
        // Hold the writer for the whole load: recovery may truncate the file,
        // and no append must interleave with the replay.
        let _writer = self.writer.lock().await;

        // 1. Snapshot, if any.
        let mut snap_water: u64 = 0;
        if tokio::fs::try_exists(&self.snapshot_path)
            .await
            .unwrap_or(false)
        {
            match File::open(&self.snapshot_path).await {
                Ok(mut f) => {
                    let mut buf = Vec::new();
                    f.read_to_end(&mut buf).await?;
                    let snap: crate::snapshot::Snapshot = serde_json::from_slice(&buf)
                        .map_err(|e| StoreError::Decode(e.to_string()))?;
                    snap_water = snap.high_water_seq;
                    debug!(
                        concepts = snap.concepts.len(),
                        seq = snap_water,
                        "restoring snapshot",
                    );
                    snap.restore(graph)?;
                }
                Err(e) => warn!(error=%e, "snapshot present but unreadable; skipping"),
            }
        }

        // 2. WAL replay — skip anything already covered by the snapshot.
        let mut high_water = snap_water;
        if !tokio::fs::try_exists(&self.log_path).await.unwrap_or(false) {
            *self.seq.lock() = high_water;
            return Ok(());
        }
        let f = File::open(&self.log_path).await?;
        let file_len = f.metadata().await?.len();
        let mut reader = BufReader::new(f);
        let mut offset: u64 = 0;
        let mut tail = Tail::Clean;
        loop {
            let record_start = offset;
            let mut len_buf = [0u8; 4];
            match reader.read_exact(&mut len_buf).await {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    if record_start < file_len {
                        tail = Tail::Torn {
                            offset: record_start,
                            reason: "length prefix cut short",
                        };
                    }
                    break;
                }
                Err(e) => return Err(e.into()),
            }
            let len = u32::from_be_bytes(len_buf) as u64;
            if record_start + 4 + len > file_len {
                tail = Tail::Torn {
                    offset: record_start,
                    reason: "payload shorter than its length prefix",
                };
                break;
            }
            let mut payload = vec![0u8; len as usize];
            reader.read_exact(&mut payload).await?;
            offset += 4 + len;

            let rec: LogRecord = match serde_json::from_slice(&payload) {
                Ok(r) => r,
                Err(e) if offset == file_len => {
                    // Undecodable *and* last: a torn write whose length prefix
                    // happened to land inside the file. Discard it.
                    tail = Tail::Torn {
                        offset: record_start,
                        reason: "trailing record does not decode",
                    };
                    debug!(error=%e, "discarding undecodable trailing record");
                    break;
                }
                Err(e) => {
                    // Undecodable with more records after it: not a torn tail,
                    // genuine corruption. Refuse to guess.
                    return Err(StoreError::Decode(format!(
                        "record at offset {record_start} is corrupt and is not the last one: {e}"
                    )));
                }
            };
            high_water = high_water.max(rec.seq);
            if rec.seq <= snap_water {
                // Already captured by the snapshot; reapplying would
                // double-add relations and could trip duplicate-name checks.
                continue;
            }
            apply(graph, rec)?;
        }
        drop(reader);

        if let Tail::Torn { offset, reason } = tail {
            warn!(
                path = %self.log_path.display(),
                discarded_bytes = file_len - offset,
                offset,
                reason,
                "torn tail in write-ahead log; truncating to last complete record"
            );
            // A write-only handle (not append) is needed to shrink the file:
            // an append handle only carries FILE_APPEND_DATA on Windows.
            let f = OpenOptions::new().write(true).open(&self.log_path).await?;
            f.set_len(offset).await?;
            f.sync_all().await?;
        }

        *self.seq.lock() = high_water;
        Ok(())
    }

    async fn snapshot(&self, graph: &Arc<OntologyGraph>) -> StoreResult<()> {
        let seq = *self.seq.lock();
        let snap = crate::snapshot::Snapshot::from_graph_with_seq(graph, seq);
        let bytes = serde_json::to_vec(&snap).map_err(|e| StoreError::Encode(e.to_string()))?;
        Self::write_atomic(&self.snapshot_path, &bytes).await
    }

    async fn compact(&self, graph: &Arc<OntologyGraph>) -> StoreResult<()> {
        // Hold the writer lock for the entire compaction so concurrent
        // appends can't slip in between snapshotting and truncating.
        let mut writer = self.writer.lock().await;
        writer.flush().await?;

        let seq = *self.seq.lock();
        let snap = crate::snapshot::Snapshot::from_graph_with_seq(graph, seq);
        let bytes = serde_json::to_vec(&snap).map_err(|e| StoreError::Encode(e.to_string()))?;

        // 1. Snapshot first — fsync + atomic rename. Only once it is durable
        //    is it safe to drop the log that it supersedes.
        Self::write_atomic(&self.snapshot_path, &bytes).await?;

        // 2. Truncate the WAL by reopening the file with `truncate(true)`.
        //    We can't call `set_len(0)` on the existing handle because it
        //    was opened with `append(true)`; on Windows that yields only
        //    `FILE_APPEND_DATA` rights, which forbid SetEndOfFile and
        //    return `ERROR_ACCESS_DENIED`. Reopening sidesteps that.
        let truncated = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&self.log_path)
            .await?;
        truncated.sync_all().await?;
        drop(truncated);
        // Reopen the append handle so subsequent writes target the now
        // empty file (the previous handle still points at the old
        // position past the prior end-of-file on some platforms).
        let fresh = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
            .await?;
        *writer = fresh;

        Ok(())
    }
}
