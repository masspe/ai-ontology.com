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

use crate::log::{LogRecord, RecordKind, RouteHint};
use crate::manifest::{Manifest, META_NS_ID};
use crate::memory::apply;
use crate::segment::active::remove_segment_files;
use crate::segment::xref::{read_xref, XrefEntry};
use crate::segment::{
    unpack_endpoints, ActiveSegment, IndexFields, Kind, RecordMeta, RecordView, SealedSegment,
    CODEC_JSON, FORMAT_VERSION,
};
use crate::store::{Store, StoreError, StoreResult};
use crate::stream::{RollPolicy, SnapshotCursor, Stream, StreamOpenReport, StreamSnapshot};

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
        let before = manifest.clone();
        let mut next_partition = manifest.next_partition_id;

        // 1. meta: no relation types to intern, no target domains.
        let meta_dir = root.join(&manifest.stream(META_NS_ID).unwrap().dir);
        let (mut meta, meta_report) = {
            let mut resolve = |kind: Kind, payload: &[u8]| -> Result<IndexFields, String> {
                let rk: RecordKind = serde_json::from_slice(payload).map_err(|e| e.to_string())?;
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
                let mut resolve = |kind: Kind, payload: &[u8]| -> Result<IndexFields, String> {
                    let rk: RecordKind =
                        serde_json::from_slice(payload).map_err(|e| e.to_string())?;
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
        let (id, fresh) = inner.manifest.intern_ns(name);
        if fresh {
            inner.manifest.save(&inner.root)?;
        }
        if !inner.graph.contains_key(&id) {
            let dir = inner.root.join(&inner.manifest.stream(id).unwrap().dir);
            let mut next_partition = inner.manifest.next_partition_id;
            let mut resolve = |_k: Kind, _p: &[u8]| -> Result<IndexFields, String> {
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
            RecordKind::Relation(rel) | RecordKind::UpdateRelation(rel) => {
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

    /// Encode, route, append, then one sync per touched stream and a roll
    /// check. Returns the number of syncs issued.
    fn commit_batch(inner: &mut Inner, records: &[LogRecord]) -> StoreResult<u64> {
        if records.is_empty() {
            return Ok(0);
        }
        let ts = Self::now_micros();
        let mut touched: Vec<u16> = Vec::new();
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
            let (ns_id, target_ns_id) = Self::route(inner, r, &meta)?;
            let fields = IndexFields {
                ns_id,
                entity_id: meta.entity_id,
                endpoints: meta.endpoints,
                rtype_sym,
                target_ns_id,
            };
            let seq = inner.next_seq;
            inner
                .stream_mut(ns_id)
                .append(seq, ts, meta.kind, &payload, fields)?;
            inner.next_seq += 1;
            if !touched.contains(&ns_id) {
                touched.push(ns_id);
            }
            // A schema record changes the router for the records after it.
            if let RecordKind::Ontology(o) = &r.kind {
                inner.ontology = o.clone();
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
            // Records that reference concepts by id may point into a domain
            // that was not loaded; in a partial load they are skipped.
            let depends_on_concepts = matches!(
                rec.kind,
                RecordKind::Relation(_)
                    | RecordKind::UpdateRelation(_)
                    | RecordKind::Rule(_)
                    | RecordKind::Action(_)
            );
            match apply(graph, rec) {
                Ok(()) => applied += 1,
                Err(StoreError::Graph(GraphError::UnknownConcept(_)))
                    if partial && depends_on_concepts =>
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
        let mut relations = graph.all_relations();
        relations.sort_by_key(|r| r.id);
        let mut seen_symmetric: HashSet<(String, u64, u64)> = HashSet::new();
        for r in relations {
            let symmetric = ontology
                .relation_types
                .get(&r.relation_type)
                .map(|rt| rt.symmetric)
                .unwrap_or(false);
            if symmetric {
                let (a, b) = if r.source <= r.target {
                    (r.source.0, r.target.0)
                } else {
                    (r.target.0, r.source.0)
                };
                if !seen_symmetric.insert((r.relation_type.clone(), a, b)) {
                    continue; // the inverse is re-materialized on replay
                }
            }
            records.push(LogRecord::relation(r));
        }
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

        // 2. Stage: one fresh partition per touched stream, fresh seqs.
        let ts = Self::now_micros();
        let mut staged: BTreeMap<u16, ActiveSegment> = BTreeMap::new();
        for r in &records {
            let payload =
                serde_json::to_vec(&r.kind).map_err(|e| StoreError::Encode(e.to_string()))?;
            let meta = RecordMeta::of(&r.kind);
            let rtype_sym = match meta.relation_type {
                Some(rt) => inner.manifest.intern_relation_type(rt).0,
                None => 0,
            };
            let (ns_id, target_ns_id) = Self::route(inner, r, &meta)?;
            let seg = match staged.entry(ns_id) {
                std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::btree_map::Entry::Vacant(v) => {
                    let dir = inner.stream_dir(ns_id);
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

        // 3. Verify: the staged partitions alone replay to the live graph.
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
        let verified = Self::replay(&candidate, &scratch, false)
            .and_then(|_| compare_semantic(graph, &scratch));
        if let Err(e) = verified {
            // Roll back: drop the staged files, keep everything as it was.
            for (ns_id, s) in sealed_new {
                let dir = inner.stream_dir(ns_id);
                let pid = s.partition_id();
                drop(s);
                let _ = remove_segment_files(&dir, pid);
            }
            return Err(StoreError::Format(format!(
                "compaction aborted, store left unchanged: {e}"
            )));
        }

        // 4. Swap every stream: [staged sealed] + fresh active; delete old.
        let mut removed = 0usize;
        let all_ids: Vec<u16> = std::iter::once(META_NS_ID)
            .chain(inner.graph.keys().copied())
            .collect();
        for ns_id in all_ids {
            let dir = inner.stream_dir(ns_id);
            let codec = inner.manifest.codec;
            let pid = inner.manifest.next_partition_id;
            inner.manifest.next_partition_id += 1;
            let fresh = ActiveSegment::create(&dir, pid, inner.next_seq, codec)?;
            let sealed: Vec<Arc<SealedSegment>> = sealed_new.remove(&ns_id).into_iter().collect();
            let entries = {
                let stream = inner.stream_mut(ns_id);
                removed += stream.replace_segments(sealed, fresh)?.len();
                stream.sealed_entries()
            };
            inner.manifest.stream_mut(ns_id).unwrap().sealed = entries;
        }
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
            if e.kind == Kind::Relation && e.target_ns_id != 0 {
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

fn to_value<T: serde::Serialize>(v: &T) -> StoreResult<serde_json::Value> {
    serde_json::to_value(v).map_err(|e| StoreError::Encode(e.to_string()))
}

/// Compaction check: same schema, same concepts, same rules/actions by id,
/// same multiset of edges `(type, source, target, weight)`. Relation ids of
/// materialized symmetric inverses may differ after a replay, so edges are
/// compared structurally.
pub fn compare_semantic(a: &OntologyGraph, b: &OntologyGraph) -> StoreResult<()> {
    let fail = |what: String| StoreError::Format(format!("verification failed: {what}"));
    if to_value(&a.ontology())? != to_value(&b.ontology())? {
        return Err(fail("ontology differs".into()));
    }
    if a.concept_count() != b.concept_count() {
        return Err(fail(format!(
            "concept count {} vs {}",
            a.concept_count(),
            b.concept_count()
        )));
    }
    for c in a.all_concepts() {
        let other = b
            .get_concept(c.id)
            .map_err(|_| fail(format!("concept {} missing", c.id)))?;
        if to_value(&c)? != to_value(&other)? {
            return Err(fail(format!("concept {} differs", c.id)));
        }
    }
    let edges = |g: &OntologyGraph| -> Vec<(String, u64, u64, u32)> {
        let mut v: Vec<_> = g
            .all_relations()
            .into_iter()
            .map(|r| (r.relation_type, r.source.0, r.target.0, r.weight.to_bits()))
            .collect();
        v.sort();
        v
    };
    if edges(a) != edges(b) {
        return Err(fail("edge multiset differs".into()));
    }
    if a.rule_count() != b.rule_count() || a.action_count() != b.action_count() {
        return Err(fail("rule or action count differs".into()));
    }
    for r in a.all_rules() {
        let other = b
            .get_rule(r.id)
            .map_err(|_| fail(format!("rule {} missing", r.id)))?;
        if to_value(&r)? != to_value(&other)? {
            return Err(fail(format!("rule {} differs", r.id)));
        }
    }
    for x in a.all_actions() {
        let other = b
            .get_action(x.id)
            .map_err(|_| fail(format!("action {} missing", x.id)))?;
        if to_value(&x)? != to_value(&other)? {
            return Err(fail(format!("action {} differs", x.id)));
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
