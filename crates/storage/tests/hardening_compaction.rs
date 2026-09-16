// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Crash windows of the compaction protocol (STORAGE.md §5, steps 3-4):
//! once the marker is saved, `open` must finish the swap whatever the
//! process managed to do before dying — nothing moved yet, some streams
//! swapped and others not, old files half deleted — and the result must
//! be the live graph, entity by entity. Partition discovery must never
//! mistake the staging directory for live data.

mod hardening_common;

use hardening_common::*;
use ontology_graph::{Ontology, OntologyGraph};
use ontology_storage::manifest::CompactionMarker;
use ontology_storage::migrate::compare_graphs;
use ontology_storage::segment::{data_path, idx_path, xref_path};
use ontology_storage::stream::list_partitions;
use ontology_storage::{Manifest, SegmentStore, Store};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// `list_partitions` only sees `NNNNNN.data` files directly in the stream
/// directory: the `compacting/` staging dir, `.old` / `.tmp` leftovers, an
/// `.idx` without data and foreign names are all ignored.
#[test]
fn partition_discovery_ignores_the_staging_dir_and_foreign_files() {
    let dir = tempdir("discovery");
    std::fs::create_dir_all(dir.join("compacting")).unwrap();
    for name in [
        "000001.data",
        "000004.data",
        "compacting/000009.data",
        "notes.data",
        "000002.idx",
        "000003.data.old",
        "000005.data.tmp",
        "000006.xref",
        "MANIFEST.json",
    ] {
        std::fs::write(dir.join(name), b"x").unwrap();
    }
    assert_eq!(list_partitions(&dir).unwrap(), vec![1, 4]);
    std::fs::remove_dir_all(&dir).ok();
}

/// A store before compaction (`pre`), the same store after (`root`, whose
/// sealed partitions are the staged output), and what the marker named.
struct Compacted {
    root: PathBuf,
    pre: PathBuf,
    /// `(ns_id, staged partition id, stream dir)`.
    staged: Vec<(u16, u32, String)>,
    /// `(ns_id, partition ids before compaction, stream dir)`.
    old: Vec<(u16, Vec<u32>, String)>,
    live: Arc<OntologyGraph>,
}

async fn compacted() -> Compacted {
    let root = tempdir("cw-root");
    {
        let store = SegmentStore::open_with(&root, roll(3)).await.unwrap();
        populate(&store).await;
    }
    let pre = tempdir("cw-pre");
    copy_dir(&root, &pre);
    let m = Manifest::load(&pre).unwrap().unwrap();
    let old: Vec<(u16, Vec<u32>, String)> = m
        .streams
        .iter()
        .map(|s| (s.ns_id, partitions(&pre.join(&s.dir)), s.dir.clone()))
        .collect();
    assert!(old.iter().any(|(_, ids, _)| ids.len() > 2), "{old:?}");

    let live = OntologyGraph::with_arc(Ontology::new());
    let store = SegmentStore::open_with(&root, roll(3)).await.unwrap();
    store.load_into(&live).await.unwrap();
    store.compact_store(&live).await.unwrap();
    let m = store.manifest();
    let staged: Vec<(u16, u32, String)> = m
        .streams
        .iter()
        .filter(|s| !s.sealed.is_empty())
        .map(|s| {
            assert_eq!(s.sealed.len(), 1, "{s:?}");
            (s.ns_id, s.sealed[0].id, s.dir.clone())
        })
        .collect();
    assert_eq!(staged.len(), 3, "meta, parties, contrats: {staged:?}");
    drop(store);
    Compacted {
        root,
        pre,
        staged,
        old,
        live,
    }
}

/// The on-disk picture right after the commit point, with the streams in
/// `swapped` already moved in and cleaned (as `compact_all` step 5 does one
/// stream at a time) and the others still staged.
fn crash_state(c: &Compacted, swapped: &[u16]) -> PathBuf {
    let crash = tempdir("cw-crash");
    copy_dir(&c.pre, &crash);
    for (ns, pid, dir) in &c.staged {
        let src = c.root.join(dir);
        let dst = if swapped.contains(ns) {
            crash.join(dir)
        } else {
            crash.join(dir).join("compacting")
        };
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::copy(data_path(&src, *pid), data_path(&dst, *pid)).unwrap();
        std::fs::copy(idx_path(&src, *pid), idx_path(&dst, *pid)).unwrap();
    }
    for (ns, olds, dir) in &c.old {
        if swapped.contains(ns) {
            for o in olds {
                for p in [
                    data_path(&crash.join(dir), *o),
                    idx_path(&crash.join(dir), *o),
                    xref_path(&crash.join(dir), *o),
                ] {
                    let _ = std::fs::remove_file(p);
                }
            }
        }
    }
    let mut m = Manifest::load(&crash).unwrap().unwrap();
    m.compaction = Some(CompactionMarker {
        staged: c.staged.iter().map(|(n, p, _)| (*n, *p)).collect(),
        remove: c.old.iter().map(|(n, o, _)| (*n, o.clone())).collect(),
    });
    m.save(&crash).unwrap();
    crash
}

