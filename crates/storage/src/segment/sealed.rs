// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! An immutable segment (H11): both files memory-mapped once, index entries
//! addressed positionally (D1), payloads sliced straight out of the map.

use std::fs::File;
use std::io;
use std::path::Path;

use memmap2::Mmap;

use super::active::{data_path, idx_path};
use super::format::{
    decode_record, record_span, DataHeader, FormatError, IdxEntry, IdxHeader, RecordView,
    FILE_HEADER_LEN,
};

pub struct SealedSegment {
    partition_id: u32,
    data_header: DataHeader,
    idx_header: IdxHeader,
    data: Mmap,
    idx: Mmap,
    count: usize,
}

impl SealedSegment {
    /// Map both files. The index header's `count` is authoritative for a
    /// sealed segment, but it is cross-checked against the file length so a
    /// half-written seal is refused rather than half-read.
    pub fn open(dir: &Path, partition_id: u32) -> io::Result<Self> {
        let data_file = File::open(data_path(dir, partition_id))?;
        let idx_file = File::open(idx_path(dir, partition_id))?;
        // SAFETY: the files are sealed — never written again by this process
        // (H11) — and the store holds them open for the map's lifetime.
        let data = unsafe { Mmap::map(&data_file)? };
        let idx = unsafe { Mmap::map(&idx_file)? };
        let data_header = DataHeader::decode(&data).map_err(fmt_err)?;
        let idx_header = IdxHeader::decode(&idx).map_err(fmt_err)?;
        let by_len = IdxEntry::count_in(idx.len() as u64);
        if idx_header.count as usize != by_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "partition {partition_id}: idx header says {} entries, file holds {by_len}",
                    idx_header.count
                ),
            ));
        }
        // The index must account for every byte of data: a segment whose
        // seal did not complete (count stamped 0, or entries lost with the
        // page cache) is not sealed, whatever its header says.
        let covered = if by_len == 0 {
            FILE_HEADER_LEN
        } else {
            let last =
                IdxEntry::decode(&idx, IdxEntry::file_offset(by_len - 1)).map_err(fmt_err)?;
            last.offset as usize + record_span(last.payload_len as usize)
        };
        if covered != data.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "partition {partition_id}: idx covers {covered} bytes of data, file holds {}",
                    data.len()
                ),
            ));
        }
        if data_header.partition_id != partition_id || idx_header.partition_id != partition_id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("partition {partition_id}: header partition id mismatch"),
            ));
        }
        #[cfg(unix)]
        {
            let _ = data.advise(memmap2::Advice::Random);
            let _ = idx.advise(memmap2::Advice::Sequential);
        }
        Ok(Self {
            partition_id,
            data_header,
            idx_header,
            data,
            idx,
            count: by_len,
        })
    }

    pub fn partition_id(&self) -> u32 {
        self.partition_id
    }
    pub fn base_seq(&self) -> u64 {
        self.data_header.base_seq
    }
    pub fn codec(&self) -> u8 {
        self.data_header.codec
    }
    pub fn record_count(&self) -> usize {
        self.count
    }
    pub fn data_len(&self) -> u64 {
        self.data.len() as u64
    }
    pub fn idx_header(&self) -> IdxHeader {
        self.idx_header
    }

    /// Entry `i` (positional, D1).
    pub fn entry(&self, i: usize) -> Option<IdxEntry> {
        if i >= self.count {
            return None;
        }
        IdxEntry::decode(&self.idx, IdxEntry::file_offset(i)).ok()
    }

    pub fn entries(&self) -> impl Iterator<Item = IdxEntry> + '_ {
        (0..self.count).filter_map(move |i| self.entry(i))
    }

    /// Position of the entry with `seq`, by binary search (seqs are
    /// increasing within a partition).
    pub fn find_seq(&self, seq: u64) -> Option<usize> {
        let mut lo = 0usize;
        let mut hi = self.count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let s = self.entry(mid)?.seq;
            match s.cmp(&seq) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(mid),
            }
        }
        None
    }

    /// Seq of the last record, if any.
    pub fn last_seq(&self) -> Option<u64> {
        self.entry(self.count.checked_sub(1)?).map(|e| e.seq)
    }

    /// The record described by `entry`. `verify_crc` on hydration and
    /// recovery paths only (`STORAGE.md` §7.3).
    pub fn record(
        &self,
        entry: &IdxEntry,
        verify_crc: bool,
    ) -> Result<RecordView<'_>, FormatError> {
        decode_record(&self.data, entry.offset as usize, verify_crc)
    }

    /// Every record in file order, straight from the data map. This is the
    /// hydration path in P0 (D4): the index is only used to know where the
    /// records are.
    pub fn records(
        &self,
        verify_crc: bool,
    ) -> impl Iterator<Item = Result<RecordView<'_>, FormatError>> {
        let mut at = FILE_HEADER_LEN;
        let end = self.data.len();
        std::iter::from_fn(move || {
            if at >= end {
                return None;
            }
            match decode_record(&self.data, at, verify_crc) {
                Ok(v) => {
                    at = v.next;
                    Some(Ok(v))
                }
                Err(e) => {
                    at = end;
                    Some(Err(e))
                }
            }
        })
    }

    /// Raw byte slice of the data map (for tools and tests).
    pub fn data_bytes(&self) -> &[u8] {
        &self.data
    }
}

impl std::fmt::Debug for SealedSegment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SealedSegment")
            .field("partition_id", &self.partition_id)
            .field("base_seq", &self.data_header.base_seq)
            .field("records", &self.count)
            .field("data_len", &self.data.len())
            .finish()
    }
}

fn fmt_err(e: FormatError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}
