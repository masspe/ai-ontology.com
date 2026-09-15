// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Durability and crash-recovery contract of `FileStore` (STORAGE-PLAN.md
//! phase 1): one `fdatasync` per batch, consecutive sequence numbers, and a
//! torn tail — at *any* byte offset — that is discarded on load while
//! anything earlier in the file is preserved byte for byte.

use ontology_graph::{Concept, ConceptType, Ontology, OntologyGraph};
use ontology_storage::{FileStore, LogRecord, RecordKind, Store};
use std::path::{Path, PathBuf};

fn tempdir(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "ontology-recovery-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn ontology() -> Ontology {
    let mut o = Ontology::new();
    o.add_concept_type(ConceptType {
        name: "Person".into(),
        ..Default::default()
    });
    o
}

static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1000);

fn concept(name: &str) -> LogRecord {
    // Explicit, process-unique ids so replay is deterministic and independent
    // of the allocator; names carry a little payload so records are not tiny.
    let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    LogRecord::concept(
        Concept::new(ontology_graph::ConceptId(id), "Person", name)
            .with_description(format!("description of {name}, padded for size")),
    )
}

/// Decode the `[u32 BE len][JSON]` framing and return `(seq, end_offset)`
/// for every complete record in `path`.
fn frames(path: &Path) -> Vec<(u64, u64)> {
    let bytes = std::fs::read(path).unwrap();
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + 4 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[off..off + 4].try_into().unwrap()) as usize;
        if off + 4 + len > bytes.len() {
            break;
        }
        let rec: LogRecord = serde_json::from_slice(&bytes[off + 4..off + 4 + len]).unwrap();
        off += 4 + len;
        out.push((rec.seq, off as u64));
    }
    out
}

async fn seeded_store(dir: &Path, n: usize) -> (FileStore, Vec<(u64, u64)>) {
    let store = FileStore::open(dir).await.unwrap();
    store
        .append(&LogRecord::ontology(ontology()))
        .await
        .unwrap();
    for i in 0..n {
        store
            .append(&concept(&format!("person-{i}")))
            .await
            .unwrap();
    }
    let f = frames(&dir.join("graph.log"));
    assert_eq!(f.len(), n + 1);
    (store, f)
}

