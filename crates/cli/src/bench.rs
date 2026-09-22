// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! `ontology bench …` — the measurements of `STORAGE-PLAN.md` phase 4.
//!
//! Everything here goes through the public APIs a server uses (`SegmentStore`,
//! `OntologyGraph`), so the numbers describe the real read and write paths:
//!
//! * `gen`     — a synthetic store written **directly on disk**, no graph in
//!   memory, so stores larger than RAM can be produced (H4 short names,
//!   ~1.3 KB payloads, `--ns` domains with intra- and cross-domain relations).
//! * `hydrate` — open + full `load_into`, then a decode-only `scan_records`
//!   pass; the difference is what `apply` (index building) costs. RSS before
//!   and after, bytes on disk, records per second.
//! * `append`  — write latency through `prepare → append → apply` (R8), one
//!   record per barrier and `--batch` records per barrier.
//! * `query`   — `GET /concepts` page 200 at random offsets, `?q=` trigram
//!   search, `expand` depth 2 — the graph calls behind those endpoints.
//! * `compact` — whole-store compaction, optionally switching the codec.
//!
//! Output is human-readable, or one JSON object per run with `--json`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use clap::Subcommand;
use ontology_graph::{
    Cardinality, Concept, ConceptId, ConceptType, Ontology, OntologyGraph, PropertyValue, Relation,
    RelationId, RelationType, TraversalSpec,
};
use ontology_storage::{
    codec_name, parse_codec, store_dir_for, LogRecord, SegmentStore, SegmentStoreConfig, Store,
};
use serde_json::json;

#[derive(Subcommand, Debug, Clone)]
pub enum BenchCmd {
    /// Write a synthetic store straight to disk (no graph in memory).
    ///
    /// Refuses to run on a non-empty store. Concept payloads are about
    /// `--payload` bytes (description + a few typed properties); names are
    /// short (`<Type>-<id>`, H4). Relations are 80 % intra-domain and 20 %
    /// cross-domain so every code path of the partitioned store is used.
    Gen {
        #[arg(long, default_value_t = 1_000_000)]
        concepts: u64,
        #[arg(long, default_value_t = 5_000_000)]
        relations: u64,
        /// Number of storage domains (each gets two concept types).
        #[arg(long, default_value_t = 5)]
        ns: u16,
        /// Target payload size of a concept record, in bytes.
        #[arg(long, default_value_t = 1300)]
        payload: usize,
        /// Write codec: `json` or `postcard`.
        #[arg(long, default_value = "json")]
        codec: String,
        #[arg(long, default_value_t = 42)]
        seed: u64,
        /// Records per durability barrier (`append_batch`).
        #[arg(long, default_value_t = 5000)]
        batch: usize,
    },
    /// Open, hydrate fully (or only `--ns` domains), then decode-only scan.
    Hydrate {
        #[arg(long, value_delimiter = ',')]
        ns: Option<Vec<String>>,
    },
    /// Write latency through the graph API: unit appends then batched appends.
    Append {
        /// Records appended in each mode.
        #[arg(long, default_value_t = 2000)]
        n: usize,
        /// Records per barrier in batched mode.
        #[arg(long, default_value_t = 100)]
        batch: usize,
    },
    /// Read latency on the hydrated graph: page 200, `?q=`, expand depth 2.
    Query {
        #[arg(long, default_value_t = 200)]
        iterations: usize,
    },
    /// Whole-store compaction, optionally switching the write codec.
    Compact {
        #[arg(long)]
        codec: Option<String>,
    },
}

pub async fn run(cmd: BenchCmd, data: PathBuf, json: bool) -> Result<()> {
    let store_dir = store_dir_for(&data);
    let out = match cmd {
        BenchCmd::Gen {
            concepts,
            relations,
            ns,
            payload,
            codec,
            seed,
            batch,
        } => {
            let codec = parse_codec(&codec).with_context(|| format!("unknown codec `{codec}`"))?;
            gen(
                &store_dir,
                GenParams {
                    concepts,
                    relations,
                    ns,
                    payload,
                    codec,
                    seed,
                    batch,
                },
            )
            .await?
        }
        BenchCmd::Hydrate { ns } => hydrate(&store_dir, ns).await?,
        BenchCmd::Append { n, batch } => append(&store_dir, n, batch).await?,
        BenchCmd::Query { iterations } => query(&store_dir, iterations).await?,
        BenchCmd::Compact { codec } => {
            let codec = match codec {
                Some(c) => Some(parse_codec(&c).with_context(|| format!("unknown codec `{c}`"))?),
                None => None,
            };
            compact(&store_dir, codec).await?
        }
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        print_human(&out);
    }
    Ok(())
}

