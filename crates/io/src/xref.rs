// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Progress OpenEdge `COMPILE … XREF-XML` reader.
//!
//! One `.xref.xml` describes one compiled program: a `<Source>` per file
//! that took part in the compile (the program, then each include), and a
//! flat list of `<Reference Reference-type=… Object-identifier=…>` with an
//! `<Object-context>` child. The file is read as a stream, one event at a
//! time, and reduced to an [`XrefFile`] summary: per table how the program
//! touches it, which programs it runs, which includes it pulls in. The raw
//! references (millions over a code base) never exist in memory at once.
//!
//! The full ontology (sheets and links) is built from these summaries by
//! the import; this module only parses and aggregates.

use async_trait::async_trait;
use ontology_graph::{
    Cardinality, Concept, ConceptId, ConceptType, PropertyValue, RelationType, RuleType,
};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};

use crate::ingest::{IngestError, Source};
use crate::record::Record;

/// What kind of compile unit the main `<Source>` is, by extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramKind {
    /// `.p`
    Procedure,
    /// `.w` (GUI window / SmartObject)
    Window,
    /// `.cls`
    Class,
    /// `.i` compiled on its own, or anything else
    Other,
}

impl ProgramKind {
    fn of(path: &str) -> Self {
        match path.rsplit('.').next() {
            Some("p") => Self::Procedure,
            Some("w") => Self::Window,
            Some("cls") => Self::Class,
            _ => Self::Other,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Procedure => "procedure",
            Self::Window => "window",
            Self::Class => "class",
            Self::Other => "other",
        }
    }
}

/// How one program uses one database table, summed over the file.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TableUse {
    /// `ACCESS`, `REFERENCE`, `SEARCH`, `SORT-ACCESS`.
    pub reads: u32,
    /// `UPDATE`.
    pub updates: u32,
    /// `CREATE`.
    pub creates: u32,
    /// `DELETE`.
    pub deletes: u32,
    /// Fields named by `ACCESS` references.
    pub fields_read: BTreeSet<String>,
    /// Fields named by `UPDATE` references.
    pub fields_updated: BTreeSet<String>,
}

/// The summary of one `.xref.xml`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct XrefFile {
    /// Normalised path of the main `<Source>` (see [`normalize_path`]).
    pub program: String,
    pub kind: Option<ProgramKind>,
    /// Fully qualified class name when the unit is a `.cls` (`CLASS` ref).
    pub class: Option<String>,
    /// Normalised include paths (`INCLUDE`).
    pub includes: BTreeSet<String>,
    /// `RUN <file>` targets (normalised path → count).
    pub runs: BTreeMap<String, u32>,
    /// `RUN <internal-procedure>` targets (bare name → count).
    pub calls: BTreeMap<String, u32>,
    /// `RUN VALUE(expr)`: targets unknown at compile time.
    pub dynamic_runs: u32,
    /// Classes used through `INVOKE`, `NEW`, `CAST` (class name → count).
    pub invokes: BTreeMap<String, u32>,
    /// Internal procedures the program defines (`PROCEDURE`).
    pub procedures: BTreeSet<String>,
    /// User-defined functions the program defines (`FUNCTION`).
    pub functions: BTreeSet<String>,
    /// `db.table` → use.
    pub tables: BTreeMap<String, TableUse>,
    /// `STRING` references (literals), counted only.
    pub strings: u32,
    /// Every `<Reference>` seen.
    pub references: u32,
}

impl XrefFile {
    /// Fold another summary of the same program into this one (two xref
    /// files for one source path: counts add up, sets unite, the first
    /// file's kind and class stand).
    pub fn merge(&mut self, other: XrefFile) {
        self.includes.extend(other.includes);
        for (k, n) in other.runs {
            *self.runs.entry(k).or_insert(0) += n;
        }
        for (k, n) in other.calls {
            *self.calls.entry(k).or_insert(0) += n;
        }
        self.dynamic_runs += other.dynamic_runs;
        for (k, n) in other.invokes {
            *self.invokes.entry(k).or_insert(0) += n;
        }
        self.procedures.extend(other.procedures);
        self.functions.extend(other.functions);
        for (t, u) in other.tables {
            let e = self.tables.entry(t).or_default();
            e.reads += u.reads;
            e.updates += u.updates;
            e.creates += u.creates;
            e.deletes += u.deletes;
            e.fields_read.extend(u.fields_read);
            e.fields_updated.extend(u.fields_updated);
        }
        self.strings += other.strings;
        self.references += other.references;
        if self.class.is_none() {
            self.class = other.class;
        }
    }
}

