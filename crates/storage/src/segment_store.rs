// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! [`SegmentStore`]: the binary, segmented, **domain-partitioned** store of
//! `STORAGE.md` §4-§5.
//!
//! Layout under the store root:
//!
//! ```text
//! MANIFEST.json
//! LOCK
//! meta/000001.data|.idx                 ontology, rules, actions (ns 0, D3)
//! graph/default/000002.data|.idx|.xref  one stream per domain (ns_of_type, H13)
//! graph/<ns>/…
//! ```
//!
//! Write path (R8 from the caller's side, §7.2 group commit here): each
//! record is routed to a stream by the **current ontology** — concepts by
//! their type's domain, relations by their type's source domain, tombstones
//! by the caller's [`RouteHint`] — appended, then **one** `fdatasync` per
//! touched stream. Sequence numbers are store-wide and monotonic (H12).
//!
//! Read path in P0 (D4): every stream is scanned sequentially and the
//! streams are merged by `seq`, so replay order is the write order whatever
//! the number of domains. Selective hydration loads only the requested
//! domains; a relation whose other endpoint lives in a domain that is not
//! loaded is skipped and counted.
//!
//! Compaction rewrites the **whole store** from the live graph, in
//! dependency order, into fresh partitions, verifies the result by replay,
//! then swaps. Per-domain compaction is not offered: with seq-ordered
//! replay, rewriting one domain would put its records after the rules and
//! cross-domain relations that depend on them (STORAGE-PLAN.md §5.6).

use std::collections::{BTreeMap, HashSet};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use ontology_graph::{GraphError, Ontology, OntologyGraph};
use parking_lot::Mutex;
use tracing::{info, warn};

use crate::codec::{self, CODEC_JSON};
use crate::log::{LogRecord, RecordKind, RouteHint};
use crate::manifest::{CompactionMarker, META_NS};
use crate::manifest::{Manifest, META_NS_ID};
use crate::memory::apply;
use crate::segment::active::{move_segment_files, remove_segment_files};
use crate::segment::xref::{read_xref, XrefEntry};
use crate::segment::{
    unpack_endpoints, ActiveSegment, IndexFields, Kind, RecordMeta, RecordView, SealedSegment,
    FORMAT_VERSION,
};
use crate::store::{Store, StoreError, StoreResult};
use crate::stream::{RollPolicy, SnapshotCursor, Stream, StreamOpenReport, StreamSnapshot};

pub const LOCK_FILE: &str = "LOCK";

/// One record ready to append: `(ns_id, kind, payload, index fields)`.
type Planned = (u16, Kind, Vec<u8>, IndexFields);

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
    /// One report per graph domain, by `ns_id`.
    pub graph: BTreeMap<u16, StreamOpenReport>,
    pub next_seq: u64,
    pub manifest_rewritten: bool,
    /// `.xref` entries materialized at open.
    pub xref_entries: usize,
}

/// Outcome of a hydration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HydrationReport {
    pub applied: u64,
    /// Domains loaded (`None` = all).
    pub domains: Option<Vec<String>>,
    /// Relation, rule and action records skipped because a concept they
    /// reference lives in a domain that was not loaded (selective
    /// hydration only).
    pub skipped_cross_domain: u64,
}

/// Outcome of a compaction.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompactionReport {
    pub records_before: u64,
    pub records_after: u64,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub partitions_removed: usize,
}

struct Inner {
    root: PathBuf,
    cfg: SegmentStoreConfig,
    manifest: Manifest,
    meta: Stream,
    graph: BTreeMap<u16, Stream>,
    /// Latest schema seen on the `meta` stream — the router (H13, H14).
    ontology: Ontology,
    next_seq: u64,
    /// Set after a batch failed part-way: the buffered tail is in an unknown
    /// state, so every further append is refused until restart, where
    /// recovery truncates whatever is torn (see `StoreError::Poisoned`).
    poisoned: bool,
    /// Held for the life of the store (H17: single writer per store).
    _lock: File,
}

impl Inner {
    fn stream_mut(&mut self, ns_id: u16) -> &mut Stream {
        if ns_id == META_NS_ID {
            &mut self.meta
        } else {
            self.graph.get_mut(&ns_id).expect("stream exists for ns")
        }
    }
    fn stream_ref(&self, ns_id: u16) -> &Stream {
        if ns_id == META_NS_ID {
            &self.meta
        } else {
            &self.graph[&ns_id]
        }
    }
    fn stream_dir(&self, ns_id: u16) -> PathBuf {
        if ns_id == META_NS_ID {
            self.meta.dir().to_path_buf()
        } else {
            self.graph[&ns_id].dir().to_path_buf()
        }
    }
    fn total_records(&self) -> u64 {
        self.meta.record_count() + self.graph.values().map(|s| s.record_count()).sum::<u64>()
    }
    fn total_bytes(&self) -> u64 {
        self.meta.data_bytes() + self.graph.values().map(|s| s.data_bytes()).sum::<u64>()
    }
}

