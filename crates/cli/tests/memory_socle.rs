// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! The memory socle end to end through the binary (`STORAGE-PLAN.md`
//! §7.1): strict mode refuses with both figures, adaptive mode loads what
//! fits and says what it left out, flags and environment variables agree,
//! bad values are rejected before anything is opened.

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "ontology-socle-{tag}-{}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn run_raw(data: Option<&PathBuf>, env: &[(&str, &str)], args: &[&str]) -> (bool, String, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ontology"));
    if let Some(d) = data {
        cmd.arg("--data").arg(d);
    }
    cmd.args(args);
    // Isolate from the developer's shell.
    for k in [
        "ONTOLOGY_MEMORY_MODE",
        "ONTOLOGY_HEAP_FRACTION",
        "ONTOLOGY_MEMORY_BUDGET_MB",
        "ONTOLOGY_TIER",
    ] {
        cmd.env_remove(k);
    }
    cmd.env("RUST_LOG", "info");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn ontology");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn run_env(data: &PathBuf, env: &[(&str, &str)], args: &[&str]) -> (bool, String, String) {
    run_raw(Some(data), env, args)
}

fn run(data: &PathBuf, args: &[&str]) -> (bool, String, String) {
    run_env(data, &[], args)
}

/// A store of three domains whose estimate is a few MiB, generated once.
fn store_with_three_domains() -> PathBuf {
    let data = tempdir("gen");
    let (ok, out, err) = run(
        &data,
        &[
            "bench",
            "gen",
            "--concepts",
            "600",
            "--relations",
            "1500",
            "--ns",
            "3",
            "--payload",
            "300",
            "--batch",
            "250",
        ],
    );
    assert!(ok, "{out}\n{err}");
    data
}

fn stats_counts(stats: &str) -> (u64, u64) {
    let grab = |key: &str| -> u64 {
        stats
            .lines()
            .find_map(|l| l.strip_prefix(key))
            .map(|v| v.trim().parse().unwrap())
            .unwrap_or_else(|| panic!("no `{key}` in:\n{stats}"))
    };
    (grab("concepts:"), grab("relations:"))
}

#[test]
fn strict_refuses_when_the_store_exceeds_the_budget() {
    let data = store_with_three_domains();
    // Everything fits by default (a generous budget): the full store loads.
    let (ok, stats, _) = run(&data, &["--memory-budget-mb", "4096", "stats"]);
    assert!(ok, "{stats}");
    assert_eq!(stats_counts(&stats), (600, 1500));

    // One MiB is below the estimate of the three domains together.
    let (ok, out, err) = run(
        &data,
        &[
            "--memory-mode",
            "strict",
            "--memory-budget-mb",
            "1",
            "stats",
        ],
    );
    assert!(!ok, "strict must refuse:\n{out}\n{err}");
    for needle in [
        "strict",
        "MiB",
        "bytes",
        "budget",
        "explicit",
        "--memory-mode adaptive",
    ] {
        assert!(err.contains(needle), "missing `{needle}` in:\n{err}");
    }
    // The store's error prefix does not mislabel a budget refusal.
    assert!(!err.contains("store format"), "{err}");
    // Nothing was hydrated: no "hydrated" log line after the refusal.
    assert!(!err.contains("segment store hydrated"), "{err}");

    // The same through the environment variable.
    let (ok, _, err) = run_env(
        &data,
        &[
            ("ONTOLOGY_MEMORY_MODE", "strict"),
            ("ONTOLOGY_MEMORY_BUDGET_MB", "1"),
        ],
        &["stats"],
    );
    assert!(!ok && err.contains("strict"), "{err}");
    // The flag wins over the variable.
    let (ok, stats, _) = run_env(
        &data,
        &[
            ("ONTOLOGY_MEMORY_MODE", "strict"),
            ("ONTOLOGY_MEMORY_BUDGET_MB", "1"),
        ],
        &["--memory-mode", "adaptive", "stats"],
    );
    assert!(ok, "{stats}");
}