fn print_human(v: &serde_json::Value) {
    if let Some(map) = v.as_object() {
        let width = map.keys().map(String::len).max().unwrap_or(0);
        for (k, val) in map {
            let shown = match val {
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            println!("{k:<width$}  {shown}");
        }
    } else {
        println!("{v}");
    }
}

// ---------------------------------------------------------------------------
// gen
// ---------------------------------------------------------------------------

struct GenParams {
    concepts: u64,
    relations: u64,
    ns: u16,
    payload: usize,
    codec: u8,
    seed: u64,
    batch: usize,
}

/// xorshift64* — deterministic, dependency-free, plenty for a data generator.
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.max(1) ^ 0x9E37_79B9_7F4A_7C15)
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next_u64() % n
        }
    }
}

const WORDS: &[&str] = &[
    "contrat",
    "facture",
    "avenant",
    "client",
    "fournisseur",
    "montant",
    "échéance",
    "livraison",
    "service",
    "licence",
    "garantie",
    "pénalité",
    "résiliation",
    "clause",
    "annexe",
    "signature",
    "paiement",
    "acompte",
    "solde",
    "devis",
    "commande",
    "réception",
    "audit",
    "conformité",
    "délai",
    "prestation",
    "maintenance",
    "support",
    "abonnement",
    "renouvellement",
];

fn lorem(rng: &mut Rng, target_bytes: usize) -> String {
    let mut s = String::with_capacity(target_bytes + 16);
    while s.len() < target_bytes {
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(WORDS[rng.below(WORDS.len() as u64) as usize]);
        if rng.below(9) == 0 {
            s.push('.');
        }
    }
    s
}

/// Domain `k` owns concept types `D{k}A` and `D{k}B`; relation `rel_{k}`
/// links A→B inside the domain, `xref_{k}` links A→A of the next domain.
fn gen_ontology(ns: u16) -> Ontology {
    let mut o = Ontology::new();
    for k in 0..ns {
        for suffix in ["A", "B"] {
            o.add_concept_type(ConceptType {
                name: format!("D{k}{suffix}"),
                ns: Some(format!("d{k}")),
                description: format!("synthetic type {suffix} of domain {k}"),
                ..Default::default()
            });
        }
    }
    for k in 0..ns {
        o.add_relation_type(RelationType {
            name: format!("rel_{k}"),
            domain: format!("D{k}A"),
            range: format!("D{k}B"),
            cardinality: Cardinality::ManyToMany,
            ..Default::default()
        })
        .expect("types exist");
        o.add_relation_type(RelationType {
            name: format!("xref_{k}"),
            domain: format!("D{k}A"),
            range: format!("D{}A", (k + 1) % ns),
            cardinality: Cardinality::ManyToMany,
            ..Default::default()
        })
        .expect("types exist");
    }
    o
}

/// Concept ids of type index `t` (0-based over the 2·ns types) are
/// `t + 1 + 2ns·q`; this picks a random one below `concepts`.
fn random_id_of_type(rng: &mut Rng, t: u64, types: u64, concepts: u64) -> u64 {
    let count = (concepts.saturating_sub(t + 1)) / types + 1;
    t + 1 + types * rng.below(count)
}

