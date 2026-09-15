// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! The one segment of a stream that still grows. Records are appended to a
//! buffered `.data` writer, then made durable with a single `fdatasync` per
//! [`ActiveSegment::commit`]; the matching `.idx` entries are written
//! **after** that sync and never synced themselves (R7: the index is
//! derived, recovery rebuilds it from the data).
//!
//! Never memory-mapped (R11): reads of the active segment go through
//! positional file reads.

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use super::format::{
    encode_record, DataHeader, IdxEntry, IdxHeader, Kind, FILE_HEADER_LEN, IDX_ENTRY_LEN,
};
use super::sealed::SealedSegment;

/// File name stem of partition `id`, zero-padded so `ls` sorts by id.
pub fn segment_stem(partition_id: u32) -> String {
    format!("{partition_id:06}")
}
pub fn data_path(dir: &Path, partition_id: u32) -> PathBuf {
    dir.join(format!("{}.data", segment_stem(partition_id)))
}
pub fn idx_path(dir: &Path, partition_id: u32) -> PathBuf {
    dir.join(format!("{}.idx", segment_stem(partition_id)))
}

/// Per-record index fields the caller resolves before appending.
#[derive(Debug, Clone, Copy)]
pub struct IndexFields {
    pub ns_id: u16,
    pub entity_id: u64,
    pub endpoints: u64,
    pub rtype_sym: u32,
    /// Domain of a relation's target when it differs from `ns_id`; 0 otherwise.
    pub target_ns_id: u16,
}

pub struct ActiveSegment {
    dir: PathBuf,
    partition_id: u32,
    base_seq: u64,
    codec: u8,
    data: BufWriter<File>,
    idx: BufWriter<File>,
    /// Bytes of `.data` that are complete records (header included), i.e.
    /// where the next record goes.
    data_len: u64,
    /// Entries already written to `.idx` (durable or not).
    count: u32,
    /// Index entries for records appended since the last commit. Written to
    /// `.idx` only once their data is synced.
    pending_idx: Vec<IdxEntry>,
    /// Bytes appended to `.data` since the last commit (for roll decisions
    /// before the sync).
    first_seq: Option<u64>,
    last_seq: u64,
}

