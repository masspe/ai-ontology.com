// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! One-way migration from the legacy `graph.log` (+ optional `graph.snap`)
//! to a [`SegmentStore`] under `<data>/store/`.
//!
//! The snapshot, when present, is unrolled into records (ontology,
//! concepts, relations, rules, actions) followed by the log records newer
//! than its watermark; every record gets a fresh store-wide `seq`. The
//! result is verified by replaying both sides into two graphs and comparing
//! them entity by entity before the legacy files are renamed to
//! `*.migrated`. Nothing is ever deleted by this code.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ontology_graph::{Ontology, OntologyGraph};
use tracing::{info, warn};

use crate::file::FileStore;
use crate::log::{LogRecord, RecordKind};
use crate::segment_store::SegmentStore;
use crate::snapshot::Snapshot;
use crate::store::{Store, StoreError, StoreResult};

pub const LEGACY_LOG: &str = "graph.log";
pub const LEGACY_SNAPSHOT: &str = "graph.snap";
pub const MIGRATED_SUFFIX: &str = "migrated";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MigrationReport {
    pub records: u64,
    pub ontology: u64,
    pub concepts: u64,
    pub relations: u64,
    pub rules: u64,
    pub actions: u64,
    pub deletes_and_updates: u64,
    pub legacy_bytes: u64,
    pub store_bytes: u64,
    pub renamed: Vec<PathBuf>,
}

/// `true` when `data_dir` holds a legacy log or snapshot that has not been
/// migrated yet.
pub fn legacy_present(data_dir: &Path) -> bool {
    data_dir.join(LEGACY_LOG).exists() || data_dir.join(LEGACY_SNAPSHOT).exists()
}

/// Read the legacy `[u32 BE len][JSON]` log, tolerating a torn tail exactly
/// like `FileStore::load_into` does (the tail is not truncated here — the
/// file is about to be renamed, not reused).
pub fn read_legacy_log(path: &Path) -> StoreResult<Vec<LogRecord>> {
    let bytes = std::fs::read(path)?;
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 4 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        if at + 4 + len > bytes.len() {
            warn!(offset = at, "legacy log: torn tail ignored");
            break;
        }
        match serde_json::from_slice::<LogRecord>(&bytes[at + 4..at + 4 + len]) {
            Ok(r) => out.push(r),
            Err(e) if at + 4 + len == bytes.len() => {
                warn!(offset = at, error = %e, "legacy log: undecodable trailing record ignored");
                break;
            }
            Err(e) => {
                return Err(StoreError::Decode(format!(
                    "legacy log record at offset {at} is corrupt and is not the last one: {e}"
                )))
            }
        }
        at += 4 + len;
    }
    Ok(out)
}

/// The legacy contents as an ordered record stream: snapshot unrolled, then
/// log records past the snapshot watermark.
pub fn legacy_records(data_dir: &Path) -> StoreResult<Vec<LogRecord>> {
    let mut records = Vec::new();
    let mut water = 0u64;
    let snap_path = data_dir.join(LEGACY_SNAPSHOT);
    if snap_path.exists() {
        let snap: Snapshot = serde_json::from_slice(&std::fs::read(&snap_path)?)
            .map_err(|e| StoreError::Decode(format!("graph.snap: {e}")))?;
        water = snap.high_water_seq;
        records.push(LogRecord::ontology(snap.ontology));
        records.extend(snap.concepts.into_iter().map(LogRecord::concept));
        records.extend(snap.relations.into_iter().map(LogRecord::relation));
        records.extend(snap.rules.into_iter().map(LogRecord::rule));
        records.extend(snap.actions.into_iter().map(LogRecord::action));
    }
    let log_path = data_dir.join(LEGACY_LOG);
    if log_path.exists() {
        for r in read_legacy_log(&log_path)? {
            if r.seq > water {
                records.push(r);
            }
        }
    }
    Ok(records)
}