async fn gen(store_dir: &Path, p: GenParams) -> Result<serde_json::Value> {
    if p.ns == 0 {
        bail!("--ns must be at least 1");
    }
    if store_dir.exists() && std::fs::read_dir(store_dir)?.next().is_some() {
        bail!(
            "{} is not empty; `bench gen` only writes into a fresh store (delete it or pick another --data)",
            store_dir.display()
        );
    }
    let types = 2 * p.ns as u64;
    if p.concepts < types {
        bail!("--concepts must be at least 2 × --ns ({types})");
    }
    let cfg = SegmentStoreConfig {
        codec: p.codec,
        ..Default::default()
    };
    let store = SegmentStore::open_with(store_dir, cfg).await?;
    let ontology = gen_ontology(p.ns);
    let started = Instant::now();
    store.append(&LogRecord::ontology(ontology)).await?;

    let mut rng = Rng::new(p.seed);
    let desc_bytes = p.payload.saturating_sub(260);
    let mut batch: Vec<LogRecord> = Vec::with_capacity(p.batch);
    let mut written = 0u64;
    let mut next_report = p.concepts / 10;
    eprintln!("gen: {} concepts …", p.concepts);
    for i in 1..=p.concepts {
        let t = (i - 1) % types;
        let (k, suffix) = (t / 2, if t.is_multiple_of(2) { "A" } else { "B" });
        let ty = format!("D{k}{suffix}");
        let mut c = Concept::new(ConceptId(i), ty.clone(), format!("{ty}-{i}"))
            .with_description(lorem(&mut rng, desc_bytes));
        c.properties.insert(
            "amount".into(),
            PropertyValue::Number((rng.below(1_000_000) as f64) / 100.0),
        );
        c.properties.insert(
            "code".into(),
            PropertyValue::Text(format!("{:08X}", rng.next_u64() as u32)),
        );
        c.properties
            .insert("active".into(), PropertyValue::Bool(rng.below(2) == 0));
        c.properties.insert(
            "tags".into(),
            PropertyValue::List(vec![
                PropertyValue::Text(WORDS[rng.below(WORDS.len() as u64) as usize].into()),
                PropertyValue::Text(WORDS[rng.below(WORDS.len() as u64) as usize].into()),
            ]),
        );
        batch.push(LogRecord::concept(c));
        if batch.len() >= p.batch {
            store.append_batch(&batch).await?;
            written += batch.len() as u64;
            batch.clear();
        }
        if i >= next_report {
            eprintln!(
                "gen: {i} concepts ({:.0} s)",
                started.elapsed().as_secs_f64()
            );
            next_report += p.concepts / 10;
        }
    }
    if !batch.is_empty() {
        store.append_batch(&batch).await?;
        written += batch.len() as u64;
        batch.clear();
    }
    let concepts_done = Instant::now();

    eprintln!("gen: {} relations …", p.relations);
    next_report = p.relations / 10;
    for j in 1..=p.relations {
        let k = rng.below(p.ns as u64);
        let src = random_id_of_type(&mut rng, 2 * k, types, p.concepts);
        let (rt, dst) = if rng.below(5) == 0 {
            let nk = (k + 1) % p.ns as u64;
            (
                format!("xref_{k}"),
                random_id_of_type(&mut rng, 2 * nk, types, p.concepts),
            )
        } else {
            (
                format!("rel_{k}"),
                random_id_of_type(&mut rng, 2 * k + 1, types, p.concepts),
            )
        };
        let mut r = Relation::new(RelationId(j), rt, ConceptId(src), ConceptId(dst));
        r.weight = 1.0;
        // Exact insert: ids are ours, no inverse to materialize (none of the
        // generated relation types is symmetric).
        batch.push(LogRecord::relation_exact(r));
        if batch.len() >= p.batch {
            store.append_batch(&batch).await?;
            written += batch.len() as u64;
            batch.clear();
        }
        if j >= next_report && p.relations > 0 {
            eprintln!(
                "gen: {j} relations ({:.0} s)",
                started.elapsed().as_secs_f64()
            );
            next_report += p.relations / 10;
        }
    }
    if !batch.is_empty() {
        store.append_batch(&batch).await?;
        written += batch.len() as u64;
    }
    let elapsed = started.elapsed();
    let syncs = store.sync_count();
    drop(store);
    let bytes = dir_bytes(store_dir);
    Ok(json!({
        "bench": "gen",
        "codec": codec_name(p.codec),
        "concepts": p.concepts,
        "relations": p.relations,
        "ns": p.ns,
        "payload_target_bytes": p.payload,
        "records_written": written + 1,
        "elapsed_s": round2(elapsed.as_secs_f64()),
        "concepts_phase_s": round2((concepts_done - started).as_secs_f64()),
        "records_per_s": round0((written + 1) as f64 / elapsed.as_secs_f64().max(1e-9)),
        "syncs": syncs,
        "disk_bytes": bytes,
        "disk_mib": round2(bytes as f64 / (1024.0 * 1024.0)),
        "bytes_per_record": round0(bytes as f64 / (written + 1) as f64),
    }))
}

