// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Progress ABL source scanner: the relations the cross-reference does not
//! carry, read from the `.p` / `.w` / `.i` files themselves.
//!
//! The compiler's xref says which program touches which table; it never
//! says how two tables relate. The programs do, thousands of times, in
//! their `WHERE` clauses (`acct.cono EQ entry.cono AND acct.year EQ
//! entry.year …`) and in `FOR EACH acct OF entry`. The screens' tabs and
//! titles are in the `.w` files too. This module reads them with a few
//! patterns over the comment-stripped text — no ABL grammar — and reduces
//! each file to an [`AblFile`] summary.
//!
//! Only joins between tables the graph already knows (the `Table` sheets
//! of the xref import) are kept: that one rule drops temp-tables, local
//! variables and widget names without a symbol table.

use async_trait::async_trait;
use ontology_graph::{Cardinality, Concept, ConceptId, ConceptType, PropertyValue, RelationType};
use regex::{Regex, RegexBuilder};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use crate::charset::decode_to_utf8;
use crate::ingest::{IngestError, Source};
use crate::record::Record;
use crate::xref::{screen_code, XREF_NS};
use ontology_graph::OntologyGraph;

fn re(pattern: &str) -> Regex {
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .dot_matches_new_line(true)
        .build()
        .expect("static pattern")
}

