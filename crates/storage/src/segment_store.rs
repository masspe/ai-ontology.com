// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! [`SegmentStore`]: the binary, segmented store of `STORAGE.md` §4, with
//! two streams — `meta` (ontology, rules, actions) and `graph/default`
//! (concepts, relations) — until partitioning by `ns` lands in phase 3.
//!
//! Layout under the store root:
//!
//! ```text
//! MANIFEST.json
//! LOCK
//! meta/000001.data|.idx
//! graph/default/000002.data|.idx
//! ```
//!
//! Write path (R8 from the caller's side, §7.2 group commit here): a batch
//! is encoded, appended to the streams it touches, then **one** `fdatasync`
//! per touched stream. Sequence numbers are store-wide and monotonic (H12).
//! Read path in P0 (D4): every record of every segment, CRC-verified,
//! decoded and applied to the graph through the public mutation methods.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use ontology_graph::OntologyGraph;
use parking_lot::Mutex;
use tracing::{info, warn};

use crate::log::{LogRecord, RecordKind};
use crate::manifest::{Manifest, DEFAULT_NS_ID, META_NS_ID};
use crate::memory::apply;
use crate::segment::{IndexFields, Kind, RecordMeta, CODEC_JSON, FORMAT_VERSION};
use crate::store::{Store, StoreError, StoreResult};
use crate::stream::{RollPolicy, Stream, StreamOpenReport, StreamReadError};

pub const LOCK_FILE: &str = "LOCK";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentStoreConfig {
    pub roll: RollPolicy,
    pub codec: u8,
}

impl Default for SegmentStoreConfig {
    fn default() -> Self {
        Self {
            roll: RollPolicy::default(),
            codec: CODEC_JSON,
        }
    }
}

/// What opening the store found and repaired; logged at `info`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenReport {
    pub created: bool,
    pub meta: StreamOpenReport,
    pub graph: StreamOpenReport,
    pub next_seq: u64,
    pub manifest_rewritten: bool,
}

struct Inner {
    root: PathBuf,
    manifest: Manifest,
    meta: Stream,
    graph: Stream,
    next_seq: u64,
    /// Set after a batch failed part-way: the buffered tail is in an unknown
    /// state, so every further append is refused until restart, where
    /// recovery truncates whatever is torn (see `StoreError::Poisoned`).
    poisoned: bool,
    /// Held for the life of the store (H17: single writer per store).
    _lock: File,
}

pub struct SegmentStore {
    inner: Arc<Mutex<Inner>>,
    root: PathBuf,
    syncs: Arc<AtomicU64>,
    report: OpenReport,
}

impl SegmentStore {
    pub async fn open(root: impl AsRef<Path>) -> StoreResult<Self> {
        Self::open_with(root, SegmentStoreConfig::default()).await
    }

    pub async fn open_with(root: impl AsRef<Path>, cfg: SegmentStoreConfig) -> StoreResult<Self> {
        let root = root.as_ref().to_path_buf();
        tokio::task::spawn_blocking(move || Self::open_sync(&root, cfg))
            .await
            .map_err(|e| StoreError::Io(std::io::Error::other(e)))?
    }

    /// Synchronous open: lock, manifest, recover both streams, resume
    /// `next_seq`, reconcile the manifest with the directory.
    pub fn open_sync(root: &Path, cfg: SegmentStoreConfig) -> StoreResult<Self> {
        std::fs::create_dir_all(root)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.join(LOCK_FILE))?;
        if let Err(e) = lock.try_lock() {
            return Err(StoreError::Locked(format!(
                "{} ({e})",
                root.join(LOCK_FILE).display()
            )));
        }

        let existing = Manifest::load(root)?;
        let created = existing.is_none();
        let mut manifest = existing.unwrap_or_else(|| Manifest::new(cfg.codec));
        if manifest.format_version != FORMAT_VERSION {
            return Err(StoreError::Format(format!(
                "store format version {} not readable by this build ({FORMAT_VERSION})",
                manifest.format_version
            )));
        }
        let before = manifest.clone();