/// Lower-cases, turns `\` into `/`, drops `./`, and strips the first of
/// `roots` that prefixes the path (compared case-insensitively, `\` or
/// `/`). Programs compiled from `D:\App\ERP\sd\x.p` and run as `sd/x.p`
/// then share one identity.
pub fn normalize_path(raw: &str, roots: &[String]) -> String {
    let mut p = raw.replace('\\', "/").to_lowercase();
    for root in roots {
        let r = root.replace('\\', "/").to_lowercase();
        let r = r.trim_end_matches('/');
        if let Some(rest) = p.strip_prefix(r) {
            if let Some(rest) = rest.strip_prefix('/') {
                p = rest.to_string();
                break;
            }
        }
    }
    while let Some(rest) = p.strip_prefix("./") {
        p = rest.to_string();
    }
    p
}

/// `db.table` when `s` names a database table: exactly one `.`, no `:`
/// (class members), no space (parameter signatures). Temp-tables have no
/// database prefix and are left out, as are .NET types (`System.Object`
/// has one dot: the known database names are checked by the import, not
/// here — the parser does not know the schema).
fn table_name(s: &str) -> Option<&str> {
    let trimmed = s.trim();
    if trimmed.is_empty()
        || trimmed.contains(':')
        || trimmed.contains(' ')
        || trimmed.matches('.').count() != 1
    {
        return None;
    }
    Some(trimmed)
}

fn attr(e: &BytesStart<'_>, key: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        if a.key.as_ref() != key {
            return None;
        }
        // Paths and identifiers carry at most `&amp;`; strings are dropped
        // anyway, so a failed unescape keeps the raw text.
        Some(
            quick_xml::escape::unescape(&a.value)
                .map(|c| c.into_owned())
                .unwrap_or_else(|_| a.value.to_string()),
        )
    })
}

fn bump(map: &mut BTreeMap<String, u32>, key: String) {
    *map.entry(key).or_insert(0) += 1;
}

/// Parse one xref document from a buffered reader.
pub fn parse_xref<R: BufRead>(input: R, roots: &[String]) -> Result<XrefFile, String> {
    let mut reader = Reader::from_reader(input);
    reader.config_mut().trim_text(true);
    let mut out = XrefFile::default();
    let mut buf = Vec::with_capacity(4096);

    // The <Reference> being read: its type, identifier and context.
    let mut ref_type: Option<String> = None;
    let mut ref_ident: Option<String> = None;
    let mut context = String::new();
    let mut in_context = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match e.name().as_ref() {
                "Source" if out.program.is_empty() => {
                    if let Some(f) = attr(&e, "File-name") {
                        out.program = normalize_path(&f, roots);
                        out.kind = Some(ProgramKind::of(&out.program));
                    }
                }
                "Reference" => {
                    ref_type = attr(&e, "Reference-type");
                    ref_ident = attr(&e, "Object-identifier");
                    context.clear();
                    out.references += 1;
                }
                "Object-context" if ref_type.is_some() => in_context = true,
                _ => {}
            },
            Ok(Event::Text(t)) if in_context => {
                context.push_str(&t.xml10_content());
            }
            Ok(Event::End(e)) => match e.name().as_ref() {
                "Object-context" => in_context = false,
                "Reference" => {
                    if let (Some(ty), Some(id)) = (ref_type.take(), ref_ident.take()) {
                        record(&mut out, &ty, id, &context, roots);
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(format!(
                    "xml parse at byte {}: {e}",
                    reader.buffer_position()
                ))
            }
            _ => {}
        }
        buf.clear();
    }
    if out.program.is_empty() {
        return Err("no <Source> element".into());
    }
    Ok(out)
}

/// Parse the file at `path`.
pub fn parse_xref_file(path: &Path, roots: &[String]) -> Result<XrefFile, String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    parse_xref(std::io::BufReader::with_capacity(1 << 16, f), roots)
}

