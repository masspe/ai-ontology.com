// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Memory budget, per-domain cost estimation and the load plan taken at
//! startup — the "socle" of `STORAGE-PLAN.md` §7.1 (STORAGE.md §8.1, R14,
//! R17, decision D5).
//!
//! * The **budget** is read from the execution environment, never from
//!   `MemTotal`: a container limited to 2 GiB on a 64 GiB host must see
//!   2 GiB. Linux: cgroup v2 `memory.max`, then cgroup v1
//!   `memory.limit_in_bytes`, then `/proc/meminfo` `MemAvailable`. Windows:
//!   `GlobalMemoryStatusEx` (available physical memory). A fraction of it
//!   (`heap_fraction`, 0.6 by default) is the heap budget; an explicit
//!   budget overrides detection.
//! * The **estimate** of a domain's P0 cost is computed from the MANIFEST
//!   counters before any segment is read (R14), with the coefficients
//!   measured in phase 4 taken as upper bounds.
//! * The **plan** decides what to load: `strict` refuses when the estimate
//!   exceeds the budget, naming both figures (R17); `adaptive` loads the
//!   domains that fit and reports the ones it leaves out.

use std::fmt;

use crate::manifest::{Manifest, META_NS_ID};

// ---------------------------------------------------------------------------
// Coefficients (STORAGE.md §7.8, measured 2026-09-16, heap after unmapping)
// ---------------------------------------------------------------------------

/// Heap bytes per concept **beyond its payload**: name index, trigrams,
/// sorted sets, properties table. Measured ~1.45 KB (2.75 KB total for a
/// 1.3 KB payload); rounded up.
pub const CONCEPT_INDEX_BYTES: u64 = 1_500;
/// Heap bytes per relation: struct, four adjacency entries, sorted set,
/// type-name clone. Measured 350–475 B; rounded up.
pub const RELATION_BYTES: u64 = 480;
/// Transient peak of the derived-index rebuild at the end of a bulk load
/// (trigram pairs, sorted keys, per-type buckets), per concept.
pub const REBUILD_PEAK_PER_CONCEPT: u64 = 130;
/// Payload bytes are held in memory as the deserialized entity; JSON text
/// and the in-memory form are of the same order, taken as 1:1.
pub const PAYLOAD_FACTOR_NUM: u64 = 1;

/// Default share of the available memory the graph may use (D5).
pub const DEFAULT_HEAP_FRACTION: f64 = 0.6;

// ---------------------------------------------------------------------------
// Budget
// ---------------------------------------------------------------------------

/// Where the available-memory figure came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetSource {
    /// `--memory-budget-mb` / `ONTOLOGY_MEMORY_BUDGET_MB`.
    Explicit,
    /// cgroup v2 `memory.max` (Linux).
    CgroupV2,
    /// cgroup v1 `memory.limit_in_bytes` (Linux).
    CgroupV1,
    /// `/proc/meminfo` `MemAvailable` (Linux).
    MemAvailable,
    /// `GlobalMemoryStatusEx` available physical memory (Windows).
    WindowsAvailPhys,
    /// Nothing could be read: the budget is unknown (treated as unlimited,
    /// with a warning by the caller).
    Unknown,
}

impl BudgetSource {
    /// `true` when the figure is an enforced limit (a cgroup or an explicit
    /// budget) rather than a reading of free memory, which changes from one
    /// boot to the next. The adaptive plan only trims under a hard limit
    /// (STORAGE.md §8.1).
    pub fn is_hard_limit(self) -> bool {
        matches!(self, Self::Explicit | Self::CgroupV2 | Self::CgroupV1)
    }
}

impl fmt::Display for BudgetSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            BudgetSource::Explicit => "explicit",
            BudgetSource::CgroupV2 => "cgroup-v2",
            BudgetSource::CgroupV1 => "cgroup-v1",
            BudgetSource::MemAvailable => "meminfo",
            BudgetSource::WindowsAvailPhys => "windows",
            BudgetSource::Unknown => "unknown",
        })
    }
}