#[test]
fn adaptive_loads_what_fits_and_reports_the_rest() {
    let data = store_with_three_domains();
    let (ok, stats, err) = run(
        &data,
        &[
            "--memory-mode",
            "adaptive",
            "--memory-budget-mb",
            "1",
            "stats",
        ],
    );
    assert!(ok, "{stats}\n{err}");
    let (concepts, relations) = stats_counts(&stats);
    assert!(
        concepts > 0 && concepts < 600,
        "a partial load: {concepts} concepts"
    );
    assert!(relations < 1500);
    assert!(err.contains("partial load"), "{err}");
    assert!(err.contains("memory plan"), "{err}");
    // Each generated domain holds 200 concepts; the loaded count is a
    // whole number of domains.
    assert_eq!(concepts % 200, 0, "{concepts}");

    // A budget too small for any domain loads nothing, but starts.
    let (ok, stats, err) = run(
        &data,
        &[
            "--memory-mode",
            "adaptive",
            "--memory-budget-mb",
            "0",
            "stats",
        ],
    );
    assert!(ok, "{stats}\n{err}");
    assert_eq!(stats_counts(&stats), (0, 0));
}

/// `--ns` is a `serve` flag and the CLI tests never start a server; that
/// the explicit list wins over the plan (and that an unknown name is
/// refused) is covered on the store itself in
/// `crates/storage/tests/budget.rs`. Here: bad values fail before any file
/// is opened, and the flags are accepted but inert without a store.
#[test]
fn bad_values_are_rejected_early_and_flags_are_inert_without_a_store() {
    let data = store_with_three_domains();
    let (ok, _, err) = run(&data, &["--memory-mode", "lenient", "stats"]);
    assert!(!ok && err.contains("--memory-mode"), "{err}");
    let (ok, _, err) = run(&data, &["--memory-budget-mb", "lots", "stats"]);
    assert!(!ok && err.contains("memory-budget-mb"), "{err}");
    for bad in ["high", "NaN", "inf", "0", "-0.5", "1.5"] {
        // `--flag=value` form: a leading minus would otherwise be read as
        // another flag by clap, which is a different (and fine) refusal.
        let flag = format!("--heap-fraction={bad}");
        let (ok, _, err) = run(&data, &[&flag, "stats"]);
        assert!(
            !ok && err.contains("heap-fraction"),
            "`{bad}` must be refused:\n{err}"
        );
    }
    let (ok, _, err) = run(&data, &["--heap-fraction", "1", "stats"]);
    assert!(ok, "1.0 is the upper bound, inclusive:\n{err}");
    // A bad mode from the environment is refused the same way (clap names
    // the flag the variable feeds, and the offending value).
    let (ok, _, err) = run_env(&data, &[("ONTOLOGY_MEMORY_MODE", "lenient")], &["stats"]);
    assert!(
        !ok && err.contains("--memory-mode") && err.contains("lenient"),
        "{err}"
    );

    // No store (in-memory mode): the memory flags are accepted and inert.
    let (ok, _, err) = run_raw(
        None,
        &[],
        &[
            "--memory-mode",
            "strict",
            "--memory-budget-mb",
            "1",
            "stats",
        ],
    );
    assert!(ok, "{err}");
}

/// `--tier p1` (P1, STORAGE.md §8.2): the same store hydrates to the same
/// counts with every payload left on disk; `p0` and `auto` behave as
/// before on a store that fits; a bad tier is refused by clap.
#[test]
fn tier_p1_hydrates_the_same_store_with_payloads_on_disk() {
    let data = store_with_three_domains();
    let (ok, stats_p0, _) = run(&data, &["--memory-budget-mb", "4096", "stats"]);
    assert!(ok, "{stats_p0}");
    let (ok, stats_p1, err) = run(
        &data,
        &["--memory-budget-mb", "4096", "--tier", "p1", "stats"],
    );
    assert!(
        ok,
        "{stats_p1}
{err}"
    );
    assert_eq!(stats_counts(&stats_p1), stats_counts(&stats_p0));
    assert!(
        err.contains("P1: concept payloads stay on disk"),
        "the tier is logged:
{err}"
    );
    let (ok, _, err) = run_env(&data, &[("ONTOLOGY_TIER", "p1")], &["stats"]);
    assert!(
        ok && err.contains("P1: concept payloads stay on disk"),
        "{err}"
    );
    let (ok, _, err) = run(&data, &["--tier", "p2", "stats"]);
    assert!(!ok && err.contains("--tier"), "{err}");
}