fn record(out: &mut XrefFile, ty: &str, id: String, context: &str, roots: &[String]) {
    match ty {
        // Field-level: identifier is the field, context the table. ABL names
        // are case-insensitive and the xref spells them as the code does
        // (`msdb.x`, `msDB.x`): one entry per table, lower-case.
        "ACCESS" | "UPDATE" => {
            if let Some(t) = table_name(context) {
                let u = out.tables.entry(t.to_lowercase()).or_default();
                if ty == "ACCESS" {
                    u.reads += 1;
                    u.fields_read.insert(id.to_lowercase());
                } else {
                    u.updates += 1;
                    u.fields_updated.insert(id.to_lowercase());
                }
            }
        }
        // Table-level: identifier is the table.
        "REFERENCE" | "SEARCH" | "SORT-ACCESS" | "CREATE" | "DELETE" => {
            if let Some(t) = table_name(&id) {
                let u = out.tables.entry(t.to_lowercase()).or_default();
                match ty {
                    "CREATE" => u.creates += 1,
                    "DELETE" => u.deletes += 1,
                    _ => u.reads += 1,
                }
            }
        }
        "RUN" => {
            let lower = id.to_ascii_lowercase();
            if lower.starts_with("value(") {
                out.dynamic_runs += 1;
            } else if lower.ends_with(".p")
                || lower.ends_with(".w")
                || lower.ends_with(".r")
                || lower.ends_with(".cls")
            {
                bump(&mut out.runs, normalize_path(&id, roots));
            } else {
                bump(&mut out.calls, lower);
            }
        }
        "INCLUDE" => {
            out.includes.insert(normalize_path(&id, roots));
        }
        "PROCEDURE" => {
            out.procedures.insert(id.to_ascii_lowercase());
        }
        "FUNCTION" => {
            out.functions.insert(id.to_ascii_lowercase());
        }
        "INVOKE" | "NEW" | "CAST" => {
            // `Class:member` → the class; a .NET generic comes assembly-
            // qualified (`"Ns.List`1[[T, Asm, Version=…]]`) → `Ns.List`.
            let class = id.trim_start_matches('"');
            let end = class.find([':', '`', '[']).unwrap_or(class.len());
            bump(&mut out.invokes, class[..end].to_string());
        }
        "CLASS" => out.class = Some(id),
        "STRING" => out.strings += 1,
        _ => {}
    }
}

/// Every `*.xref.xml` under a path, parsed.
#[derive(Debug, Default)]
pub struct ParsedDir {
    pub files: Vec<XrefFile>,
    /// Program identities of every parsed file (kept when `files` is moved
    /// out by [`XrefSource::from_path`]): what a `--sync` keeps.
    pub programs: BTreeSet<String>,
    /// Files the parser refused, with the reason.
    pub errors: Vec<(PathBuf, String)>,
    pub bytes: u64,
}

