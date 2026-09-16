// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Drives the built `ontology` binary on the bundled finance example and on
//! synthetic data directories: ingest/stats/export round trips, compaction,
//! reset (and its LOCK refusal), legacy `graph.log` migration, in-memory
//! mode, path finding and input validation. No server is started.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Fresh, unique data directory under the OS temp dir.
fn tempdir(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "ontology-cli-{tag}-{}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn finance() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .join("finance")
}

/// The binary with logging silenced so stdout holds only command output.
fn ontology(data: Option<&Path>) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ontology"));
    cmd.env("RUST_LOG", "error");
    if let Some(d) = data {
        cmd.arg("--data").arg(d);
    }
    cmd
}

fn run(data: Option<&Path>, args: &[&str]) -> Output {
    ontology(data).args(args).output().expect("spawn ontology")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

/// Run and require success; returns stdout.
fn ok(data: Option<&Path>, args: &[&str]) -> String {
    let out = run(data, args);
    assert!(
        out.status.success(),
        "`ontology {}` failed ({:?})\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        out.status.code(),
        text(&out.stdout),
        text(&out.stderr)
    );
    text(&out.stdout)
}

/// Run and require a non-zero exit; returns stderr.
fn fail(data: Option<&Path>, args: &[&str]) -> String {
    let out = run(data, args);
    assert!(
        !out.status.success(),
        "`ontology {}` unexpectedly succeeded\nstdout:\n{}",
        args.join(" "),
        text(&out.stdout)
    );
    assert_ne!(out.status.code(), Some(0));
    text(&out.stderr)
}

/// `stats` as a `key -> count` map.
fn stats(data: Option<&Path>) -> BTreeMap<String, u64> {
    ok(data, &["stats"])
        .lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(": ")?;
            Some((k.trim().to_string(), v.trim().parse().ok()?))
        })
        .collect()
}

fn expect_stats(
    data: &Path,
    concepts: u64,
    relations: u64,
    concept_types: u64,
    relation_types: u64,
) -> BTreeMap<String, u64> {
    let s = stats(Some(data));
    assert_eq!(s["concepts"], concepts, "{s:?}");
    assert_eq!(s["relations"], relations, "{s:?}");
    assert_eq!(s["concept_types"], concept_types, "{s:?}");
    assert_eq!(s["relation_types"], relation_types, "{s:?}");
    s
}