        // Recovery needs to resolve payloads to index fields, which means
        // interning relation types; work on a copy of the symbol table and
        // fold it back afterwards.
        let mut next_partition = manifest.next_partition_id;
        let mut syms = manifest.clone();
        let (meta, meta_report, graph, graph_report) = {
            let mut resolve = |kind: Kind, payload: &[u8]| -> Result<IndexFields, String> {
                let rk: RecordKind = serde_json::from_slice(payload).map_err(|e| e.to_string())?;
                let m = RecordMeta::of(&rk);
                if m.kind != kind {
                    return Err(format!("header kind {kind:?} but payload is {:?}", m.kind));
                }
                let sym = m
                    .relation_type
                    .map(|rt| syms.intern_relation_type(rt).0)
                    .unwrap_or(0);
                Ok(IndexFields {
                    ns_id: if kind.is_meta() {
                        META_NS_ID
                    } else {
                        DEFAULT_NS_ID
                    },
                    entity_id: m.entity_id,
                    endpoints: m.endpoints,
                    rtype_sym: sym,
                })
            };
            let meta_dir = root.join(&manifest.stream(META_NS_ID).unwrap().dir);
            let graph_dir = root.join(&manifest.stream(DEFAULT_NS_ID).unwrap().dir);
            let (meta, meta_report) = Stream::open(
                &meta_dir,
                META_NS_ID,
                manifest.codec,
                cfg.roll,
                &mut next_partition,
                1,
                &mut resolve,
            )?;
            let (graph, graph_report) = Stream::open(
                &graph_dir,
                DEFAULT_NS_ID,
                manifest.codec,
                cfg.roll,
                &mut next_partition,
                1,
                &mut resolve,
            )?;
            (meta, meta_report, graph, graph_report)
        };
        let (mut meta, mut graph) = (meta, graph);
        manifest.relation_types = syms.relation_types;
        manifest.next_partition_id = next_partition;

        let next_seq = meta
            .last_seq()
            .into_iter()
            .chain(graph.last_seq())
            .max()
            .map(|s| s + 1)
            .unwrap_or(1);

        // The directory is the truth for sealed partitions; the manifest
        // caches their zone maps.
        manifest.stream_mut(META_NS_ID).unwrap().sealed = meta.sealed_entries();
        manifest.stream_mut(DEFAULT_NS_ID).unwrap().sealed = graph.sealed_entries();
        let manifest_rewritten = created || manifest != before;
        if manifest_rewritten {
            manifest.save(root)?;
        }
        // Make sure both active segments exist on disk even for an empty
        // store (they do: Stream::open creates them).
        let _ = (&mut meta, &mut graph);

        let report = OpenReport {
            created,
            meta: meta_report,
            graph: graph_report,
            next_seq,
            manifest_rewritten,
        };
        info!(
            root = %root.display(),
            created,
            next_seq,
            meta_records = meta.record_count(),
            graph_records = graph.record_count(),
            sealed = manifest.sealed_records(),
            "segment store opened"
        );

        Ok(Self {
            inner: Arc::new(Mutex::new(Inner {
                root: root.to_path_buf(),
                manifest,
                meta,
                graph,
                next_seq,
                poisoned: false,
                _lock: lock,
            })),
            root: root.to_path_buf(),
            syncs: Arc::new(AtomicU64::new(0)),
            report,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    /// Number of `fdatasync` calls issued on data files since open: one per
    /// stream touched by each batch.
    pub fn sync_count(&self) -> u64 {
        self.syncs.load(Ordering::Relaxed)
    }
    pub fn open_report(&self) -> &OpenReport {
        &self.report
    }
    /// `true` once a batch failed part-way; every append is refused since.
    pub fn is_poisoned(&self) -> bool {
        self.inner.lock().poisoned
    }
    /// Test hook: simulate a failed batch.
    #[doc(hidden)]
    pub fn poison_for_test(&self) {
        self.inner.lock().poisoned = true;
    }

    pub fn manifest(&self) -> Manifest {
        self.inner.lock().manifest.clone()
    }
    /// Next store-wide sequence number.
    pub fn next_seq(&self) -> u64 {
        self.inner.lock().next_seq
    }
    /// Records in the store, all streams and segments.
    pub fn record_count(&self) -> u64 {
        let g = self.inner.lock();
        g.meta.record_count() + g.graph.record_count()
    }

    fn now_micros() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0)
    }