#[tokio::test]
async fn append_batch_pays_one_sync_and_numbers_records_consecutively() {
    let dir = tempdir("batch");
    let store = FileStore::open(&dir).await.unwrap();
    assert_eq!(store.sync_count(), 0);

    store
        .append(&LogRecord::ontology(ontology()))
        .await
        .unwrap();
    assert_eq!(store.sync_count(), 1, "one append = one sync");

    let batch: Vec<LogRecord> = (0..50).map(|i| concept(&format!("b{i}"))).collect();
    store.append_batch(&batch).await.unwrap();
    assert_eq!(store.sync_count(), 2, "a 50-record batch = one sync");

    store.append_batch(&[]).await.unwrap();
    assert_eq!(store.sync_count(), 2, "an empty batch syncs nothing");

    let seqs: Vec<u64> = frames(&dir.join("graph.log"))
        .into_iter()
        .map(|(s, _)| s)
        .collect();
    let expected: Vec<u64> = (1..=51).collect();
    assert_eq!(
        seqs, expected,
        "seqs are consecutive across append and append_batch"
    );

    // Everything replays.
    let graph = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&graph).await.unwrap();
    assert_eq!(graph.concept_count(), 50);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn torn_tail_at_every_byte_offset_recovers_the_complete_prefix() {
    // Reference log: ontology + 6 concepts, written and synced normally.
    let src = tempdir("torn-src");
    let (_store, frames_ref) = seeded_store(&src, 6).await;
    let log = std::fs::read(src.join("graph.log")).unwrap();
    let file_len = log.len() as u64;

    // Cut the file at every offset of the last record (where a torn write
    // actually lands), at every record boundary ±1 and ±4 (length prefix cut
    // short), and at a sample of offsets elsewhere; 0 = empty file, file_len
    // = intact. Recovery itself is cheap; the fsync of the follow-up append
    // is not, so it is exercised on a subset only.
    let tail_start = frames_ref[frames_ref.len() - 2].1;
    let mut cuts: Vec<u64> = (0..tail_start).step_by(17).collect();
    cuts.extend(tail_start..=file_len);
    for (_, end) in &frames_ref {
        cuts.extend([*end, end + 1, end + 4, end.saturating_sub(1)]);
    }
    cuts.sort_unstable();
    cuts.dedup();
    cuts.retain(|c| *c <= file_len);
    let boundaries: Vec<u64> = frames_ref.iter().map(|(_, end)| *end).collect();

    let dir = tempdir("torn-cut");
    let log_path = dir.join("graph.log");
    for (i, cut) in cuts.iter().copied().enumerate() {
        std::fs::write(&log_path, &log[..cut as usize]).unwrap();

        let store = FileStore::open(&dir).await.unwrap();
        let graph = OntologyGraph::with_arc(Ontology::new());
        store
            .load_into(&graph)
            .await
            .unwrap_or_else(|e| panic!("cut at {cut}: load failed: {e}"));

        // Complete records are those whose end offset fits in the cut.
        let complete: Vec<&(u64, u64)> = frames_ref.iter().filter(|(_, end)| *end <= cut).collect();
        let expected_concepts = complete.len().saturating_sub(1); // minus ontology
        assert_eq!(
            graph.concept_count(),
            expected_concepts,
            "cut at {cut}: wrong number of concepts replayed"
        );

        // The file was truncated back to the last complete record — never
        // shorter, never longer — and the prefix is byte-identical.
        let boundary = complete.last().map(|(_, end)| *end).unwrap_or(0);
        let recovered = std::fs::read(&log_path).unwrap();
        assert_eq!(
            recovered.len() as u64,
            boundary,
            "cut at {cut}: file not truncated to boundary"
        );
        assert_eq!(
            &recovered[..],
            &log[..boundary as usize],
            "cut at {cut}: prefix altered"
        );

        // Writing continues from the recovered high-water mark (one fsync;
        // sampled so the sweep stays fast on slow disks).
        if i % 9 == 0 || boundaries.contains(&cut) || cut == file_len {
            let high_water = complete.last().map(|(seq, _)| *seq).unwrap_or(0);
            store.append(&concept("after-recovery")).await.unwrap();
            let f = frames(&log_path);
            assert_eq!(f.len(), complete.len() + 1);
            assert_eq!(
                f.last().unwrap().0,
                high_water + 1,
                "cut at {cut}: seq continues"
            );
        }
        drop(store);
    }
    std::fs::remove_dir_all(&dir).ok();
    std::fs::remove_dir_all(&src).ok();
}

