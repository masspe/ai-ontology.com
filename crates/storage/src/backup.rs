// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Backup and restore of a [`SegmentStore`].
//!
//! A backup is a copy of the store's files taken under the writer lock:
//! `MANIFEST.json` and every partition's `.data` and `.idx`. Sealed
//! partitions are immutable (H11), so a file already at the destination
//! with the same size is not copied again — a repeated backup to the same
//! place costs the active partition and whatever was sealed since. The
//! store side lives in [`SegmentStore::backup`]; this module holds the
//! reports and [`restore`], which rebuilds a store from a backup the way
//! the migration does: in a staging directory, verified by a full replay,
//! then renamed into place.

use std::path::{Path, PathBuf};

use ontology_graph::{Ontology, OntologyGraph};
use serde::Serialize;
use tracing::{info, warn};

use crate::manifest::{Manifest, MANIFEST_FILE};
use crate::segment_store::{SegmentStore, LOCK_FILE};
use crate::store::{Store, StoreError, StoreResult};

/// What a backup copied.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BackupReport {
    /// Files the backup consists of (manifest included).
    pub files: usize,
    /// Files actually copied this time (the rest were already there).
    pub copied: usize,
    /// Bytes copied this time.
    pub bytes: u64,
    /// Records in the store at the time of the backup.
    pub records: u64,
}

/// What a restore rebuilt.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RestoreReport {
    pub files: usize,
    pub bytes: u64,
    pub records: u64,
    pub concepts: usize,
    pub relations: usize,
}

/// Directory a store is rebuilt in before it is renamed to its final name.
pub fn restore_staging_dir_for(store_dir: &Path) -> PathBuf {
    let mut name = store_dir
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| "store".into());
    name.push(".restoring");
    store_dir.with_file_name(name)
}

/// Rebuild the store at `store_dir` from the backup at `src`.
///
/// Refuses to touch an existing `store_dir`. The files are copied into a
/// staging directory, the store is opened there (recovery, manifest
/// reconciliation) and replayed in full into a scratch graph — a backup
/// that does not replay is rejected before anything is renamed.
pub async fn restore(src: &Path, store_dir: &Path) -> StoreResult<RestoreReport> {
    if !src.join(MANIFEST_FILE).is_file() {
        return Err(StoreError::Format(format!(
            "{} is not a backup: no {MANIFEST_FILE}",
            src.display()
        )));
    }
    if store_dir.exists() {
        return Err(StoreError::Format(format!(
            "{} already exists; refusing to restore over it (move it away first)",
            store_dir.display()
        )));
    }
    let staging = restore_staging_dir_for(store_dir);
    if staging.exists() {
        warn!(path = %staging.display(), "discarding an interrupted restore");
        std::fs::remove_dir_all(&staging)?;
    }
    let mut report = RestoreReport::default();
    copy_tree(src, &staging, &mut report)?;
    {
        // Its own identity: backing this copy up over the origin's backup
        // is refused (partition ids would collide silently).
        let mut m = Manifest::load(&staging)?.expect("checked above");
        m.store_id = crate::manifest::new_store_id();
        m.save(&staging)?;
        let store = SegmentStore::open(&staging).await?;
        let graph = OntologyGraph::with_arc(Ontology::new());
        store.load_into(&graph).await?;
        report.records = store.record_count();
        report.concepts = graph.concept_count();
        report.relations = graph.relation_count();
        // `store` drops here: its LOCK is released before the rename.
    }
    std::fs::rename(&staging, store_dir)?;
    info!(
        records = report.records,
        concepts = report.concepts,
        relations = report.relations,
        "store restored"
    );
    Ok(report)
}

/// Copy every regular file under `from` to `to`, except a `LOCK`.
fn copy_tree(from: &Path, to: &Path, report: &mut RestoreReport) -> StoreResult<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let dst = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_tree(&e.path(), &dst, report)?;
        } else if e.file_name() != LOCK_FILE {
            report.bytes += std::fs::copy(e.path(), dst)?;
            report.files += 1;
        }
    }
    Ok(())
}