// ---------------------------------------------------------------------------
// hydrate
// ---------------------------------------------------------------------------

/// Resident set (working set on Windows): heap **plus** touched pages of
/// memory-mapped segments.
fn rss_mib() -> Option<f64> {
    memory_stats::memory_stats().map(|m| m.physical_mem as f64 / (1024.0 * 1024.0))
}

/// Committed private memory. On Windows this is `PagefileUsage`, i.e. the
/// heap without file-backed mappings; on Linux it is the virtual size and
/// only its delta is indicative.
fn commit_mib() -> Option<f64> {
    memory_stats::memory_stats().map(|m| m.virtual_mem as f64 / (1024.0 * 1024.0))
}

async fn hydrate(store_dir: &Path, ns: Option<Vec<String>>) -> Result<serde_json::Value> {
    let rss_start = rss_mib();
    let commit_start = commit_mib();
    let t0 = Instant::now();
    let store = SegmentStore::open(store_dir).await?;
    let open = t0.elapsed();
    let graph = OntologyGraph::with_arc(Ontology::new());
    let t1 = Instant::now();
    match &ns {
        Some(domains) if !domains.is_empty() => {
            store.load_domains(&graph, domains).await?;
        }
        _ => store.load_into(&graph).await?,
    }
    let load = t1.elapsed();
    let rss_loaded = rss_mib();
    let commit_loaded = commit_mib();
    let concepts = graph.concept_count();
    let relations = graph.relation_count();

    // The sealed segments are memory-mapped and the replay touched every
    // page, so the working set counts file pages as well as the heap.
    // Dropping the store unmaps them: what remains above the baseline is
    // the graph's own memory, the figure the memory tiers are sized on.
    drop(store);
    let rss_unmapped = rss_mib();
    let store = SegmentStore::open(store_dir).await?;

    // The memory socle's estimate for what was just loaded (STORAGE.md
    // §8.1), from the MANIFEST alone, next to the measured heap: the gap
    // is what calibrates the coefficients.
    let estimate_bytes: u64 = store
        .estimate_domains()?
        .iter()
        .filter(|d| match &ns {
            Some(domains) if !domains.is_empty() => domains.contains(&d.ns),
            _ => true,
        })
        .map(|d| d.estimated_bytes)
        .sum();
    let estimate_mib = estimate_bytes as f64 / (1024.0 * 1024.0);

    // Decode-only pass over the same bytes: the read path minus `apply`.
    let t2 = Instant::now();
    let (records, payload_bytes) = {
        let store = &store;
        tokio::task::block_in_place(|| store.scan_records(|_| {}))?
    };
    let scan = t2.elapsed();
    let disk = dir_bytes(store_dir);
    // The decode-only scan always reads the whole store, so the split
    // between decoding and `apply` is only meaningful for a full load.
    let full_load = ns.as_ref().is_none_or(|d| d.is_empty());
    let apply = load.saturating_sub(scan);
    Ok(json!({
        "bench": "hydrate",
        "codec": codec_name(store.codec()),
        "domains": ns,
        "open_ms": round2(ms(open)),
        "hydrate_ms": round2(ms(load)),
        "decode_only_ms": round2(ms(scan)),
        "apply_estimate_ms": full_load.then(|| round2(ms(apply))),
        "decode_share_pct": full_load.then(|| round0(100.0 * scan.as_secs_f64() / load.as_secs_f64().max(1e-9))),
        // The scan always covers the whole store: on a partial load the
        // whole-store figures would be computed on a different base than
        // the load, so they are omitted rather than reported wrongly.
        "records": full_load.then_some(records),
        "records_per_s": full_load.then(|| round0(records as f64 / load.as_secs_f64().max(1e-9))),
        "concepts": concepts,
        "relations": relations,
        "payload_bytes": full_load.then_some(payload_bytes),
        "disk_bytes": disk,
        "disk_mib": round2(disk as f64 / (1024.0 * 1024.0)),
        "read_mib_per_s": full_load.then(|| round0(disk as f64 / (1024.0 * 1024.0) / scan.as_secs_f64().max(1e-9))),
        "rss_start_mib": rss_start.map(round0),
        "rss_loaded_mib": rss_loaded.map(round0),
        "rss_after_unmap_mib": rss_unmapped.map(round0),
        // Working set including the mapped segment pages (upper bound).
        "rss_delta_mib": match (rss_start, rss_loaded) { (Some(a), Some(b)) => Some(round0(b - a)), _ => None },
        // The graph alone, once the store's mappings are released.
        "heap_delta_mib": match (rss_start, rss_unmapped) { (Some(a), Some(b)) => Some(round0(b - a)), _ => None },
        "commit_delta_mib": match (commit_start, commit_loaded) { (Some(a), Some(b)) => Some(round0(b - a)), _ => None },
        // STORAGE.md §8.1 estimate of the loaded domains, and its ratio to
        // the measured heap (> 100 = the estimate is conservative).
        "estimate_mib": round0(estimate_mib),
        "estimate_vs_heap_pct": match (rss_start, rss_unmapped) {
            (Some(a), Some(b)) if b - a > 1.0 => Some(round0(100.0 * estimate_mib / (b - a))),
            _ => None,
        },
        "bytes_in_ram_per_record": match (rss_start, rss_unmapped) {
            (Some(a), Some(b)) if full_load && records > 0 => Some(round0((b - a) * 1024.0 * 1024.0 / records as f64)),
            _ => None,
        },
    }))
}