#[tokio::test]
async fn undecodable_trailing_record_is_discarded() {
    let dir = tempdir("trailing-garbage");
    let (_store, frames_ref) = seeded_store(&dir, 3).await;
    let boundary = frames_ref.last().unwrap().1;

    // A full frame whose payload is not JSON — e.g. a write that landed
    // with the right length but scrambled bytes.
    let garbage = b"this is not a log record";
    let mut tail = (garbage.len() as u32).to_be_bytes().to_vec();
    tail.extend_from_slice(garbage);
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join("graph.log"))
            .unwrap();
        f.write_all(&tail).unwrap();
    }

    let store = FileStore::open(&dir).await.unwrap();
    let graph = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&graph).await.unwrap();
    assert_eq!(graph.concept_count(), 3);
    assert_eq!(
        std::fs::metadata(dir.join("graph.log")).unwrap().len(),
        boundary,
        "garbage frame removed"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn corruption_before_the_tail_is_fatal() {
    let dir = tempdir("mid-corruption");
    let (_store, frames_ref) = seeded_store(&dir, 4).await;

    // Flip bytes inside the second record's payload (not the last record).
    let mut bytes = std::fs::read(dir.join("graph.log")).unwrap();
    let second_start = frames_ref[0].1 as usize + 4;
    for b in &mut bytes[second_start + 2..second_start + 8] {
        *b = b'#';
    }
    std::fs::write(dir.join("graph.log"), &bytes).unwrap();

    let store = FileStore::open(&dir).await.unwrap();
    let graph = OntologyGraph::with_arc(Ontology::new());
    let err = store
        .load_into(&graph)
        .await
        .expect_err("must refuse to guess");
    assert!(
        err.to_string().contains("not the last one"),
        "unexpected error: {err}"
    );
    // Nothing was truncated: the operator decides.
    assert_eq!(
        std::fs::metadata(dir.join("graph.log")).unwrap().len(),
        bytes.len() as u64
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn snapshot_then_torn_tail_replays_only_what_the_snapshot_misses() {
    let dir = tempdir("snap-torn");
    let store = FileStore::open(&dir).await.unwrap();
    let graph = OntologyGraph::with_arc(ontology());
    store
        .append(&LogRecord::ontology(ontology()))
        .await
        .unwrap();
    for i in 0..3 {
        let mut c = Concept::new(Default::default(), "Person", format!("p{i}"));
        graph.prepare_concept(&mut c).unwrap();
        store.append(&LogRecord::concept(c.clone())).await.unwrap();
        graph.apply_prepared_concept(c).unwrap();
    }
    store.snapshot(&graph).await.unwrap();
    // Two more after the snapshot, then a torn write.
    for i in 3..5 {
        let mut c = Concept::new(Default::default(), "Person", format!("p{i}"));
        graph.prepare_concept(&mut c).unwrap();
        store.append(&LogRecord::concept(c.clone())).await.unwrap();
        graph.apply_prepared_concept(c).unwrap();
    }
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join("graph.log"))
            .unwrap();
        f.write_all(&[0, 0, 1]).unwrap(); // a length prefix cut short
    }
    drop(store);

    let store = FileStore::open(&dir).await.unwrap();
    let fresh = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&fresh).await.unwrap();
    assert_eq!(fresh.concept_count(), 5);
    let kinds: Vec<bool> = frames(&dir.join("graph.log"))
        .iter()
        .map(|(seq, _)| *seq > 0)
        .collect();
    assert_eq!(kinds.len(), 6, "ontology + 5 concepts, torn bytes gone");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn record_kind_round_trips_through_the_framing() {
    // Guards the wire format itself: what goes in comes out, in order, with
    // the store-assigned seq.
    let dir = tempdir("roundtrip");
    let store = FileStore::open(&dir).await.unwrap();
    let records = vec![
        LogRecord::ontology(ontology()),
        concept("alpha"),
        LogRecord::delete_concept(ontology_graph::ConceptId(7), "Person"),
    ];
    store.append_batch(&records).await.unwrap();
    let bytes = std::fs::read(dir.join("graph.log")).unwrap();
    let mut off = 0;
    let mut seen = Vec::new();
    while off < bytes.len() {
        let len = u32::from_be_bytes(bytes[off..off + 4].try_into().unwrap()) as usize;
        let rec: LogRecord = serde_json::from_slice(&bytes[off + 4..off + 4 + len]).unwrap();
        seen.push(rec);
        off += 4 + len;
    }
    assert_eq!(seen.len(), 3);
    assert!(matches!(seen[0].kind, RecordKind::Ontology(_)));
    assert!(matches!(seen[1].kind, RecordKind::Concept(_)));
    assert!(matches!(seen[2].kind, RecordKind::DeleteConcept(_)));
    assert_eq!(
        seen.iter().map(|r| r.seq).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    std::fs::remove_dir_all(&dir).ok();
}
