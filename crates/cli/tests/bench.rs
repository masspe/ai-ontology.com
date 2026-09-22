// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! `ontology bench` end to end on a tiny synthetic store: the generator
//! produces a store the regular commands can read, every bench reports the
//! fields the phase-4 tables are built from, a codec switch keeps the data,
//! and the generator refuses to clobber an existing store.

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    // pid + counter + clock: the clock alone repeats on Windows (coarse
    // ticks) when two tests create their directory in the same instant.
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "ontology-bench-{tag}-{}-{}-{}",
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

fn run(data: &PathBuf, args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_ontology"))
        .arg("--data")
        .arg(data)
        .args(args)
        .output()
        .expect("spawn ontology");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The CLI's tracing lines share stdout with the report; the JSON object
/// is the first line starting with `{` and everything after it.
fn run_json(data: &PathBuf, args: &[&str]) -> Value {
    let (ok, stdout, stderr) = run(data, args);
    assert!(ok, "{args:?} failed:\n{stdout}\n{stderr}");
    let start = if stdout.starts_with('{') {
        0
    } else {
        stdout
            .find("\n{")
            .map(|i| i + 1)
            .unwrap_or_else(|| panic!("{args:?}: no JSON object in:\n{stdout}"))
    };
    serde_json::from_str(&stdout[start..])
        .unwrap_or_else(|e| panic!("{args:?}: not JSON ({e}):\n{stdout}"))
}

#[test]
fn bench_commands_run_end_to_end_on_a_small_store() {
    let data = tempdir("e2e");
    let gen = run_json(
        &data,
        &[
            "bench",
            "--json",
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
    assert_eq!(gen["bench"], "gen");
    assert_eq!(gen["codec"], "json");
    assert_eq!(gen["records_written"], 1 + 600 + 1500);
    assert!(gen["disk_bytes"].as_u64().unwrap() > 600 * 200, "{gen}");
    assert!(gen["syncs"].as_u64().unwrap() >= 1);

    // The regular commands read the generated store.
    let (ok, stats, _) = run(&data, &["stats"]);
    assert!(ok);
    assert!(stats.contains("concepts: 600"), "{stats}");
    assert!(stats.contains("relations: 1500"), "{stats}");

    let h = run_json(&data, &["bench", "--json", "hydrate"]);
    assert_eq!(h["concepts"], 600);
    assert_eq!(h["relations"], 1500);
    assert_eq!(h["records"], 1 + 600 + 1500);
    for key in [
        "open_ms",
        "hydrate_ms",
        "decode_only_ms",
        "apply_estimate_ms",
        "records_per_s",
    ] {
        assert!(h[key].is_number(), "{key} missing in {h}");
    }
    assert!(h["decode_share_pct"].as_f64().unwrap() <= 100.0);
    // The memory socle's estimate rides along (STORAGE.md §8.1): a whole
    // store of 2 101 records is a few MiB at most, never zero MiB below
    // the payload it holds.
    assert!(h["estimate_mib"].is_number(), "{h}");
    assert!(
        h["estimate_mib"].as_f64().unwrap() >= h["payload_bytes"].as_f64().unwrap() / 1048576.0,
        "{h}"
    );

    // Selective hydration of one domain: only that domain's concepts.
    let h1 = run_json(&data, &["bench", "--json", "hydrate", "--ns", "d0"]);
    assert_eq!(h1["concepts"], 200, "{h1}");
    // The estimate follows the selection: one domain out of three.
    assert!(
        h1["estimate_mib"].as_f64().unwrap() <= h["estimate_mib"].as_f64().unwrap(),
        "{h1}"
    );
    assert!(h1["relations"].as_u64().unwrap() < 1500);
    for key in [
        "apply_estimate_ms",
        "records",
        "records_per_s",
        "payload_bytes",
        "read_mib_per_s",
        "bytes_in_ram_per_record",
    ] {
        assert!(
            h1[key].is_null(),
            "{key} must be null on a partial load: {h1}"
        );
    }

    let q = run_json(&data, &["bench", "--json", "query", "--iterations", "10"]);
    for key in ["page200_p50_us", "search_q_p50_us", "expand_d2_p50_us"] {
        assert!(q[key].is_number(), "{key} missing in {q}");
    }
    assert_eq!(q["page200_rows_avg"], 200.0);

    let a = run_json(
        &data,
        &["bench", "--json", "append", "--n", "20", "--batch", "5"],
    );
    assert_eq!(a["n"], 20);
    assert!(a["unit_p50_us"].as_f64().unwrap() > 0.0);
    assert!(a["batched_per_record_p50_us"].as_f64().unwrap() > 0.0);
    // 20 unit appends + 20 batched concepts landed.
    let (_, stats, _) = run(&data, &["stats"]);
    assert!(stats.contains("concepts: 640"), "{stats}");

    // Codec switch through compaction: same graph, denser, later reads fine.
    let c = run_json(
        &data,
        &["bench", "--json", "compact", "--codec", "postcard"],
    );
    assert_eq!(c["codec_before"], "json");
    assert_eq!(c["codec_after"], "postcard");
    assert!(
        c["bytes_after"].as_u64().unwrap() < c["bytes_before"].as_u64().unwrap(),
        "{c}"
    );
    let h2 = run_json(&data, &["bench", "--json", "hydrate"]);
    assert_eq!(h2["codec"], "postcard");
    assert_eq!(h2["concepts"], 640);
    assert_eq!(h2["relations"], 1500);
    let (_, stats, _) = run(&data, &["stats"]);
    assert!(
        stats.contains("concepts: 640") && stats.contains("relations: 1500"),
        "{stats}"
    );

    // The generator never writes into a non-empty store.
    let (ok, _, stderr) = run(
        &data,
        &[
            "bench",
            "gen",
            "--concepts",
            "10",
            "--relations",
            "0",
            "--ns",
            "1",
        ],
    );
    assert!(!ok);
    assert!(stderr.contains("not empty"), "{stderr}");
}

#[test]
fn bench_needs_a_data_dir_and_valid_arguments() {
    let out = Command::new(env!("CARGO_BIN_EXE_ontology"))
        .args(["bench", "hydrate"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--data"));

    let data = tempdir("args");
    let (ok, _, stderr) = run(
        &data,
        &[
            "bench",
            "gen",
            "--concepts",
            "10",
            "--relations",
            "0",
            "--ns",
            "0",
        ],
    );
    assert!(!ok && stderr.contains("--ns"), "{stderr}");
    let (ok, _, stderr) = run(
        &data,
        &[
            "bench",
            "gen",
            "--concepts",
            "3",
            "--relations",
            "0",
            "--ns",
            "2",
        ],
    );
    assert!(!ok && stderr.contains("--concepts"), "{stderr}");
    let (ok, _, stderr) = run(
        &data,
        &[
            "bench",
            "gen",
            "--concepts",
            "10",
            "--relations",
            "0",
            "--ns",
            "1",
            "--codec",
            "bincode",
        ],
    );
    assert!(!ok && stderr.contains("unknown codec"), "{stderr}");
}

/// `ontology compact --codec` (the regular command, not the bench) switches
/// the codec of an existing store and keeps every record; a non-JSON store
/// declares format_version 2 in its manifest.
#[test]
fn compact_codec_switches_an_existing_store() {
    let data = tempdir("compact-codec");
    let gen = run_json(
        &data,
        &[
            "bench",
            "--json",
            "gen",
            "--concepts",
            "300",
            "--relations",
            "600",
            "--ns",
            "2",
            "--payload",
            "200",
            "--batch",
            "100",
        ],
    );
    assert_eq!(gen["codec"], "json");
    let (ok, out, err) = run(&data, &["compact", "--codec", "postcard"]);
    assert!(ok, "{out}\n{err}");
    assert!(out.contains("json -> postcard"), "{out}");
    let (ok, stats, _) = run(&data, &["stats"]);
    assert!(
        ok && stats.contains("concepts: 300") && stats.contains("relations: 600"),
        "{stats}"
    );
    let h = run_json(&data, &["bench", "--json", "hydrate"]);
    assert_eq!(h["codec"], "postcard");
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(data.join("store").join("MANIFEST.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["format_version"], 2);
    assert_eq!(manifest["codec"], 1);
    // Back to JSON: format_version 1 again, nothing lost.
    let (ok, out, _) = run(&data, &["compact", "--codec", "json"]);
    assert!(ok && out.contains("postcard -> json"), "{out}");
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(data.join("store").join("MANIFEST.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["format_version"], 1);
    let (_, stats, _) = run(&data, &["stats"]);
    assert!(
        stats.contains("concepts: 300") && stats.contains("relations: 600"),
        "{stats}"
    );
    let (ok, _, err) = run(&data, &["compact", "--codec", "bincode"]);
    assert!(!ok && err.contains("unknown codec"), "{err}");
    // Without --data the codec switch has no store to work on.
    let out = Command::new(env!("CARGO_BIN_EXE_ontology"))
        .args(["compact", "--codec", "postcard"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--data"));
}
