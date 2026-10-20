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
//! their `WHERE` clauses (`cali.nste EQ ecri.nste AND cali.exer EQ
//! ecri.exer …`) and in `FOR EACH cali OF ecri`. The screens' tabs and
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
use std::sync::{mpsc, Arc, LazyLock, Mutex};

use crate::charset::decode_to_utf8;
use crate::ingest::{IngestError, Source};
use crate::record::Record;
use crate::xref::XREF_NS;

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
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    while let Some(pos) = lower[from..].find("folder-labels") {
        let mut start = from + pos;
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
        from = start + 1 + "folder-labels".len();
    }
    for var in variables {
        let assign = re(&format!(
            r#"\b{}\s*=\s*(?:{}\s*\+\s*)?["']([^"']*\|[^"']*)["']"#,
            regex::escape(&var),
            regex::escape(&var)
        ));
        for m in assign.captures_iter(text) {
            push_literal(&m[1]);
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

fn collect_sources(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let p = entry?.path();
        if p.is_dir() {
            collect_sources(&p, out)?;
        } else if p
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "p" | "w" | "i" | "cls"))
        {
            out.push(p);
        }
    }
    Ok(())
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
    let mut paths = Vec::new();
    collect_sources(root, &mut paths)?;
    paths.sort();
    let file_count = paths.len();
    let threads = threads
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()))
        .max(1);
    let root: Arc<Path> = root.into();
    let tables = Arc::new(tables.clone());
    let queue = Arc::new(Mutex::new(paths.into_iter()));
    let (tx, rx) = mpsc::channel::<(PathBuf, u64, Result<AblFile, String>)>();
    let workers: Vec<_> = (0..threads)
        .map(|_| {
            let queue = Arc::clone(&queue);
            let tx = tx.clone();
            let root = Arc::clone(&root);
            let tables = Arc::clone(&tables);
            std::thread::spawn(move || loop {
                let next = queue.lock().unwrap_or_else(|e| e.into_inner()).next();
                let Some(p) = next else { break };
                let result = std::fs::read(&p).map_err(|e| e.to_string()).map(|bytes| {
                    let program = p
                        .strip_prefix(&*root)
                        .unwrap_or(&p)
                        .to_string_lossy()
                        .replace('\\', "/")
                        .to_lowercase();
                    (
                        bytes.len() as u64,
                        scan_abl(&decode_to_utf8(&bytes).text, &program, &tables),
                    )
                });
                let (size, parsed) = match result {
                    Ok((size, f)) => (size, Ok(f)),
                    Err(e) => (0, Err(e)),
                };
                if tx.send((p, size, parsed)).is_err() {
                    break;
                }
            })
        })
        .collect();
    drop(tx);
    let mut out = ScannedDir {
        file_count,
        ..Default::default()
    };
    for (p, size, parsed) in rx {
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
    for w in workers {
        let _ = w.join();
    }
    out.files.sort_by(|a, b| a.program.cmp(&b.program));
    Ok(out)
}

fn text(s: &str) -> PropertyValue {
    PropertyValue::Text(s.to_string())
}

/// Turns scanned sources into records: `Screen` sheets linked to their
/// `Program`, and `joined_on` links between `Table` sheets carrying the
/// key list. Run after the xref import: `Program` and `Table` must exist.
pub struct AblSource {
    records: Box<dyn Iterator<Item = Record> + Send + Sync>,
}

