// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Recovery edge cases (STORAGE.md §9, R7) not covered by the byte-offset
//! sweeps: an `.idx` torn in the middle of an entry, a record whose kind
//! byte is unknown, a `seq` that does not increase, a header whose kind
//! disagrees with its payload, and an active segment whose header is only
//! partly on disk. The rule under test everywhere: a bad record is a torn
//! tail **only in last position**; with valid data after it, recovery
//! refuses to guess and the file stays untouched.

mod hardening_common;

use hardening_common::*;
use ontology_graph::{Concept, ConceptId, Ontology, OntologyGraph};
use ontology_storage::manifest::DEFAULT_NS_ID;
use ontology_storage::segment::{
    data_path, encode_record, idx_path, recover_segment, ActiveSegment, DataHeader, IdxEntry,
    IndexFields, Kind, CODEC_JSON, FILE_HEADER_LEN, IDX_ENTRY_LEN, RECORD_HEADER_LEN,
};
use ontology_storage::{LogRecord, RecordKind, SegmentStore, Store};
use std::io::Write;

/// Deterministic index fields so a rebuilt index equals the original.
fn fields(payload: &[u8]) -> IndexFields {
    IndexFields {
        ns_id: 1,
        entity_id: payload.len() as u64,
        endpoints: 0,
        rtype_sym: 0,
        target_ns_id: 0,
    }
}
fn resolve(_k: Kind, p: &[u8]) -> Result<IndexFields, String> {
    Ok(fields(p))
}
fn onto_person() -> Ontology {
    let mut o = Ontology::new();
    o.add_concept_type(ct("Person", None, None));
    o
}
fn payload(i: u64) -> Vec<u8> {
    format!(
        "{{\"i\":{i},\"pad\":\"{}\"}}",
        "x".repeat((i % 5) as usize * 3)
    )
    .into_bytes()
}

/// `n` records, one commit each.
fn write_segment(dir: &std::path::Path, n: u64) {
    let mut seg = ActiveSegment::create(dir, 1, 1, CODEC_JSON).unwrap();
    for i in 0..n {
        let p = payload(i);
        seg.append(i + 1, 0, Kind::Concept, &p, fields(&p)).unwrap();
        seg.commit().unwrap();
    }
    drop(seg);
}

fn append_raw(path: &std::path::Path, bytes: &[u8]) {
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(bytes).unwrap();
}

/// R7: an `.idx` cut inside an entry (a crash during the index write) is
/// completed from the first incomplete entry and ends at exactly
/// `header + n × 48` bytes.
#[test]
fn an_index_torn_mid_entry_is_completed_to_the_exact_length() {
    let dir = tempdir("idx-mid-entry");
    write_segment(&dir, 6);
    let ipath = idx_path(&dir, 1);
    let f = std::fs::OpenOptions::new()
        .write(true)
        .open(&ipath)
        .unwrap();
    f.set_len((FILE_HEADER_LEN + 3 * IDX_ENTRY_LEN + 17) as u64)
        .unwrap();
    drop(f);

    let r = recover_segment(&dir, 1, resolve).unwrap();
    assert_eq!(r.count, 6);
    assert_eq!(r.truncated_bytes, 0, "the data was fine");
    assert_eq!(
        r.idx_rewritten, 3,
        "entries 3, 4, 5 (the torn one included)"
    );
    let len = std::fs::metadata(&ipath).unwrap().len();
    assert_eq!(len, ActiveSegment::idx_len_for(6));
    assert_eq!(entries(&dir, 1).len(), 6);
    assert_eq!(entries(&dir, 1)[5].seq, 6);
    std::fs::remove_dir_all(&dir).ok();
}

