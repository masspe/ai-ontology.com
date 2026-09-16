// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Segment I/O contract: append → commit → seal round-trips through the
//! memory map; a lost or stale `.idx` is rebuilt identically from `.data`
//! (R7); a torn tail at any offset is truncated and the index reconciled
//! (§9); a corrupt sealed payload is caught by the CRC.

use ontology_storage::segment::{
    data_path, idx_path, recover_segment, ActiveSegment, IdxEntry, IndexFields, Kind,
    SealedSegment, CODEC_JSON, FILE_HEADER_LEN, IDX_ENTRY_LEN, RECORD_HEADER_LEN,
};
use std::path::{Path, PathBuf};

fn tempdir(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "ontology-segment-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// Test resolver: index fields are a deterministic function of the payload
/// so a rebuilt index must equal the original one byte for byte.
fn resolve(_kind: Kind, _codec: u8, payload: &[u8]) -> Result<IndexFields, String> {
    if payload.first() == Some(&0xFF) {
        return Err("poison payload".into());
    }
    Ok(fields_for(payload))
}
fn fields_for(payload: &[u8]) -> IndexFields {
    let sum: u64 = payload.iter().map(|b| *b as u64).sum();
    IndexFields {
        ns_id: 1,
        entity_id: sum,
        endpoints: sum << 1,
        rtype_sym: (sum % 7) as u32,
        target_ns_id: (sum % 3) as u16,
    }
}

fn payload(i: u64) -> Vec<u8> {
    format!(
        "{{\"i\":{i},\"pad\":\"{}\"}}",
        "x".repeat((i % 13) as usize * 3)
    )
    .into_bytes()
}

/// Write `n` records in `batches` commits; returns the (offset, span) of each.
fn write_segment(dir: &Path, n: u64, batches: usize) -> ActiveSegment {
    let mut seg = ActiveSegment::create(dir, 1, 1, CODEC_JSON).unwrap();
    let per = (n as usize).div_ceil(batches.max(1));
    for i in 0..n {
        let p = payload(i);
        seg.append(i + 1, 1_000 + i, Kind::Concept, &p, fields_for(&p))
            .unwrap();
        if (i as usize + 1).is_multiple_of(per) {
            seg.commit().unwrap();
        }
    }
    seg.commit().unwrap();
    seg
}

fn entries_of(dir: &Path, partition: u32) -> Vec<IdxEntry> {
    let bytes = std::fs::read(idx_path(dir, partition)).unwrap();
    (0..IdxEntry::count_in(bytes.len() as u64))
        .map(|i| IdxEntry::decode(&bytes, IdxEntry::file_offset(i)).unwrap())
        .collect()
}

#[test]
fn append_commit_seal_round_trips_through_the_map() {
    let dir = tempdir("roundtrip");
    let seg = write_segment(&dir, 25, 3);
    assert_eq!(seg.record_count(), 25);
    assert_eq!(seg.last_seq(), 25);
    let sealed = seg.seal().unwrap();

    assert_eq!(sealed.record_count(), 25);
    assert_eq!(sealed.base_seq(), 1);
    assert_eq!(sealed.last_seq(), Some(25));
    assert_eq!(sealed.idx_header().count, 25);

    // Positional entries agree with the records they point at.
    let mut i = 0u64;
    for e in sealed.entries() {
        assert_eq!(e.seq, i + 1);
        let expect = fields_for(&payload(i));
        assert_eq!(
            (e.entity_id, e.endpoints, e.rtype_sym),
            (expect.entity_id, expect.endpoints, expect.rtype_sym)
        );
        let v = sealed.record(&e, true).unwrap();
        assert_eq!(v.payload, &payload(i)[..]);
        assert_eq!(v.header.seq, e.seq);
        assert_eq!(v.header.ts_micros, 1_000 + i);
        i += 1;
    }
    assert_eq!(i, 25);

    // Sequential scan sees the same records, in order.
    let scanned: Vec<u64> = sealed
        .records(true)
        .map(|r| r.unwrap().header.seq)
        .collect();
    assert_eq!(scanned, (1..=25).collect::<Vec<_>>());

    // Binary search by seq (D1).
    assert_eq!(sealed.find_seq(1), Some(0));
    assert_eq!(sealed.find_seq(13), Some(12));
    assert_eq!(sealed.find_seq(25), Some(24));
    assert_eq!(sealed.find_seq(26), None);
    assert_eq!(sealed.find_seq(0), None);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_empty_segment_seals_and_reopens() {
    let dir = tempdir("empty");
    let seg = ActiveSegment::create(&dir, 7, 100, CODEC_JSON).unwrap();
    let sealed = seg.seal().unwrap();
    assert_eq!(sealed.record_count(), 0);
    assert_eq!(sealed.last_seq(), None);
    assert_eq!(sealed.records(true).count(), 0);
    drop(sealed);
    let r = recover_segment(&dir, 7, resolve).unwrap();
    assert_eq!(r.count, 0);
    assert_eq!(r.last_seq, None);
    assert_eq!(r.truncated_bytes, 0);
    assert_eq!(r.data_len, FILE_HEADER_LEN as u64);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_lost_index_is_rebuilt_byte_for_byte() {
    let dir = tempdir("lost-idx");
    let seg = write_segment(&dir, 40, 4);
    drop(seg);
    let original = std::fs::read(idx_path(&dir, 1)).unwrap();
    std::fs::remove_file(idx_path(&dir, 1)).unwrap();

    let r = recover_segment(&dir, 1, resolve).unwrap();
    assert_eq!(r.count, 40);
    assert_eq!(r.last_seq, Some(40));
    assert_eq!(r.truncated_bytes, 0);
    assert_eq!(r.idx_rewritten, 40);

    let rebuilt = std::fs::read(idx_path(&dir, 1)).unwrap();
    // The active header carries count 0 (only a seal stamps it); everything
    // after the header must be identical.
    assert_eq!(&rebuilt[FILE_HEADER_LEN..], &original[FILE_HEADER_LEN..]);
    assert_eq!(rebuilt.len(), original.len());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_stale_index_is_completed_not_rewritten() {
    let dir = tempdir("stale-idx");
    let seg = write_segment(&dir, 30, 1);
    drop(seg);
    // Simulate a crash between the data sync and the idx write of the last
    // 10 records: chop the index.
    let ipath = idx_path(&dir, 1);
    let f = std::fs::OpenOptions::new()
        .write(true)
        .open(&ipath)
        .unwrap();
    f.set_len((FILE_HEADER_LEN + 20 * IDX_ENTRY_LEN) as u64)
        .unwrap();
    drop(f);

    let r = recover_segment(&dir, 1, resolve).unwrap();
    assert_eq!(r.count, 30);
    assert_eq!(r.idx_rewritten, 10, "only the missing tail is written");
    assert_eq!(r.truncated_bytes, 0);
    let entries = entries_of(&dir, 1);
    assert_eq!(entries.len(), 30);
    assert_eq!(entries[29].seq, 30);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_disagreeing_index_is_rewritten_from_the_first_bad_entry() {
    let dir = tempdir("bad-idx");
    let seg = write_segment(&dir, 12, 1);
    drop(seg);
    let ipath = idx_path(&dir, 1);
    let mut bytes = std::fs::read(&ipath).unwrap();
    // Corrupt entry 5's entity_id.
    let at = IdxEntry::file_offset(5) + 24;
    bytes[at] ^= 0x55;
    std::fs::write(&ipath, &bytes).unwrap();

    let r = recover_segment(&dir, 1, resolve).unwrap();
    assert_eq!(r.idx_rewritten, 12 - 5);
    let good = entries_of(&dir, 1);
    assert_eq!(good[5].entity_id, fields_for(&payload(5)).entity_id);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn torn_tail_at_every_offset_truncates_to_the_last_complete_record() {
    let dir = tempdir("torn-src");
    let seg = write_segment(&dir, 8, 8);
    drop(seg);
    let data = std::fs::read(data_path(&dir, 1)).unwrap();
    let idx = std::fs::read(idx_path(&dir, 1)).unwrap();
    let entries = entries_of(&dir, 1);
    let ends: Vec<u64> = entries
        .iter()
        .map(|e| e.offset + (RECORD_HEADER_LEN + e.payload_len.div_ceil(8) as usize * 8) as u64)
        .collect();
    assert_eq!(*ends.last().unwrap(), data.len() as u64);

    let work = tempdir("torn-cut");
    // Every offset of the last two records, every boundary ±1/±4, a sample
    // of the rest.
    let tail_start = ends[ends.len() - 3];
    let mut cuts: Vec<u64> = (FILE_HEADER_LEN as u64..tail_start).step_by(11).collect();
    cuts.extend(tail_start..=data.len() as u64);
    for e in &ends {
        cuts.extend([*e, e + 1, e + 4, e.saturating_sub(1)]);
    }
    cuts.push(FILE_HEADER_LEN as u64);
    cuts.sort_unstable();
    cuts.dedup();
    cuts.retain(|c| *c >= FILE_HEADER_LEN as u64 && *c <= data.len() as u64);

    for cut in cuts {
        std::fs::write(data_path(&work, 1), &data[..cut as usize]).unwrap();
        // The index is whatever the crash left: sometimes complete, sometimes
        // behind. Alternate to exercise both.
        if cut % 2 == 0 {
            std::fs::write(idx_path(&work, 1), &idx).unwrap();
        } else {
            let _ = std::fs::remove_file(idx_path(&work, 1));
        }

        let r = recover_segment(&work, 1, resolve).unwrap_or_else(|e| panic!("cut {cut}: {e}"));
        let complete = ends.iter().filter(|e| **e <= cut).count();
        let boundary = ends
            .get(complete.wrapping_sub(1))
            .copied()
            .unwrap_or(FILE_HEADER_LEN as u64);
        assert_eq!(r.count as usize, complete, "cut {cut}");
        assert_eq!(r.data_len, boundary, "cut {cut}");
        assert_eq!(r.truncated_bytes, cut - boundary, "cut {cut}");
        assert_eq!(
            r.last_seq,
            if complete == 0 {
                None
            } else {
                Some(complete as u64)
            }
        );
        assert_eq!(
            std::fs::metadata(data_path(&work, 1)).unwrap().len(),
            boundary
        );
        let rebuilt = entries_of(&work, 1);
        assert_eq!(
            rebuilt,
            entries[..complete].to_vec(),
            "cut {cut}: index != prefix"
        );

        // And the segment reopens for appending exactly where it stopped.
        let mut seg = ActiveSegment::reopen(
            &work,
            1,
            r.header,
            r.data_len,
            r.count,
            r.last_seq.unwrap_or(0),
        )
        .unwrap();
        let p = payload(99);
        seg.append(
            r.last_seq.unwrap_or(0) + 1,
            0,
            Kind::Concept,
            &p,
            fields_for(&p),
        )
        .unwrap();
        seg.commit().unwrap();
        let sealed = seg.seal().unwrap();
        assert_eq!(sealed.record_count(), complete + 1);
        let all: Vec<_> = sealed
            .records(true)
            .map(|r| r.unwrap().header.seq)
            .collect();
        let mut expect: Vec<u64> = (1..=complete as u64).collect();
        expect.push(complete as u64 + 1);
        assert_eq!(all, expect, "cut {cut}");
        drop(sealed);
    }
    std::fs::remove_dir_all(&dir).ok();
    std::fs::remove_dir_all(&work).ok();
}

#[test]
fn a_corrupt_payload_in_a_sealed_segment_is_caught_by_the_crc() {
    let dir = tempdir("crc");
    let seg = write_segment(&dir, 5, 1);
    drop(seg);
    let dpath = data_path(&dir, 1);
    let mut bytes = std::fs::read(&dpath).unwrap();
    let e = entries_of(&dir, 1)[2];
    bytes[e.offset as usize + RECORD_HEADER_LEN + 3] ^= 0x01;
    std::fs::write(&dpath, &bytes).unwrap();

    // Sealed read: the verifying path reports the CRC on record 3, the
    // records before it are fine.
    // (Stamp the idx header count as a seal would, so open() accepts it.)
    let ipath = idx_path(&dir, 1);
    let mut ib = std::fs::read(&ipath).unwrap();
    ib[20..24].copy_from_slice(&5u32.to_le_bytes());
    std::fs::write(&ipath, &ib).unwrap();
    let sealed = SealedSegment::open(&dir, 1).unwrap();
    let results: Vec<_> = sealed.records(true).collect();
    assert_eq!(results.len(), 3, "two good records then the error");
    assert!(results[0].is_ok() && results[1].is_ok());
    let err = results[2].as_ref().unwrap_err();
    assert!(err.to_string().contains("crc mismatch"), "{err}");
    // Non-verifying read still returns the frame (hot path, §7.3).
    assert!(sealed.record(&e, false).is_ok());
    drop(sealed);

    // Recovery refuses to truncate: valid records follow the bad one, so
    // this is corruption (or a newer format), not a torn tail.
    let err = recover_segment(&dir, 1, resolve).unwrap_err();
    assert!(err.to_string().contains("refusing to truncate"), "{err}");
    assert_eq!(
        std::fs::read(&dpath).unwrap(),
        bytes,
        "the file is left exactly as it was"
    );

    // The same corruption on the *last* record is a torn tail: discarded.
    std::fs::remove_dir_all(&dir).ok();
    let dir = tempdir("crc-last");
    let seg = write_segment(&dir, 5, 1);
    drop(seg);
    let dpath = data_path(&dir, 1);
    let mut bytes = std::fs::read(&dpath).unwrap();
    let last = entries_of(&dir, 1)[4];
    bytes[last.offset as usize + RECORD_HEADER_LEN + 3] ^= 0x01;
    std::fs::write(&dpath, &bytes).unwrap();
    let r = recover_segment(&dir, 1, resolve).unwrap();
    assert_eq!(r.count, 4);
    assert!(r.truncated_bytes > 0);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_sealed_index_that_does_not_cover_the_data_is_refused() {
    let dir = tempdir("uncovered");
    let seg = write_segment(&dir, 6, 1);
    let sealed = seg.seal().unwrap();
    drop(sealed);
    // Pretend the seal never happened: header count 0, no entries — as if
    // the page cache holding the index was lost before the seal's sync.
    let ipath = idx_path(&dir, 1);
    let mut ib = std::fs::read(&ipath).unwrap();
    ib.truncate(FILE_HEADER_LEN);
    ib[20..24].copy_from_slice(&0u32.to_le_bytes());
    std::fs::write(&ipath, &ib).unwrap();
    let err = SealedSegment::open(&dir, 1).unwrap_err();
    assert!(err.to_string().contains("covers"), "{err}");
    // Recovery rebuilds the index and the segment seals again.
    let r = recover_segment(&dir, 1, resolve).unwrap();
    assert_eq!(r.count, 6);
    assert_eq!(r.idx_rewritten, 6);
    let seg = ActiveSegment::reopen(&dir, 1, r.header, r.data_len, r.count, 6).unwrap();
    let sealed = seg.seal().unwrap();
    assert_eq!(sealed.record_count(), 6);
    assert_eq!(sealed.last_seq(), Some(6));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_payload_the_store_cannot_interpret_stops_the_scan() {
    let dir = tempdir("poison");
    let mut seg = ActiveSegment::create(&dir, 1, 1, CODEC_JSON).unwrap();
    let good = payload(1);
    seg.append(1, 0, Kind::Concept, &good, fields_for(&good))
        .unwrap();
    seg.append(
        2,
        0,
        Kind::Concept,
        &[0xFF, 1, 2],
        fields_for(&[0xFF, 1, 2]),
    )
    .unwrap();
    seg.commit().unwrap();
    drop(seg);
    let r = recover_segment(&dir, 1, resolve).unwrap();
    assert_eq!(r.count, 1, "the poison record and everything after are cut");
    assert_eq!(r.last_seq, Some(1));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn seal_refuses_a_half_written_index() {
    let dir = tempdir("half-seal");
    let seg = write_segment(&dir, 4, 1);
    let sealed = seg.seal().unwrap();
    drop(sealed);
    // Chop one entry off the sealed idx: header says 4, file holds 3.
    let ipath = idx_path(&dir, 1);
    let f = std::fs::OpenOptions::new()
        .write(true)
        .open(&ipath)
        .unwrap();
    f.set_len((FILE_HEADER_LEN + 3 * IDX_ENTRY_LEN) as u64)
        .unwrap();
    drop(f);
    let err = SealedSegment::open(&dir, 1).unwrap_err();
    assert!(err.to_string().contains("header says 4"), "{err}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn active_segment_reads_back_its_own_records_before_sealing() {
    let dir = tempdir("read-active");
    let mut seg = write_segment(&dir, 3, 1);
    let e = entries_of(&dir, 1)[1];
    let span = RECORD_HEADER_LEN + (e.payload_len as usize).div_ceil(8) * 8;
    let bytes = seg.read_span(e.offset, span).unwrap();
    let v = ontology_storage::segment::decode_record(&bytes, 0, true).unwrap();
    assert_eq!(v.payload, &payload(1)[..]);
    // Uncommitted records are readable too (flushed on demand).
    let p = payload(50);
    let off = seg.append(4, 0, Kind::Concept, &p, fields_for(&p)).unwrap();
    let bytes = seg
        .read_span(off, RECORD_HEADER_LEN + p.len().div_ceil(8) * 8)
        .unwrap();
    assert_eq!(
        ontology_storage::segment::decode_record(&bytes, 0, true)
            .unwrap()
            .payload,
        &p[..]
    );
    std::fs::remove_dir_all(&dir).ok();
}