/// The memory the graph may use, and how that figure was obtained.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MemoryBudget {
    /// Memory available to the process (limit or free memory), when known.
    pub available_bytes: Option<u64>,
    pub source: BudgetSource,
    pub heap_fraction: f64,
    /// `available_bytes × heap_fraction`, or the explicit budget; `None`
    /// when unknown.
    pub budget_bytes: Option<u64>,
}

impl MemoryBudget {
    /// An explicit budget (D5: `--memory-budget-mb`).
    pub fn fixed(budget_bytes: u64) -> Self {
        Self {
            available_bytes: Some(budget_bytes),
            source: BudgetSource::Explicit,
            heap_fraction: 1.0,
            budget_bytes: Some(budget_bytes),
        }
    }

    /// Unknown environment: no limit can be enforced.
    pub fn unknown(heap_fraction: f64) -> Self {
        Self {
            available_bytes: None,
            source: BudgetSource::Unknown,
            heap_fraction,
            budget_bytes: None,
        }
    }

    /// Build from an available-memory reading.
    pub fn from_available(available_bytes: u64, source: BudgetSource, heap_fraction: f64) -> Self {
        let f = heap_fraction.clamp(0.05, 1.0);
        Self {
            available_bytes: Some(available_bytes),
            source,
            heap_fraction: f,
            budget_bytes: Some((available_bytes as f64 * f) as u64),
        }
    }

    /// Read the environment (see module docs). Never fails: an unreadable
    /// environment yields [`MemoryBudget::unknown`].
    pub fn detect(heap_fraction: f64) -> Self {
        match platform::available_memory() {
            Some((bytes, source)) => Self::from_available(bytes, source, heap_fraction),
            None => Self::unknown(heap_fraction),
        }
    }
}

/// Parse a cgroup v2 `memory.max` file: a byte count, or `max` (no limit).
pub fn parse_cgroup_v2_max(text: &str) -> Option<u64> {
    let t = text.trim();
    if t.is_empty() || t == "max" {
        return None;
    }
    t.parse::<u64>().ok()
}

/// Parse a cgroup v1 `memory.limit_in_bytes` file: a byte count; the kernel
/// writes a huge sentinel (≥ 2^62) when there is no limit.
pub fn parse_cgroup_v1_limit(text: &str) -> Option<u64> {
    let v = text.trim().parse::<u64>().ok()?;
    if v >= (1u64 << 62) {
        None
    } else {
        Some(v)
    }
}

/// Parse `/proc/meminfo` and return `MemAvailable` in bytes (the kernel
/// reports kB).
pub fn parse_meminfo_available(text: &str) -> Option<u64> {
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("MemAvailable:") {
            let kb = rest
                .trim()
                .trim_end_matches("kB")
                .trim()
                .parse::<u64>()
                .ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

/// The process's cgroup v2 path from `/proc/self/cgroup` (the `0::/...`
/// line). `/sys/fs/cgroup/memory.max` only exists at the root of the
/// hierarchy (a Docker container); a systemd unit with `MemoryMax=` lives
/// in a sub-path and its limit must be read there.
pub fn parse_cgroup_v2_path(text: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.strip_prefix("0::"))
        .map(|p| p.trim().trim_end_matches('/').to_string())
}

/// Every `memory.max` candidate from the process's own cgroup up to the
/// root, deepest first. The effective limit is the smallest along the way.
pub fn cgroup_v2_candidates(own_path: Option<&str>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(p) = own_path {
        let mut cur = p.to_string();
        while !cur.is_empty() {
            out.push(format!("/sys/fs/cgroup{cur}/memory.max"));
            match cur.rfind('/') {
                Some(i) => cur.truncate(i),
                None => break,
            }
        }
    }
    out.push("/sys/fs/cgroup/memory.max".to_string());
    out
}

mod platform {
    use super::BudgetSource;

    #[cfg(target_os = "linux")]
    pub fn available_memory() -> Option<(u64, BudgetSource)> {
        use std::fs;
        let own = fs::read_to_string("/proc/self/cgroup")
            .ok()
            .and_then(|t| super::parse_cgroup_v2_path(&t));
        let v2 = super::cgroup_v2_candidates(own.as_deref())
            .into_iter()
            .filter_map(|p| fs::read_to_string(p).ok())
            .filter_map(|t| super::parse_cgroup_v2_max(&t))
            .min();
        if let Some(v) = v2 {
            return Some((v, BudgetSource::CgroupV2));
        }
        if let Ok(t) = fs::read_to_string("/sys/fs/cgroup/memory/memory.limit_in_bytes") {
            if let Some(v) = super::parse_cgroup_v1_limit(&t) {
                return Some((v, BudgetSource::CgroupV1));
            }
        }
        if let Ok(t) = fs::read_to_string("/proc/meminfo") {
            if let Some(v) = super::parse_meminfo_available(&t) {
                return Some((v, BudgetSource::MemAvailable));
            }
        }
        None
    }

    #[cfg(windows)]
    pub fn available_memory() -> Option<(u64, BudgetSource)> {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            dwMemoryLoad: 0,
            ullTotalPhys: 0,
            ullAvailPhys: 0,
            ullTotalPageFile: 0,
            ullAvailPageFile: 0,
            ullTotalVirtual: 0,
            ullAvailVirtual: 0,
            ullAvailExtendedVirtual: 0,
        };
        // SAFETY: `status` is a properly sized, writable MEMORYSTATUSEX and
        // the call only writes into it.
        let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
        if ok == 0 {
            return None;
        }
        Some((status.ullAvailPhys, BudgetSource::WindowsAvailPhys))
    }

    #[cfg(not(any(target_os = "linux", windows)))]
    pub fn available_memory() -> Option<(u64, BudgetSource)> {
        None
    }
}

