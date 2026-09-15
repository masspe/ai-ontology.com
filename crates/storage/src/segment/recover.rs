// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Recovery of a segment that may have been interrupted mid-write
//! (`STORAGE.md` §9). Only the active segment of a stream can be
//! inconsistent (H11 protects the sealed ones), but the procedure is the
//! same for any segment and is also what regenerates a lost `.idx` (R7).
//!
//! 1. Scan `.data` from the header, decoding and CRC-checking each record.
//! 2. Stop at the first record that is torn (short) or fails its CRC or
//!    carries an unknown kind; truncate the file there.
//! 3. Rebuild the `.idx` from the records actually found — reusing the
//!    existing entries as long as they agree, rewriting from the first
//!    disagreement.
//! 4. Report `data_len`, `count` and the last valid seq so the caller can
//!    resume `next_seq`.
//!
//! The index fields that cannot be derived from a record header alone
//! (`ns_id`, `entity_id`, `endpoints`, `rtype_sym`) are recomputed from the
//! decoded payload by the caller-supplied resolver — the store knows how to
//! turn a payload back into those fields; this module does not.

use std::fs::OpenOptions;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use tracing::warn;

use super::active::{data_path, idx_path, IndexFields};
use super::format::{
    decode_record, record_span, DataHeader, FormatError, IdxEntry, IdxHeader, Kind, RecordHeader,
    RecordView, FILE_HEADER_LEN, IDX_ENTRY_LEN,
};

/// What recovery established about a segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovered {
    pub header: DataHeader,
    /// Length of `.data` after truncation: header + complete records.
    pub data_len: u64,
    /// Complete records found (= entries in the rebuilt `.idx`).
    pub count: u32,
    /// Seq of the last complete record, `None` when empty.
    pub last_seq: Option<u64>,
    /// Bytes discarded from the tail of `.data` (0 when clean).
    pub truncated_bytes: u64,
    /// Number of `.idx` entries rewritten (0 when the index was in sync).
    pub idx_rewritten: usize,
}

