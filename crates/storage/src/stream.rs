// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! A stream is one directory of segments: any number of sealed
//! (immutable, mapped) partitions plus exactly one active partition that
//! receives appends. Segments roll by size or record count (`STORAGE.md`
//! §4). One stream per `ns`; `meta` is a stream too.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::{info, warn};

use crate::manifest::PartitionEntry;
use crate::segment::{
    decode_record, recover_segment, ActiveSegment, FormatError, IndexFields, Kind, RecordView,
    SealedSegment, FILE_HEADER_LEN,
};

/// When the active segment is sealed and a fresh one opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RollPolicy {
    pub max_bytes: u64,
    pub max_records: u32,
}

impl Default for RollPolicy {
    fn default() -> Self {
        Self {
            max_bytes: 64 * 1024 * 1024,
            max_records: 100_000,
        }
    }
}

/// What opening a stream found and repaired.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StreamOpenReport {
    pub sealed: usize,
    /// Partitions that were not readable as sealed and had to be recovered
    /// and sealed again (crash between seal and manifest write).
    pub resealed: Vec<u32>,
    pub active_partition: u32,
    pub active_records: u32,
    pub truncated_bytes: u64,
    pub idx_rewritten: usize,
    pub last_seq: Option<u64>,
}

/// Resolver turning a decoded payload into its index fields; supplied by
/// the store, which owns the symbol table.
pub type Resolver<'a> = dyn FnMut(Kind, &[u8]) -> Result<IndexFields, String> + 'a;

pub struct Stream {
    dir: PathBuf,
    ns_id: u16,
    codec: u8,
    roll: RollPolicy,
    sealed: Vec<Arc<SealedSegment>>,
    active: ActiveSegment,
}

impl Stream {
    /// Open (or create) the stream in `dir`. Every `.data` file is
    /// rediscovered from the directory; the highest id is the active
    /// segment and is recovered (§9), the others are mapped as sealed —
    /// or recovered and sealed if a previous seal did not complete.
    /// `next_partition` and `next_seq` are the store-wide allocators.
    pub fn open(
        dir: &Path,
        ns_id: u16,
        codec: u8,
        roll: RollPolicy,
        next_partition: &mut u32,
        next_seq: u64,
        resolve: &mut Resolver<'_>,
    ) -> io::Result<(Self, StreamOpenReport)> {
        std::fs::create_dir_all(dir)?;
        let mut ids = list_partitions(dir)?;
        let mut report = StreamOpenReport::default();
        let mut sealed = Vec::new();

        if ids.is_empty() {
            let id = *next_partition;
            *next_partition += 1;
            let active = ActiveSegment::create(dir, id, next_seq, codec)?;
            report.active_partition = id;
            return Ok((
                Self {
                    dir: dir.to_path_buf(),
                    ns_id,
                    codec,
                    roll,
                    sealed,
                    active,
                },
                report,
            ));
        }

        let last = ids.pop().unwrap();
        for id in ids {
            match SealedSegment::open(dir, id) {
                Ok(s) => sealed.push(Arc::new(s)),
                Err(e) => {
                    warn!(partition = id, error = %e, "partition not sealed cleanly; recovering and sealing");
                    let r = recover_segment(dir, id, &mut *resolve)?;
                    report.truncated_bytes += r.truncated_bytes;
                    report.idx_rewritten += r.idx_rewritten;
                    let seg = ActiveSegment::reopen(
                        dir,
                        id,
                        r.header,
                        r.data_len,
                        r.count,
                        r.last_seq.unwrap_or(r.header.base_seq.saturating_sub(1)),
                    )?;
                    sealed.push(Arc::new(seg.seal()?));
                    report.resealed.push(id);
                }
            }
        }
        // A crash between `create_new` and the header sync leaves an empty
        // (or header-short) `.data`: nothing was ever appended to it, so it is
        // simply recreated under the same id.
        let data_len = std::fs::metadata(crate::segment::data_path(dir, last))
            .map(|m| m.len())
            .unwrap_or(0);
        if data_len < crate::segment::FILE_HEADER_LEN as u64 {
            warn!(
                partition = last,
                bytes = data_len,
                "active segment has no header; recreating it"
            );
            let _ = std::fs::remove_file(crate::segment::data_path(dir, last));
            let _ = std::fs::remove_file(crate::segment::idx_path(dir, last));
            let active = ActiveSegment::create(dir, last, next_seq, codec)?;
            if *next_partition <= last {
                *next_partition = last + 1;
            }
            report.sealed = sealed.len();
            report.active_partition = last;
            report.last_seq = sealed.iter().rev().find_map(|s| s.last_seq());
            return Ok((
                Self {
                    dir: dir.to_path_buf(),
                    ns_id,
                    codec,
                    roll,
                    sealed,
                    active,
                },
                report,
            ));
        }
        let r = recover_segment(dir, last, &mut *resolve)?;
        if r.truncated_bytes > 0 || r.idx_rewritten > 0 {
            info!(
                partition = last,
                truncated = r.truncated_bytes,
                idx_rewritten = r.idx_rewritten,
                "active segment recovered"
            );
        }
        report.truncated_bytes += r.truncated_bytes;
        report.idx_rewritten += r.idx_rewritten;
        let last_seq_active = r.last_seq.unwrap_or(r.header.base_seq.saturating_sub(1));
        let active =
            ActiveSegment::reopen(dir, last, r.header, r.data_len, r.count, last_seq_active)?;
        if *next_partition <= last {
            *next_partition = last + 1;
        }
        report.sealed = sealed.len();
        report.active_partition = last;
        report.active_records = r.count;
        report.last_seq = r
            .last_seq
            .or_else(|| sealed.iter().rev().find_map(|s| s.last_seq()));
        Ok((
            Self {
                dir: dir.to_path_buf(),
                ns_id,
                codec,
                roll,
                sealed,
                active,
            },
            report,
        ))
    }