pub struct SegmentStore {
    inner: Arc<Mutex<Inner>>,
    root: PathBuf,
    syncs: Arc<AtomicU64>,
    report: OpenReport,
    last_hydration: Arc<Mutex<Option<HydrationReport>>>,
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

    /// Synchronous open: lock, manifest, recover every stream, resume
    /// `next_seq`, rebuild `.xref`s, reconcile the manifest.
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
        // A compaction interrupted after its commit point is finished now,
        // before any stream is opened; one aborted before it is discarded.
        finish_compaction(root, &mut manifest)?;
        let before = manifest.clone();
        let mut next_partition = manifest.next_partition_id;

        // 1. meta: no relation types to intern, no target domains.
        let meta_dir = root.join(&manifest.stream(META_NS_ID).unwrap().dir);
        let (mut meta, meta_report) = {
            let mut resolve =
                |kind: Kind, codec: u8, payload: &[u8]| -> Result<IndexFields, String> {
                    let rk: RecordKind =
                        codec::decode(codec, payload).map_err(|e| e.to_string())?;
                    let m = RecordMeta::of(&rk);
                    if m.kind != kind {
                        return Err(format!("header kind {kind:?} but payload is {:?}", m.kind));
                    }
                    Ok(IndexFields {
                        ns_id: META_NS_ID,
                        entity_id: m.entity_id,
                        endpoints: 0,
                        rtype_sym: 0,
                        target_ns_id: 0,
                    })
                };
            Stream::open(
                &meta_dir,
                META_NS_ID,
                manifest.codec,
                cfg.roll,
                &mut next_partition,
                1,
                &mut resolve,
            )?
        };

        // 2. The router is the latest ontology on disk.
        let ontology = latest_ontology(&mut meta)?.unwrap_or_default();

        // 3. Graph streams, one per live domain in the manifest.
        let mut graph: BTreeMap<u16, Stream> = BTreeMap::new();
        let mut graph_reports = BTreeMap::new();
        let mut syms = manifest.clone();
        for ns_id in manifest.graph_ns_ids() {
            let dir = root.join(&manifest.stream(ns_id).unwrap().dir);
            let (stream, report) = {
                let onto = &ontology;
                let lookup = &manifest;
                let mut resolve = |kind: Kind,
                                   codec: u8,
                                   payload: &[u8]|
                 -> Result<IndexFields, String> {
                    let rk: RecordKind =
                        codec::decode(codec, payload).map_err(|e| e.to_string())?;
                    let m = RecordMeta::of(&rk);
                    if m.kind != kind {
                        return Err(format!("header kind {kind:?} but payload is {:?}", m.kind));
                    }
                    let (sym, target_ns_id) = match m.relation_type {
                        Some(rt) => {
                            let sym = syms.intern_relation_type(rt).0;
                            let target = onto
                                .ns_of_relation_type(rt)
                                .ok()
                                .map(|(_, dst)| dst.to_string())
                                .and_then(|dst| lookup.ns_id(&dst))
                                .filter(|id| *id != ns_id)
                                .unwrap_or(0);
                            (sym, target)
                        }
                        None => (0, 0),
                    };
                    Ok(IndexFields {
                        ns_id,
                        entity_id: m.entity_id,
                        endpoints: m.endpoints,
                        rtype_sym: sym,
                        target_ns_id,
                    })
                };
                Stream::open(
                    &dir,
                    ns_id,
                    manifest.codec,
                    cfg.roll,
                    &mut next_partition,
                    1,
                    &mut resolve,
                )?
            };
            graph.insert(ns_id, stream);
            graph_reports.insert(ns_id, report);
        }
        manifest.relation_types = syms.relation_types;
        manifest.next_partition_id = next_partition;

        let next_seq = meta
            .last_seq()
            .into_iter()
            .chain(graph.values().filter_map(|s| s.last_seq()))
            .max()
            .map(|s| s + 1)
            .unwrap_or(1);

        // The directory is the truth for sealed partitions; the manifest
        // caches their zone maps.
        manifest.stream_mut(META_NS_ID).unwrap().sealed = meta.sealed_entries();
        for (ns_id, s) in graph.iter() {
            manifest.stream_mut(*ns_id).unwrap().sealed = s.sealed_entries();
        }
        let manifest_rewritten = created || manifest != before;
        if manifest_rewritten {
            manifest.save(root)?;
        }

        let mut inner = Inner {
            root: root.to_path_buf(),
            cfg,
            manifest,
            meta,
            graph,
            ontology,
            next_seq,
            poisoned: false,
            _lock: lock,
        };
        let xref_entries = rebuild_xrefs(&mut inner)?;

        let report = OpenReport {
            created,
            meta: meta_report,
            graph: graph_reports,
            next_seq,
            manifest_rewritten,
            xref_entries,
        };
        info!(
            root = %root.display(),
            created,
            next_seq,
            domains = inner.graph.len(),
            meta_records = inner.meta.record_count(),
            graph_records = inner.graph.values().map(|s| s.record_count()).sum::<u64>(),
            xref_entries,
            "segment store opened"
        );