impl ActiveSegment {
    /// Create a fresh, empty segment: both headers written and synced so a
    /// crash right after leaves two well-formed empty files.
    pub fn create(dir: &Path, partition_id: u32, base_seq: u64, codec: u8) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let mut data = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(data_path(dir, partition_id))?;
        data.write_all(&DataHeader::new(partition_id, base_seq, codec).encode())?;
        data.sync_all()?;
        let mut idx = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(idx_path(dir, partition_id))?;
        idx.write_all(&IdxHeader::new(partition_id, base_seq, 0).encode())?;
        idx.sync_all()?;
        // The directory entries must survive a crash too, or acknowledged
        // records could live in a file nobody can find (ext4 delayed alloc).
        fsync_dir(dir);
        Ok(Self {
            dir: dir.to_path_buf(),
            partition_id,
            base_seq,
            codec,
            data: BufWriter::with_capacity(256 * 1024, data),
            idx: BufWriter::with_capacity(64 * 1024, idx),
            data_len: FILE_HEADER_LEN as u64,
            count: 0,
            pending_idx: Vec::new(),
            first_seq: None,
            last_seq: 0,
        })
    }

    /// Reopen an existing segment for appending, **after** recovery has
    /// made `.data` and `.idx` consistent (`super::recover`). `data_len`
    /// and `count` are what recovery found; `last_seq` is the last record's
    /// seq (or `base_seq - 1` for an empty segment).
    pub fn reopen(
        dir: &Path,
        partition_id: u32,
        header: DataHeader,
        data_len: u64,
        count: u32,
        last_seq: u64,
    ) -> io::Result<Self> {
        let mut data = OpenOptions::new()
            .write(true)
            .open(data_path(dir, partition_id))?;
        data.seek(SeekFrom::Start(data_len))?;
        let mut idx = OpenOptions::new()
            .write(true)
            .open(idx_path(dir, partition_id))?;
        idx.seek(SeekFrom::Start(IdxEntry::file_offset(count as usize) as u64))?;
        Ok(Self {
            dir: dir.to_path_buf(),
            partition_id,
            base_seq: header.base_seq,
            codec: header.codec,
            data: BufWriter::with_capacity(256 * 1024, data),
            idx: BufWriter::with_capacity(64 * 1024, idx),
            data_len,
            count,
            pending_idx: Vec::new(),
            first_seq: None,
            last_seq,
        })
    }

    pub fn partition_id(&self) -> u32 {
        self.partition_id
    }
    pub fn base_seq(&self) -> u64 {
        self.base_seq
    }
    pub fn codec(&self) -> u8 {
        self.codec
    }
    /// Bytes of `.data` including the header and uncommitted records.
    pub fn data_len(&self) -> u64 {
        self.data_len
    }
    /// Records in the segment, committed or pending.
    pub fn record_count(&self) -> u32 {
        self.count + self.pending_idx.len() as u32
    }
    pub fn last_seq(&self) -> u64 {
        self.last_seq
    }
    pub fn has_pending(&self) -> bool {
        !self.pending_idx.is_empty()
    }

    /// Buffer one record. Not durable until [`commit`](Self::commit).
    /// Returns the record's offset in `.data`.
    pub fn append(
        &mut self,
        seq: u64,
        ts_micros: u64,
        kind: Kind,
        payload: &[u8],
        fields: IndexFields,
    ) -> io::Result<u64> {
        debug_assert!(seq > self.last_seq || self.first_seq.is_none() && self.count == 0);
        let offset = self.data_len;
        let mut buf = Vec::with_capacity(payload.len() + 40);
        encode_record(&mut buf, seq, ts_micros, kind, self.codec, payload);
        self.data.write_all(&buf)?;
        self.data_len += buf.len() as u64;
        self.pending_idx.push(IdxEntry {
            seq,
            offset,
            payload_len: payload.len() as u32,
            kind,
            flags: 0,
            ns_id: fields.ns_id,
            entity_id: fields.entity_id,
            endpoints: fields.endpoints,
            rtype_sym: fields.rtype_sym,
            target_ns_id: fields.target_ns_id,
        });
        self.first_seq.get_or_insert(seq);
        self.last_seq = seq;
        Ok(offset)
    }

    /// Durability barrier: flush + `fdatasync` the data, **then** write the
    /// pending index entries (flushed, not synced). Returns the number of
    /// records made durable.
    pub fn commit(&mut self) -> io::Result<usize> {
        if self.pending_idx.is_empty() {
            return Ok(0);
        }
        self.data.flush()?;
        self.data.get_ref().sync_data()?;
        let n = self.pending_idx.len();
        for e in self.pending_idx.drain(..) {
            self.idx.write_all(&e.encode())?;
        }
        self.idx.flush()?;
        self.count += n as u32;
        Ok(n)
    }

    /// Read back the record at `offset` (header + payload span) with a
    /// positional read on the data file. Used for reads that hit the
    /// active segment before it is sealed.
    pub fn read_span(&mut self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        // Anything still buffered must reach the file first.
        self.data.flush()?;
        let mut f = File::open(data_path(&self.dir, self.partition_id))?;
        f.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0u8; len];
        f.read_exact(&mut buf)?;
        Ok(buf)
    }

    /// Every committed index entry, read back from the `.idx` file (the
    /// pending ones are not included). Used to rebuild `.xref` files at open
    /// and after compaction.
    pub fn committed_entries(&mut self) -> io::Result<Vec<IdxEntry>> {
        self.idx.flush()?;
        let mut bytes = Vec::new();
        File::open(idx_path(&self.dir, self.partition_id))?.read_to_end(&mut bytes)?;
        let n = IdxEntry::count_in(bytes.len() as u64).min(self.count as usize);
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            out.push(
                IdxEntry::decode(&bytes, IdxEntry::file_offset(i))
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?,
            );
        }
        Ok(out)
    }

    /// Committed bytes of `.data` past the file header, for a sequential
    /// scan of the active segment (hydration, D4).
    pub fn committed_data(&mut self) -> io::Result<Vec<u8>> {
        let len = self.data_len as usize;
        if len <= FILE_HEADER_LEN {
            return Ok(Vec::new());
        }
        self.read_span(FILE_HEADER_LEN as u64, len - FILE_HEADER_LEN)
    }

    /// Delete this segment's files. Consumes the segment; used by
    /// compaction once the replacement is durable.
    pub fn remove_files(self) -> io::Result<()> {
        let (dir, id) = (self.dir.clone(), self.partition_id);
        drop(self);
        remove_segment_files(&dir, id)
    }

    /// Close the segment for good: commit, stamp the final count into the
    /// `.idx` header, sync both files once, and hand back the immutable,
    /// memory-mapped form. After this the files never change (H11).
    pub fn seal(mut self) -> io::Result<SealedSegment> {
        self.commit()?;
        self.data.flush()?;
        self.data.get_ref().sync_all()?;
        self.idx.flush()?;
        {
            let f = self.idx.get_mut();
            f.seek(SeekFrom::Start(0))?;
            f.write_all(&IdxHeader::new(self.partition_id, self.base_seq, self.count).encode())?;
            f.sync_all()?;
        }
        drop(self.data);
        drop(self.idx);
        SealedSegment::open(&self.dir, self.partition_id)
    }

    /// Size the `.idx` file must have for `count` entries.
    pub fn idx_len_for(count: u32) -> u64 {
        (FILE_HEADER_LEN + count as usize * IDX_ENTRY_LEN) as u64
    }
}

