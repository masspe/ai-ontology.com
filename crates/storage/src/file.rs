use async_trait::async_trait;
use ontology_graph::OntologyGraph;
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tracing::{debug, warn};

use crate::log::LogRecord;
use crate::memory::apply;
use crate::store::{Store, StoreError, StoreResult};

/// Append-only log file with a sibling bincode snapshot for fast cold start.
///
/// Wire format per record:
///
/// ```text
/// [u32 length BE] [bincode-encoded LogRecord]
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
        let bytes = bincode::serialize(&r).map_err(|e| StoreError::Encode(e.to_string()))?;
        let len = u32::try_from(bytes.len()).map_err(|_| StoreError::Encode("record too large".into()))?;
        let mut w = self.writer.lock().await;
        w.write_all(&len.to_be_bytes()).await?;
        w.write_all(&bytes).await?;
        w.flush().await?;
        Ok(())
    }

    async fn load_into(&self, graph: &Arc<OntologyGraph>) -> StoreResult<()> {
        // 1. Snapshot, if any.
        let mut high_water: u64 = 0;
        if tokio::fs::try_exists(&self.snapshot_path).await.unwrap_or(false) {
            match File::open(&self.snapshot_path).await {
                Ok(mut f) => {
                    let mut buf = Vec::new();
                    f.read_to_end(&mut buf).await?;
                    let snap: crate::snapshot::Snapshot = bincode::deserialize(&buf)
                        .map_err(|e| StoreError::Decode(e.to_string()))?;
                    debug!(concepts = snap.concepts.len(), "restoring snapshot");
                    snap.restore(graph)?;
                    // We don't currently track snapshot seq; treat it as floor 0.
                }
                Err(e) => warn!(error=%e, "snapshot present but unreadable; skipping"),
            }
        }

        // 2. WAL replay.
        if !tokio::fs::try_exists(&self.log_path).await.unwrap_or(false) {
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

            let rec: LogRecord = bincode::deserialize(&payload)
                .map_err(|e| StoreError::Decode(e.to_string()))?;
            high_water = high_water.max(rec.seq);
            apply(graph, rec)?;
        }
        *self.seq.lock() = high_water;
        Ok(())
    }

    async fn snapshot(&self, graph: &Arc<OntologyGraph>) -> StoreResult<()> {
        let snap = crate::snapshot::Snapshot::from_graph(graph);
        let bytes = bincode::serialize(&snap).map_err(|e| StoreError::Encode(e.to_string()))?;
        let tmp = self.snapshot_path.with_extension("snap.tmp");
        tokio::fs::write(&tmp, &bytes).await?;
        tokio::fs::rename(&tmp, &self.snapshot_path).await?;
        Ok(())
    }
}