        Ok(Self {
            inner: Arc::new(Mutex::new(inner)),
            root: root.to_path_buf(),
            syncs: Arc::new(AtomicU64::new(0)),
            report,
            last_hydration: Arc::new(Mutex::new(None)),
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

    /// Codec new records are written with (`MANIFEST.codec`).
    pub fn codec(&self) -> u8 {
        self.inner.lock().manifest.codec
    }

    /// Whole-store compaction that also switches the write codec: the
    /// rewritten segments are in `codec`, and so is every later append.
    /// Reading never depended on the manifest's codec (each record header
    /// carries its own), so a store is readable at every step.
    pub async fn compact_with_codec(
        &self,
        graph: &Arc<OntologyGraph>,
        codec: u8,
    ) -> StoreResult<CompactionReport> {
        if !codec::is_known(codec) {
            return Err(StoreError::Format(format!(
                "codec {codec} not supported by this build"
            )));
        }
        let inner = self.inner.clone();
        let graph = graph.clone();
        tokio::task::spawn_blocking(move || {
            let mut g = inner.lock();
            if g.poisoned {
                return Err(StoreError::Poisoned(g.root.display().to_string()));
            }
            let before = g.manifest.codec;
            g.manifest.codec = codec;
            match Self::compact_all(&mut g, &graph) {
                Ok(report) => {
                    info!(
                        from = codec::codec_name(before),
                        to = codec::codec_name(codec),
                        "store codec switched"
                    );
                    Ok(report)
                }
                Err(e) => {
                    // compact_all left the old segments in place; keep the
                    // old codec so the next rolls stay consistent with them.
                    g.manifest.codec = before;
                    let _ = g.manifest.save(&g.root);
                    Err(e)
                }
            }
        })
        .await
        .map_err(|e| StoreError::Io(std::io::Error::other(e)))?
    }

    /// Visit every record of the store, decoded, in global `seq` order
    /// (H12), without touching any graph. Returns `(records, payload bytes)`.
    /// This is the read path minus `apply`: the bench uses it to split
    /// hydration time between decoding and index building.
    pub fn scan_records(&self, mut visit: impl FnMut(&LogRecord)) -> StoreResult<(u64, u64)> {
        let mut g = self.inner.lock();
        let mut snapshots = vec![g.meta.snapshot()?];
        let ns_ids: Vec<u16> = g.graph.keys().copied().collect();
        for ns_id in ns_ids {
            snapshots.push(g.stream_mut(ns_id).snapshot()?);
        }
        drop(g);
        let mut cursors: Vec<SnapshotCursor<'_>> =
            snapshots.iter().map(|s| s.cursor(true)).collect();
        let mut heads: Vec<Option<(u64, LogRecord)>> = Vec::with_capacity(cursors.len());
        for (i, c) in cursors.iter_mut().enumerate() {
            heads.push(next_decoded_sized(snapshots[i].ns_id, c)?);
        }
        let (mut records, mut bytes) = (0u64, 0u64);
        loop {
            let mut best: Option<usize> = None;
            for (i, h) in heads.iter().enumerate() {
                if let Some((_, r)) = h {
                    if best.is_none_or(|b| r.seq < heads[b].as_ref().unwrap().1.seq) {
                        best = Some(i);
                    }
                }
            }
            let Some(i) = best else { break };
            let (len, rec) = heads[i].take().unwrap();
            visit(&rec);
            records += 1;
            bytes += len;
            heads[i] = next_decoded_sized(snapshots[i].ns_id, &mut cursors[i])?;
        }
        Ok((records, bytes))
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
        self.inner.lock().total_records()
    }
    /// Records per domain name (`meta` included).
    pub fn record_counts_by_domain(&self) -> BTreeMap<String, u64> {
        let g = self.inner.lock();
        let mut out = BTreeMap::new();
        out.insert("meta".to_string(), g.meta.record_count());
        for (ns_id, s) in g.graph.iter() {
            let name = g.manifest.ns_name(*ns_id).unwrap_or("?").to_string();
            out.insert(name, s.record_count());
        }
        out
    }
    /// Incoming cross-domain edges recorded for domain `ns` (from its
    /// `.xref` files), by partition.
    pub fn xrefs(&self, ns: &str) -> StoreResult<Vec<(u32, Vec<XrefEntry>)>> {
        let g = self.inner.lock();
        let id = g
            .manifest
            .ns_id(ns)
            .ok_or_else(|| StoreError::Format(format!("unknown domain `{ns}`")))?;
        let stream = g
            .graph
            .get(&id)
            .ok_or_else(|| StoreError::Format(format!("domain `{ns}` has no stream")))?;
        let mut out = Vec::new();
        for pid in stream.partition_ids() {
            out.push((pid, read_xref(stream.dir(), pid)?));
        }
        Ok(out)
    }
    /// Report of the most recent `load_into` / `load_domains`.
    pub fn last_hydration(&self) -> Option<HydrationReport> {
        self.last_hydration.lock().clone()
    }

    fn now_micros() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0)
    }