// ---------------------------------------------------------------------------
// append
// ---------------------------------------------------------------------------

async fn append(store_dir: &Path, n: usize, batch: usize) -> Result<serde_json::Value> {
    if n == 0 || batch == 0 {
        bail!("--n and --batch must be positive");
    }
    let store = SegmentStore::open(store_dir).await?;
    let graph = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&graph).await?;
    let ty = graph
        .with_ontology(|o| {
            let mut names: Vec<&String> = o.concept_types.keys().collect();
            names.sort();
            names.first().map(|s| s.to_string())
        })
        .context("the store has no concept type; run `bench gen` or ingest first")?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);

    // Unit: one record per durability barrier (the HTTP path).
    let mut unit = Vec::with_capacity(n);
    for i in 0..n {
        let mut c = Concept::new(ConceptId(0), ty.clone(), format!("bench-u-{stamp}-{i}"))
            .with_description(lorem(&mut Rng::new(i as u64 + 1), 1000));
        let t = Instant::now();
        graph.prepare_concept(&mut c)?;
        store.append(&LogRecord::concept(c.clone())).await?;
        graph.apply_prepared_concept(c)?;
        unit.push(t.elapsed());
    }
    // Batched: `batch` records per barrier (the ingest path).
    let mut batched = Vec::with_capacity(n / batch + 1);
    let mut done = 0;
    while done < n {
        let size = batch.min(n - done);
        let mut records = Vec::with_capacity(size);
        let mut prepared = Vec::with_capacity(size);
        let t = Instant::now();
        for i in 0..size {
            let mut c = Concept::new(
                ConceptId(0),
                ty.clone(),
                format!("bench-b-{stamp}-{}", done + i),
            )
            .with_description(lorem(&mut Rng::new((done + i) as u64 + 7), 1000));
            graph.prepare_concept(&mut c)?;
            records.push(LogRecord::concept(c.clone()));
            prepared.push(c);
        }
        store.append_batch(&records).await?;
        for c in prepared {
            graph.apply_prepared_concept(c)?;
        }
        batched.push(t.elapsed() / size as u32);
        done += size;
    }
    Ok(json!({
        "bench": "append",
        "codec": codec_name(store.codec()),
        "concept_type": ty,
        "n": n,
        "unit_p50_us": round0(us(percentile(&unit, 50.0))),
        "unit_p99_us": round0(us(percentile(&unit, 99.0))),
        "unit_mean_us": round0(us(mean(&unit))),
        "batch": batch,
        "batched_per_record_p50_us": round0(us(percentile(&batched, 50.0))),
        "batched_per_record_p99_us": round0(us(percentile(&batched, 99.0))),
        "batched_per_record_mean_us": round0(us(mean(&batched))),
        "speedup_batched_vs_unit": round2(mean(&unit).as_secs_f64() / mean(&batched).as_secs_f64().max(1e-12)),
        "syncs": store.sync_count(),
    }))
}