// ---------------------------------------------------------------------------
// Estimation (R14)
// ---------------------------------------------------------------------------

/// Estimated P0 cost of one storage domain, from the MANIFEST counters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainEstimate {
    pub ns_id: u16,
    pub ns: String,
    /// Records in the domain's graph stream (all kinds, deletes included).
    pub records: u64,
    /// Relation records (edges written, not net of deletes).
    pub edges: u64,
    pub payload_bytes: u64,
    /// Upper-bound heap estimate in P0 (see the coefficients).
    pub estimated_bytes: u64,
}

/// Per-stream counters not yet in the MANIFEST (the active segment).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ActiveCounters {
    pub records: u64,
    pub edges: u64,
    pub payload_bytes: u64,
}

/// P0 heap estimate for a domain with these counters. Deletes and updates
/// are counted as concepts, so the figure is an upper bound.
pub fn estimate_bytes(records: u64, edges: u64, payload_bytes: u64) -> u64 {
    let concept_records = records.saturating_sub(edges);
    payload_bytes * PAYLOAD_FACTOR_NUM
        + concept_records * (CONCEPT_INDEX_BYTES + REBUILD_PEAK_PER_CONCEPT)
        + edges * RELATION_BYTES
}

/// Estimate every graph domain of `manifest` (meta excluded — it is always
/// loaded and small, H3), adding the active-segment counters supplied per
/// `ns_id`. Sorted by `ns_id`.
pub fn estimate_domains(
    manifest: &Manifest,
    active: &dyn Fn(u16) -> ActiveCounters,
) -> Vec<DomainEstimate> {
    let mut out = Vec::new();
    for ns_id in manifest.graph_ns_ids() {
        if ns_id == META_NS_ID {
            continue;
        }
        let Some(stream) = manifest.stream(ns_id) else {
            continue;
        };
        let mut records = 0u64;
        let mut edges = 0u64;
        let mut payload = 0u64;
        for p in &stream.sealed {
            records += p.records;
            edges += p.edges;
            payload += p.payload_bytes;
        }
        let a = active(ns_id);
        records += a.records;
        edges += a.edges;
        payload += a.payload_bytes;
        out.push(DomainEstimate {
            ns_id,
            ns: manifest.ns_name(ns_id).unwrap_or("?").to_string(),
            records,
            edges,
            payload_bytes: payload,
            estimated_bytes: estimate_bytes(records, edges, payload),
        });
    }
    out.sort_by_key(|d| d.ns_id);
    out
}

// ---------------------------------------------------------------------------
// Plan (R17)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryMode {
    /// Refuse to start when the estimate exceeds the budget.
    Strict,
    /// Load the domains that fit, report the rest.
    Adaptive,
}

