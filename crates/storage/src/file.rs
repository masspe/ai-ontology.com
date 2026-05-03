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
use std::sync::Arc;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};
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
pub struct FileStore {
    log_path: PathBuf,
    snapshot_path: PathBuf,
    writer: tokio::sync::Mutex<File>,
    seq: Mutex<u64>,
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
        })
    }
}

#[async_trait]
impl Store for FileStore {
    async fn append(&self, record: &LogRecord) -> StoreResult<()> {
        let mut r = record.clone();
        {
            let mut s = self.seq.lock();
            *s += 1;
            r.seq = *s;
        }
        let bytes = serde_json::to_vec(&r).map_err(|e| StoreError::Encode(e.to_string()))?;
        let len = u32::try_from(bytes.len())
            .map_err(|_| StoreError::Encode("record too large".into()))?;
        let mut w = self.writer.lock().await;
        w.write_all(&len.to_be_bytes()).await?;
        w.write_all(&bytes).await?;
        w.flush().await?;
        Ok(())
    }

    async fn load_into(&self, graph: &Arc<OntologyGraph>) -> StoreResult<()> {
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
        let mut reader = BufReader::new(f);
        let mut offset: u64 = 0;
        loop {
            let mut len_buf = [0u8; 4];
            match reader.read_exact(&mut len_buf).await {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e.into()),
            }
            let len = u32::from_be_bytes(len_buf) as usize;
            let mut payload = vec![0u8; len];
            if reader.read_exact(&mut payload).await.is_err() {
                return Err(StoreError::Corrupt(offset));
            }
            offset += 4 + len as u64;

            let rec: LogRecord =
                serde_json::from_slice(&payload).map_err(|e| StoreError::Decode(e.to_string()))?;
            high_water = high_water.max(rec.seq);
            if rec.seq <= snap_water {
                // Already captured by the snapshot; reapplying would
                // double-add relations and could trip duplicate-name checks.
                continue;
            }
            apply(graph, rec)?;
        }
        *self.seq.lock() = high_water;
        Ok(())
    }

    async fn snapshot(&self, graph: &Arc<OntologyGraph>) -> StoreResult<()> {
        let seq = *self.seq.lock();
        let snap = crate::snapshot::Snapshot::from_graph_with_seq(graph, seq);
        let bytes = serde_json::to_vec(&snap).map_err(|e| StoreError::Encode(e.to_string()))?;
        let tmp = self.snapshot_path.with_extension("snap.tmp");
        tokio::fs::write(&tmp, &bytes).await?;
        tokio::fs::rename(&tmp, &self.snapshot_path).await?;
        Ok(())
    }

    async fn compact(&self, graph: &Arc<OntologyGraph>) -> StoreResult<()> {
        // Hold the writer lock for the entire compaction so concurrent
        // appends can't slip in between snapshotting and truncating.
        let mut writer = self.writer.lock().await;
        writer.flush().await?;

        let seq = *self.seq.lock();
        let snap = crate::snapshot::Snapshot::from_graph_with_seq(graph, seq);
        let bytes = serde_json::to_vec(&snap).map_err(|e| StoreError::Encode(e.to_string()))?;

        // 1. Snapshot first — atomic via temp+rename.
        let tmp = self.snapshot_path.with_extension("snap.tmp");
        tokio::fs::write(&tmp, &bytes).await?;
        tokio::fs::rename(&tmp, &self.snapshot_path).await?;

        // 2. Truncate the WAL. After this point, replay only sees records
        //    with seq > high_water_seq (i.e. none, until the next append).
        writer.set_len(0).await?;
        writer.seek(std::io::SeekFrom::Start(0)).await?;
        writer.flush().await?;

        Ok(())
    }
}