/// A last record with an unknown kind byte has no decodable header, so its
/// extent is only known through the `.idx`: with the entry present it is
/// a torn tail and is cut; without it, recovery refuses to truncate rather
/// than guess how far the unknown record goes.
#[test]
fn an_unknown_kind_in_the_last_record_is_cut_only_when_its_extent_is_known() {
    let dir = tempdir("unknown-kind");
    write_segment(&dir, 3);
    let dpath = data_path(&dir, 1);
    let good_len = std::fs::metadata(&dpath).unwrap().len();
    let idx_good = std::fs::read(idx_path(&dir, 1)).unwrap();

    let p = payload(9);
    let mut rec = Vec::new();
    encode_record(&mut rec, 4, 0, Kind::Concept, CODEC_JSON, &p);
    rec[24] = 200; // no such kind
    append_raw(&dpath, &rec);

    // No index entry for it: extent unknown, refused, file untouched.
    let err = recover_segment(&dir, 1, resolve).unwrap_err();
    assert!(err.to_string().contains("extent is unknown"), "{err}");
    assert_eq!(
        std::fs::metadata(&dpath).unwrap().len(),
        good_len + rec.len() as u64
    );
    assert_eq!(std::fs::read(idx_path(&dir, 1)).unwrap(), idx_good);

    // With its index entry (data synced, idx written, then the kind byte
    // rotted or a newer build wrote it): it is the last record, so it is
    // discarded and the index shrinks back.
    let entry = IdxEntry {
        seq: 4,
        offset: good_len,
        payload_len: p.len() as u32,
        kind: Kind::Concept,
        flags: 0,
        ns_id: 1,
        entity_id: 0,
        endpoints: 0,
        rtype_sym: 0,
        target_ns_id: 0,
    };
    append_raw(&idx_path(&dir, 1), &entry.encode());
    let r = recover_segment(&dir, 1, resolve).unwrap();
    assert_eq!(r.count, 3);
    assert_eq!(r.last_seq, Some(3));
    assert_eq!(r.truncated_bytes, rec.len() as u64);
    assert_eq!(std::fs::metadata(&dpath).unwrap().len(), good_len);
    assert_eq!(std::fs::read(idx_path(&dir, 1)).unwrap(), idx_good);
    std::fs::remove_dir_all(&dir).ok();
}

/// H12 inside a partition: a record whose `seq` is not greater than its
/// predecessor's is discarded in last position and fatal anywhere else.
#[test]
fn a_non_increasing_seq_is_a_torn_tail_only_in_last_position() {
    let dir = tempdir("seq-order");
    let mut buf = DataHeader::new(1, 1, CODEC_JSON).encode().to_vec();
    let (a, b, c) = (payload(1), payload(2), payload(3));
    encode_record(&mut buf, 1, 0, Kind::Concept, CODEC_JSON, &a);
    let good_len = buf.len() as u64;
    let dup = encode_record(&mut buf, 1, 0, Kind::Concept, CODEC_JSON, &b);
    std::fs::write(data_path(&dir, 1), &buf).unwrap();

    let r = recover_segment(&dir, 1, resolve).unwrap();
    assert_eq!(r.count, 1);
    assert_eq!(r.last_seq, Some(1));
    assert_eq!(r.truncated_bytes, dup as u64);
    assert_eq!(
        std::fs::metadata(data_path(&dir, 1)).unwrap().len(),
        good_len
    );
    assert_eq!(r.idx_rewritten, 1, "index regenerated from scratch");

    // Same duplicate, but a valid record follows it.
    encode_record(&mut buf, 2, 0, Kind::Concept, CODEC_JSON, &c);
    std::fs::write(data_path(&dir, 1), &buf).unwrap();
    let err = recover_segment(&dir, 1, resolve).unwrap_err();
    assert!(err.to_string().contains("refusing to truncate"), "{err}");
    assert!(err.to_string().contains("seq 1 <= previous 1"), "{err}");
    assert_eq!(std::fs::read(data_path(&dir, 1)).unwrap(), buf);
    std::fs::remove_dir_all(&dir).ok();
}

