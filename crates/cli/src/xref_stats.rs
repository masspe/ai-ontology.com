// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! `ontology xref-stats`: the dry run of an xref import. Parses every
//! `*.xref.xml` under a path on all cores and prints what the import would
//! produce, so the size of the resulting graph is known before a byte is
//! written.

use anyhow::{Context, Result};
use ontology_io::{parse_xref_file, TableUse, XrefFile};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Instant;

fn collect(path: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if path.is_file() {
        out.push(path.to_path_buf());
        return Ok(());
    }
    for entry in std::fs::read_dir(path)? {
        let p = entry?.path();
        if p.is_dir() {
            collect(&p, out)?;
        } else if p
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.to_ascii_lowercase().ends_with(".xref.xml"))
        {
            out.push(p);
        }
    }
    Ok(())
}

#[derive(Default)]
struct Totals {
    files: usize,
    errors: Vec<(PathBuf, String)>,
    bytes: u64,
    references: u64,
    strings: u64,
    dynamic_runs: u64,
    programs: BTreeMap<String, &'static str>,
    includes: BTreeSet<String>,
    classes: BTreeSet<String>,
    tables: BTreeMap<String, (TableUse, u32)>,
    fields: BTreeSet<String>,
    procedures: u64,
    functions: u64,
    run_pairs: u64,
    run_raw: u64,
    run_targets: BTreeMap<String, u64>,
    call_pairs: u64,
    call_raw: u64,
    include_pairs: u64,
    invoke_pairs: u64,
    table_pairs: u64,
    field_pairs: u64,
}

impl Totals {
    fn add(&mut self, f: XrefFile) {
        self.files += 1;
        self.references += u64::from(f.references);
        self.strings += u64::from(f.strings);
        self.dynamic_runs += u64::from(f.dynamic_runs);
        self.programs
            .insert(f.program, f.kind.map_or("other", |k| k.as_str()));
        self.includes.extend(f.includes.iter().cloned());
        self.include_pairs += f.includes.len() as u64;
        self.classes.extend(f.invokes.keys().cloned());
        self.invoke_pairs += f.invokes.len() as u64;
        self.procedures += f.procedures.len() as u64;
        self.functions += f.functions.len() as u64;
        self.run_pairs += f.runs.len() as u64;
        for (t, n) in f.runs {
            self.run_raw += u64::from(n);
            *self.run_targets.entry(t).or_insert(0) += u64::from(n);
        }
        self.call_pairs += f.calls.len() as u64;
        self.call_raw += f.calls.values().map(|n| u64::from(*n)).sum::<u64>();
        for (table, u) in f.tables {
            self.table_pairs += 1;
            self.field_pairs += u.fields_read.union(&u.fields_updated).count() as u64;
            for field in u.fields_read.iter().chain(u.fields_updated.iter()) {
                self.fields.insert(format!("{table}.{field}"));
            }
            let e = self.tables.entry(table).or_default();
            e.1 += 1;
            e.0.reads += u.reads;
            e.0.updates += u.updates;
            e.0.creates += u.creates;
            e.0.deletes += u.deletes;
            e.0.fields_read.extend(u.fields_read);
            e.0.fields_updated.extend(u.fields_updated);
        }
    }
}

