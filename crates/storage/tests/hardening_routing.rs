// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Write path of `SegmentStore` (STORAGE.md §7.2, D3, H13): a batch that
//! cannot be routed leaves the store exactly as it was — counters, seq,
//! router and poison flag included; an empty batch is a no-op; the
//! reserved `meta` name and malformed domain names never become streams;
//! the roll policy seals exactly one partition per `max_records` and never
//! splits a batch; a stream's entries come back in `seq` order across
//! partitions.

mod hardening_common;

use hardening_common::*;
use ontology_graph::{Concept, ConceptId, Ontology, OntologyGraph, Relation};
use ontology_storage::manifest::DEFAULT_NS_ID;
use ontology_storage::segment::{IndexFields, Kind, CODEC_JSON};
use ontology_storage::stream::Stream;
use ontology_storage::{LogRecord, RollPolicy, SegmentStore, Store, StoreError};

/// A relation whose type the store's ontology does not know cannot be
/// routed (H14). The whole batch is refused before the first append: the
/// valid concept before it is not written, `seq` and the sync counter do
/// not move, the store is not poisoned and keeps accepting writes.
#[tokio::test]
async fn a_relation_of_an_unknown_type_is_refused_and_nothing_in_the_batch_lands() {
    let root = tempdir("unknown-rtype");
    let store = SegmentStore::open(&root).await.unwrap();
    let fx = Fixture::new(&store).await;
    let acme = fx.concept(&store, "Company", "Acme").await;
    let (records, seq, syncs) = (store.record_count(), store.next_seq(), store.sync_count());
    let counts = store.record_counts_by_domain();

    let err = store
        .append_batch(&[
            LogRecord::concept(Concept::new(ConceptId(99), "Company", "Globex")),
            LogRecord::relation(Relation::new(Default::default(), "sponsors", acme, acme)),
        ])
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::Format(_)), "{err}");
    assert!(err.to_string().contains("sponsors"), "{err}");
    assert!(
        err.to_string().contains("unknown to the store's ontology"),
        "{err}"
    );
    assert_eq!(store.record_count(), records);
    assert_eq!(store.next_seq(), seq);
    assert_eq!(store.sync_count(), syncs);
    assert_eq!(store.record_counts_by_domain(), counts);
    assert!(!store.is_poisoned());
    // Not asserted either way: the symbol `sponsors` was interned while the
    // batch was being planned and stays frozen in the MANIFEST although no
    // record uses it — harmless under R10 (symbols are never reused).

    fx.concept(&store, "Company", "Globex").await;
    assert_eq!(store.record_count(), records + 1);
    assert_eq!(store.next_seq(), seq + 1);
    let g = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&g).await.unwrap();
    assert_eq!(g.concept_count(), 2);
    assert!(g.find_by_name("Company", "Globex").is_some());
    std::fs::remove_dir_all(&root).ok();
}

/// The router follows `Ontology` records inside a batch; when the batch
/// fails, the schema that was never written must not keep routing. After
/// the failure — and after a restart — a concept of the type the failed
/// schema introduced routes by the last **durable** schema (`default`).
#[tokio::test]
async fn a_failed_batch_restores_the_router_to_the_last_durable_schema() {
    let root = tempdir("router-restore");
    let store = SegmentStore::open(&root).await.unwrap();
    let fx = Fixture::new(&store).await;
    let acme = fx.concept(&store, "Company", "Acme").await;
    let mut onto = ontology();
    onto.add_concept_type(ct("Asset", Some("actifs"), None));

    let err = store
        .append_batch(&[
            LogRecord::ontology(onto),
            LogRecord::delete_concept_unrouted(acme),
        ])
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::Format(_)), "{err}");
    assert_eq!(store.record_counts_by_domain()["meta"], 1);

    let truck = LogRecord::concept(Concept::new(ConceptId(77), "Asset", "Truck"));
    store.append(&truck).await.unwrap();
    let counts = store.record_counts_by_domain();
    assert_eq!(counts["default"], 1, "{counts:?}");
    assert_eq!(counts.get("actifs").copied().unwrap_or(0), 0, "{counts:?}");

    drop(store);
    let store = SegmentStore::open(&root).await.unwrap();
    store.append(&truck).await.unwrap();
    let counts = store.record_counts_by_domain();
    assert_eq!(
        counts["default"], 2,
        "same routing after restart: {counts:?}"
    );
    std::fs::remove_dir_all(&root).ok();
}