/// Recover the segment `partition_id` in `dir`. `resolve` turns a decoded
/// record into its index fields (the store resolves ids and symbols).
pub fn recover_segment(
    dir: &Path,
    partition_id: u32,
    mut resolve: impl FnMut(Kind, &[u8]) -> Result<IndexFields, String>,
) -> io::Result<Recovered> {
    let dpath = data_path(dir, partition_id);
    let ipath = idx_path(dir, partition_id);

    let mut data = Vec::new();
    OpenOptions::new()
        .read(true)
        .open(&dpath)?
        .read_to_end(&mut data)?;
    let header = DataHeader::decode(&data).map_err(|e| invalid(partition_id, e.to_string()))?;

    // The existing index, if any, is read first: it tells the span of a
    // record whose header no longer decodes, which is what separates a torn
    // tail from corruption in the middle of the file.
    let mut idx_bytes = Vec::new();
    let idx_exists = ipath.exists();
    if idx_exists {
        OpenOptions::new()
            .read(true)
            .open(&ipath)?
            .read_to_end(&mut idx_bytes)?;
    }

    // 1-2. Scan.
    let mut entries: Vec<IdxEntry> = Vec::new();
    let mut at = FILE_HEADER_LEN;
    let mut last_seq = None;
    let mut cut_reason: Option<String> = None;
    while at < data.len() {
        // A record that fails for any reason other than running past EOF is
        // only discarded when it is the *last* one: a torn write can only be
        // at the tail. A bad record with valid data after it is corruption or
        // a newer format, and truncating would destroy acknowledged records.
        let refuse_unless_last = |at: usize, what: String| -> io::Result<String> {
            let span = match RecordHeader::decode(&data, at) {
                Ok(h) if Kind::from_u8(h.kind as u8).is_ok() => {
                    Some(record_span(h.payload_len as usize))
                }
                _ => IdxEntry::decode(&idx_bytes, IdxEntry::file_offset(entries.len()))
                    .ok()
                    .filter(|e| e.offset as usize == at)
                    .map(|e| record_span(e.payload_len as usize)),
            };
            match span {
                Some(span) if at + span >= data.len() => Ok(what),
                Some(_) => Err(invalid(
                    partition_id,
                    format!("{what}; valid records follow it, refusing to truncate"),
                )),
                None => Err(invalid(
                    partition_id,
                    format!("{what}; its extent is unknown, refusing to truncate"),
                )),
            }
        };
        match decode_record(&data, at, true) {
            Ok(v) => {
                let fields = match resolve(v.header.kind, v.payload) {
                    Ok(f) => f,
                    Err(e) => {
                        cut_reason = Some(refuse_unless_last(
                            at,
                            format!("record at {at} does not decode: {e}"),
                        )?);
                        break;
                    }
                };
                if let Some(prev) = last_seq {
                    if v.header.seq <= prev {
                        cut_reason = Some(refuse_unless_last(
                            at,
                            format!("record at {at} has seq {} <= previous {prev}", v.header.seq),
                        )?);
                        break;
                    }
                }
                entries.push(entry_for(&v, fields));
                last_seq = Some(v.header.seq);
                at = v.next;
            }
            Err(e @ FormatError::Truncated { .. }) => {
                cut_reason = Some(format!("torn record at {at}: {e}"));
                break;
            }
            Err(e) => {
                cut_reason = Some(refuse_unless_last(
                    at,
                    format!("corrupt record at {at}: {e}"),
                )?);
                break;
            }
        }
    }
    let data_len = at as u64;
    let truncated_bytes = data.len() as u64 - data_len;
    if let Some(reason) = cut_reason {
        warn!(
            partition = partition_id,
            discarded_bytes = truncated_bytes,
            offset = data_len,
            reason,
            "segment tail discarded during recovery"
        );
        let f = OpenOptions::new().write(true).open(&dpath)?;
        f.set_len(data_len)?;
        f.sync_all()?;
    }

    // 3. Reconcile the index with what the data actually holds.
    let header_ok = IdxHeader::decode(&idx_bytes)
        .map(|h| h.partition_id == partition_id && h.base_seq == header.base_seq)
        .unwrap_or(false);
    let mut agree = 0usize;
    if header_ok {
        let present = IdxEntry::count_in(idx_bytes.len() as u64);
        while agree < present && agree < entries.len() {
            match IdxEntry::decode(&idx_bytes, IdxEntry::file_offset(agree)) {
                Ok(e) if e == entries[agree] => agree += 1,
                _ => break,
            }
        }
    }
    let must_rewrite_from = if header_ok { agree } else { 0 };
    let idx_rewritten = entries.len() - must_rewrite_from;
    let expected_len = (FILE_HEADER_LEN + entries.len() * IDX_ENTRY_LEN) as u64;
    if idx_rewritten > 0 || !header_ok || idx_bytes.len() as u64 != expected_len {
        if !header_ok && idx_exists {
            warn!(
                partition = partition_id,
                "idx header unusable; regenerating index"
            );
        }
        let mut f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&ipath)?;
        if !header_ok {
            f.seek(SeekFrom::Start(0))?;
            f.write_all(&IdxHeader::new(partition_id, header.base_seq, 0).encode())?;
        }
        f.seek(SeekFrom::Start(
            IdxEntry::file_offset(must_rewrite_from) as u64
        ))?;
        let mut buf = Vec::with_capacity(idx_rewritten * IDX_ENTRY_LEN);
        for e in &entries[must_rewrite_from..] {
            buf.extend_from_slice(&e.encode());
        }
        f.write_all(&buf)?;
        f.set_len(expected_len)?;
        f.flush()?;
        // The index is derived (R7); syncing it here is cheap insurance on
        // a path that only runs at startup.
        f.sync_all()?;
    }

    Ok(Recovered {
        header,
        data_len,
        count: entries.len() as u32,
        last_seq,
        truncated_bytes,
        idx_rewritten,
    })
}

fn entry_for(v: &RecordView<'_>, f: IndexFields) -> IdxEntry {
    IdxEntry {
        seq: v.header.seq,
        offset: v.offset as u64,
        payload_len: v.header.payload_len,
        kind: v.header.kind,
        flags: 0,
        ns_id: f.ns_id,
        entity_id: f.entity_id,
        endpoints: f.endpoints,
        rtype_sym: f.rtype_sym,
        target_ns_id: f.target_ns_id,
    }
}

fn invalid(partition_id: u32, msg: String) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("partition {partition_id}: {msg}"),
    )
}