/// Delete the `.data`, `.idx` and `.xref` of a partition. On Windows a file
/// that is still memory-mapped cannot be deleted; it is renamed to `.old`
/// instead and swept at the next open (`sweep_old_files`).
pub fn remove_segment_files(dir: &Path, partition_id: u32) -> io::Result<()> {
    for p in [
        data_path(dir, partition_id),
        idx_path(dir, partition_id),
        super::xref::xref_path(dir, partition_id),
    ] {
        if !p.exists() {
            continue;
        }
        if let Err(e) = std::fs::remove_file(&p) {
            let old = p.with_extension(format!(
                "{}.old",
                p.extension().and_then(|x| x.to_str()).unwrap_or("bin")
            ));
            std::fs::rename(&p, &old).map_err(|_| e)?;
        }
    }
    Ok(())
}

/// Remove leftover `*.old` (undeletable mapped files) and `*.tmp`
/// (interrupted atomic writes) from a previous run.
pub fn sweep_old_files(dir: &Path) -> io::Result<usize> {
    let mut n = 0;
    for entry in std::fs::read_dir(dir)? {
        let p = entry?.path();
        let ext = p.extension().and_then(|e| e.to_str());
        if matches!(ext, Some("old") | Some("tmp")) && std::fs::remove_file(&p).is_ok() {
            n += 1;
        }
    }
    Ok(n)
}

/// Move the `.data` / `.idx` (and `.xref` if any) of a partition from one
/// directory to another (same filesystem: a rename per file).
pub fn move_segment_files(from: &Path, to: &Path, partition_id: u32) -> io::Result<()> {
    std::fs::create_dir_all(to)?;
    for (src, dst) in [
        (data_path(from, partition_id), data_path(to, partition_id)),
        (idx_path(from, partition_id), idx_path(to, partition_id)),
        (
            super::xref::xref_path(from, partition_id),
            super::xref::xref_path(to, partition_id),
        ),
    ] {
        if src.exists() {
            std::fs::rename(&src, &dst)?;
        }
    }
    fsync_dir(to);
    Ok(())
}

/// Flush a directory's entries to disk where the platform allows (Unix);
/// on Windows metadata writes are not reordered past a file's own sync.
pub fn fsync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}