/// The store's resolver cross-checks the header kind against the payload.
/// A disagreeing record is cut when last, and refused — naming the
/// partition — when acknowledged records follow it.
#[tokio::test]
async fn a_record_whose_header_kind_disagrees_with_its_payload_is_cut_or_refused() {
    let root = tempdir("kind-mismatch");
    {
        let store = SegmentStore::open(&root).await.unwrap();
        store
            .append(&LogRecord::ontology(onto_person()))
            .await
            .unwrap();
        for i in 1..=2u64 {
            store
                .append(&LogRecord::concept(Concept::new(
                    ConceptId(i),
                    "Person",
                    format!("p{i}"),
                )))
                .await
                .unwrap();
        }
    }
    let gdir = root.join("graph/default");
    let pid = partitions(&gdir)[0];
    let last_seq = entries(&gdir, pid).last().unwrap().seq;
    let dpath = data_path(&gdir, pid);
    let good_len = std::fs::metadata(&dpath).unwrap().len();

    // Header says Concept, payload is an Ontology.
    let mismatch = serde_json::to_vec(&RecordKind::Ontology(Ontology::new())).unwrap();
    let mut bad = Vec::new();
    encode_record(
        &mut bad,
        last_seq + 1,
        0,
        Kind::Concept,
        CODEC_JSON,
        &mismatch,
    );
    append_raw(&dpath, &bad);
    {
        let store = SegmentStore::open(&root).await.unwrap();
        let r = &store.open_report().graph[&DEFAULT_NS_ID];
        assert_eq!(r.truncated_bytes, bad.len() as u64);
        assert_eq!(r.active_records, 2);
        assert_eq!(store.next_seq(), last_seq + 1);
        let g = OntologyGraph::with_arc(Ontology::new());
        store.load_into(&g).await.unwrap();
        assert_eq!(g.concept_count(), 2);
    }
    assert_eq!(std::fs::metadata(&dpath).unwrap().len(), good_len);

    // The same bad record followed by a perfectly valid one.
    let good = serde_json::to_vec(&RecordKind::Concept(Concept::new(
        ConceptId(3),
        "Person",
        "p3",
    )))
    .unwrap();
    let mut tail = bad.clone();
    encode_record(&mut tail, last_seq + 2, 0, Kind::Concept, CODEC_JSON, &good);
    append_raw(&dpath, &tail);
    let err = SegmentStore::open(&root).await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains(&format!("partition {pid}")), "{msg}");
    assert!(msg.contains("refusing to truncate"), "{msg}");
    assert!(msg.contains("header kind Concept"), "{msg}");
    assert_eq!(
        std::fs::metadata(&dpath).unwrap().len(),
        good_len + tail.len() as u64,
        "the file is left for the operator"
    );
    std::fs::remove_dir_all(&root).ok();
}

/// A crash between `create_new` and the header sync can leave a `.data`
/// shorter than its 32-byte header (not only empty): nothing was ever
/// appended to it, so it is recreated under the same partition id, the
/// `.idx` with it, and the allocator does not skip an id.
#[tokio::test]
async fn an_active_segment_with_a_partial_header_is_recreated_under_the_same_id() {
    let root = tempdir("partial-header");
    {
        let store = SegmentStore::open_with(&root, roll(3)).await.unwrap();
        for i in 1..=3u64 {
            store
                .append(&LogRecord::concept(Concept::new(
                    ConceptId(i),
                    "Person",
                    format!("p{i}"),
                )))
                .await
                .unwrap();
        }
    }
    let gdir = root.join("graph/default");
    assert_eq!(partitions(&gdir), vec![2, 3], "2 sealed, 3 fresh and empty");
    let header = DataHeader::new(3, 4, CODEC_JSON).encode();
    std::fs::write(data_path(&gdir, 3), &header[..17]).unwrap();
    std::fs::write(idx_path(&gdir, 3), b"garbage").unwrap();

    let store = SegmentStore::open_with(&root, roll(3)).await.unwrap();
    assert_eq!(partitions(&gdir), vec![2, 3]);
    let r = &store.open_report().graph[&DEFAULT_NS_ID];
    assert_eq!(r.active_partition, 3);
    assert_eq!(r.active_records, 0);
    assert_eq!(r.sealed, 1);
    assert_eq!(r.truncated_bytes, 0);
    assert_eq!(store.manifest().next_partition_id, 4);
    assert_eq!(
        std::fs::metadata(data_path(&gdir, 3)).unwrap().len(),
        FILE_HEADER_LEN as u64
    );
    assert_eq!(
        std::fs::metadata(idx_path(&gdir, 3)).unwrap().len(),
        FILE_HEADER_LEN as u64
    );
    // No `Ontology` record was journaled: replay into a graph that knows
    // the type.
    let g = OntologyGraph::with_arc(onto_person());
    store.load_into(&g).await.unwrap();
    assert_eq!(g.concept_count(), 3);
    store
        .append(&LogRecord::concept(Concept::new(
            ConceptId(4),
            "Person",
            "p4",
        )))
        .await
        .unwrap();
    let e = entries(&gdir, 3);
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].seq, 4);
    assert_eq!(e[0].offset, FILE_HEADER_LEN as u64);
    assert!(e[0].payload_len as usize > RECORD_HEADER_LEN);
    std::fs::remove_dir_all(&root).ok();
}