    /// Make sure domain `name` has a frozen id and an open stream; persists
    /// the manifest first when the domain is new (R10: the id must be on
    /// disk before any record refers to it).
    fn ensure_domain(inner: &mut Inner, name: &str) -> StoreResult<u16> {
        if name == META_NS {
            return Err(StoreError::Format(format!(
                "`{META_NS}` is the schema stream, not a graph domain"
            )));
        }
        let (id, fresh) = inner.manifest.intern_ns(name);
        if fresh {
            inner.manifest.save(&inner.root)?;
        }
        if !inner.graph.contains_key(&id) {
            let dir = inner.root.join(&inner.manifest.stream(id).unwrap().dir);
            let mut next_partition = inner.manifest.next_partition_id;
            let mut resolve = |_k: Kind, _c: u8, _p: &[u8]| -> Result<IndexFields, String> {
                Err("fresh domain stream has no records to recover".into())
            };
            let (stream, _) = Stream::open(
                &dir,
                id,
                inner.manifest.codec,
                inner.cfg.roll,
                &mut next_partition,
                inner.next_seq,
                &mut resolve,
            )?;
            inner.manifest.next_partition_id = next_partition;
            inner.manifest.save(&inner.root)?;
            inner.graph.insert(id, stream);
            info!(ns = name, ns_id = id, "domain stream created");
        }
        Ok(id)
    }

    /// Stream and target domain for one record, according to the current
    /// ontology and the record's routing hint.
    fn route(inner: &mut Inner, r: &LogRecord, meta: &RecordMeta<'_>) -> StoreResult<(u16, u16)> {
        if meta.kind.is_meta() {
            return Ok((META_NS_ID, 0));
        }
        let unroutable = |what: &str| {
            StoreError::Format(format!(
                "{what} record cannot be routed to a domain: no routing hint \
                 (use LogRecord::delete_* with the entity's type)"
            ))
        };
        match &r.kind {
            RecordKind::Concept(c) | RecordKind::UpdateConcept(c) => {
                let ns = inner.ontology.ns_of_type(&c.concept_type).to_string();
                Ok((Self::ensure_domain(inner, &ns)?, 0))
            }
            RecordKind::Relation(rel)
            | RecordKind::UpdateRelation(rel)
            | RecordKind::RelationExact(rel) => {
                Self::route_relation_type(inner, &rel.relation_type)
            }
            RecordKind::DeleteConcept(_) => match &r.route {
                Some(RouteHint::ConceptType(t)) => {
                    let ns = inner.ontology.ns_of_type(t).to_string();
                    Ok((Self::ensure_domain(inner, &ns)?, 0))
                }
                _ => Err(unroutable("DeleteConcept")),
            },
            RecordKind::DeleteRelation(_) => match &r.route {
                Some(RouteHint::RelationType(t)) => {
                    let (src, _) = Self::route_relation_type(inner, t)?;
                    Ok((src, 0))
                }
                _ => Err(unroutable("DeleteRelation")),
            },
            _ => Ok((META_NS_ID, 0)),
        }
    }

    fn route_relation_type(inner: &mut Inner, relation_type: &str) -> StoreResult<(u16, u16)> {
        let (src, dst) = inner
            .ontology
            .ns_of_relation_type(relation_type)
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .map_err(|e| {
                StoreError::Format(format!(
                    "relation type `{relation_type}` unknown to the store's ontology: {e}"
                ))
            })?;
        let src_id = Self::ensure_domain(inner, &src)?;
        let dst_id = if dst == src {
            0
        } else {
            Self::ensure_domain(inner, &dst)?
        };
        Ok((src_id, dst_id))
    }