impl AblSource {
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
                description: "A window of the ERP (.w): its title, folder tabs and menu entries.".into(),
                properties: Some(["path", "title", "tabs", "menu_items"].iter().map(|s| s.to_string()).collect()),
                ns: Some(XREF_NS.into()),
                ..Default::default()
            }),
            Record::RelationTypeDecl(RelationType {
                name: "implemented_by".into(),
                domain: "Screen".into(),
                range: "Program".into(),
                cardinality: Cardinality::ManyToOne,
                description: "the program behind the screen".into(),
                ..Default::default()
            }),
            Record::RelationTypeDecl(RelationType {
                name: "joined_on".into(),
                domain: "Table".into(),
                range: "Table".into(),
                cardinality: Cardinality::ManyToMany,
                symmetric: true,
                description: "the code joins the two tables; `keys` lists the field pairs (most used first), `programs` how many programs do it, weight = number of join comparisons".into(),
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

        let screens = files.into_iter().flat_map(move |f| {
            let mut out = Vec::with_capacity(2);
            if f.title.is_some() || !f.tabs.is_empty() || !f.menu_items.is_empty() {
                let mut c = Concept::new(ConceptId(0), "Screen", &f.program)
                    .with_property("path", text(&f.program))
                    .with_property(
                        "tabs",
                        PropertyValue::List(f.tabs.iter().map(|t| text(t)).collect()),
                    )
                    .with_property(
                        "menu_items",
                        PropertyValue::List(f.menu_items.iter().map(|t| text(t)).collect()),
                    );
                if let Some(t) = &f.title {
                    c = c.with_property("title", text(t));
                    c.description = t.clone();
                }
                out.push(Record::Concept(c));
                if programs.contains(&f.program) {
                    out.push(Record::NamedRelation {
                        relation_type: "implemented_by".into(),
                        source_type: "Screen".into(),
                        source_name: f.program.clone(),
                        target_type: "Program".into(),
                        target_name: f.program.clone(),
                        weight: 1.0,
                        properties: Default::default(),
                    });
                }
            }
            out
        });

        let links = joins.into_iter().map(|((a, b), (u, programs))| {
            let mut keys: Vec<(&String, &u32)> = u.keys.iter().collect();
            keys.sort_by(|x, y| y.1.cmp(x.1).then(x.0.cmp(y.0)));
            let mut properties = ahash::AHashMap::default();
            properties.insert(
                "keys".to_string(),
                PropertyValue::List(keys.iter().map(|(k, _)| text(k)).collect()),
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
                "msdb.cali",
                "msdb.ecri",
                "msdb.crli",
                "msdb.util",
                "msec.util",
                "msec.ecri2",
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
        assert_eq!(t.resolve("util"), Some("msdb.util"));
        assert_eq!(t.resolve("msec.util"), Some("msec.util"));
        assert_eq!(t.resolve("nothing"), None);
    }

    #[test]
    fn finds_joins_through_aliases_and_across_lines() {
        let src = r#"
DEFINE BUFFER b-ecri FOR ecri.
DEFINE TEMP-TABLE crli NO-UNDO FIELD nste AS INTEGER.
/* FIND cali WHERE cali.nste = ecri.nste. (commented out) */
FIND cali USE-INDEX calino WHERE cali.nste EQ b-ecri.nste
                           AND   cali.exer EQ b-ecri.exer
                           AND   cali.ecri eq b-ecri.ecri NO-LOCK NO-ERROR.
FOR EACH crli WHERE crli.nste = ecri.nste: END.   /* crli is a temp-table here */
FOR EACH cali OF ecri: END.
ASSIGN ecri.nste = ecri.nste.
"#;
        let f = scan_abl(src, "ca/x.p", &tables());
        assert_eq!(f.joins.len(), 1, "{:?}", f.joins);
        let u = &f.joins[&("msdb.cali".to_string(), "msdb.ecri".to_string())];
        assert_eq!(u.count, 3);
        assert_eq!(u.implicit, 1);
        assert_eq!(u.keys["nste=nste"], 1);
        assert_eq!(u.keys["ecri=ecri"], 1);
        assert!(f.title.is_none(), "a .p has no screen");
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
CREATE WINDOW W-Win ASSIGN TITLE = "ca100 - Comptabilité analytique".
"#;
        let f = scan_abl(src, "ca/w-ca100.w", &tables());
        assert_eq!(f.title.as_deref(), Some("ca100 - Comptabilité analytique"));
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