async fn assert_finished(crash: &Path, c: &Compacted) {
    let store = SegmentStore::open_with(crash, roll(3)).await.unwrap();
    assert!(store.manifest().compaction.is_none(), "marker cleared");
    for (_, pid, dir) in &c.staged {
        let d = crash.join(dir);
        assert!(!d.join("compacting").exists(), "{dir}: staging left");
        assert_eq!(
            partitions(&d),
            vec![*pid],
            "{dir}: the staged partition alone"
        );
        assert_eq!(files_with_ext(&d, "old"), 0, "{dir}");
    }
    for (_, olds, dir) in &c.old {
        for o in olds {
            assert!(
                !data_path(&crash.join(dir), *o).exists(),
                "{dir}: old partition {o} still there"
            );
        }
    }
    let g = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&g).await.unwrap();
    compare_graphs(&c.live, &g).unwrap();
    assert_eq!(
        store.open_report().xref_entries,
        3,
        "3 live between edges: {:?}",
        store.open_report()
    );
    // The store writes on, and a plain reopen finds everything.
    let fx = Fixture { graph: g.clone() };
    fx.concept(&store, "Person", "after-crash").await;
    drop(store);
    let store = SegmentStore::open_with(crash, roll(3)).await.unwrap();
    let again = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&again).await.unwrap();
    assert_eq!(again.concept_count(), c.live.concept_count() + 1);
    assert!(again.find_by_name("Person", "after-crash").is_some());
}

/// Marker saved, nothing moved yet: every stream is swapped at open.
#[tokio::test]
async fn a_compaction_interrupted_right_after_its_commit_point_is_finished_at_open() {
    let c = compacted().await;
    let crash = crash_state(&c, &[]);
    assert_finished(&crash, &c).await;
    for d in [&c.root, &c.pre, &crash] {
        std::fs::remove_dir_all(d).ok();
    }
}

/// Marker saved, `meta` already swapped and its old files deleted, the
/// graph streams still staged, a junk `.old` leftover in one of them:
/// finishing is idempotent — done streams are left alone, pending ones
/// are completed, the leftover is swept.
#[tokio::test]
async fn a_compaction_interrupted_mid_swap_is_finished_idempotently() {
    let c = compacted().await;
    let meta = c.staged.iter().find(|(_, _, d)| d == "meta").unwrap().0;
    let crash = crash_state(&c, &[meta]);
    let contrats = Manifest::load(&crash)
        .unwrap()
        .unwrap()
        .ns_id("contrats")
        .unwrap();
    std::fs::write(
        crash.join("graph/contrats").join("000001.data.old"),
        b"junk",
    )
    .unwrap();
    assert_finished(&crash, &c).await;
    // The reopen inside assert_finished ran after the sweep; check the
    // sweep itself on a fresh open report of the crash state replayed.
    let crash2 = crash_state(&c, &[meta]);
    std::fs::write(
        crash2.join("graph/contrats").join("000001.data.old"),
        b"junk",
    )
    .unwrap();
    let store = SegmentStore::open_with(&crash2, roll(3)).await.unwrap();
    assert_eq!(store.open_report().graph[&contrats].swept_old, 1);
    drop(store);
    for d in [&c.root, &c.pre, &crash, &crash2] {
        std::fs::remove_dir_all(d).ok();
    }
}

/// Marker saved, every stream swapped and every old file deleted, only the
/// marker itself not cleared (a crash between step 5 and the final save):
/// open clears it without touching anything.
#[tokio::test]
async fn a_compaction_interrupted_before_the_marker_is_cleared_only_clears_it() {
    let c = compacted().await;
    let all: Vec<u16> = c.staged.iter().map(|(n, _, _)| *n).collect();
    let crash = crash_state(&c, &all);
    // The `default` stream had no staged output: its old active partition
    // is still there and named in `remove`, exactly as at a real crash.
    let default_dir = crash.join("graph/default");
    assert_eq!(partitions(&default_dir).len(), 1);
    let before: Vec<(String, u64)> = c
        .staged
        .iter()
        .map(|(_, pid, dir)| {
            let p = data_path(&crash.join(dir), *pid);
            (dir.clone(), std::fs::metadata(&p).unwrap().len())
        })
        .collect();
    assert_finished(&crash, &c).await;
    for (dir, len) in before {
        let pid = c.staged.iter().find(|(_, _, d)| *d == dir).unwrap().1;
        let now = std::fs::metadata(data_path(&crash.join(&dir), pid))
            .unwrap()
            .len();
        // The staged partition of `parties` received one concept after the
        // finish (see assert_finished); the others are byte-identical.
        if dir == "graph/parties" {
            assert!(now > len, "{dir}");
        } else {
            assert_eq!(now, len, "{dir}: sealed partition rewritten");
        }
    }
    for d in [&c.root, &c.pre, &crash] {
        std::fs::remove_dir_all(d).ok();
    }
}