    pub fn ns_id(&self) -> u16 {
        self.ns_id
    }
    pub fn dir(&self) -> &Path {
        &self.dir
    }
    pub fn sealed(&self) -> &[Arc<SealedSegment>] {
        &self.sealed
    }
    pub fn active(&self) -> &ActiveSegment {
        &self.active
    }
    /// Seq of the last record in the stream, sealed or active.
    pub fn last_seq(&self) -> Option<u64> {
        if self.active.record_count() > 0 {
            Some(self.active.last_seq())
        } else {
            self.sealed.iter().rev().find_map(|s| s.last_seq())
        }
    }
    /// Records across every segment, active included.
    pub fn record_count(&self) -> u64 {
        self.sealed
            .iter()
            .map(|s| s.record_count() as u64)
            .sum::<u64>()
            + self.active.record_count() as u64
    }

    /// Buffer one record on the active segment.
    pub fn append(
        &mut self,
        seq: u64,
        ts_micros: u64,
        kind: Kind,
        payload: &[u8],
        fields: IndexFields,
    ) -> io::Result<()> {
        self.active.append(seq, ts_micros, kind, payload, fields)?;
        Ok(())
    }

    pub fn has_pending(&self) -> bool {
        self.active.has_pending()
    }

    /// Durability barrier for everything appended since the last commit.
    pub fn commit(&mut self) -> io::Result<usize> {
        self.active.commit()
    }

    /// Seal the active segment and open a fresh one when the roll policy
    /// says so. Returns the zone map of the sealed partition, for the
    /// manifest. Must be called after `commit`.
    pub fn maybe_roll(
        &mut self,
        next_partition: &mut u32,
        next_seq: u64,
    ) -> io::Result<Option<PartitionEntry>> {
        let due = self.active.data_len() >= self.roll.max_bytes
            || self.active.record_count() >= self.roll.max_records;
        if !due || self.active.record_count() == 0 {
            return Ok(None);
        }
        let id = *next_partition;
        *next_partition += 1;
        let fresh = ActiveSegment::create(&self.dir, id, next_seq, self.codec)?;
        let old = std::mem::replace(&mut self.active, fresh);
        let sealed = old.seal()?;
        let entry = PartitionEntry::of(&sealed);
        info!(
            ns = self.ns_id,
            partition = entry.id,
            records = entry.records,
            bytes = entry.data_bytes,
            "segment sealed"
        );
        self.sealed.push(Arc::new(sealed));
        Ok(Some(entry))
    }

    /// Zone maps of every sealed partition, recomputed from the indexes.
    pub fn sealed_entries(&self) -> Vec<PartitionEntry> {
        self.sealed.iter().map(|s| PartitionEntry::of(s)).collect()
    }

    /// Visit every committed record in stream order: sealed partitions from
    /// their maps, then the active segment from a fresh read of its file.
    /// This is the P0 hydration path (D4).
    pub fn for_each_record<E>(
        &mut self,
        verify_crc: bool,
        mut f: impl FnMut(u32, RecordView<'_>) -> Result<(), E>,
    ) -> Result<(), StreamReadError<E>> {
        for seg in &self.sealed {
            for r in seg.records(verify_crc) {
                let v = r.map_err(|e| StreamReadError::Format {
                    partition: seg.partition_id(),
                    error: e,
                })?;
                f(seg.partition_id(), v).map_err(StreamReadError::Visitor)?;
            }
        }
        let pid = self.active.partition_id();
        let len = self.active.data_len() as usize;
        if len > FILE_HEADER_LEN {
            let bytes = self
                .active
                .read_span(FILE_HEADER_LEN as u64, len - FILE_HEADER_LEN)
                .map_err(StreamReadError::Io)?;
            let mut at = 0usize;
            while at < bytes.len() {
                let v =
                    decode_record(&bytes, at, verify_crc).map_err(|e| StreamReadError::Format {
                        partition: pid,
                        error: e,
                    })?;
                // Offsets in the view are relative to `bytes`; callers only
                // use header + payload, which is what we hand over.
                let next = v.next;
                f(pid, v).map_err(StreamReadError::Visitor)?;
                at = next;
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum StreamReadError<E> {
    Io(io::Error),
    Format { partition: u32, error: FormatError },
    Visitor(E),
}

/// Partition ids present in `dir`, ascending, from the `NNNNNN.data` files.
pub fn list_partitions(dir: &Path) -> io::Result<Vec<u32>> {
    let mut ids = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let p = entry?.path();
        if p.extension().and_then(|e| e.to_str()) != Some("data") {
            continue;
        }
        if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
            if let Ok(id) = stem.parse::<u32>() {
                ids.push(id);
            }
        }
    }
    ids.sort_unstable();
    Ok(ids)
}