    /// Encode and route the whole batch first (nothing touches a file until
    /// every record is known to be writable), then append, then one sync per
    /// touched stream **in the order the streams were first touched** — so a
    /// crash between two syncs leaves a durable *prefix* of the batch in seq
    /// order — then a roll check. Any failure after the first append poisons
    /// the store. Returns the number of syncs issued.
    fn commit_batch(inner: &mut Inner, records: &[LogRecord]) -> StoreResult<u64> {
        if records.is_empty() {
            return Ok(0);
        }
        if inner.poisoned {
            return Err(StoreError::Poisoned(inner.root.display().to_string()));
        }
        // 1. Encode + route without side effects on the streams.
        let mut planned: Vec<(u16, Kind, Vec<u8>, IndexFields)> = Vec::with_capacity(records.len());
        for r in records {
            let payload =
                serde_json::to_vec(&r.kind).map_err(|e| StoreError::Encode(e.to_string()))?;
            let meta = RecordMeta::of(&r.kind);
            let rtype_sym = match meta.relation_type {
                Some(rt) => {
                    let (sym, fresh) = inner.manifest.intern_relation_type(rt);
                    if fresh {
                        // The symbol must be on disk before any record that
                        // uses it (D2) — rare: once per relation type.
                        inner.manifest.save(&inner.root)?;
                    }
                    sym
                }
                None => 0,
            };
            let ns_id = if meta.kind.is_meta() {
                META_NS_ID
            } else {
                DEFAULT_NS_ID
            };
            planned.push((
                ns_id,
                meta.kind,
                payload,
                IndexFields {
                    ns_id,
                    entity_id: meta.entity_id,
                    endpoints: meta.endpoints,
                    rtype_sym,
                },
            ));
        }
        // 2. Append + sync; from here on a failure poisons the store.
        match Self::write_planned(inner, planned) {
            Ok(n) => Ok(n),
            Err(e) => {
                inner.poisoned = true;
                warn!(error = %e, root = %inner.root.display(), "batch failed part-way; store poisoned until restart");
                Err(e)
            }
        }
    }

    fn write_planned(
        inner: &mut Inner,
        planned: Vec<(u16, Kind, Vec<u8>, IndexFields)>,
    ) -> StoreResult<u64> {
        let ts = Self::now_micros();
        let mut touched: Vec<u16> = Vec::new();
        for (ns_id, kind, payload, fields) in &planned {
            let seq = inner.next_seq;
            let stream = if *ns_id == META_NS_ID {
                &mut inner.meta
            } else {
                &mut inner.graph
            };
            stream.append(seq, ts, *kind, payload, *fields)?;
            inner.next_seq += 1;
            if !touched.contains(ns_id) {
                touched.push(*ns_id);
            }
        }
        let mut syncs = 0;
        let mut manifest_dirty = false;
        let next_seq = inner.next_seq;
        for ns_id in touched {
            let stream = if ns_id == META_NS_ID {
                &mut inner.meta
            } else {
                &mut inner.graph
            };
            stream.commit()?;
            syncs += 1;
            if let Some(entry) =
                stream.maybe_roll(&mut inner.manifest.next_partition_id, next_seq)?
            {
                inner
                    .manifest
                    .stream_mut(ns_id)
                    .expect("stream declared in manifest")
                    .sealed
                    .push(entry);
                manifest_dirty = true;
            }
        }
        if manifest_dirty {
            inner.manifest.save(&inner.root)?;
        }
        Ok(syncs)
    }

    /// Decode one on-disk record into a `LogRecord`, naming its location on
    /// failure.
    fn decode(
        ns: u16,
        partition: u32,
        v: &crate::segment::RecordView<'_>,
    ) -> StoreResult<LogRecord> {
        if v.header.codec != CODEC_JSON {
            return Err(StoreError::Format(format!(
                "ns {ns} partition {partition} seq {}: codec {} not supported",
                v.header.seq, v.header.codec
            )));
        }
        let kind: RecordKind = serde_json::from_slice(v.payload).map_err(|e| {
            StoreError::Decode(format!(
                "ns {ns} partition {partition} seq {}: {e}",
                v.header.seq
            ))
        })?;
        Ok(LogRecord {
            seq: v.header.seq,
            kind,
        })
    }