fn p(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Load the whole finance example the way its README does.
fn ingest_finance(data: &Path) {
    let f = finance();
    let onto = p(&f.join("ontology.json"));
    let out = ok(
        Some(data),
        &["ingest", "--ontology", &onto, &p(&f.join("seed.jsonl"))],
    );
    assert_eq!(
        out.trim(),
        "ingested: 6 concepts, 3 relations, 0 ontology updates"
    );
    let out = ok(
        Some(data),
        &[
            "ingest",
            "--text-type",
            "Contract",
            &p(&f.join("contracts")),
        ],
    );
    assert_eq!(
        out.trim(),
        "ingested: 3 concepts, 0 relations, 0 ontology updates"
    );
    let out = ok(
        Some(data),
        &[
            "ingest",
            "--xlsx-type",
            "Invoice",
            &p(&f.join("invoices.xlsx")),
        ],
    );
    assert_eq!(
        out.trim(),
        "ingested: 5 concepts, 0 relations, 0 ontology updates"
    );
    let out = ok(
        Some(data),
        &[
            "ingest",
            "--xlsx-type",
            "LineItem",
            &p(&f.join("line_items.xlsx")),
        ],
    );
    assert_eq!(
        out.trim(),
        "ingested: 8 concepts, 0 relations, 0 ontology updates"
    );
    let out = ok(Some(data), &["ingest", &p(&f.join("relations.jsonl"))]);
    assert_eq!(
        out.trim(),
        "ingested: 0 concepts, 35 relations, 0 ontology updates"
    );
}

/// Finance example end to end: expected counts after ingest; re-applying
/// the same ontology is a no-op for the schema; export → re-ingest into a
/// fresh store reproduces the counts; compact and reset keep `store/`
/// valid; `path` walks the graph.
#[test]
fn finance_ingest_export_reingest_compact_reset_round_trip() {
    let data = tempdir("finance");
    ingest_finance(&data);
    let full = expect_stats(&data, 22, 38, 6, 9);
    assert_eq!(full["rule_types"], 3);
    assert_eq!(full["action_types"], 3);
    assert!(data.join("store").join("MANIFEST.json").is_file());
    assert!(data.join("store").join("LOCK").is_file());
    assert!(
        !data.join("graph.log").exists(),
        "no legacy log is ever created"
    );

    // Re-applying the identical schema (with nothing else to ingest) keeps
    // the store loadable and the counts unchanged.
    let empty = data.join("empty.jsonl");
    std::fs::write(&empty, "").unwrap();
    let out = ok(
        Some(&data),
        &[
            "ingest",
            "--ontology",
            &p(&finance().join("ontology.json")),
            &p(&empty),
        ],
    );
    assert_eq!(
        out.trim(),
        "ingested: 0 concepts, 0 relations, 0 ontology updates"
    );
    assert_eq!(expect_stats(&data, 22, 38, 6, 9), full);

    // Shortest path across three relation hops, plus the two failure modes.
    let out = ok(
        Some(&data),
        &[
            "path",
            "--from-type",
            "Person",
            "--from-name",
            "Alice Martin",
            "--to-type",
            "Company",
            "--to-name",
            "Globex",
        ],
    );
    assert_eq!(
        out.trim(),
        "Alice Martin -[signed_by]-> C-2025-001 -[between]-> Globex\n(2 hops)"
    );
    let out = ok(
        Some(&data),
        &[
            "path",
            "--from-type",
            "Person",
            "--from-name",
            "Alice Martin",
            "--to-type",
            "Company",
            "--to-name",
            "Globex",
            "--max-depth",
            "1",
        ],
    );
    assert_eq!(out.trim(), "no path within depth 1");
    let err = fail(
        Some(&data),
        &[
            "path",
            "--from-type",
            "Person",
            "--from-name",
            "Nobody",
            "--to-type",
            "Company",
            "--to-name",
            "Globex",
        ],
    );
    assert!(err.contains("no concept (Person) Nobody"), "{err}");

    // Export → re-ingest into a fresh directory → identical stats.
    let export = data.join("export.jsonl");
    let out = ok(Some(&data), &["export", &p(&export)]);
    assert!(
        out.starts_with("exported: 22 concepts, 38 relations -> "),
        "{out}"
    );
    let dump = std::fs::read_to_string(&export).unwrap();
    assert_eq!(
        dump.lines()
            .filter(|l| l.contains("\"kind\":\"ontology\""))
            .count(),
        1,
        "exactly one schema record in the export"
    );
    let fresh = tempdir("reingest");
    let out = ok(Some(&fresh), &["ingest", &p(&export)]);
    assert_eq!(
        out.trim(),
        "ingested: 22 concepts, 38 relations, 1 ontology updates"
    );
    assert_eq!(expect_stats(&fresh, 22, 38, 6, 9), full);

    // Compaction rewrites the store but changes nothing observable.
    ok(Some(&data), &["compact"]);
    assert_eq!(expect_stats(&data, 22, 38, 6, 9), full);
    assert!(data.join("store").join("MANIFEST.json").is_file());
    let out = ok(Some(&data), &["snapshot"]);
    assert!(out.contains("no-op"), "{out}");

    // Reset empties everything, durably, and the store stays usable.
    let out = ok(Some(&data), &["reset"]);
    assert!(out.starts_with("reset: store emptied"), "{out}");
    let zero = stats(Some(&data));
    assert!(zero.values().all(|v| *v == 0), "{zero:?}");
    assert_eq!(zero.len(), 6);
    assert!(data.join("store").join("MANIFEST.json").is_file());
    ingest_finance(&data);
    assert_eq!(expect_stats(&data, 22, 38, 6, 9), full);

    let _ = std::fs::remove_dir_all(&data);
    let _ = std::fs::remove_dir_all(&fresh);
}

/// `--chunk-chars` below the document size yields `<Type>Fragment`
/// concepts linked by `fragment_of_<type>`, declared in the schema.
#[test]
fn chunked_text_ingest_creates_fragment_concepts() {
    let data = tempdir("fragments");
    let f = finance();
    let out = ok(
        Some(&data),
        &[
            "ingest",
            "--ontology",
            &p(&f.join("ontology.json")),
            "--text-type",
            "Contract",
            "--chunk-chars",
            "500",
            &p(&f.join("contracts")),
        ],
    );
    assert_eq!(
        out.trim(),
        "ingested: 12 concepts, 9 relations, 1 ontology updates"
    );
    // 3 contracts + 9 fragments; the fragment type and its relation type
    // are added to the finance schema (6 → 7, 9 → 10).
    expect_stats(&data, 12, 9, 7, 10);
    let export = data.join("export.jsonl");
    ok(Some(&data), &["export", &p(&export)]);
    let dump = std::fs::read_to_string(&export).unwrap();
    assert_eq!(
        dump.matches("\"concept_type\":\"ContractFragment\"")
            .count(),
        9
    );
    assert_eq!(dump.matches("\"concept_type\":\"Contract\"").count(), 3);
    assert_eq!(
        dump.matches("\"relation_type\":\"fragment_of_contract\"")
            .count(),
        9
    );
    // `0` keeps documents whole: no fragments at all.
    let whole = tempdir("whole");
    let out = ok(
        Some(&whole),
        &[
            "ingest",
            "--ontology",
            &p(&f.join("ontology.json")),
            "--text-type",
            "Contract",
            "--chunk-chars",
            "0",
            &p(&f.join("contracts")),
        ],
    );
    assert_eq!(
        out.trim(),
        "ingested: 3 concepts, 0 relations, 0 ontology updates"
    );
    expect_stats(&whole, 3, 0, 6, 9);
    let _ = std::fs::remove_dir_all(&data);
    let _ = std::fs::remove_dir_all(&whole);
}

/// While another process holds `store/LOCK`, every command that opens the
/// store — `reset` included — is refused with a message naming the lock;
/// once the lock is released the same command succeeds.
#[tokio::test]
async fn reset_is_refused_while_another_process_holds_the_lock() {
    let data = tempdir("locked");
    ingest_finance(&data);
    let store_dir = ontology_storage::store_dir_for(&data);
    let held = ontology_storage::SegmentStore::open(&store_dir)
        .await
        .expect("open store from the test process");

    let err = fail(Some(&data), &["reset"]);
    assert!(
        err.contains("locked by another process"),
        "expected the LOCK refusal, got:\n{err}"
    );
    assert!(err.contains("LOCK"), "{err}");
    let err = fail(Some(&data), &["stats"]);
    assert!(err.contains("locked by another process"), "{err}");

    drop(held);
    let out = ok(Some(&data), &["reset"]);
    assert!(out.starts_with("reset: store emptied"), "{out}");
    let zero = stats(Some(&data));
    assert!(zero.values().all(|v| *v == 0), "{zero:?}");
    let _ = std::fs::remove_dir_all(&data);
}

/// Build a legacy `graph.log` (pre-segment-store WAL) holding a schema,
/// three concepts and two relations.
async fn write_legacy(dir: &Path) {
    use ontology_graph::{Ontology, OntologyGraph};
    use ontology_io::{ingest_records, TripleSource};
    use ontology_storage::{FileStore, LogRecord, Store};

    let store = FileStore::open(dir).await.unwrap();
    let mut ont = Ontology::new();
    for name in ["Person", "Paper", "Topic"] {
        ont.add_concept_type(ontology_graph::ConceptType {
            name: name.into(),
            ..Default::default()
        });
    }
    ont.add_relation_type(ontology_graph::RelationType {
        name: "authored".into(),
        domain: "Person".into(),
        range: "Paper".into(),
        ..Default::default()
    })
    .unwrap();
    ont.add_relation_type(ontology_graph::RelationType {
        name: "covers".into(),
        domain: "Paper".into(),
        range: "Topic".into(),
        ..Default::default()
    })
    .unwrap();
    let graph = OntologyGraph::with_arc(ont.clone());
    store.append(&LogRecord::ontology(ont)).await.unwrap();
    let triples = dir.join("seed.triples");
    std::fs::write(
        &triples,
        "Person:Alice authored Paper:RAG\nPaper:RAG covers Topic:Retrieval\n",
    )
    .unwrap();
    let mut src = TripleSource::open(&triples).await.unwrap();
    let s = ingest_records(&mut src, &graph, Some(&store as &dyn Store))
        .await
        .unwrap();
    assert_eq!((s.concepts, s.relations), (3, 2));
    assert!(dir.join("graph.log").is_file());
    assert!(!dir.join("store").exists());
}

/// A legacy `graph.log` is migrated into `store/` on the first start
/// (renamed to `graph.log.migrated`) whatever the command — `migrate`
/// included — and a directory holding both a legacy log and a `store/`
/// is refused without touching either.
#[tokio::test]
async fn legacy_graph_log_is_migrated_on_startup_and_by_migrate() {
    // Implicit migration on the first command that opens the store.
    let auto = tempdir("legacy-auto");
    write_legacy(&auto).await;
    expect_stats(&auto, 3, 2, 3, 2);
    assert!(auto.join("graph.log.migrated").is_file());
    assert!(!auto.join("graph.log").exists());
    assert!(auto.join("store").join("MANIFEST.json").is_file());
    let out = ok(Some(&auto), &["migrate"]);
    assert!(out.starts_with("nothing to migrate: no graph.log"), "{out}");
    // A restart replays the migrated store, not the retired log.
    expect_stats(&auto, 3, 2, 3, 2);

    // `migrate` as the very first command: the store is opened (and the
    // legacy log migrated) at startup, so the command itself finds nothing
    // left to do and says so — the data is there all the same.
    let explicit = tempdir("legacy-explicit");
    write_legacy(&explicit).await;
    let out = ok(Some(&explicit), &["migrate"]);
    assert!(out.starts_with("nothing to migrate: no graph.log"), "{out}");
    assert!(out.contains("already migrated at startup"), "{out}");
    assert!(explicit.join("graph.log.migrated").is_file());
    assert!(!explicit.join("graph.log").exists());
    assert!(explicit.join("store").join("MANIFEST.json").is_file());
    expect_stats(&explicit, 3, 2, 3, 2);

    // Both histories present: refuse to guess, name both locations.
    let both = tempdir("legacy-both");
    write_legacy(&both).await;
    std::fs::create_dir_all(both.join("store")).unwrap();
    let err = fail(Some(&both), &["stats"]);
    assert!(err.contains("graph.log"), "{err}");
    assert!(err.contains("store"), "{err}");
    assert!(err.contains("did not complete"), "{err}");
    assert!(
        both.join("graph.log").is_file(),
        "refusal must not touch the legacy files"
    );

    // `migrate` needs a data directory.
    let err = fail(None, &["migrate"]);
    assert!(err.contains("--data is required"), "{err}");

    for d in [auto, explicit, both] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// Without `--data` the store is in memory: commands work, nothing is
/// written to the working directory; malformed inputs fail with a clear
/// message and a non-zero exit code.
#[test]
fn in_memory_mode_writes_nothing_and_rejects_bad_inputs() {
    let cwd = tempdir("memory");
    let f = finance();
    let run_in = |args: &[&str]| {
        ontology(None)
            .current_dir(&cwd)
            .args(args)
            .output()
            .expect("spawn")
    };
    let out = run_in(&["stats"]);
    assert!(out.status.success());
    let s: BTreeMap<String, u64> = text(&out.stdout)
        .lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(": ")?;
            Some((k.to_string(), v.parse().ok()?))
        })
        .collect();
    assert_eq!(s.len(), 6);
    assert!(s.values().all(|v| *v == 0));

    let out = run_in(&[
        "ingest",
        "--ontology",
        &p(&f.join("ontology.json")),
        &p(&f.join("seed.jsonl")),
    ]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout).trim(),
        "ingested: 6 concepts, 3 relations, 0 ontology updates"
    );
    // Ephemeral: the next process starts empty again.
    let out = run_in(&["stats"]);
    assert!(text(&out.stdout).contains("concepts: 0"));
    assert!(
        std::fs::read_dir(&cwd).unwrap().next().is_none(),
        "in-memory mode must not write into the working directory"
    );

    // Unknown extension.
    let bogus = cwd.join("data.parquet");
    let out = run_in(&["ingest", &p(&bogus)]);
    assert!(!out.status.success());
    assert_ne!(out.status.code(), Some(0));
    assert!(
        text(&out.stderr).contains("unsupported extension"),
        "{}",
        text(&out.stderr)
    );
    // Directory without --text-type, spreadsheet without --xlsx-type.
    let out = run_in(&["ingest", &p(&f.join("contracts"))]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("--text-type required"),
        "{}",
        text(&out.stderr)
    );
    let out = run_in(&["ingest", &p(&f.join("invoices.xlsx"))]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("--xlsx-type required"),
        "{}",
        text(&out.stderr)
    );
    // Nothing in `cwd` besides the bogus path we never created.
    assert!(std::fs::read_dir(&cwd).unwrap().next().is_none());
    let _ = std::fs::remove_dir_all(&cwd);
}