// ---------------------------------------------------------------------------
// query
// ---------------------------------------------------------------------------

async fn query(store_dir: &Path, iterations: usize) -> Result<serde_json::Value> {
    if iterations == 0 {
        bail!("--iterations must be positive");
    }
    let store = SegmentStore::open(store_dir).await?;
    let graph = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&graph).await?;
    let total = graph.concept_count();
    if total == 0 {
        bail!("empty store; run `bench gen` or ingest first");
    }
    let mut rng = Rng::new(7);
    // A sample of real ids and names for seeds and substrings.
    let (_, sample) = graph.list_concepts_page(None, None, 0, 256, false, true);
    if sample.is_empty() {
        bail!("no concept listed");
    }

    let mut page = Vec::with_capacity(iterations);
    let mut search = Vec::with_capacity(iterations);
    let mut expand = Vec::with_capacity(iterations);
    let mut page_rows = 0usize;
    let mut hits = 0usize;
    let mut expanded_nodes = 0usize;
    let spec = TraversalSpec {
        max_depth: 2,
        max_nodes: 200,
        ..Default::default()
    };
    for _ in 0..iterations {
        // Random offsets defeat the per-generation query cache, so this is
        // the uncached cost of a page (a repeated page is a hash lookup).
        let offset = (rng.below((total / 200).max(1) as u64) as usize) * 200;
        let t = Instant::now();
        let (_, rows) = graph.list_concepts_page(None, None, offset, 200, false, true);
        page.push(t.elapsed());
        page_rows += rows.len();

        let name = &sample[rng.below(sample.len() as u64) as usize].name;
        let lower = name.to_lowercase();
        let chars: Vec<char> = lower.chars().collect();
        let needle: String = if chars.len() >= 3 {
            let start = rng.below((chars.len() - 2) as u64) as usize;
            chars[start..start + 3].iter().collect()
        } else {
            lower.clone()
        };
        let t = Instant::now();
        let (_, rows) = graph.list_concepts_page(None, Some(&needle), 0, 50, false, true);
        search.push(t.elapsed());
        hits += rows.len();

        let seed = sample[rng.below(sample.len() as u64) as usize].id;
        let t = Instant::now();
        let sg = graph.expand(&[seed], &spec);
        expand.push(t.elapsed());
        expanded_nodes += sg.concepts.len();
    }
    Ok(json!({
        "bench": "query",
        "concepts": total,
        "relations": graph.relation_count(),
        "iterations": iterations,
        "page200_p50_us": round0(us(percentile(&page, 50.0))),
        "page200_p99_us": round0(us(percentile(&page, 99.0))),
        "page200_rows_avg": round0(page_rows as f64 / iterations as f64),
        "search_q_p50_us": round0(us(percentile(&search, 50.0))),
        "search_q_p99_us": round0(us(percentile(&search, 99.0))),
        "search_hits_avg": round2(hits as f64 / iterations as f64),
        "expand_d2_p50_us": round0(us(percentile(&expand, 50.0))),
        "expand_d2_p99_us": round0(us(percentile(&expand, 99.0))),
        "expand_nodes_avg": round0(expanded_nodes as f64 / iterations as f64),
    }))
}

// ---------------------------------------------------------------------------
// compact
// ---------------------------------------------------------------------------