/// An empty batch syncs nothing, allocates no `seq`, writes nothing.
#[tokio::test]
async fn an_empty_batch_is_a_no_op() {
    let root = tempdir("empty-batch");
    let store = SegmentStore::open(&root).await.unwrap();
    Fixture::new(&store).await;
    let (records, seq, syncs) = (store.record_count(), store.next_seq(), store.sync_count());
    store.append_batch(&[]).await.unwrap();
    assert_eq!(store.record_count(), records);
    assert_eq!(store.next_seq(), seq);
    assert_eq!(store.sync_count(), syncs);
    std::fs::remove_dir_all(&root).ok();
}

/// `meta` is the schema stream (D3): a type declared in it can be written
/// as schema (the store does not validate schemas), but no instance of it
/// can be routed, and the refusal writes nothing and poisons nothing.
#[tokio::test]
async fn a_type_declared_in_the_reserved_meta_domain_cannot_receive_instances() {
    let root = tempdir("meta-ns");
    let store = SegmentStore::open(&root).await.unwrap();
    let mut onto = Ontology::new();
    onto.add_concept_type(ct("Ghost", Some("meta"), None));
    store.append(&LogRecord::ontology(onto)).await.unwrap();
    let before = store.record_count();

    let err = store
        .append(&LogRecord::concept(Concept::new(
            ConceptId(1),
            "Ghost",
            "g",
        )))
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::Format(_)), "{err}");
    assert!(err.to_string().contains("meta"), "{err}");
    assert_eq!(store.record_count(), before);
    assert!(!store.is_poisoned());
    assert_eq!(store.manifest().ns.len(), 2, "no domain interned");
    std::fs::remove_dir_all(&root).ok();
}