impl MemoryMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "strict" => Some(Self::Strict),
            "adaptive" => Some(Self::Adaptive),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Adaptive => "adaptive",
        }
    }
}

impl fmt::Display for MemoryMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A domain the plan leaves out, with its estimate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedDomain {
    pub ns: String,
    pub estimated_bytes: u64,
}

/// What startup decided to load.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadPlan {
    pub mode: MemoryMode,
    pub budget: MemoryBudget,
    /// Sum of the estimates of every domain in the store.
    pub estimated_total_bytes: u64,
    /// Sum of the estimates of the domains that will be loaded.
    pub estimated_loaded_bytes: u64,
    /// Domains to load, by name; `None` means everything (no selection).
    pub domains: Option<Vec<String>>,
    pub skipped: Vec<SkippedDomain>,
    /// `true` when the selection came from an explicit `--ns`.
    pub explicit: bool,
    /// `true` when everything is loaded although the estimate exceeds the
    /// budget: adaptive mode under a **soft** source (free memory, not a
    /// limit), where trimming would make the set of loaded domains depend
    /// on what else ran at boot. The caller warns loudly (R17).
    pub over_budget: bool,
    /// Domains loaded in **P1** (payloads on disk, STORAGE.md §8.2): in
    /// adaptive mode under a hard limit, a domain that does not fit in P0
    /// but fits without its payload is loaded that way instead of being
    /// skipped. Subset of `domains`.
    pub p1_domains: Vec<String>,
    /// Every domain's estimate, for reporting.
    pub estimates: Vec<DomainEstimate>,
}

impl LoadPlan {
    pub fn is_partial(&self) -> bool {
        !self.skipped.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum BudgetError {
    /// R17: strict mode, the store does not fit.
    #[error(
        "the store needs an estimated {required_mib:.1} MiB of heap in P0 ({required_bytes} bytes) but \
         the memory budget is {budget_mib:.1} MiB ({budget_bytes} bytes; {budget_source}, {fraction:.0}% \
         of {available_mib:.1} MiB available); refusing to start in strict mode. Largest domains: \
         {largest}. Use --memory-mode adaptive to load what fits, --ns to pick domains, or raise \
         --heap-fraction / --memory-budget-mb."
    )]
    Exceeds {
        required_mib: f64,
        required_bytes: u64,
        budget_mib: f64,
        budget_bytes: u64,
        available_mib: f64,
        budget_source: BudgetSource,
        fraction: f64,
        largest: String,
    },
    #[error("unknown domain `{0}` in --ns (known: {1})")]
    UnknownDomain(String, String),
}

fn mib(bytes: u64) -> u64 {
    bytes.div_ceil(1024 * 1024)
}