/// Migrate `data_dir`'s legacy files into a fresh store at `store_dir`.
/// Fails without touching anything if `store_dir` already holds records.
pub async fn migrate_legacy(data_dir: &Path, store_dir: &Path) -> StoreResult<MigrationReport> {
    if !legacy_present(data_dir) {
        return Err(StoreError::Format(format!(
            "nothing to migrate in {}: no {LEGACY_LOG} or {LEGACY_SNAPSHOT}",
            data_dir.display()
        )));
    }
    let mut report = MigrationReport::default();
    for name in [LEGACY_LOG, LEGACY_SNAPSHOT] {
        if let Ok(m) = std::fs::metadata(data_dir.join(name)) {
            report.legacy_bytes += m.len();
        }
    }

    let records = legacy_records(data_dir)?;
    for r in &records {
        report.records += 1;
        match r.kind {
            RecordKind::Ontology(_) => report.ontology += 1,
            RecordKind::Concept(_) => report.concepts += 1,
            RecordKind::Relation(_) => report.relations += 1,
            RecordKind::Rule(_) => report.rules += 1,
            RecordKind::Action(_) => report.actions += 1,
            _ => report.deletes_and_updates += 1,
        }
    }

    // 1. Write.
    {
        let store = SegmentStore::open(store_dir).await?;
        if store.record_count() > 0 {
            return Err(StoreError::Format(format!(
                "{} already holds {} records; refusing to migrate into it",
                store_dir.display(),
                store.record_count()
            )));
        }
        for chunk in records.chunks(1000) {
            store.append_batch(chunk).await?;
        }
        report.store_bytes = dir_size(store_dir)?;

        // 2. Verify: both sides replay to the same graph.
        let legacy = FileStore::open(data_dir).await?;
        let a = OntologyGraph::with_arc(Ontology::new());
        legacy.load_into(&a).await?;
        let b = OntologyGraph::with_arc(Ontology::new());
        store.load_into(&b).await?;
        compare_graphs(&a, &b)?;
        // `store` and `legacy` drop here: the store's LOCK is released for
        // whoever opens it next.
    }

    // 3. Retire the legacy files (rename, never delete).
    for name in [LEGACY_LOG, LEGACY_SNAPSHOT] {
        let p = data_dir.join(name);
        if p.exists() {
            let dest = data_dir.join(format!("{name}.{MIGRATED_SUFFIX}"));
            std::fs::rename(&p, &dest)?;
            report.renamed.push(dest);
        }
    }
    info!(
        records = report.records,
        concepts = report.concepts,
        relations = report.relations,
        legacy_bytes = report.legacy_bytes,
        store_bytes = report.store_bytes,
        "legacy store migrated"
    );
    Ok(report)
}

/// Entity-by-entity comparison; any difference is a migration failure.
pub fn compare_graphs(a: &OntologyGraph, b: &OntologyGraph) -> StoreResult<()> {
    let mismatch =
        |what: String| StoreError::Format(format!("migration verification failed: {what}"));
    let ja = serde_json::to_value(a.ontology()).map_err(|e| StoreError::Encode(e.to_string()))?;
    let jb = serde_json::to_value(b.ontology()).map_err(|e| StoreError::Encode(e.to_string()))?;
    if ja != jb {
        return Err(mismatch("ontology differs".into()));
    }
    if a.concept_count() != b.concept_count() {
        return Err(mismatch(format!(
            "concept count {} vs {}",
            a.concept_count(),
            b.concept_count()
        )));
    }
    if a.relation_count() != b.relation_count() {
        return Err(mismatch(format!(
            "relation count {} vs {}",
            a.relation_count(),
            b.relation_count()
        )));
    }
    if a.rule_count() != b.rule_count() || a.action_count() != b.action_count() {
        return Err(mismatch("rule or action count differs".into()));
    }
    for c in a.all_concepts() {
        let other = b
            .get_concept(c.id)
            .map_err(|_| mismatch(format!("concept {} missing", c.id)))?;
        let (x, y) = (
            serde_json::to_value(&c).map_err(|e| StoreError::Encode(e.to_string()))?,
            serde_json::to_value(&other).map_err(|e| StoreError::Encode(e.to_string()))?,
        );
        if x != y {
            return Err(mismatch(format!("concept {} differs", c.id)));
        }
    }
    for r in a.all_relations() {
        let other = b
            .get_relation(r.id)
            .map_err(|_| mismatch(format!("relation {} missing", r.id)))?;
        let (x, y) = (
            serde_json::to_value(&r).map_err(|e| StoreError::Encode(e.to_string()))?,
            serde_json::to_value(&other).map_err(|e| StoreError::Encode(e.to_string()))?,
        );
        if x != y {
            return Err(mismatch(format!("relation {} differs", r.id)));
        }
    }
    for r in a.all_rules() {
        b.get_rule(r.id)
            .map_err(|_| mismatch(format!("rule {} missing", r.id)))?;
    }
    for x in a.all_actions() {
        b.get_action(x.id)
            .map_err(|_| mismatch(format!("action {} missing", x.id)))?;
    }
    Ok(())
}

fn dir_size(dir: &Path) -> std::io::Result<u64> {
    let mut total = 0;
    for entry in std::fs::read_dir(dir)? {
        let p = entry?.path();
        if p.is_dir() {
            total += dir_size(&p)?;
        } else {
            total += p.metadata()?.len();
        }
    }
    Ok(total)
}

/// Helper for the CLI: the store directory that goes with a data directory.
pub fn store_dir_for(data_dir: &Path) -> PathBuf {
    data_dir.join("store")
}

/// Keep `Arc` in scope for the doc-level signature parity with `Store`.
#[allow(dead_code)]
fn _arc_marker(_: Arc<()>) {}