static BUFFER: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"\bDEFINE\s+(?:NEW\s+)?(?:SHARED\s+)?BUFFER\s+([A-Za-z_][\w-]*)\s+FOR\s+(TEMP-TABLE\s+)?([A-Za-z_][\w-]*(?:\.[A-Za-z_][\w-]*)?)",
    )
});
static LOCAL_TABLE: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"\bDEFINE\s+(?:NEW\s+)?(?:SHARED\s+)?(?:TEMP-TABLE|WORK-TABLE|WORKFILE)\s+([A-Za-z_][\w-]*)",
    )
});
static JOIN: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"\b([A-Za-z_][\w-]*(?:\.[A-Za-z_][\w-]*)?)\.([A-Za-z_][\w-]*)\s*(?:=|\bEQ\b)\s*([A-Za-z_][\w-]*(?:\.[A-Za-z_][\w-]*)?)\.([A-Za-z_][\w-]*)",
    )
});
static OF: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"\b(?:FOR\s+(?:EACH|FIRST|LAST)|FIND(?:\s+(?:FIRST|LAST|PREV|NEXT))?|CAN-FIND\s*\(\s*(?:FIRST\s+|LAST\s+)?)\s*([A-Za-z_][\w.-]*)\s+OF\s+([A-Za-z_][\w.-]*)",
    )
});
static TITLE: LazyLock<Regex> = LazyLock::new(|| re(r#"\bTITLE\s*=\s*"([^"]*)""#));
static MENU_ITEM: LazyLock<Regex> =
    LazyLock::new(|| re(r#"\bMENU-ITEM\s+[\w-]+\s+LABEL\s+"([^"]*)""#));
static QUOTED: LazyLock<Regex> = LazyLock::new(|| re(r#"'([^']*)'|"([^"]*)""#));
static IDENT: LazyLock<Regex> = LazyLock::new(|| re(r"\b([A-Za-z_][\w-]*)\s*\+"));
static FOLDER_LABELS: LazyLock<Regex> = LazyLock::new(|| re(r"folder-labels"));
/// `vfolder = "a|b"` or `vfolder = vfolder + "|c"`: a tab list assigned to a
/// variable, any variable; the caller keeps the ones a `FOLDER-LABELS`
/// statement concatenates.
static TAB_ASSIGN: LazyLock<Regex> = LazyLock::new(|| {
    re(r#"\b([A-Za-z_][\w-]*)\s*=\s*(?:[A-Za-z_][\w-]*\s*\+\s*)?["']([^"']*\|[^"']*)["']"#)
});

/// The tables the graph knows, by `db.table` and by bare name.
#[derive(Debug, Default, Clone)]
pub struct TableIndex {
    qualified: BTreeSet<String>,
    by_bare: HashMap<String, String>,
}

impl TableIndex {
    /// `tables` are `db.table` names. A bare name present in several
    /// databases resolves to the database that has the most tables — the
    /// application's main database, which is what unqualified code means.
    pub fn new(tables: impl IntoIterator<Item = String>) -> Self {
        let qualified: BTreeSet<String> = tables.into_iter().map(|t| t.to_lowercase()).collect();
        let mut per_db: HashMap<&str, usize> = HashMap::new();
        for q in &qualified {
            if let Some((db, _)) = q.split_once('.') {
                *per_db.entry(db).or_insert(0) += 1;
            }
        }
        let mut by_bare: HashMap<String, String> = HashMap::new();
        for q in &qualified {
            let Some((db, bare)) = q.split_once('.') else {
                continue;
            };
            match by_bare.get(bare) {
                Some(prev) => {
                    let prev_db = prev.split_once('.').map_or("", |(d, _)| d);
                    if per_db.get(db) > per_db.get(prev_db) {
                        by_bare.insert(bare.to_string(), q.clone());
                    }
                }
                None => {
                    by_bare.insert(bare.to_string(), q.clone());
                }
            }
        }
        Self { qualified, by_bare }
    }

    /// `db.table` for a (lower-cased) `table` or `db.table` reference.
    fn resolve(&self, name: &str) -> Option<&str> {
        if name.contains('.') {
            return self.qualified.get(name).map(String::as_str);
        }
        self.by_bare.get(name).map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.qualified.is_empty()
    }
}

/// One join between two tables, summed over a file.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct JoinUse {
    /// Explicit `a.f = b.g` comparisons.
    pub count: u32,
    /// `f=g` pairs, oriented from the first table of the pair to the second.
    pub keys: BTreeMap<String, u32>,
    /// `FOR EACH a OF b`: a join on the fields the schema says they share.
    pub implicit: u32,
}

/// The summary of one source file.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AblFile {
    /// Path relative to the scanned root, lower-case, `/` separators —
    /// the same identity the xref import gives the program.
    pub program: String,
    /// The window's title (`TITLE = "ad100 - Impression des adresses"`).
    pub title: Option<String>,
    /// Folder tabs (`FOLDER-LABELS = 'Liste|Compte|…'`).
    pub tabs: Vec<String>,
    /// `MENU-ITEM … LABEL "…"`.
    pub menu_items: Vec<String>,
    /// `(db.table, db.table)`, the pair sorted → use.
    pub joins: BTreeMap<(String, String), JoinUse>,
}

/// Remove `/* … */` comments, which nest in ABL. Text inside string
/// literals is left alone only as far as it contains no comment opener:
/// a `/*` inside a string is rare enough in this code base to ignore.
pub fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            depth += 1;
            i += 2;
        } else if depth > 0 && bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            depth -= 1;
            i += 2;
        } else if depth > 0 {
            // Keep line structure so nothing after a comment is glued to
            // what came before it.
            if bytes[i] == b'\n' {
                out.push('\n');
            }
            i += 1;
        } else {
            // Copy one UTF-8 scalar.
            let ch = text[i..].chars().next().unwrap_or('\u{FFFD}');
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn clean_label(s: &str) -> String {
    s.replace('&', "").trim().to_string()
}

fn is_placeholder_tab(t: &str) -> bool {
    let lower = t.to_ascii_lowercase();
    lower.is_empty()
        || lower
            .strip_prefix("page ")
            .is_some_and(|n| n.chars().all(|c| c.is_ascii_digit()))
}

/// Tabs named in `FOLDER-LABELS` statements, including the literals
/// assigned to a variable the statement concatenates (`vfolder`).
fn folder_tabs(text: &str) -> Vec<String> {
    let mut tabs: Vec<String> = Vec::new();
    let mut push_literal = |lit: &str| {
        for t in lit.split('|') {
            let t = clean_label(t);
            if !is_placeholder_tab(&t) && !tabs.iter().any(|x| x.eq_ignore_ascii_case(&t)) {
                tabs.push(t);
            }
        }
    };
    let mut variables: BTreeSet<String> = BTreeSet::new();
    for m in FOLDER_LABELS.find_iter(text) {
        let mut start = m.start();
        // `'FOLDER-LABELS = ':U + 'Liste|Compte'`: the keyword sits inside
        // a literal, so the window opens at that literal's quote and the
        // literals then pair up correctly.
        if start > 0 && (text.as_bytes()[start - 1] == b'\'' || text.as_bytes()[start - 1] == b'"')
        {
            start -= 1;
        }
        let end = (start + 400).min(text.len());
        // Stay on a char boundary.
        let end = (end..=text.len())
            .find(|&e| text.is_char_boundary(e))
            .unwrap_or(text.len());
        let window = &text[start..end];
        for m in QUOTED.captures_iter(window) {
            let lit = m.get(1).or_else(|| m.get(2)).map_or("", |g| g.as_str());
            if lit.contains('|') {
                push_literal(lit);
            }
        }
        for m in IDENT.captures_iter(window) {
            let id = m[1].to_ascii_lowercase();
            if id != "u" && id != "folder-labels" {
                variables.insert(id);
            }
        }
    }
    if !variables.is_empty() {
        for m in TAB_ASSIGN.captures_iter(text) {
            if variables.contains(&m[1].to_ascii_lowercase()) {
                push_literal(&m[2]);
            }
        }
    }
    tabs
}

/// Scan one file's text. `program` is the file's identity (see
/// [`AblFile::program`]).
pub fn scan_abl(text: &str, program: &str, tables: &TableIndex) -> AblFile {
    let text = strip_comments(text);
    let mut out = AblFile {
        program: program.to_string(),
        ..Default::default()
    };

    // Names that mean something else than a database table in this file.
    let mut aliases: HashMap<String, String> = HashMap::new();
    let mut shadowed: BTreeSet<String> = BTreeSet::new();
    for m in BUFFER.captures_iter(&text) {
        let alias = m[1].to_lowercase();
        if m.get(2).is_some() {
            shadowed.insert(alias);
        } else {
            aliases.insert(alias, m[3].to_lowercase());
        }
    }
    for m in LOCAL_TABLE.captures_iter(&text) {
        shadowed.insert(m[1].to_lowercase());
    }
    let resolve = |name: &str| -> Option<String> {
        let lower = name.to_lowercase();
        if shadowed.contains(&lower) {
            return None;
        }
        let target = aliases.get(&lower).map(String::as_str).unwrap_or(&lower);
        if shadowed.contains(target) {
            return None;
        }
        tables.resolve(target).map(str::to_string)
    };

    for m in JOIN.captures_iter(&text) {
        let (Some(a), Some(b)) = (resolve(&m[1]), resolve(&m[3])) else {
            continue;
        };
        if a == b {
            continue;
        }
        let (fa, fb) = (m[2].to_lowercase(), m[4].to_lowercase());
        let (pair, key) = if a <= b {
            ((a, b), format!("{fa}={fb}"))
        } else {
            ((b, a), format!("{fb}={fa}"))
        };
        let u = out.joins.entry(pair).or_default();
        u.count += 1;
        *u.keys.entry(key).or_insert(0) += 1;
    }
    for m in OF.captures_iter(&text) {
        let (Some(a), Some(b)) = (resolve(&m[1]), resolve(&m[2])) else {
            continue;
        };
        if a == b {
            continue;
        }
        let pair = if a <= b { (a, b) } else { (b, a) };
        out.joins.entry(pair).or_default().implicit += 1;
    }

    if program.ends_with(".w") {
        out.title = TITLE
            .captures_iter(&text)
            .map(|m| m[1].trim().to_string())
            .find(|t| !t.is_empty() && !t.starts_with('<'));
        out.tabs = folder_tabs(&text);
        for m in MENU_ITEM.captures_iter(&text) {
            let label = clean_label(&m[1]);
            if !label.is_empty() && !out.menu_items.contains(&label) {
                out.menu_items.push(label);
            }
        }
    }
    out
}

/// Every source file under a root, scanned.
#[derive(Debug, Default)]
pub struct ScannedDir {
    pub files: Vec<AblFile>,
    pub errors: Vec<(PathBuf, String)>,
    pub bytes: u64,
    /// Files found under the root.
    pub file_count: usize,
    /// Files that had a join or screen information (set by `from_dir`,
    /// which moves `files` out).
    pub summarised: usize,
}

/// Scan every `.p`, `.w`, `.i`, `.cls` under `root` on `threads` threads.
/// Blocking. Files are decoded with the charset sniffer (the code base is
/// ISO-8859-15), comments stripped, and summarised; files without any join
/// or screen information are dropped from the result.
pub fn scan_dir(
    root: &Path,
    tables: &TableIndex,
    threads: Option<usize>,
) -> std::io::Result<ScannedDir> {
    let paths = crate::scan::walk_files(root, |p| {
        p.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "p" | "w" | "i" | "cls"))
    })?;
    let file_count = paths.len();
    let root: Arc<Path> = root.into();
    let tables = Arc::new(tables.clone());
    let results = crate::scan::map_files(paths, threads, move |p| match std::fs::read(p) {
        Ok(bytes) => {
            let program = p
                .strip_prefix(&*root)
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/")
                .to_lowercase();
            (
                bytes.len() as u64,
                Ok(scan_abl(&decode_to_utf8(&bytes).text, &program, &tables)),
            )
        }
        Err(e) => (0, Err(e.to_string())),
    });
    let mut out = ScannedDir {
        file_count,
        ..Default::default()
    };
    for (p, size, parsed) in results {
        out.bytes += size;
        match parsed {
            Ok(f) => {
                if !f.joins.is_empty() || f.title.is_some() || !f.tabs.is_empty() {
                    out.files.push(f);
                }
            }
            Err(e) => out.errors.push((p, e)),
        }
    }
    out.files.sort_by(|a, b| a.program.cmp(&b.program));
    Ok(out)
}

/// One ERP screen: the window, browser, viewer, … parts of a code folded
/// together (`w-gl100.w` + `b-gl100.w` → `gl100`).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ScreenAgg {
    pub title: Option<String>,
    pub tabs: Vec<String>,
    pub menu_items: Vec<String>,
    /// Program paths, in the order scanned.
    pub programs: Vec<String>,
}

/// Group the windows that carry a title, tabs or menu by screen code (the
/// path itself when the file name has no code). The `w-` part's title wins
/// over a browser's or viewer's.
pub fn group_screens(files: &[AblFile]) -> BTreeMap<String, ScreenAgg> {
    let mut screens: BTreeMap<String, ScreenAgg> = BTreeMap::new();
    for f in files {
        if f.title.is_none() && f.tabs.is_empty() && f.menu_items.is_empty() {
            continue;
        }
        let name = screen_code(&f.program).unwrap_or_else(|| f.program.clone());
        let e = screens.entry(name).or_default();
        let is_window = f
            .program
            .rsplit('/')
            .next()
            .is_some_and(|file| file.starts_with("w-"));
        if f.title.is_some() && (e.title.is_none() || is_window) {
            e.title = f.title.clone();
        }
        for t in &f.tabs {
            if !e.tabs.iter().any(|x| x.eq_ignore_ascii_case(t)) {
                e.tabs.push(t.clone());
            }
        }
        for m in &f.menu_items {
            if !e.menu_items.contains(m) {
                e.menu_items.push(m.clone());
            }
        }
        e.programs.push(f.program.clone());
    }
    screens
}

/// Turns scanned sources into records: `Screen` sheets linked to their
/// `Program`, and `joined_on` links between `Table` sheets carrying the
/// key list. Run after the xref import: `Program` and `Table` must exist.
pub struct AblSource {
    records: Box<dyn Iterator<Item = Record> + Send + Sync>,
}

impl AblSource {
    /// The scanner needs the `Table` and `Program` sheets of the xref
    /// import (see `xref_ontology_records`): take them from the graph.
    pub fn from_graph(
        root: &Path,
        graph: &OntologyGraph,
        threads: Option<usize>,
    ) -> Result<(Self, ScannedDir), IngestError> {
        let tables = TableIndex::new(
            graph
                .concepts_of_type("Table", false)
                .into_iter()
                .map(|c| c.name),
        );
        if tables.is_empty() {
            return Err(IngestError::Source(
                "no Table sheets in the graph: import the cross-reference first (`ingest --xref`)"
                    .into(),
            ));
        }
        let programs = graph
            .concepts_of_type("Program", false)
            .into_iter()
            .map(|c| c.name)
            .collect();
        Self::from_dir(root, &tables, programs, threads)
    }

    pub fn from_dir(
        root: &Path,
        tables: &TableIndex,
        programs: BTreeSet<String>,
        threads: Option<usize>,
    ) -> Result<(Self, ScannedDir), IngestError> {
        let mut scanned = scan_dir(root, tables, threads)?;
        let files = std::mem::take(&mut scanned.files);
        scanned.summarised = files.len();
        Ok((Self::from_files(files, programs), scanned))
    }

    /// `programs`: the `Program` sheets the graph has; a screen is linked
    /// to its program only when the program exists.
    pub fn from_files(files: Vec<AblFile>, programs: BTreeSet<String>) -> Self {
        let decls = vec![
            Record::ConceptTypeDecl(ConceptType {
                name: "Screen".into(),
                description: "A screen of the ERP, named by its code (gl100): its title, folder tabs and menu entries; the window, browser and viewer programs behind it.".into(),
                properties: Some(["code", "title", "tabs", "menu_items", "programs"].iter().map(|s| s.to_string()).collect()),
                ns: Some(XREF_NS.into()),
                ..Default::default()
            }),
            Record::RelationTypeDecl(RelationType {
                name: "implemented_by".into(),
                domain: "Screen".into(),
                range: "Program".into(),
                cardinality: Cardinality::ManyToMany,
                description: "is implemented by".into(),
                ..Default::default()
            }),
            Record::RelationTypeDecl(RelationType {
                name: "joined_on".into(),
                domain: "Table".into(),
                range: "Table".into(),
                cardinality: Cardinality::ManyToMany,
                symmetric: true,
                description: "is joined in the code with".into(),
                ..Default::default()
            }),
        ];

        // Joins summed over the code base.
        let mut joins: BTreeMap<(String, String), (JoinUse, u32)> = BTreeMap::new();
        for f in &files {
            for (pair, u) in &f.joins {
                let e = joins.entry(pair.clone()).or_default();
                e.0.count += u.count;
                e.0.implicit += u.implicit;
                for (k, n) in &u.keys {
                    *e.0.keys.entry(k.clone()).or_insert(0) += n;
                }
                e.1 += 1;
            }
        }

        let grouped = group_screens(&files);
        let screens = grouped.into_iter().flat_map(move |(name, agg)| {
            let mut out = Vec::with_capacity(1 + agg.programs.len());
            let mut c = Concept::new(ConceptId(0), "Screen", &name)
                .with_property("code", PropertyValue::from(&name))
                .with_property(
                    "tabs",
                    PropertyValue::List(agg.tabs.iter().map(PropertyValue::from).collect()),
                )
                .with_property(
                    "menu_items",
                    PropertyValue::List(agg.menu_items.iter().map(PropertyValue::from).collect()),
                )
                .with_property(
                    "programs",
                    PropertyValue::List(agg.programs.iter().map(PropertyValue::from).collect()),
                );
            if let Some(t) = &agg.title {
                c = c.with_property("title", PropertyValue::from(t));
                c.description = t.clone();
            }
            out.push(Record::Concept(c));
            for p in agg.programs.iter().filter(|p| programs.contains(*p)) {
                out.push(Record::NamedRelation {
                    relation_type: "implemented_by".into(),
                    source_type: "Screen".into(),
                    source_name: name.clone(),
                    target_type: "Program".into(),
                    target_name: p.clone(),
                    weight: 1.0,
                    properties: Default::default(),
                });
            }
            out
        });

        let links = joins.into_iter().map(|((a, b), (u, programs))| {
            let mut keys: Vec<(&String, &u32)> = u.keys.iter().collect();
            keys.sort_by(|x, y| y.1.cmp(x.1).then(x.0.cmp(y.0)));
            let mut properties = ahash::AHashMap::default();
            properties.insert(
                "keys".to_string(),
                PropertyValue::list(keys.iter().map(|(k, _)| k.as_str())),
            );
            properties.insert(
                "programs".to_string(),
                PropertyValue::Number(f64::from(programs)),
            );
            properties.insert(
                "implicit".to_string(),
                PropertyValue::Number(f64::from(u.implicit)),
            );
            Record::NamedRelation {
                relation_type: "joined_on".into(),
                source_type: "Table".into(),
                source_name: a,
                target_type: "Table".into(),
                target_name: b,
                weight: (u.count + u.implicit) as f32,
                properties,
            }
        });

        Self {
            records: Box::new(decls.into_iter().chain(screens).chain(links)),
        }
    }
}

#[async_trait]
impl Source for AblSource {
    async fn next(&mut self) -> Result<Option<Record>, IngestError> {
        Ok(self.records.next())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables() -> TableIndex {
        TableIndex::new(
            [
                "appdb.acct",
                "appdb.entry",
                "appdb.line",
                "appdb.util",
                "secdb.util",
                "secdb.ecri2",
            ]
            .iter()
            .map(|s| s.to_string()),
        )
    }

    #[test]
    fn strips_nested_comments_and_keeps_lines() {
        let src = "a /* one /* two */ still */ b\nc /* x\ny */ d";
        assert_eq!(strip_comments(src), "a  b\nc \n d");
    }

    #[test]
    fn bare_table_names_resolve_to_the_main_database() {
        let t = tables();
        assert_eq!(t.resolve("util"), Some("appdb.util"));
        assert_eq!(t.resolve("secdb.util"), Some("secdb.util"));
        assert_eq!(t.resolve("nothing"), None);
    }

    #[test]
    fn finds_joins_through_aliases_and_across_lines() {
        let src = r#"
DEFINE BUFFER b-entry FOR entry.
DEFINE TEMP-TABLE line NO-UNDO FIELD cono AS INTEGER.
/* FIND acct WHERE acct.cono = entry.cono. (commented out) */
FIND acct USE-INDEX acctno WHERE acct.cono EQ b-entry.cono
                           AND   acct.year EQ b-entry.year
                           AND   acct.entry eq b-entry.entry NO-LOCK NO-ERROR.
FOR EACH line WHERE line.cono = entry.cono: END.   /* line is a temp-table here */
FOR EACH acct OF entry: END.
ASSIGN entry.cono = entry.cono.
"#;
        let f = scan_abl(src, "gl/x.p", &tables());
        assert_eq!(f.joins.len(), 1, "{:?}", f.joins);
        let u = &f.joins[&("appdb.acct".to_string(), "appdb.entry".to_string())];
        assert_eq!(u.count, 3);
        assert_eq!(u.implicit, 1);
        assert_eq!(u.keys["cono=cono"], 1);
        assert_eq!(u.keys["entry=entry"], 1);
        assert!(f.title.is_none(), "a .p has no screen");
    }

    #[test]
    fn screens_group_their_parts_by_code() {
        let browser = AblFile {
            program: "gl/b-gl100.w".into(),
            title: Some("Liste".into()),
            tabs: vec!["Liste".into(), "Compte".into()],
            ..Default::default()
        };
        let window = AblFile {
            program: "gl/w-gl100.w".into(),
            title: Some("gl100 - Grand livre".into()),
            tabs: vec!["Compte".into(), "Cumuls".into()],
            menu_items: vec!["Masquer".into()],
            ..Default::default()
        };
        let silent = AblFile {
            program: "gl/v-gl100.w".into(),
            ..Default::default()
        };
        let other = AblFile {
            program: "ba/w-menu.w".into(),
            title: Some("Menu".into()),
            ..Default::default()
        };
        let g = group_screens(&[browser, window, silent, other]);
        assert_eq!(g.len(), 2, "{g:?}");
        let gl100 = &g["gl100"];
        assert_eq!(gl100.title.as_deref(), Some("gl100 - Grand livre"));
        assert_eq!(gl100.tabs, ["Liste", "Compte", "Cumuls"]);
        assert_eq!(gl100.menu_items, ["Masquer"]);
        assert_eq!(gl100.programs, ["gl/b-gl100.w", "gl/w-gl100.w"]);
        assert!(
            g.contains_key("ba/w-menu.w"),
            "no code: the path is the name"
        );
    }

    #[test]
    fn a_window_ending_on_the_keyword_does_not_panic() {
        let f = scan_abl("RUN x ('FOLDER-LABELS", "gl/w-x.w", &tables());
        assert!(f.tabs.is_empty());
        let f = scan_abl("FOLDER-LABELS", "gl/w-x.w", &tables());
        assert!(f.tabs.is_empty());
    }

    #[test]
    fn reads_title_tabs_and_menu_of_a_window() {
        let src = r#"
TITLE = "<insert SmartWindow title>"
DEFINE MENU POPUP-MENU-Edit-1
       MENU-ITEM m_Essentiel_Rouge LABEL "&Essentiel (Rouge)"
       MENU-ITEM m_Masquer LABEL "Masquer".
ASSIGN vfolder = "Liste|Compte".
IF x THEN ASSIGN vfolder = vfolder + "|Historique|Cumuls|Ecr. effacées".
RUN set-attribute-list IN h_folder ('FOLDER-LABELS = ':U + vfolder + ',
                                     FOLDER-TAB-TYPE = 2':U).
RUN set-attribute-list IN h_other ('FOLDER-LABELS = ':U + 'Base|Page 8|Page 9|Divers' + ',').
CREATE WINDOW W-Win ASSIGN TITLE = "gl100 - Grand livre".
"#;
        let f = scan_abl(src, "gl/w-gl100.w", &tables());
        assert_eq!(f.title.as_deref(), Some("gl100 - Grand livre"));
        assert_eq!(
            f.tabs,
            [
                "Base",
                "Divers",
                "Liste",
                "Compte",
                "Historique",
                "Cumuls",
                "Ecr. effacées"
            ]
        );
        assert_eq!(f.menu_items, ["Essentiel (Rouge)", "Masquer"]);
    }
}