pub fn run(path: &Path, roots: &[String], threads: Option<usize>, top: usize) -> Result<()> {
    let started = Instant::now();
    let mut files = Vec::new();
    collect(path, &mut files).with_context(|| format!("reading {}", path.display()))?;
    files.sort();
    let total_files = files.len();
    eprintln!("{total_files} xref files under {}", path.display());

    let threads = threads
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()))
        .max(1);
    let roots: Arc<[String]> = roots.into();
    let queue = Arc::new(Mutex::new(files.into_iter()));
    let (tx, rx) = mpsc::channel::<(PathBuf, u64, Result<XrefFile, String>)>();

    let workers: Vec<_> = (0..threads)
        .map(|_| {
            let queue = Arc::clone(&queue);
            let tx = tx.clone();
            let roots = Arc::clone(&roots);
            std::thread::spawn(move || loop {
                let next = queue.lock().unwrap().next();
                let Some(p) = next else { break };
                let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                let parsed = parse_xref_file(&p, &roots);
                if tx.send((p, size, parsed)).is_err() {
                    break;
                }
            })
        })
        .collect();
    drop(tx);

    let mut t = Totals::default();
    let mut done = 0usize;
    for (p, size, parsed) in rx {
        t.bytes += size;
        match parsed {
            Ok(f) => t.add(f),
            Err(e) => t.errors.push((p, e)),
        }
        done += 1;
        if done.is_multiple_of(500) {
            eprintln!("  {done}/{total_files} ({:.0?})", started.elapsed());
        }
    }
    for w in workers {
        let _ = w.join();
    }
    let elapsed = started.elapsed();

    // RUN targets: exact match on the normalised path, else a unique
    // basename match (the PROPATH resolves `pcto.p` to `sd/pcto.p`).
    let mut by_basename: HashMap<&str, Vec<&str>> = HashMap::new();
    for p in t.programs.keys() {
        by_basename
            .entry(p.rsplit('/').next().unwrap_or(p))
            .or_default()
            .push(p);
    }
    let (mut exact, mut by_name, mut unresolved) = (0u64, 0u64, 0u64);
    let mut unresolved_list: Vec<(&String, &u64)> = Vec::new();
    for (target, n) in &t.run_targets {
        if t.programs.contains_key(target) {
            exact += n;
        } else if by_basename
            .get(target.rsplit('/').next().unwrap_or(target))
            .is_some_and(|v| v.len() == 1)
        {
            by_name += n;
        } else {
            unresolved += n;
            unresolved_list.push((target, n));
        }
    }
    unresolved_list.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));

    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    for k in t.programs.values() {
        *kinds.entry(k).or_insert(0) += 1;
    }
    let databases: BTreeSet<&str> = t
        .tables
        .keys()
        .filter_map(|k| k.split('.').next())
        .collect();
    let modules: BTreeSet<&str> = t
        .programs
        .keys()
        .map(|p| p.split_once('/').map_or("", |(m, _)| m))
        .collect();

    let mb = t.bytes as f64 / 1_048_576.0;
    println!("== corpus ==");
    println!(
        "files parsed:       {} ({} failed)",
        t.files,
        t.errors.len()
    );
    println!(
        "bytes:              {mb:.0} MB in {elapsed:.1?} ({:.0} MB/s on {threads} threads)",
        mb / elapsed.as_secs_f64()
    );
    println!(
        "references:         {} (of which STRING {})",
        t.references, t.strings
    );
    if let Some(m) = memory_stats::memory_stats() {
        println!("rss now:            {} MB", m.physical_mem / 1_048_576);
    }

    println!("\n== sheets the import would create ==");
    println!("programs:           {} {:?}", t.programs.len(), kinds);
    println!("modules:            {}", modules.len());
    println!("includes:           {}", t.includes.len());
    println!("classes (used):     {}", t.classes.len());
    println!("databases:          {} {:?}", databases.len(), databases);
    println!("tables:             {}", t.tables.len());
    println!("fields:             {}", t.fields.len());
    println!("procedures (defs):  {}", t.procedures);
    println!("functions (defs):   {}", t.functions);
    let lean =
        t.programs.len() + t.includes.len() + t.tables.len() + databases.len() + modules.len();
    let full =
        lean + t.fields.len() + t.procedures as usize + t.functions as usize + t.classes.len();
    println!("total lean / full:  {lean} / {full}");

    println!("\n== links the import would create ==");
    println!(
        "runs    (prog→prog):   {} distinct, {} raw",
        t.run_pairs, t.run_raw
    );
    println!(
        "calls   (prog→proc):   {} distinct, {} raw",
        t.call_pairs, t.call_raw
    );
    println!("includes:              {}", t.include_pairs);
    println!("invokes (prog→class):  {}", t.invoke_pairs);
    println!("table use (prog→tbl):  {}", t.table_pairs);
    println!(
        "field use (prog→fld):  {}  [full level only]",
        t.field_pairs
    );
    println!("defines (prog→proc/fn):{}", t.procedures + t.functions);
    let lean_links = t.programs.len() as u64
        + t.run_pairs
        + t.include_pairs
        + t.invoke_pairs
        + t.table_pairs
        + t.tables.len() as u64;
    let full_links = lean_links
        + t.field_pairs
        + t.fields.len() as u64
        + t.procedures
        + t.functions
        + t.call_pairs;
    println!("total lean / full:     {lean_links} / {full_links}");

    println!("\n== RUN resolution ==");
    println!("exact path:         {exact}");
    println!("unique basename:    {by_name}");
    println!(
        "unresolved:         {unresolved} ({} distinct targets)",
        unresolved_list.len()
    );
    println!("dynamic VALUE():    {}", t.dynamic_runs);
    for (target, n) in unresolved_list.iter().take(top) {
        println!("    {n:>6}  {target}");
    }

    println!("\n== most used tables (programs, reads, updates, creates, deletes, fields) ==");
    let mut tables: Vec<_> = t.tables.iter().collect();
    tables.sort_by(|a, b| b.1 .1.cmp(&a.1 .1).then(a.0.cmp(b.0)));
    for (name, (u, programs)) in tables.iter().take(top) {
        println!(
            "    {programs:>5}  {:>8} {:>7} {:>6} {:>6} {:>5}  {name}",
            u.reads,
            u.updates,
            u.creates,
            u.deletes,
            u.fields_read.union(&u.fields_updated).count()
        );
    }

    if !t.errors.is_empty() {
        println!("\n== parse errors (first {top}) ==");
        for (p, e) in t.errors.iter().take(top) {
            println!("    {}: {e}", p.display());
        }
    }
    Ok(())
}
