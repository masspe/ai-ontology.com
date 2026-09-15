// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! `.xref` — incoming cross-domain edges of a partition (`STORAGE.md`
//! §4.4): for every relation whose **target** lives in this partition's
//! domain but whose record was written to another domain's stream, one
//! 24-byte entry `(target_id, source_id, seq)`. No payload: the target
//! domain keeps a complete incoming adjacency even when loaded alone.
//!
//! The file is fully **derived** (R7): its content is a pure function of
//! the other streams' indexes, so it is never synced, and it is rebuilt from
//! scratch at every open — which also makes it exact after a crash or a
//! compaction. It sits next to `.data`/`.idx` but is *not* covered by their
//! immutability rule: rebuilding a sealed partition's `.xref` is allowed.
//!
//! Which partition an entry belongs to is decided by **seq**: partition `p`
//! of the target domain holds the entries whose relation `seq` falls in
//! `[p.base_seq, next_partition.base_seq)`.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use super::active::segment_stem;
use super::format::{FormatError, FILE_HEADER_LEN, FORMAT_VERSION};

pub const XREF_MAGIC: [u8; 4] = *b"GRFX";
pub const XREF_ENTRY_LEN: usize = 24;

pub fn xref_path(dir: &Path, partition_id: u32) -> PathBuf {
    dir.join(format!("{}.xref", segment_stem(partition_id)))
}

/// One incoming cross-domain edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct XrefEntry {
    pub seq: u64,
    pub target_id: u64,
    pub source_id: u64,
}

impl XrefEntry {
    pub fn encode(&self) -> [u8; XREF_ENTRY_LEN] {
        let mut b = [0u8; XREF_ENTRY_LEN];
        b[0..8].copy_from_slice(&self.target_id.to_le_bytes());
        b[8..16].copy_from_slice(&self.source_id.to_le_bytes());
        b[16..24].copy_from_slice(&self.seq.to_le_bytes());
        b
    }
    pub fn decode(buf: &[u8], at: usize) -> Result<Self, FormatError> {
        let b = buf
            .get(at..at + XREF_ENTRY_LEN)
            .ok_or(FormatError::Truncated {
                at,
                need: XREF_ENTRY_LEN,
                have: buf.len().saturating_sub(at),
            })?;
        Ok(Self {
            target_id: u64::from_le_bytes(b[0..8].try_into().unwrap()),
            source_id: u64::from_le_bytes(b[8..16].try_into().unwrap()),
            seq: u64::from_le_bytes(b[16..24].try_into().unwrap()),
        })
    }
}

/// Header of an `.xref` file (32 bytes, same shape as the others):
/// magic, version u16, entry_size u16, partition_id u32, base_seq u64,
/// count u32, reserved.
fn encode_header(partition_id: u32, base_seq: u64, count: u32) -> [u8; FILE_HEADER_LEN] {
    let mut b = [0u8; FILE_HEADER_LEN];
    b[0..4].copy_from_slice(&XREF_MAGIC);
    b[4..6].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    b[6..8].copy_from_slice(&(XREF_ENTRY_LEN as u16).to_le_bytes());
    b[8..12].copy_from_slice(&partition_id.to_le_bytes());
    b[12..20].copy_from_slice(&base_seq.to_le_bytes());
    b[20..24].copy_from_slice(&count.to_le_bytes());
    b
}

/// Write (replace) the `.xref` of `partition_id` with `entries`, sorted by
/// seq. Written to a temp file and renamed so a reader never sees a torn
/// file; not synced — derived data (R7).
pub fn write_xref(
    dir: &Path,
    partition_id: u32,
    base_seq: u64,
    entries: &mut [XrefEntry],
) -> io::Result<()> {
    entries.sort();
    let dest = xref_path(dir, partition_id);
    let tmp = dir.join(format!("{}.xref.tmp", segment_stem(partition_id)));
    {
        let mut f = File::create(&tmp)?;
        let mut buf = Vec::with_capacity(FILE_HEADER_LEN + entries.len() * XREF_ENTRY_LEN);
        buf.extend_from_slice(&encode_header(partition_id, base_seq, entries.len() as u32));
        for e in entries.iter() {
            buf.extend_from_slice(&e.encode());
        }
        f.write_all(&buf)?;
        f.flush()?;
    }
    std::fs::rename(&tmp, &dest)?;
    Ok(())
}

/// Read the `.xref` of `partition_id`; an absent file is an empty list.
pub fn read_xref(dir: &Path, partition_id: u32) -> io::Result<Vec<XrefEntry>> {
    let p = xref_path(dir, partition_id);
    if !p.exists() {
        return Ok(Vec::new());
    }
    let mut bytes = Vec::new();
    File::open(&p)?.read_to_end(&mut bytes)?;
    if bytes.len() < FILE_HEADER_LEN || bytes[0..4] != XREF_MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: not an xref file", p.display()),
        ));
    }
    let count = (bytes.len() - FILE_HEADER_LEN) / XREF_ENTRY_LEN;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        out.push(
            XrefEntry::decode(&bytes, FILE_HEADER_LEN + i * XREF_ENTRY_LEN)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?,
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xref_round_trips_sorted_by_seq() {
        let dir = std::env::temp_dir().join(format!(
            "ontology-xref-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read_xref(&dir, 3).unwrap().is_empty());
        let mut entries = vec![
            XrefEntry {
                seq: 9,
                target_id: 1,
                source_id: 2,
            },
            XrefEntry {
                seq: 4,
                target_id: 1,
                source_id: 3,
            },
        ];
        write_xref(&dir, 3, 1, &mut entries).unwrap();
        let back = read_xref(&dir, 3).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].seq, 4);
        assert_eq!(back[1].seq, 9);
        // Rewrite replaces.
        write_xref(&dir, 3, 1, &mut []).unwrap();
        assert!(read_xref(&dir, 3).unwrap().is_empty());
        assert!(!dir.join("000003.xref.tmp").exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