fn mib_f(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn largest(estimates: &[DomainEstimate], n: usize) -> String {
    let mut v: Vec<&DomainEstimate> = estimates.iter().collect();
    v.sort_by_key(|d| std::cmp::Reverse(d.estimated_bytes));
    let parts: Vec<String> = v
        .iter()
        .take(n)
        .map(|d| format!("{} ({} MiB)", d.ns, mib(d.estimated_bytes)))
        .collect();
    if parts.is_empty() {
        "none".into()
    } else {
        parts.join(", ")
    }
}

/// Decide what to load (R14 before any file, R17 explicit refusal).
///
/// * `explicit`: an operator's `--ns` list. It is honoured as is; in strict
///   mode its estimate is still checked against the budget.
/// * Unknown budget: nothing can be enforced — everything is loaded (the
///   caller logs a warning).
/// * Adaptive under a **hard** limit (cgroup, explicit budget): domains are
///   taken **smallest first** until the budget is exhausted, which loads as
///   many domains as possible; an operator who wants a specific large
///   domain names it with `--ns`.
/// * Adaptive under a **soft** source (free memory): everything is loaded
///   and the plan is flagged `over_budget` for the caller to warn about.
pub fn plan_load(
    estimates: Vec<DomainEstimate>,
    budget: MemoryBudget,
    mode: MemoryMode,
    explicit: Option<&[String]>,
) -> Result<LoadPlan, BudgetError> {
    let total: u64 = estimates.iter().map(|d| d.estimated_bytes).sum();
    let known: Vec<&str> = estimates.iter().map(|d| d.ns.as_str()).collect();

    if let Some(wanted) = explicit {
        for w in wanted {
            if !known.contains(&w.as_str()) {
                return Err(BudgetError::UnknownDomain(w.clone(), known.join(", ")));
            }
        }
        let loaded: u64 = estimates
            .iter()
            .filter(|d| wanted.contains(&d.ns))
            .map(|d| d.estimated_bytes)
            .sum();
        if mode == MemoryMode::Strict {
            check_fits(loaded, &budget, &estimates)?;
        }
        let skipped = estimates
            .iter()
            .filter(|d| !wanted.contains(&d.ns))
            .map(|d| SkippedDomain {
                ns: d.ns.clone(),
                estimated_bytes: d.estimated_bytes,
            })
            .collect();
        return Ok(LoadPlan {
            mode,
            budget,
            estimated_total_bytes: total,
            estimated_loaded_bytes: loaded,
            domains: Some(wanted.to_vec()),
            skipped,
            explicit: true,
            over_budget: false,
            p1_domains: Vec::new(),
            estimates,
        });
    }

    let Some(budget_bytes) = budget.budget_bytes else {
        return Ok(LoadPlan {
            mode,
            budget,
            estimated_total_bytes: total,
            estimated_loaded_bytes: total,
            domains: None,
            skipped: Vec::new(),
            explicit: false,
            over_budget: false,
            p1_domains: Vec::new(),
            estimates,
        });
    };

    if total <= budget_bytes {
        return Ok(LoadPlan {
            mode,
            budget,
            estimated_total_bytes: total,
            estimated_loaded_bytes: total,
            domains: None,
            skipped: Vec::new(),
            explicit: false,
            over_budget: false,
            p1_domains: Vec::new(),
            estimates,
        });
    }

    match mode {
        MemoryMode::Strict => {
            check_fits(total, &budget, &estimates)?;
            unreachable!("check_fits fails when total > budget")
        }
        MemoryMode::Adaptive if !budget.source.is_hard_limit() => {
            // Free memory is a reading, not a limit: trimming on it would
            // load a different set of domains on every boot. Load
            // everything and let the caller warn; `strict` or an explicit
            // budget are the tools to enforce a bound on such a host.
            Ok(LoadPlan {
                mode,
                budget,
                estimated_total_bytes: total,
                estimated_loaded_bytes: total,
                domains: None,
                skipped: Vec::new(),
                explicit: false,
                over_budget: true,
                p1_domains: Vec::new(),
                estimates,
            })
        }
        MemoryMode::Adaptive => {
            let mut order: Vec<&DomainEstimate> = estimates.iter().collect();
            order.sort_by(|a, b| {
                a.estimated_bytes
                    .cmp(&b.estimated_bytes)
                    .then_with(|| a.ns.cmp(&b.ns))
            });
            let mut used = 0u64;
            let mut chosen: Vec<String> = Vec::new();
            let mut p1: Vec<String> = Vec::new();
            let mut skipped: Vec<SkippedDomain> = Vec::new();
            for d in order {
                // P1 cost: the same domain without its payloads in heap.
                let p1_cost = d.estimated_bytes - d.payload_bytes;
                if used + d.estimated_bytes <= budget_bytes {
                    used += d.estimated_bytes;
                    chosen.push(d.ns.clone());
                } else if used + p1_cost <= budget_bytes {
                    used += p1_cost;
                    chosen.push(d.ns.clone());
                    p1.push(d.ns.clone());
                } else {
                    skipped.push(SkippedDomain {
                        ns: d.ns.clone(),
                        estimated_bytes: d.estimated_bytes,
                    });
                }
            }
            chosen.sort();
            p1.sort();
            skipped.sort_by(|a, b| a.ns.cmp(&b.ns));
            Ok(LoadPlan {
                mode,
                budget,
                estimated_total_bytes: total,
                estimated_loaded_bytes: used,
                domains: Some(chosen),
                skipped,
                explicit: false,
                over_budget: false,
                p1_domains: p1,
                estimates,
            })
        }
    }
}

fn check_fits(
    required: u64,
    budget: &MemoryBudget,
    estimates: &[DomainEstimate],
) -> Result<(), BudgetError> {
    match budget.budget_bytes {
        Some(b) if required > b => Err(BudgetError::Exceeds {
            required_mib: mib_f(required),
            required_bytes: required,
            budget_mib: mib_f(b),
            budget_bytes: b,
            available_mib: mib_f(budget.available_bytes.unwrap_or(b)),
            budget_source: budget.source,
            fraction: budget.heap_fraction * 100.0,
            largest: largest(estimates, 3),
        }),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cgroup_and_meminfo_parsers() {
        assert_eq!(parse_cgroup_v2_max("max\n"), None);
        assert_eq!(parse_cgroup_v2_max(""), None);
        assert_eq!(parse_cgroup_v2_max("2147483648\n"), Some(2_147_483_648));
        assert_eq!(parse_cgroup_v2_max("garbage"), None);
        assert_eq!(
            parse_cgroup_v1_limit("9223372036854771712\n"),
            None,
            "the no-limit sentinel is not a limit"
        );
        assert_eq!(parse_cgroup_v1_limit("1073741824"), Some(1 << 30));
        let meminfo = "MemTotal:       16000000 kB\nMemFree:         1000000 kB\nMemAvailable:    4200000 kB\nBuffers:          300000 kB\n";
        assert_eq!(parse_meminfo_available(meminfo), Some(4_200_000 * 1024));
        assert_eq!(parse_meminfo_available("MemTotal: 1 kB\n"), None);
    }

    #[test]
    fn cgroup_v2_path_is_resolved_and_walked_up_to_the_root() {
        let proc_self = "0::/system.slice/ontology.service\n";
        assert_eq!(
            parse_cgroup_v2_path(proc_self).as_deref(),
            Some("/system.slice/ontology.service")
        );
        // Hybrid v1 lines are ignored; only the v2 line counts.
        let hybrid = "12:memory:/docker/abc\n0::/\n";
        assert_eq!(parse_cgroup_v2_path(hybrid).as_deref(), Some(""));
        assert_eq!(parse_cgroup_v2_path("no v2 line\n"), None);
        assert_eq!(
            cgroup_v2_candidates(Some("/system.slice/ontology.service")),
            [
                "/sys/fs/cgroup/system.slice/ontology.service/memory.max",
                "/sys/fs/cgroup/system.slice/memory.max",
                "/sys/fs/cgroup/memory.max",
            ]
        );
        assert_eq!(
            cgroup_v2_candidates(Some("")),
            ["/sys/fs/cgroup/memory.max"]
        );
        assert_eq!(cgroup_v2_candidates(None), ["/sys/fs/cgroup/memory.max"]);
    }

    #[test]
    fn hard_and_soft_sources() {
        for s in [
            BudgetSource::Explicit,
            BudgetSource::CgroupV2,
            BudgetSource::CgroupV1,
        ] {
            assert!(s.is_hard_limit(), "{s}");
        }
        for s in [
            BudgetSource::MemAvailable,
            BudgetSource::WindowsAvailPhys,
            BudgetSource::Unknown,
        ] {
            assert!(!s.is_hard_limit(), "{s}");
        }
    }

    #[test]
    fn adaptive_under_free_memory_loads_everything_and_flags_it() {
        let e = vec![est("big", 2, 900), est("small", 3, 100)];
        for src in [BudgetSource::MemAvailable, BudgetSource::WindowsAvailPhys] {
            let b = MemoryBudget::from_available(500, src, 1.0);
            let p = plan_load(e.clone(), b, MemoryMode::Adaptive, None).unwrap();
            assert_eq!(p.domains, None, "{src}");
            assert!(p.over_budget && !p.is_partial(), "{src}");
            assert_eq!(p.estimated_loaded_bytes, 1000);
            // Strict on the same reading still refuses: the operator asked.
            let err = plan_load(e.clone(), b, MemoryMode::Strict, None).unwrap_err();
            assert!(matches!(err, BudgetError::Exceeds { .. }));
        }
        // Under a hard limit, the same figures trim.
        let b = MemoryBudget::from_available(500, BudgetSource::CgroupV2, 1.0);
        let p = plan_load(e, b, MemoryMode::Adaptive, None).unwrap();
        assert_eq!(p.domains, Some(vec!["small".to_string()]));
        assert!(!p.over_budget && p.is_partial());
    }

    #[test]
    fn budget_applies_the_fraction_and_clamps_it() {
        let b = MemoryBudget::from_available(10_000, BudgetSource::MemAvailable, 0.6);
        assert_eq!(b.budget_bytes, Some(6_000));
        let b = MemoryBudget::from_available(10_000, BudgetSource::MemAvailable, 5.0);
        assert_eq!(b.budget_bytes, Some(10_000), "fraction clamped to 1.0");
        let b = MemoryBudget::from_available(10_000, BudgetSource::MemAvailable, -1.0);
        assert_eq!(b.budget_bytes, Some(500), "fraction clamped to 0.05");
        let f = MemoryBudget::fixed(42);
        assert_eq!(
            (f.budget_bytes, f.source),
            (Some(42), BudgetSource::Explicit)
        );
        assert_eq!(MemoryBudget::unknown(0.6).budget_bytes, None);
        // Detection never fails, whatever the platform.
        let d = MemoryBudget::detect(0.6);
        assert!(d.budget_bytes.is_some() || d.source == BudgetSource::Unknown);
    }

    #[test]
    fn estimate_is_an_upper_bound_built_from_the_coefficients() {
        assert_eq!(estimate_bytes(0, 0, 0), 0);
        // 10 concepts of 1300 B, 50 relations.
        let e = estimate_bytes(60, 50, 13_000);
        assert_eq!(
            e,
            13_000 + 10 * (CONCEPT_INDEX_BYTES + REBUILD_PEAK_PER_CONCEPT) + 50 * RELATION_BYTES
        );
        // More edges than records (cannot happen, but must not underflow).
        assert_eq!(estimate_bytes(1, 5, 0), 5 * RELATION_BYTES);
    }

    fn est(ns: &str, id: u16, bytes: u64) -> DomainEstimate {
        DomainEstimate {
            ns_id: id,
            ns: ns.into(),
            records: 0,
            edges: 0,
            payload_bytes: 0,
            estimated_bytes: bytes,
        }
    }

    #[test]
    fn plan_loads_everything_when_it_fits_or_when_the_budget_is_unknown() {
        let e = vec![est("a", 2, 100), est("b", 3, 200)];
        let p = plan_load(
            e.clone(),
            MemoryBudget::fixed(300),
            MemoryMode::Strict,
            None,
        )
        .unwrap();
        assert_eq!(p.domains, None);
        assert!(!p.is_partial());
        assert_eq!(p.estimated_total_bytes, 300);
        let p = plan_load(e, MemoryBudget::unknown(0.6), MemoryMode::Strict, None).unwrap();
        assert_eq!(p.domains, None);
        assert_eq!(p.estimated_loaded_bytes, 300);
    }

    #[test]
    fn strict_refuses_with_both_figures_and_the_largest_domains() {
        let e = vec![est("small", 2, 100), est("big", 3, 900), est("mid", 4, 300)];
        let err = plan_load(e, MemoryBudget::fixed(1000), MemoryMode::Strict, None).unwrap_err();
        match &err {
            BudgetError::Exceeds {
                required_mib,
                budget_mib,
                largest,
                ..
            } => {
                assert!(required_mib > budget_mib, "{required_mib} vs {budget_mib}");
                assert!(largest.starts_with("big"), "{largest}");
            }
            other => panic!("{other:?}"),
        }
        let msg = err.to_string();
        assert!(
            msg.contains("strict") && msg.contains("--memory-mode adaptive"),
            "{msg}"
        );
        // Both figures stay distinguishable even one byte apart (R17).
        assert!(
            msg.contains("(1300 bytes)") && msg.contains("(1000 bytes;"),
            "{msg}"
        );
    }

    #[test]
    fn adaptive_takes_the_smallest_domains_first_and_reports_the_rest() {
        let e = vec![est("big", 2, 900), est("small", 3, 100), est("mid", 4, 300)];
        let p = plan_load(e, MemoryBudget::fixed(450), MemoryMode::Adaptive, None).unwrap();
        assert_eq!(
            p.domains,
            Some(vec!["mid".to_string(), "small".to_string()])
        );
        assert_eq!(p.estimated_loaded_bytes, 400);
        assert_eq!(p.skipped.len(), 1);
        assert_eq!(p.skipped[0].ns, "big");
        assert!(p.is_partial() && !p.explicit);
        // Budget below every domain: nothing loads, everything reported.
        let e = vec![est("a", 2, 100), est("b", 3, 200)];
        let p = plan_load(e, MemoryBudget::fixed(50), MemoryMode::Adaptive, None).unwrap();
        assert_eq!(p.domains, Some(vec![]));
        assert_eq!(p.skipped.len(), 2);
        // Exactly equal fits.
        let e = vec![est("a", 2, 100), est("b", 3, 200)];
        let p = plan_load(e, MemoryBudget::fixed(300), MemoryMode::Adaptive, None).unwrap();
        assert_eq!(p.domains, None);
    }

    #[test]
    fn adaptive_falls_back_to_p1_when_only_the_payload_is_too_big() {
        // `big` is 900 in P0 of which 600 is payload: 300 in P1.
        let mut big = est("big", 2, 900);
        big.payload_bytes = 600;
        let e = vec![big, est("small", 3, 100)];
        let p = plan_load(
            e.clone(),
            MemoryBudget::fixed(450),
            MemoryMode::Adaptive,
            None,
        )
        .unwrap();
        assert_eq!(
            p.domains,
            Some(vec!["big".to_string(), "small".to_string()])
        );
        assert_eq!(p.p1_domains, ["big"]);
        assert_eq!(p.estimated_loaded_bytes, 400);
        assert!(!p.is_partial());
        // Below even the P1 cost, the domain is skipped as before.
        let p = plan_load(
            e.clone(),
            MemoryBudget::fixed(350),
            MemoryMode::Adaptive,
            None,
        )
        .unwrap();
        assert_eq!(p.domains, Some(vec!["small".to_string()]));
        assert!(p.p1_domains.is_empty() && p.skipped.len() == 1);
        // Strict does not fall back: refusing is its contract.
        assert!(plan_load(e, MemoryBudget::fixed(450), MemoryMode::Strict, None).is_err());
    }

    #[test]
    fn explicit_domains_win_but_strict_still_checks_them() {
        let e = vec![est("a", 2, 100), est("b", 3, 900)];
        let want = vec!["b".to_string()];
        let p = plan_load(
            e.clone(),
            MemoryBudget::fixed(1000),
            MemoryMode::Adaptive,
            Some(&want),
        )
        .unwrap();
        assert_eq!(p.domains, Some(want.clone()));
        assert!(p.explicit);
        assert_eq!(
            p.skipped.iter().map(|s| s.ns.as_str()).collect::<Vec<_>>(),
            ["a"]
        );
        let err = plan_load(
            e.clone(),
            MemoryBudget::fixed(500),
            MemoryMode::Strict,
            Some(&want),
        )
        .unwrap_err();
        assert!(matches!(err, BudgetError::Exceeds { .. }));
        // Adaptive with an explicit list too large is the operator's call.
        let p = plan_load(
            e.clone(),
            MemoryBudget::fixed(500),
            MemoryMode::Adaptive,
            Some(&want),
        )
        .unwrap();
        assert_eq!(p.domains, Some(want));
        let bad = vec!["zzz".to_string()];
        let err = plan_load(
            e,
            MemoryBudget::fixed(500),
            MemoryMode::Adaptive,
            Some(&bad),
        )
        .unwrap_err();
        assert!(matches!(err, BudgetError::UnknownDomain(..)));
        assert!(err.to_string().contains("known: a, b"));
    }

    #[test]
    fn mode_parses_case_insensitively() {
        assert_eq!(MemoryMode::parse("STRICT"), Some(MemoryMode::Strict));
        assert_eq!(MemoryMode::parse(" adaptive "), Some(MemoryMode::Adaptive));
        assert_eq!(MemoryMode::parse("lenient"), None);
        assert_eq!(MemoryMode::parse("adapt"), None, "no undocumented alias");
        assert_eq!(MemoryMode::Strict.to_string(), "strict");
    }
}