fn collect_xref_files(path: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if path.is_file() {
        out.push(path.to_path_buf());
        return Ok(());
    }
    for entry in std::fs::read_dir(path)? {
        let p = entry?.path();
        if p.is_dir() {
            collect_xref_files(&p, out)?;
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

/// Parse every `*.xref.xml` under `path` (searched recursively; a single
/// file works too) on `threads` threads (default: the cores). Blocking:
/// call it from a blocking context. The result is sorted by program path
/// so the import order — and the ids it allocates — is deterministic.
pub fn parse_dir(
    path: &Path,
    roots: &[String],
    threads: Option<usize>,
) -> std::io::Result<ParsedDir> {
    let mut paths = Vec::new();
    collect_xref_files(path, &mut paths)?;
    paths.sort();
    let threads = threads
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()))
        .max(1);
    let roots: Arc<[String]> = roots.into();
    let queue = Arc::new(Mutex::new(paths.into_iter()));
    let (tx, rx) = mpsc::channel::<(PathBuf, u64, Result<XrefFile, String>)>();
    let workers: Vec<_> = (0..threads)
        .map(|_| {
            let queue = Arc::clone(&queue);
            let tx = tx.clone();
            let roots = Arc::clone(&roots);
            std::thread::spawn(move || loop {
                let next = queue.lock().unwrap_or_else(|e| e.into_inner()).next();
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
    let mut out = ParsedDir::default();
    for (p, size, parsed) in rx {
        out.bytes += size;
        match parsed {
            Ok(f) => out.files.push(f),
            Err(e) => out.errors.push((p, e)),
        }
    }
    for w in workers {
        let _ = w.join();
    }
    out.files.sort_by(|a, b| a.program.cmp(&b.program));
    Ok(out)
}

/// Storage domain of every type the xref import declares.
pub const XREF_NS: &str = "erp";

fn concept_type(name: &str, description: &str, properties: &[&str]) -> Record {
    Record::ConceptTypeDecl(ConceptType {
        name: name.into(),
        description: description.into(),
        properties: Some(properties.iter().map(|p| p.to_string()).collect()),
        ns: Some(XREF_NS.into()),
        ..Default::default()
    })
}

fn relation_type(
    name: &str,
    domain: &str,
    range: &str,
    cardinality: Cardinality,
    description: &str,
) -> Record {
    Record::RelationTypeDecl(RelationType {
        name: name.into(),
        domain: domain.into(),
        range: range.into(),
        cardinality,
        description: description.into(),
        ..Default::default()
    })
}

fn rule(name: &str, when: &str, then: &str, applies_to: &[&str]) -> Record {
    Record::RuleTypeDecl(RuleType {
        name: name.into(),
        when: when.into(),
        then: then.into(),
        applies_to: applies_to.iter().map(|t| t.to_string()).collect(),
        strict: false,
        description: String::new(),
    })
}

/// The `erp` model: types, link types and the rules the prompt needs to
/// reason about impact. Every link's `weight` is the number of references
/// in the program's compiled code.
pub fn xref_ontology_records() -> Vec<Record> {
    use Cardinality::{ManyToMany as NN, ManyToOne as N1, OneToMany as ONE_N};
    vec![
        concept_type("Module", "A directory of the ERP source tree: the unit people talk about (payroll, accounting, …).", &["path", "program_count"]),
        concept_type("Program", "A compiled OpenEdge program (.p procedure, .w window, .cls class). `code` is the ERP screen code users know (ca100); `missing` marks a program other programs run but whose compiled cross-reference is absent.", &["path", "code", "kind", "module", "missing", "procedures", "functions", "dynamic_runs", "strings", "references"]),
        concept_type("Include", "An OpenEdge include file (.i), compiled into every program that includes it.", &["path"]),
        concept_type("Class", "An ABL or .NET class used by programs.", &["name"]),
        concept_type("Database", "An OpenEdge database (msdb, msec, …).", &[]),
        concept_type("Table", "A database table, named db.table.", &["db", "table"]),
        concept_type("Field", "A table field, named db.table.field.", &["table", "field"]),
        relation_type("belongs_to", "Program", "Module", N1, "the program's directory"),
        relation_type("runs", "Program", "Program", NN, "RUN of another program; weight = number of RUN statements"),
        relation_type("includes", "Program", "Include", NN, "the include is compiled into the program"),
        relation_type("invokes", "Program", "Class", NN, "INVOKE, NEW or CAST of the class; weight = number of uses"),
        relation_type("in_database", "Table", "Database", N1, "the table's database"),
        relation_type("has_field", "Table", "Field", ONE_N, "the table's field"),
        relation_type("reads", "Program", "Table", NN, "the program reads the table (ACCESS, REFERENCE, SEARCH); weight = number of references"),
        relation_type("updates", "Program", "Table", NN, "the program updates fields of the table; weight = number of references"),
        relation_type("creates", "Program", "Table", NN, "the program creates rows; weight = number of CREATE statements"),
        relation_type("deletes", "Program", "Table", NN, "the program deletes rows; weight = number of DELETE statements"),
        relation_type("reads_field", "Program", "Field", NN, "the program reads the field"),
        relation_type("updates_field", "Program", "Field", NN, "the program updates the field"),
        rule("table_change_impacts_writers", "a Table or Field changes (type, format, meaning)", "every Program that updates, creates, deletes or reads it is impacted; writers first", &["Table", "Field"]),
        rule("include_change_impacts_all", "an Include changes", "every Program that includes it must be recompiled and is impacted", &["Include"]),
        rule("answer_at_module_level", "a question is about an area of the ERP rather than a specific program", "answer with Modules and the most used Programs, not a list of every program", &["Module"]),
    ]
}

fn text(s: &str) -> PropertyValue {
    PropertyValue::Text(s.to_string())
}

fn names(set: &BTreeSet<String>) -> PropertyValue {
    PropertyValue::List(set.iter().map(|s| text(s)).collect())
}

fn module_of(path: &str) -> &str {
    path.split_once('/').map_or("root", |(m, _)| m)
}

/// The ERP's screen code of a program path — what business users and
/// developers call it: `ca/w-ca100.w` → `ca100`, `db/b6-db100.w` →
/// `db100`, `net/sd/gui/planning.cls` → `None`. The short ADM prefix
/// (`w-` window, `b-` browser, `v-` viewer, `d-` dialog, `b6-`, …) names a
/// part of the screen; the code is what follows: two letters, three
/// digits, then anything (`sd100dat`).
pub fn screen_code(path: &str) -> Option<String> {
    let file = path.rsplit('/').next().unwrap_or(path);
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
    let code = match stem.split_once('-') {
        Some((prefix, rest))
            if (1..=3).contains(&prefix.len())
                && prefix
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic())
                && prefix.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            rest
        }
        _ => stem,
    };
    let b = code.as_bytes();
    let looks_like_code = b.len() >= 5
        && b[..2].iter().all(u8::is_ascii_alphabetic)
        && b[2..5].iter().all(u8::is_ascii_digit);
    looks_like_code.then(|| code.to_string())
}

fn concept(ty: &str, name: &str) -> Concept {
    Concept::new(ConceptId(0), ty, name)
}

fn link(ty: &str, st: &str, sn: &str, tt: &str, tn: &str, weight: u32) -> Record {
    Record::NamedRelation {
        relation_type: ty.into(),
        source_type: st.into(),
        source_name: sn.into(),
        target_type: tt.into(),
        target_name: tn.into(),
        weight: weight as f32,
        properties: Default::default(),
    }
}

/// Turns parsed xref summaries into the records of the `erp` model, in
/// dependency order (types, then sheets, then links), so every link finds
/// its endpoints already applied. Lazy: the links are produced as the
/// ingester pulls them.
pub struct XrefSource {
    records: Box<dyn Iterator<Item = Record> + Send + Sync>,
}

impl XrefSource {
    /// Parse everything under `path` (see [`parse_dir`]) and build the source.
    pub fn from_path(
        path: &Path,
        roots: &[String],
        threads: Option<usize>,
    ) -> Result<(Self, ParsedDir), IngestError> {
        let mut parsed = parse_dir(path, roots, threads)?;
        let files = std::mem::take(&mut parsed.files);
        parsed.programs = files.iter().map(|f| f.program.clone()).collect();
        Ok((Self::from_files(files), parsed))
    }

    pub fn from_files(files: Vec<XrefFile>) -> Self {
        // One sheet per program: two xref files of one source path merge.
        let mut by_program: BTreeMap<String, XrefFile> = BTreeMap::new();
        for f in files {
            match by_program.entry(f.program.clone()) {
                std::collections::btree_map::Entry::Vacant(e) => {
                    e.insert(f);
                }
                std::collections::btree_map::Entry::Occupied(mut e) => e.get_mut().merge(f),
            }
        }
        let files: Vec<XrefFile> = by_program.into_values().collect();
        // Identities first: a RUN target resolves to a known program by exact
        // path, else by unique basename (the PROPATH finds `pcto.p` in `sd/`),
        // else it is a `missing` program.
        let known: BTreeSet<String> = files.iter().map(|f| f.program.clone()).collect();
        let mut by_basename: HashMap<&str, Vec<&str>> = HashMap::new();
        for p in &known {
            by_basename
                .entry(p.rsplit('/').next().unwrap_or(p))
                .or_default()
                .push(p);
        }
        let resolve = |target: &str| -> String {
            if known.contains(target) {
                return target.to_string();
            }
            match by_basename.get(target.rsplit('/').next().unwrap_or(target)) {
                Some(v) if v.len() == 1 => v[0].to_string(),
                _ => target.to_string(),
            }
        };

        let mut missing: BTreeSet<String> = BTreeSet::new();
        let mut modules: BTreeMap<String, u32> = BTreeMap::new();
        let mut tables: BTreeSet<String> = BTreeSet::new();
        let mut fields: BTreeSet<String> = BTreeSet::new();
        let mut includes: BTreeSet<String> = BTreeSet::new();
        // ABL names are case-insensitive and the graph's name uniqueness
        // too: `System.object` and `System.Object` are one class, spelt as
        // first seen.
        let mut classes: BTreeMap<String, String> = BTreeMap::new();
        // Per program: its RUN targets after resolution.
        let mut resolved_runs: Vec<BTreeMap<String, u32>> = Vec::with_capacity(files.len());
        for f in &files {
            *modules
                .entry(module_of(&f.program).to_string())
                .or_insert(0) += 1;
            includes.extend(f.includes.iter().cloned());
            for c in f.invokes.keys() {
                classes.entry(c.to_lowercase()).or_insert_with(|| c.clone());
            }
            let mut runs = BTreeMap::new();
            for (t, n) in &f.runs {
                let r = resolve(t);
                if !known.contains(&r) {
                    missing.insert(r.clone());
                }
                *runs.entry(r).or_insert(0) += n;
            }
            resolved_runs.push(runs);
            for (t, u) in &f.tables {
                let t = t.to_lowercase();
                for field in u.fields_read.iter().chain(&u.fields_updated) {
                    fields.insert(format!("{t}.{}", field.to_lowercase()));
                }
                tables.insert(t);
            }
        }
        for m in missing.iter() {
            modules.entry(module_of(m).to_string()).or_insert(0);
        }
        let databases: BTreeSet<String> = tables
            .iter()
            .filter_map(|t| t.split_once('.').map(|(db, _)| db.to_string()))
            .collect();

        let sheets: Vec<Record> = xref_ontology_records()
            .into_iter()
            .chain(
                databases
                    .iter()
                    .map(|d| Record::Concept(concept("Database", d))),
            )
            .chain(modules.iter().map(|(m, n)| {
                Record::Concept(
                    concept("Module", m)
                        .with_property("path", text(m))
                        .with_property("program_count", PropertyValue::Number(f64::from(*n))),
                )
            }))
            .chain(tables.iter().map(|t| {
                let (db, table) = t.split_once('.').unwrap_or(("", t));
                Record::Concept(
                    concept("Table", t)
                        .with_property("db", text(db))
                        .with_property("table", text(table)),
                )
            }))
            .chain(fields.iter().map(|f| {
                let (table, field) = f.rsplit_once('.').unwrap_or(("", f));
                Record::Concept(
                    concept("Field", f)
                        .with_property("table", text(table))
                        .with_property("field", text(field)),
                )
            }))
            .chain(
                includes
                    .iter()
                    .map(|i| Record::Concept(concept("Include", i).with_property("path", text(i)))),
            )
            .chain(
                classes
                    .values()
                    .map(|c| Record::Concept(concept("Class", c).with_property("name", text(c)))),
            )
            .chain(files.iter().map(|f| {
                let mut c = concept("Program", &f.program)
                    .with_property("path", text(&f.program))
                    .with_property("kind", text(f.kind.map_or("other", |k| k.as_str())))
                    .with_property("module", text(module_of(&f.program)))
                    .with_property("missing", PropertyValue::Bool(false))
                    .with_property("procedures", names(&f.procedures))
                    .with_property("functions", names(&f.functions))
                    .with_property(
                        "dynamic_runs",
                        PropertyValue::Number(f64::from(f.dynamic_runs)),
                    )
                    .with_property("strings", PropertyValue::Number(f64::from(f.strings)))
                    .with_property("references", PropertyValue::Number(f64::from(f.references)));
                if let Some(code) = screen_code(&f.program) {
                    c = c.with_property("code", text(&code));
                }
                Record::Concept(c)
            }))
            .chain(missing.iter().map(|m| {
                Record::Concept(
                    concept("Program", m)
                        .with_property("path", text(m))
                        .with_property("kind", text(ProgramKind::of(m).as_str()))
                        .with_property("module", text(module_of(m)))
                        .with_property("missing", PropertyValue::Bool(true)),
                )
            }))
            .collect();

        let schema_links: Vec<Record> = tables
            .iter()
            .filter_map(|t| {
                t.split_once('.')
                    .map(|(db, _)| link("in_database", "Table", t, "Database", db, 1))
            })
            .chain(fields.iter().filter_map(|f| {
                f.rsplit_once('.')
                    .map(|(t, _)| link("has_field", "Table", t, "Field", f, 1))
            }))
            .chain(
                missing
                    .iter()
                    .map(|m| link("belongs_to", "Program", m, "Module", module_of(m), 1)),
            )
            .collect();

        // One program's links at a time; the per-program vector is small.
        let program_links = files
            .into_iter()
            .zip(resolved_runs)
            .flat_map(move |(f, runs)| {
                let p = f.program;
                let mut out: Vec<Record> = Vec::with_capacity(16 + f.tables.len() * 4);
                out.push(link(
                    "belongs_to",
                    "Program",
                    &p,
                    "Module",
                    module_of(&p),
                    1,
                ));
                out.extend(
                    runs.iter()
                        .map(|(t, n)| link("runs", "Program", &p, "Program", t, *n)),
                );
                out.extend(
                    f.includes
                        .iter()
                        .map(|i| link("includes", "Program", &p, "Include", i, 1)),
                );
                // `System.Object` and `System.object` are one class: one link.
                let mut invoked: BTreeMap<&str, u32> = BTreeMap::new();
                for (c, n) in &f.invokes {
                    *invoked
                        .entry(classes[&c.to_lowercase()].as_str())
                        .or_insert(0) += n;
                }
                out.extend(
                    invoked
                        .iter()
                        .map(|(c, n)| link("invokes", "Program", &p, "Class", c, *n)),
                );
                for (t, u) in &f.tables {
                    let t = t.to_lowercase();
                    for (ty, n) in [
                        ("reads", u.reads),
                        ("updates", u.updates),
                        ("creates", u.creates),
                        ("deletes", u.deletes),
                    ] {
                        if n > 0 {
                            out.push(link(ty, "Program", &p, "Table", &t, n));
                        }
                    }
                    out.extend(u.fields_read.iter().map(|fl| {
                        link(
                            "reads_field",
                            "Program",
                            &p,
                            "Field",
                            &format!("{t}.{}", fl.to_lowercase()),
                            1,
                        )
                    }));
                    out.extend(u.fields_updated.iter().map(|fl| {
                        link(
                            "updates_field",
                            "Program",
                            &p,
                            "Field",
                            &format!("{t}.{}", fl.to_lowercase()),
                            1,
                        )
                    }));
                }
                out
            });

        Self {
            records: Box::new(sheets.into_iter().chain(schema_links).chain(program_links)),
        }
    }
}

/// After a re-import: delete the `Program` sheets (and their links) that
/// the compiled set no longer contains — `keep` is the set of programs
/// just parsed. Sheets flagged `missing` (never compiled) stay: they are
/// run targets, not compiled units. Returns (sheets, links) removed.
pub async fn retire_programs(
    graph: &ontology_graph::OntologyGraph,
    store: Option<&dyn ontology_storage::Store>,
    keep: &BTreeSet<String>,
) -> Result<crate::ingest::DeleteReport, IngestError> {
    let gone: Vec<ConceptId> = graph
        .concepts_of_type("Program", false)
        .into_iter()
        .filter(|c| c.properties.get("missing") != Some(&PropertyValue::Bool(true)))
        .filter(|c| !keep.contains(&c.name))
        .map(|c| c.id)
        .collect();
    crate::ingest::delete_concepts(graph, store, &gone).await
}

#[async_trait]
impl Source for XrefSource {
    async fn next(&mut self) -> Result<Option<Record>, IngestError> {
        Ok(self.records.next())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(ty: &str, id: &str, ctx: &str) -> String {
        format!(
            r#"<Reference Reference-type="{ty}" Object-identifier="{id}">
  <Source-guid>g</Source-guid><File-num>1</File-num><Ref-seq>1</Ref-seq><Line-num>3</Line-num>
  <Object-context>{ctx}</Object-context><Access-mode/><Detail/>
</Reference>"#
        )
    }

    fn doc(refs: &[String]) -> String {
        format!(
            r#"<?xml version="1.0"?>
<Cross-reference xmlns="uri:schemas-progress-com:XREFD:0005">
  <Source File-name="D:\Mediasoft\App\MS.ERP2\sd\Pcto.p"><Source-guid>g</Source-guid><File-num>1</File-num>
  {}
  </Source>
  <Source File-name=".\sd\inc\tt.i"><Source-guid>g</Source-guid><File-num>2</File-num></Source>
</Cross-reference>"#,
            refs.join("\n")
        )
    }

    #[test]
    fn summarises_one_program() {
        let roots = vec!["D:\\Mediasoft\\App\\MS.ERP2".to_string()];
        let xml = doc(&[
            reference("COMPILE", "D:\\Mediasoft\\App\\MS.ERP2\\sd\\Pcto.p", ""),
            reference("STRING", "Soci&amp;t&#233;", ""),
            reference("ACCESS", "nste", "msdb.sdva"),
            reference("ACCESS", "PERI", "msDB.sdva"),
            reference("UPDATE", "nste", "msdb.sdva"),
            reference("ACCESS", "pAppl", "PUBLIC-PROPERTY"),
            reference("SEARCH", "msdb.batc", ""),
            reference("SEARCH", "Net.sd.Gui.Planning:ttSDBA", ""),
            reference("CREATE", "msdb.planning", ""),
            reference("DELETE", "msdb.planning", ""),
            reference("RUN", "sd/pcto2.p", ""),
            reference("RUN", "Net\\Sd\\Proc\\DrVacanGZ.p", ""),
            reference("RUN", "dispatch", ""),
            reference("RUN", "VALUE(cProg)", ""),
            reference("INCLUDE", "net/sd/inc/tt/ttsdba.i", ""),
            reference("PROCEDURE", "adm-create-controls", ""),
            reference("FUNCTION", "fGetDate", ""),
            reference("INVOKE", "Net.Class.Config:Get", ""),
            reference("NEW", "Net.Class.Config", ""),
            reference(
                "NEW",
                "&quot;System.Collections.Generic.List`1[[DevExpress.XtraBars.BarItem, DevExpress.XtraBars.v15.2, Version=15.2.10.0]]",
                "",
            ),
        ]);
        let f = parse_xref(xml.as_bytes(), &roots).unwrap();

        assert_eq!(f.program, "sd/pcto.p");
        assert_eq!(f.kind, Some(ProgramKind::Procedure));
        assert_eq!(f.references, 20);
        assert_eq!(f.strings, 1);

        let sdva = &f.tables["msdb.sdva"];
        assert_eq!((sdva.reads, sdva.updates), (2, 1));
        assert_eq!(sdva.fields_read.len(), 2);
        assert!(sdva.fields_updated.contains("nste"));
        assert_eq!(f.tables["msdb.batc"].reads, 1);
        let planning = &f.tables["msdb.planning"];
        assert_eq!((planning.creates, planning.deletes), (1, 1));
        assert_eq!(
            f.tables.len(),
            3,
            "class property and temp-table are not tables"
        );

        assert_eq!(f.runs["sd/pcto2.p"], 1);
        assert_eq!(f.runs["net/sd/proc/drvacangz.p"], 1);
        assert_eq!(f.calls["dispatch"], 1);
        assert_eq!(f.dynamic_runs, 1);
        assert!(f.includes.contains("net/sd/inc/tt/ttsdba.i"));
        assert!(f.procedures.contains("adm-create-controls"));
        assert!(f.functions.contains("fgetdate"));
        assert_eq!(f.invokes["Net.Class.Config"], 2);
        assert_eq!(f.invokes["System.Collections.Generic.List"], 1);
    }

    #[test]
    fn normalises_paths() {
        let roots = vec![
            "d:/mediasoft/app/ms.erp/".to_string(),
            "D:\\Mediasoft\\App\\MS.ERP2".to_string(),
        ];
        assert_eq!(
            normalize_path("D:\\Mediasoft\\App\\MS.ERP2\\Sd\\X.P", &roots),
            "sd/x.p"
        );
        assert_eq!(
            normalize_path("D:\\Mediasoft\\App\\MS.ERP\\sd\\x.p", &roots),
            "sd/x.p"
        );
        assert_eq!(
            normalize_path(".\\net\\web\\inc\\WebConfig.i", &roots),
            "net/web/inc/webconfig.i"
        );
        assert_eq!(normalize_path("sd/x.p", &roots), "sd/x.p");
        // A root that is only a prefix of a segment does not match.
        assert_eq!(
            normalize_path("D:\\Mediasoft\\App\\MS.ERP2x\\y.p", &roots),
            "d:/mediasoft/app/ms.erp2x/y.p"
        );
    }

    #[test]
    fn screen_codes_come_from_the_file_name() {
        assert_eq!(screen_code("ca/w-ca100.w").as_deref(), Some("ca100"));
        assert_eq!(screen_code("ca/b-ca100.w").as_deref(), Some("ca100"));
        assert_eq!(screen_code("db/b6-db100.w").as_deref(), Some("db100"));
        assert_eq!(screen_code("sd/d-sd100dat.p").as_deref(), Some("sd100dat"));
        assert_eq!(screen_code("ca/ca600tot.p").as_deref(), Some("ca600tot"));
        assert_eq!(screen_code("net/sd/gui/planning.cls"), None);
        assert_eq!(screen_code("scripts/#8663 - purge sdva.p"), None);
        assert_eq!(screen_code("ba/w-menu.w"), None);
    }

    #[test]
    fn rejects_documents_without_source() {
        assert!(parse_xref(
            br#"<?xml version="1.0"?><Cross-reference/>"#.as_slice(),
            &[]
        )
        .is_err());
        assert!(parse_xref(b"<Cross-reference><Source".as_slice(), &[]).is_err());
    }
}