/// A domain name becomes a directory under the store root, so the store
/// refuses a malformed one itself instead of trusting the schema record:
/// no directory is created anywhere, nothing is interned, nothing written.
#[tokio::test]
async fn a_malformed_domain_name_is_refused_before_any_directory_is_created() {
    let root = tempdir("bad-ns");
    let store = SegmentStore::open(&root).await.unwrap();
    let mut onto = Ontology::new();
    onto.add_concept_type(ct("Escapee", Some("../../hardening-escape"), None));
    onto.add_concept_type(ct("Shouty", Some("Bad Name"), None));
    let long = "a".repeat(33);
    onto.add_concept_type(ct("Long", Some(long.as_str()), None));
    store.append(&LogRecord::ontology(onto)).await.unwrap();
    let before = store.record_count();
    let ns_before = store.manifest().ns.clone();

    for (ty, name) in [("Escapee", "e"), ("Shouty", "s"), ("Long", "l")] {
        let err = store
            .append(&LogRecord::concept(Concept::new(ConceptId(1), ty, name)))
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::Format(_)), "{ty}: {err}");
        assert!(
            err.to_string().contains("not a valid domain name"),
            "{ty}: {err}"
        );
    }
    assert_eq!(store.record_count(), before);
    assert!(!store.is_poisoned());
    assert_eq!(store.manifest().ns, ns_before, "nothing interned");
    let escaped = root.parent().unwrap().join("hardening-escape");
    assert!(!escaped.exists(), "{}", escaped.display());
    assert!(!root.join("hardening-escape").exists());
    assert!(!root.join("graph").join("Bad Name").exists());
    let graph_dirs: Vec<String> = std::fs::read_dir(root.join("graph"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(graph_dirs, vec!["default".to_string()]);
    std::fs::remove_dir_all(&root).ok();
}

/// `RollPolicy { max_records: n }`: after `k` single appends the stream
/// holds `k / n` sealed partitions of exactly `n` records and `k % n` in
/// the active one. A batch is committed as a whole: it never splits across
/// partitions, even when it is longer than `n`.
#[tokio::test]
async fn roll_policy_seals_one_partition_per_max_records_and_never_splits_a_batch() {
    for n in [1u32, 2, 3] {
        let root = tempdir(&format!("roll-{n}"));
        let store = SegmentStore::open_with(&root, roll(n)).await.unwrap();
        for k in 1..=7u64 {
            store
                .append(&LogRecord::concept(Concept::new(
                    ConceptId(k),
                    "Person",
                    format!("p{k}"),
                )))
                .await
                .unwrap();
            let m = store.manifest();
            let sealed = &m.stream(DEFAULT_NS_ID).unwrap().sealed;
            assert_eq!(sealed.len() as u64, k / n as u64, "n={n} k={k}");
            assert!(
                sealed.iter().all(|p| p.records == n as u64),
                "n={n} k={k}: {sealed:?}"
            );
            let active = store.record_count() - sealed.iter().map(|p| p.records).sum::<u64>();
            assert_eq!(active, k % n as u64, "n={n} k={k}");
            assert_eq!(
                partitions(&root.join("graph/default")).len() as u64,
                k / n as u64 + 1,
                "n={n} k={k}: sealed + one active"
            );
        }
        std::fs::remove_dir_all(&root).ok();
    }

    let root = tempdir("roll-batch");
    let store = SegmentStore::open_with(&root, roll(3)).await.unwrap();
    let batch: Vec<LogRecord> = (1..=7u64)
        .map(|k| LogRecord::concept(Concept::new(ConceptId(k), "Person", format!("p{k}"))))
        .collect();
    store.append_batch(&batch).await.unwrap();
    let m = store.manifest();
    let sealed = &m.stream(DEFAULT_NS_ID).unwrap().sealed;
    assert_eq!(sealed.len(), 1, "{sealed:?}");
    assert_eq!(sealed[0].records, 7, "the whole batch in one partition");
    assert_eq!(sealed[0].base_seq, 1);
    assert_eq!(sealed[0].last_seq, 7);
    assert_eq!(store.sync_count(), 1);
    std::fs::remove_dir_all(&root).ok();
}

/// `Stream::all_entries` returns every committed entry in `seq` order with
/// its partition, partitions non-decreasing, each entry inside the `seq`
/// range `partition_bases` announces for its partition — and identically
/// after a reopen.
#[test]
fn stream_entries_come_back_in_seq_order_across_partitions() {
    let dir = tempdir("stream-order");
    let policy = RollPolicy {
        max_bytes: u64::MAX,
        max_records: 3,
    };
    let mut next_partition = 1u32;
    let mut resolve = |_k: Kind, p: &[u8]| -> Result<IndexFields, String> {
        Ok(IndexFields {
            ns_id: 1,
            entity_id: p.len() as u64,
            endpoints: 0,
            rtype_sym: 0,
            target_ns_id: 0,
        })
    };
    let check = |all: &[(u32, ontology_storage::segment::IdxEntry)], bases: &[(u32, u64)]| {
        let seqs: Vec<u64> = all.iter().map(|(_, e)| e.seq).collect();
        assert_eq!(seqs, (1..=10).collect::<Vec<_>>());
        let pids: Vec<u32> = all.iter().map(|(p, _)| *p).collect();
        assert!(pids.windows(2).all(|w| w[0] <= w[1]), "{pids:?}");
        assert_eq!(bases.len(), 4, "3 sealed + active: {bases:?}");
        for (pid, e) in all {
            let i = bases.iter().position(|(p, _)| p == pid).unwrap();
            let lo = bases[i].1;
            let hi = bases.get(i + 1).map(|(_, b)| *b).unwrap_or(u64::MAX);
            assert!(
                e.seq >= lo && e.seq < hi,
                "seq {} in partition {pid} [{lo},{hi})",
                e.seq
            );
        }
    };

    let (mut s, _) = Stream::open(
        &dir,
        1,
        CODEC_JSON,
        policy,
        &mut next_partition,
        1,
        &mut resolve,
    )
    .unwrap();
    for seq in 1..=10u64 {
        let p = format!("record-{seq}");
        s.append(
            seq,
            0,
            Kind::Concept,
            p.as_bytes(),
            IndexFields {
                ns_id: 1,
                entity_id: p.len() as u64,
                endpoints: 0,
                rtype_sym: 0,
                target_ns_id: 0,
            },
        )
        .unwrap();
        s.commit().unwrap();
        s.maybe_roll(&mut next_partition, seq + 1).unwrap();
    }
    assert_eq!(s.sealed().len(), 3);
    assert_eq!(s.active().record_count(), 1);
    assert_eq!(s.last_seq(), Some(10));
    assert_eq!(s.record_count(), 10);
    let all = s.all_entries().unwrap();
    let bases = s.partition_bases();
    check(&all, &bases);
    assert_eq!(bases[0], (1, 1));
    assert_eq!(bases[3], (4, 10));
    drop(s);

    let (mut s, report) = Stream::open(
        &dir,
        1,
        CODEC_JSON,
        policy,
        &mut next_partition,
        11,
        &mut resolve,
    )
    .unwrap();
    assert_eq!(report.sealed, 3);
    assert_eq!(report.active_records, 1);
    assert_eq!(report.last_seq, Some(10));
    assert!(report.resealed.is_empty());
    assert_eq!(report.truncated_bytes, 0);
    assert_eq!(next_partition, 5);
    let again = s.all_entries().unwrap();
    assert_eq!(again, all);
    check(&again, &s.partition_bases());
    std::fs::remove_dir_all(&dir).ok();
}