async fn compact(store_dir: &Path, codec: Option<u8>) -> Result<serde_json::Value> {
    let store = SegmentStore::open(store_dir).await?;
    let graph = OntologyGraph::with_arc(Ontology::new());
    let t0 = Instant::now();
    store.load_into(&graph).await?;
    let load = t0.elapsed();
    let from = store.codec();
    let t1 = Instant::now();
    let report = match codec {
        Some(c) => store.compact_with_codec(&graph, c).await?,
        None => store.compact_store(&graph).await?,
    };
    let elapsed = t1.elapsed();
    Ok(json!({
        "bench": "compact",
        "codec_before": codec_name(from),
        "codec_after": codec_name(store.codec()),
        "hydrate_ms": round2(ms(load)),
        "compact_ms": round2(ms(elapsed)),
        "records_before": report.records_before,
        "records_after": report.records_after,
        "bytes_before": report.bytes_before,
        "bytes_after": report.bytes_after,
        "size_ratio_after_over_before": round2(report.bytes_after as f64 / report.bytes_before.max(1) as f64),
        "partitions_removed": report.partitions_removed,
        "records_per_s": round0(report.records_after as f64 / elapsed.as_secs_f64().max(1e-9)),
    }))
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn dir_bytes(dir: &Path) -> u64 {
    let mut total = 0;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                total += dir_bytes(&p);
            } else if let Ok(m) = p.metadata() {
                total += m.len();
            }
        }
    }
    total
}

fn percentile(samples: &[Duration], pct: f64) -> Duration {
    if samples.is_empty() {
        return Duration::ZERO;
    }
    let mut v: Vec<Duration> = samples.to_vec();
    v.sort();
    let rank = ((pct / 100.0) * (v.len() as f64 - 1.0)).round() as usize;
    v[rank.min(v.len() - 1)]
}

fn mean(samples: &[Duration]) -> Duration {
    if samples.is_empty() {
        return Duration::ZERO;
    }
    samples.iter().sum::<Duration>() / samples.len() as u32
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}
fn us(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}
fn round0(x: f64) -> f64 {
    x.round()
}
fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic_and_bounded() {
        let mut a = Rng::new(3);
        let mut b = Rng::new(3);
        for _ in 0..100 {
            let x = a.below(17);
            assert_eq!(x, b.below(17));
            assert!(x < 17);
        }
        assert_eq!(Rng::new(0).below(0), 0);
    }

    #[test]
    fn ids_of_a_type_stay_in_range_and_on_their_stride() {
        let mut rng = Rng::new(1);
        let (types, concepts) = (6u64, 1_000u64);
        for t in 0..types {
            for _ in 0..500 {
                let id = random_id_of_type(&mut rng, t, types, concepts);
                assert!(id >= 1 && id <= concepts, "{id}");
                assert_eq!((id - 1) % types, t);
            }
        }
        // Tiny store: exactly one concept per type.
        for t in 0..types {
            assert_eq!(random_id_of_type(&mut rng, t, types, types), t + 1);
        }
    }

    #[test]
    fn generated_ontology_has_two_types_and_two_relations_per_domain() {
        let o = gen_ontology(3);
        assert_eq!(o.concept_types.len(), 6);
        assert_eq!(o.relation_types.len(), 6);
        assert_eq!(o.ns_of_type("D2B"), "d2");
        let x = &o.relation_types["xref_2"];
        assert_eq!((x.domain.as_str(), x.range.as_str()), ("D2A", "D0A"));
        o.validate_namespaces().unwrap();
    }

    #[test]
    fn lorem_reaches_its_target_size() {
        let mut rng = Rng::new(9);
        let s = lorem(&mut rng, 1000);
        assert!(s.len() >= 1000 && s.len() < 1040, "{}", s.len());
        assert!(lorem(&mut rng, 0).is_empty());
    }

    #[test]
    fn percentiles_and_mean() {
        // 101 samples: the median is the 51st, p99 the 100th (nearest rank).
        let d: Vec<Duration> = (1..=101).map(Duration::from_micros).collect();
        assert_eq!(percentile(&d, 50.0), Duration::from_micros(51));
        assert_eq!(percentile(&d, 99.0), Duration::from_micros(100));
        assert_eq!(percentile(&d, 100.0), Duration::from_micros(101));
        assert_eq!(percentile(&d, 0.0), Duration::from_micros(1));
        assert_eq!(mean(&d), Duration::from_micros(51));
        assert_eq!(percentile(&[], 50.0), Duration::ZERO);
    }
}