    fn read_error(ns: u16, e: StreamReadError<StoreError>) -> StoreError {
        match e {
            StreamReadError::Visitor(e) => e,
            StreamReadError::Io(e) => StoreError::Io(e),
            StreamReadError::Format { partition, error } => {
                StoreError::Corrupt(format!("ns {ns} partition {partition}: {error}"))
            }
        }
    }

    /// Replay every record in **global `seq` order** (H12). Streams are
    /// independent files, so the order across them is a merge: a rule in
    /// `meta` may reference concepts written to `graph` just before it, and
    /// an ontology change in `meta` must precede the concepts that rely on
    /// it. `meta` is small (H3), so it is buffered and merged into the
    /// graph stream's sequential scan.
    fn hydrate(inner: &mut Inner, graph: &Arc<OntologyGraph>) -> StoreResult<u64> {
        let meta_ns = inner.meta.ns_id();
        let mut meta: Vec<LogRecord> = Vec::new();
        inner
            .meta
            .for_each_record(true, |partition, v| -> StoreResult<()> {
                meta.push(Self::decode(meta_ns, partition, &v)?);
                Ok(())
            })
            .map_err(|e| Self::read_error(meta_ns, e))?;

        let mut applied = 0u64;
        let mut next_meta = 0usize;
        let graph_ns = inner.graph.ns_id();
        inner
            .graph
            .for_each_record(true, |partition, v| -> StoreResult<()> {
                let rec = Self::decode(graph_ns, partition, &v)?;
                while next_meta < meta.len() && meta[next_meta].seq < rec.seq {
                    apply(graph, meta[next_meta].clone())?;
                    next_meta += 1;
                    applied += 1;
                }
                apply(graph, rec)?;
                applied += 1;
                Ok(())
            })
            .map_err(|e| Self::read_error(graph_ns, e))?;
        for rec in meta.drain(next_meta..) {
            apply(graph, rec)?;
            applied += 1;
        }
        Ok(applied)
    }
}

#[async_trait]
impl Store for SegmentStore {
    async fn append(&self, record: &LogRecord) -> StoreResult<()> {
        self.append_batch(std::slice::from_ref(record)).await
    }

    async fn append_batch(&self, records: &[LogRecord]) -> StoreResult<()> {
        if records.is_empty() {
            return Ok(());
        }
        let inner = self.inner.clone();
        let syncs = self.syncs.clone();
        let records = records.to_vec();
        tokio::task::spawn_blocking(move || {
            let mut g = inner.lock();
            let n = Self::commit_batch(&mut g, &records)?;
            syncs.fetch_add(n, Ordering::Relaxed);
            Ok(())
        })
        .await
        .map_err(|e| StoreError::Io(std::io::Error::other(e)))?
    }

    async fn load_into(&self, graph: &Arc<OntologyGraph>) -> StoreResult<()> {
        let inner = self.inner.clone();
        let graph = graph.clone();
        let applied = tokio::task::spawn_blocking(move || {
            let mut g = inner.lock();
            Self::hydrate(&mut g, &graph)
        })
        .await
        .map_err(|e| StoreError::Io(std::io::Error::other(e)))??;
        info!(records = applied, "segment store hydrated");
        Ok(())
    }

    async fn snapshot(&self, _graph: &Arc<OntologyGraph>) -> StoreResult<()> {
        // Every acknowledged write is already durable; there is nothing a
        // snapshot would add. Compaction (phase 3) replaces it.
        Ok(())
    }

    async fn compact(&self, _graph: &Arc<OntologyGraph>) -> StoreResult<()> {
        warn!(
            "compaction of the segment store arrives with per-domain partitioning (phase 3); no-op"
        );
        Ok(())
    }
}

impl std::fmt::Debug for SegmentStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SegmentStore")
            .field("root", &self.root)
            .field("syncs", &self.sync_count())
            .finish_non_exhaustive()
    }
}