    /// Encode and route the whole batch first (creating domain streams and
    /// interning symbols as needed, but appending nothing), then append,
    /// then one sync per touched stream **in the order the streams were
    /// first touched** — so a crash between two syncs leaves a durable
    /// *prefix* of the batch in seq order — then a roll check. A routing
    /// failure leaves the store as it was; any failure after the first
    /// append poisons it. Returns the number of syncs issued.
    fn commit_batch(inner: &mut Inner, records: &[LogRecord]) -> StoreResult<u64> {
        if records.is_empty() {
            return Ok(0);
        }
        if inner.poisoned {
            return Err(StoreError::Poisoned(inner.root.display().to_string()));
        }
        // 1. Plan: encode + route. The router (latest ontology) advances as
        //    Ontology records are met so the records after them route by the
        //    new schema; it is restored if planning fails part-way.
        let router_before = inner.ontology.clone();
        let planned = match Self::plan_batch(inner, records) {
            Ok(p) => p,
            Err(e) => {
                inner.ontology = router_before;
                return Err(e);
            }
        };
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

    fn plan_batch(inner: &mut Inner, records: &[LogRecord]) -> StoreResult<Vec<Planned>> {
        let mut planned = Vec::with_capacity(records.len());
        for r in records {
            let payload = codec::encode(inner.manifest.codec, &r.kind)
                .map_err(|e| StoreError::Encode(e.to_string()))?;
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
            let (ns_id, target_ns_id) = Self::route(inner, r, &meta)?;
            planned.push((
                ns_id,
                meta.kind,
                payload,
                IndexFields {
                    ns_id,
                    entity_id: meta.entity_id,
                    endpoints: meta.endpoints,
                    rtype_sym,
                    target_ns_id,
                },
            ));
            // A schema record changes the router for the records after it.
            if let RecordKind::Ontology(o) = &r.kind {
                inner.ontology = o.clone();
            }
        }
        Ok(planned)
    }

    fn write_planned(inner: &mut Inner, planned: Vec<Planned>) -> StoreResult<u64> {
        let ts = Self::now_micros();
        let mut touched: Vec<u16> = Vec::new();
        for (ns_id, kind, payload, fields) in &planned {
            let seq = inner.next_seq;
            inner
                .stream_mut(*ns_id)
                .append(seq, ts, *kind, payload, *fields)?;
            inner.next_seq += 1;
            if !touched.contains(ns_id) {
                touched.push(*ns_id);
            }
        }
        let mut syncs = 0;
        let mut manifest_dirty = false;
        let next_seq = inner.next_seq;
        for ns_id in touched {
            let mut next_partition = inner.manifest.next_partition_id;
            let rolled = {
                let stream = inner.stream_mut(ns_id);
                stream.commit()?;
                syncs += 1;
                stream.maybe_roll(&mut next_partition, next_seq)?
            };
            inner.manifest.next_partition_id = next_partition;
            if let Some(entry) = rolled {
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
    fn decode(ns: u16, partition: u32, v: &RecordView<'_>) -> StoreResult<LogRecord> {
        // The header names the codec of *this* payload: a store may hold
        // JSON sealed segments next to postcard ones (codec::*).
        let kind = codec::decode(v.header.codec, v.payload).map_err(|e| match e {
            codec::CodecError::Unknown(c) => StoreError::Format(format!(
                "ns {ns} partition {partition} seq {}: codec {c} not supported",
                v.header.seq
            )),
            other => StoreError::Decode(format!(
                "ns {ns} partition {partition} seq {}: {other}",
                v.header.seq
            )),
        })?;
        Ok(LogRecord {
            seq: v.header.seq,
            kind,
            route: None,
        })
    }

    /// Replay records into `graph`, **merged by global `seq`** across the
    /// given snapshots (H12). `partial` is `true` when only some domains
    /// are loaded: a relation whose endpoint is missing is then skipped
    /// rather than fatal. Returns `(applied, skipped)`.
    fn replay(
        snapshots: &[StreamSnapshot],
        graph: &Arc<OntologyGraph>,
        partial: bool,
    ) -> StoreResult<(u64, u64)> {
        let mut cursors: Vec<SnapshotCursor<'_>> =
            snapshots.iter().map(|s| s.cursor(true)).collect();
        // One decoded head per stream: streams are internally seq-ordered,
        // so a k-way merge of their heads yields the global order.
        let mut heads: Vec<Option<LogRecord>> = Vec::with_capacity(cursors.len());
        for (i, c) in cursors.iter_mut().enumerate() {
            heads.push(next_decoded(snapshots[i].ns_id, c)?);
        }
        let mut applied = 0u64;
        let mut skipped = 0u64;
        loop {
            let mut best: Option<usize> = None;
            for (i, h) in heads.iter().enumerate() {
                if let Some(r) = h {
                    let better = match best {
                        Some(b) => r.seq < heads[b].as_ref().unwrap().seq,
                        None => true,
                    };
                    if better {
                        best = Some(i);
                    }
                }
            }
            let Some(i) = best else { break };
            let rec = heads[i].take().unwrap();
            // In a partial load, a record may reference a concept of a domain
            // that was not loaded: a relation's *target* (its source lives in
            // the stream being read, so a missing source is real corruption),
            // or any concept of a rule / action. Only those are skipped.
            let (skippable, target) = match &rec.kind {
                RecordKind::Relation(r)
                | RecordKind::UpdateRelation(r)
                | RecordKind::RelationExact(r) => (true, Some(r.target)),
                RecordKind::Rule(_) | RecordKind::Action(_) => (true, None),
                _ => (false, None),
            };
            match apply(graph, rec) {
                Ok(()) => applied += 1,
                Err(StoreError::Graph(GraphError::UnknownConcept(id)))
                    if partial && skippable && target.is_none_or(|t| t == id) =>
                {
                    skipped += 1;
                }
                Err(e) => return Err(e),
            }
            heads[i] = next_decoded(snapshots[i].ns_id, &mut cursors[i])?;
        }
        Ok((applied, skipped))
    }

    fn hydrate(
        inner: &mut Inner,
        graph: &Arc<OntologyGraph>,
        domains: Option<&[String]>,
    ) -> StoreResult<HydrationReport> {
        let mut snapshots = vec![inner.meta.snapshot()?];
        let selected: Option<HashSet<u16>> = match domains {
            None => None,
            Some(names) => {
                let mut ids = HashSet::new();
                for n in names {
                    match inner.manifest.ns_id(n) {
                        Some(id) => {
                            ids.insert(id);
                        }
                        None => {
                            warn!(ns = %n, "requested domain has no stream yet; nothing to load")
                        }
                    }
                }
                Some(ids)
            }
        };
        for (ns_id, s) in inner.graph.iter_mut() {
            if selected.as_ref().is_none_or(|sel| sel.contains(ns_id)) {
                snapshots.push(s.snapshot()?);
            }
        }
        let partial = selected.is_some();
        let (applied, skipped) = Self::replay(&snapshots, graph, partial)?;
        Ok(HydrationReport {
            applied,
            domains: domains.map(|d| d.to_vec()),
            skipped_cross_domain: skipped,
        })
    }

    /// Live state of `graph` as records, in dependency order: schema,
    /// concepts, relations (canonical direction only for symmetric types),
    /// rules, actions.
    fn live_records(graph: &OntologyGraph) -> Vec<LogRecord> {
        let ontology = graph.ontology();
        let mut records: Vec<LogRecord> = vec![LogRecord::ontology(ontology.clone())];
        let mut concepts = graph.all_concepts();
        concepts.sort_by_key(|c| c.id);
        records.extend(concepts.into_iter().map(LogRecord::concept));
        // Every live relation, both directions of a symmetric pair included,
        // as `RelationExact` with its live id: replay keeps the ids, so the
        // tombstones the live graph journals afterwards target records that
        // exist on disk.
        let mut relations = graph.all_relations();
        relations.sort_by_key(|r| r.id);
        records.extend(relations.into_iter().map(LogRecord::relation_exact));
        let mut rules = graph.all_rules();
        rules.sort_by_key(|r| r.id);
        records.extend(rules.into_iter().map(LogRecord::rule));
        let mut actions = graph.all_actions();
        actions.sort_by_key(|a| a.id);
        records.extend(actions.into_iter().map(LogRecord::action));
        records
    }

    /// Rewrite the whole store from `graph` into fresh partitions, verify by
    /// replay, swap, delete the old files, rebuild `.xref`s.
    fn compact_all(inner: &mut Inner, graph: &Arc<OntologyGraph>) -> StoreResult<CompactionReport> {
        let records_before = inner.total_records();
        let bytes_before = inner.total_bytes();

        // 1. Live state, routed by the live schema.
        let records = Self::live_records(graph);
        inner.ontology = graph.ontology();

        // 2. Stage: one fresh partition per touched stream, fresh seqs, built
        //    in `<stream>/compacting/` — a directory partition discovery
        //    ignores, so a crash here leaves nothing a later open could take
        //    for a live partition.
        let ts = Self::now_micros();
        let mut staged: BTreeMap<u16, ActiveSegment> = BTreeMap::new();
        for r in &records {
            let payload = codec::encode(inner.manifest.codec, &r.kind)
                .map_err(|e| StoreError::Encode(e.to_string()))?;
            let meta = RecordMeta::of(&r.kind);
            let rtype_sym = match meta.relation_type {
                Some(rt) => inner.manifest.intern_relation_type(rt).0,
                None => 0,
            };
            let (ns_id, target_ns_id) = Self::route(inner, r, &meta)?;
            let seg = match staged.entry(ns_id) {
                std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::btree_map::Entry::Vacant(v) => {
                    let dir = staging_dir(&inner.stream_dir(ns_id));
                    let pid = inner.manifest.next_partition_id;
                    inner.manifest.next_partition_id += 1;
                    v.insert(ActiveSegment::create(
                        &dir,
                        pid,
                        inner.next_seq,
                        inner.manifest.codec,
                    )?)
                }
            };
            seg.append(
                inner.next_seq,
                ts,
                meta.kind,
                &payload,
                IndexFields {
                    ns_id,
                    entity_id: meta.entity_id,
                    endpoints: meta.endpoints,
                    rtype_sym,
                    target_ns_id,
                },
            )?;
            inner.next_seq += 1;
        }
        inner.manifest.save(&inner.root)?;
        let mut sealed_new: BTreeMap<u16, Arc<SealedSegment>> = BTreeMap::new();
        for (ns_id, seg) in staged {
            sealed_new.insert(ns_id, Arc::new(seg.seal()?));
        }

        // 3. Verify: the staged partitions alone replay to the live graph,
        //    entity by entity, ids included.
        let verified = {
            let candidate: Vec<StreamSnapshot> = sealed_new
                .iter()
                .map(|(ns_id, s)| StreamSnapshot {
                    ns_id: *ns_id,
                    sealed: vec![s.clone()],
                    active_partition: 0,
                    active_bytes: Vec::new(),
                })
                .collect();
            let scratch = OntologyGraph::with_arc(Ontology::new());
            Self::replay(&candidate, &scratch, false)
                .and_then(|_| crate::migrate::compare_graphs(graph, &scratch))
            // `candidate` (and its mmaps) drop here.
        };
        let staged_ids: Vec<(u16, u32)> = sealed_new
            .iter()
            .map(|(ns, s)| (*ns, s.partition_id()))
            .collect();
        // The mappings of the staged files are released before any rename or
        // deletion (Windows cannot move or delete a mapped file).
        drop(sealed_new);
        if let Err(e) = verified {
            for (ns_id, pid) in &staged_ids {
                let _ = remove_segment_files(&staging_dir(&inner.stream_dir(*ns_id)), *pid);
            }
            inner.manifest.save(&inner.root)?;
            return Err(StoreError::Format(format!(
                "compaction aborted, store left unchanged: {e}"
            )));
        }

        // 4. Commit point: the marker names what replaces what. From here a
        //    crash is finished by `finish_compaction` at the next open.
        let all_ids: Vec<u16> = std::iter::once(META_NS_ID)
            .chain(inner.graph.keys().copied())
            .collect();
        let marker = CompactionMarker {
            staged: staged_ids.clone(),
            remove: all_ids
                .iter()
                .map(|ns| (*ns, inner.stream_ref(*ns).partition_ids()))
                .collect(),
        };
        inner.manifest.compaction = Some(marker);
        inner.manifest.save(&inner.root)?;

        // 5. Swap every stream: staged partition moved in, fresh active,
        //    old files deleted.
        let mut removed = 0usize;
        for ns_id in all_ids {
            let dir = inner.stream_dir(ns_id);
            let codec = inner.manifest.codec;
            let mut sealed: Vec<Arc<SealedSegment>> = Vec::new();
            if let Some((_, pid)) = staged_ids.iter().find(|(ns, _)| *ns == ns_id) {
                move_segment_files(&staging_dir(&dir), &dir, *pid)?;
                sealed.push(Arc::new(SealedSegment::open(&dir, *pid)?));
            }
            let pid = inner.manifest.next_partition_id;
            inner.manifest.next_partition_id += 1;
            let fresh = ActiveSegment::create(&dir, pid, inner.next_seq, codec)?;
            let entries = {
                let stream = inner.stream_mut(ns_id);
                removed += stream.replace_segments(sealed, fresh)?.len();
                // Rolls after a codec change must create segments in the
                // store's current codec, not the one this stream opened with.
                stream.set_codec(codec);
                stream.sealed_entries()
            };
            inner.manifest.stream_mut(ns_id).unwrap().sealed = entries;
            let _ = std::fs::remove_dir(staging_dir(&dir));
        }
        inner.manifest.compaction = None;
        inner.manifest.save(&inner.root)?;
        rebuild_xrefs(inner)?;

        let report = CompactionReport {
            records_before,
            records_after: inner.total_records(),
            bytes_before,
            bytes_after: inner.total_bytes(),
            partitions_removed: removed,
        };
        info!(?report, "store compacted");
        Ok(report)
    }

    /// Load only the given domains (plus `meta`, always). Relations whose
    /// other endpoint is not loaded are skipped; see [`HydrationReport`].
    pub async fn load_domains_report(
        &self,
        graph: &Arc<OntologyGraph>,
        domains: &[String],
    ) -> StoreResult<HydrationReport> {
        let inner = self.inner.clone();
        let graph = graph.clone();
        let domains = domains.to_vec();
        let report = tokio::task::spawn_blocking(move || {
            let mut g = inner.lock();
            Self::hydrate(&mut g, &graph, Some(&domains))
        })
        .await
        .map_err(|e| StoreError::Io(std::io::Error::other(e)))??;
        info!(?report, "segment store hydrated (selected domains)");
        *self.last_hydration.lock() = Some(report.clone());
        Ok(report)
    }

    /// Whole-store compaction; see the module docs.
    pub async fn compact_store(&self, graph: &Arc<OntologyGraph>) -> StoreResult<CompactionReport> {
        let inner = self.inner.clone();
        let graph = graph.clone();
        tokio::task::spawn_blocking(move || {
            let mut g = inner.lock();
            Self::compact_all(&mut g, &graph)
        })
        .await
        .map_err(|e| StoreError::Io(std::io::Error::other(e)))?
    }
}

/// Decode the next record of a cursor, mapping format errors to store errors.
fn next_decoded(ns: u16, cursor: &mut SnapshotCursor<'_>) -> StoreResult<Option<LogRecord>> {
    match cursor.next() {
        None => Ok(None),
        Some(Ok((partition, v))) => Ok(Some(SegmentStore::decode(ns, partition, &v)?)),
        Some(Err((partition, e))) => Err(StoreError::Corrupt(format!(
            "ns {ns} partition {partition}: {e}"
        ))),
    }
}

/// Latest `Ontology` record on the meta stream, if any.
/// Like `next_decoded`, also reporting the payload length of the record.
fn next_decoded_sized(
    ns_id: u16,
    cursor: &mut SnapshotCursor<'_>,
) -> StoreResult<Option<(u64, LogRecord)>> {
    match cursor.next() {
        None => Ok(None),
        Some(Err((p, e))) => Err(StoreError::Corrupt(format!(
            "ns {ns_id} partition {p}: {e}"
        ))),
        Some(Ok((p, v))) => {
            let len = v.payload.len() as u64;
            SegmentStore::decode(ns_id, p, &v).map(|r| Some((len, r)))
        }
    }
}

fn latest_ontology(meta: &mut Stream) -> StoreResult<Option<Ontology>> {
    let snap = meta.snapshot()?;
    let mut cursor = snap.cursor(true);
    let mut last: Option<Ontology> = None;
    while let Some(item) = cursor.next() {
        let (partition, v) = item
            .map_err(|(p, e)| StoreError::Corrupt(format!("ns {META_NS_ID} partition {p}: {e}")))?;
        if v.header.kind == Kind::Ontology {
            let rec = SegmentStore::decode(META_NS_ID, partition, &v)?;
            if let RecordKind::Ontology(o) = rec.kind {
                last = Some(o);
            }
        }
    }
    Ok(last)
}

/// Rebuild every graph stream's `.xref` files from the indexes of the other
/// streams (`STORAGE.md` §4.4, R7). Returns the number of entries written.
fn rebuild_xrefs(inner: &mut Inner) -> StoreResult<usize> {
    let mut by_target: BTreeMap<u16, Vec<XrefEntry>> = BTreeMap::new();
    for stream in inner.graph.values_mut() {
        for (_pid, e) in stream.all_entries()? {
            if matches!(e.kind, Kind::Relation | Kind::RelationExact) && e.target_ns_id != 0 {
                let (source_id, target_id) = unpack_endpoints(e.endpoints);
                by_target
                    .entry(e.target_ns_id)
                    .or_default()
                    .push(XrefEntry {
                        seq: e.seq,
                        target_id,
                        source_id,
                    });
            }
        }
    }
    let mut written = 0;
    for (ns_id, stream) in inner.graph.iter() {
        let entries = by_target.remove(ns_id).unwrap_or_default();
        written += stream.write_xrefs(entries)?;
    }
    Ok(written)
}

/// Where a stream's compaction output is built before the swap.
fn staging_dir(stream_dir: &Path) -> PathBuf {
    stream_dir.join("compacting")
}

/// Complete or discard a compaction the previous process did not finish.
/// With a marker (commit point passed): move the staged partitions in,
/// delete the old ones, clear the marker — idempotent, so a crash *here*
/// is finished by the next open again. Without a marker, any
/// `compacting/` directory is an aborted staging and is removed.
fn finish_compaction(root: &Path, manifest: &mut Manifest) -> StoreResult<()> {
    let dir_of = |manifest: &Manifest, ns_id: u16| -> Option<PathBuf> {
        manifest.stream(ns_id).map(|s| root.join(&s.dir))
    };
    if let Some(marker) = manifest.compaction.clone() {
        warn!(?marker, "finishing an interrupted compaction");
        for (ns_id, pid) in &marker.staged {
            let Some(dir) = dir_of(manifest, *ns_id) else {
                continue;
            };
            let staging = staging_dir(&dir);
            if crate::segment::data_path(&staging, *pid).exists() {
                move_segment_files(&staging, &dir, *pid)?;
            }
        }
        for (ns_id, olds) in &marker.remove {
            let Some(dir) = dir_of(manifest, *ns_id) else {
                continue;
            };
            for pid in olds {
                remove_segment_files(&dir, *pid)?;
            }
        }
        for entry in manifest.streams.iter() {
            let _ = std::fs::remove_dir(staging_dir(&root.join(&entry.dir)));
        }
        manifest.compaction = None;
        manifest.save(root)?;
    } else {
        for entry in manifest.streams.iter() {
            let staging = staging_dir(&root.join(&entry.dir));
            if staging.exists() {
                warn!(path = %staging.display(), "discarding an aborted compaction staging");
                std::fs::remove_dir_all(&staging)?;
            }
        }
    }
    Ok(())
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
        let report = tokio::task::spawn_blocking(move || {
            let mut g = inner.lock();
            Self::hydrate(&mut g, &graph, None)
        })
        .await
        .map_err(|e| StoreError::Io(std::io::Error::other(e)))??;
        info!(records = report.applied, "segment store hydrated");
        *self.last_hydration.lock() = Some(report);
        Ok(())
    }

    async fn load_domains(
        &self,
        graph: &Arc<OntologyGraph>,
        domains: &[String],
    ) -> StoreResult<()> {
        self.load_domains_report(graph, domains).await.map(|_| ())
    }

    async fn snapshot(&self, _graph: &Arc<OntologyGraph>) -> StoreResult<()> {
        // Every acknowledged write is already durable; there is nothing a
        // snapshot would add.
        Ok(())
    }

    async fn compact(&self, graph: &Arc<OntologyGraph>) -> StoreResult<()> {
        self.compact_store(graph).await.map(|_| ())
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
