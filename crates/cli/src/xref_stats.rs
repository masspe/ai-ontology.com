// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! `ontology xref-stats`: the dry run of an xref import. Parses every
//! `*.xref.xml` under a path on all cores, builds the very records
//! `ingest --xref` would write, and counts them instead of writing them —
//! so the numbers are the import's own, not a second model of it.

use anyhow::{Context, Result};
use ontology_graph::PropertyValue;
use ontology_io::{parse_dir, Record, Source, XrefSource};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

pub async fn run(path: &Path, roots: &[String], threads: Option<usize>, top: usize) -> Result<()> {
    let started = Instant::now();
    let parsed =
        parse_dir(path, roots, threads).with_context(|| format!("reading {}", path.display()))?;
    let parse_time = started.elapsed();
    let files = parsed.files.len();
    let (mut references, mut strings, mut dynamic_runs, mut procedures, mut functions) =
        (0u64, 0u64, 0u64, 0u64, 0u64);
    for f in &parsed.files {
        references += u64::from(f.references);
        strings += u64::from(f.strings);
        dynamic_runs += u64::from(f.dynamic_runs);
        procedures += f.procedures.len() as u64;
        functions += f.functions.len() as u64;
    }

    // The import's records, counted.
    let mut sheets: BTreeMap<String, u64> = BTreeMap::new();
    let mut links: BTreeMap<String, (u64, f64)> = BTreeMap::new();
    let mut missing: BTreeMap<String, u64> = BTreeMap::new();
    let mut table_use: BTreeMap<String, [u64; 4]> = BTreeMap::new();
    let mut table_programs: BTreeMap<String, u64> = BTreeMap::new();
    let mut src = XrefSource::from_files(parsed.files);
    while let Some(rec) = src.next().await? {
        match rec {
            Record::Concept(c) => {
                *sheets.entry(c.concept_type.clone()).or_insert(0) += 1;
                if c.concept_type == "Program"
                    && c.properties.get("missing") == Some(&PropertyValue::Bool(true))
                {
                    missing.entry(c.name).or_insert(0);
                }
            }
            Record::NamedRelation {
                relation_type,
                source_type,
                target_name,
                weight,
                ..
            } => {
                let e = links.entry(relation_type.clone()).or_insert((0, 0.0));
                e.0 += 1;
                e.1 += f64::from(weight);
                if relation_type == "runs" {
                    if let Some(n) = missing.get_mut(&target_name) {
                        *n += u64::from(weight as u32);
                    }
                }
                if source_type == "Program" {
                    let slot = match relation_type.as_str() {
                        "reads" => Some(0),
                        "updates" => Some(1),
                        "creates" => Some(2),
                        "deletes" => Some(3),
                        _ => None,
                    };
                    if let Some(i) = slot {
                        table_use.entry(target_name.clone()).or_insert([0; 4])[i] +=
                            u64::from(weight as u32);
                        if i == 0 {
                            *table_programs.entry(target_name).or_insert(0) += 1;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    println!("== corpus ==");
    println!(
        "files parsed:       {} ({} failed)",
        files,
        parsed.errors.len()
    );
    println!(
        "bytes:              {} MB in {parse_time:.1?}",
        parsed.bytes / 1_048_576
    );
    println!("references:         {references} (of which STRING {strings})");
    println!("procedures / functions defined: {procedures} / {functions} (properties, not sheets)");
    println!("dynamic RUN VALUE(): {dynamic_runs}");

    println!("\n== sheets the import creates ==");
    for (ty, n) in &sheets {
        println!("{ty:<12} {n:>8}");
    }
    println!("{:<12} {:>8}", "total", sheets.values().sum::<u64>());

    println!("\n== links the import creates (count, summed weight) ==");
    for (ty, (n, w)) in &links {
        println!("{ty:<14} {n:>9} {w:>12.0}");
    }
    println!(
        "{:<14} {:>9}",
        "total",
        links.values().map(|(n, _)| n).sum::<u64>()
    );

    println!(
        "\n== programs run but never compiled: {} (callers' RUN statements) ==",
        missing.len()
    );
    let mut by_calls: Vec<(&String, &u64)> = missing.iter().collect();
    by_calls.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    for (name, n) in by_calls.iter().take(top) {
        println!("    {n:>6}  {name}");
    }

    println!("\n== most read tables (programs, reads, updates, creates, deletes) ==");
    let mut tables: Vec<(&String, &[u64; 4])> = table_use.iter().collect();
    tables.sort_by(|a, b| {
        table_programs
            .get(b.0)
            .cmp(&table_programs.get(a.0))
            .then(a.0.cmp(b.0))
    });
    for (name, u) in tables.iter().take(top) {
        println!(
            "    {:>5}  {:>8} {:>7} {:>6} {:>6}  {name}",
            table_programs.get(*name).copied().unwrap_or(0),
            u[0],
            u[1],
            u[2],
            u[3]
        );
    }

    if !parsed.errors.is_empty() {
        println!("\n== parse errors (first {top}) ==");
        for (p, e) in parsed.errors.iter().take(top) {
            println!("    {}: {e}", p.display());
        }
    }
    println!("\ntotal: {:.1?}", started.elapsed());
    Ok(())
}
