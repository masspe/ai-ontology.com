// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Progress OpenEdge data-definition dump (`.df`) reader.
//!
//! A `.df` is the schema of one database as the Data Dictionary dumps it:
//! `ADD TABLE "t"`, `ADD FIELD "f" OF "t" AS type`, `ADD INDEX "i" ON "t"`,
//! `ADD SEQUENCE "s"`, each followed by indented attribute lines
//! (`LABEL`, `DESCRIPTION`, `FORMAT`, `INDEX-FIELD`, …). It carries what
//! the code never says: what a table and a field *mean*, in the users'
//! language, and which fields identify a row. The reader is line-oriented
//! and emits records into the same `erp` model as the cross-reference
//! import: the labels become sheet descriptions, so a question in French
//! finds the table; the indexes become the table's keys.

use async_trait::async_trait;
use ontology_graph::{Concept, ConceptId, PropertyValue};
use std::collections::BTreeSet;
use std::path::Path;

use crate::charset::decode_to_utf8;
use crate::ingest::{IngestError, Source};
use crate::record::Record;
use crate::xref::{normalize_path, xref_ontology_records};

/// One table of the dump.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DfTable {
    pub name: String,
    pub description: String,
    pub dump_name: String,
    /// `(event, procedure path)` from `TABLE-TRIGGER "WRITE" … PROCEDURE "trig\x.p"`.
    pub triggers: Vec<(String, String)>,
    pub fields: Vec<DfField>,
    pub indexes: Vec<DfIndex>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DfField {
    pub name: String,
    pub data_type: String,
    pub label: String,
    pub column_label: String,
    pub description: String,
    pub help: String,
    pub format: String,
    pub decimals: Option<u32>,
    pub extent: Option<u32>,
    pub order: Option<u32>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DfIndex {
    pub name: String,
    pub unique: bool,
    pub primary: bool,
    pub fields: Vec<String>,
}

/// The first quoted string in `s`, with the dump's `""` escape resolved.
fn quoted(s: &str) -> Option<String> {
    let start = s.find('"')? + 1;
    let rest = &s[start..];
    let mut out = String::new();
    let mut chars = rest.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            if chars.peek() == Some(&'"') {
                chars.next();
                out.push('"');
            } else {
                return Some(out);
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

/// Every quoted string in `s`, in order.
fn all_quoted(mut s: &str) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(q) = quoted(s) {
        // Skip past this literal: its opening quote, its content with
        // escapes doubled, its closing quote.
        let consumed = s.find('"').unwrap_or(0) + 1 + q.replace('"', "\"\"").len() + 1;
        out.push(q);
        if consumed >= s.len() {
            break;
        }
        s = &s[consumed..];
    }
    out
}

fn number(s: &str) -> Option<u32> {
    s.split_whitespace().next()?.parse().ok()
}

/// Parse a dump. Statements the model has no use for (`ADD SEQUENCE`,
/// areas, CRCs, the trailer) are skipped; nothing is an error except a
/// field or index that names a table the dump has not declared.
pub fn parse_df(text: &str) -> Result<Vec<DfTable>, String> {
    #[derive(PartialEq)]
    enum At {
        Nothing,
        Table(usize),
        Field(usize),
        Index(usize),
    }
    let mut tables: Vec<DfTable> = Vec::new();
    let mut at = At::Nothing;
    let find = |tables: &[DfTable], name: &str, line: usize| -> Result<usize, String> {
        tables
            .iter()
            .position(|t| t.name == name)
            .ok_or_else(|| format!("line {line}: table `{name}` was not declared"))
    };
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        if !line.starts_with(' ') {
            // A statement.
            let upper = line.to_ascii_uppercase();
            at = if upper.starts_with("ADD TABLE ") {
                let name = quoted(line).unwrap_or_default().to_lowercase();
                tables.push(DfTable {
                    name,
                    ..Default::default()
                });
                At::Table(tables.len() - 1)
            } else if upper.starts_with("ADD FIELD ") {
                let names = all_quoted(line);
                let (Some(field), Some(table)) = (names.first(), names.get(1)) else {
                    return Err(format!("line {}: malformed ADD FIELD", i + 1));
                };
                let ti = find(&tables, &table.to_lowercase(), i + 1)?;
                let data_type = line
                    .rsplit_once(" AS ")
                    .map(|(_, t)| t.trim().to_lowercase())
                    .unwrap_or_default();
                tables[ti].fields.push(DfField {
                    name: field.to_lowercase(),
                    data_type,
                    ..Default::default()
                });
                At::Field(ti)
            } else if upper.starts_with("ADD INDEX ") {
                let names = all_quoted(line);
                let (Some(index), Some(table)) = (names.first(), names.get(1)) else {
                    return Err(format!("line {}: malformed ADD INDEX", i + 1));
                };
                let ti = find(&tables, &table.to_lowercase(), i + 1)?;
                tables[ti].indexes.push(DfIndex {
                    name: index.to_lowercase(),
                    ..Default::default()
                });
                At::Index(ti)
            } else {
                At::Nothing
            };
            continue;
        }
        // An attribute of the current statement.
        let attr = line.trim_start();
        let (key, value) = attr.split_once(' ').unwrap_or((attr, ""));
        match at {
            At::Table(ti) => {
                let t = &mut tables[ti];
                match key {
                    "DESCRIPTION" => t.description = quoted(value).unwrap_or_default(),
                    "DUMP-NAME" => t.dump_name = quoted(value).unwrap_or_default(),
                    "TABLE-TRIGGER" => {
                        let q = all_quoted(value);
                        if let (Some(event), Some(proc_)) = (q.first(), q.get(1)) {
                            t.triggers
                                .push((event.to_uppercase(), normalize_path(proc_, &[])));
                        }
                    }
                    _ => {}
                }
            }
            At::Field(ti) => {
                let f = tables[ti].fields.last_mut().expect("a field was pushed");
                match key {
                    "LABEL" => f.label = quoted(value).unwrap_or_default(),
                    "COLUMN-LABEL" => f.column_label = quoted(value).unwrap_or_default(),
                    "DESCRIPTION" => f.description = quoted(value).unwrap_or_default(),
                    "HELP" => f.help = quoted(value).unwrap_or_default(),
                    "FORMAT" => f.format = quoted(value).unwrap_or_default(),
                    "DECIMALS" => f.decimals = number(value),
                    "EXTENT" => f.extent = number(value),
                    "ORDER" => f.order = number(value),
                    _ => {}
                }
            }
            At::Index(ti) => {
                let x = tables[ti].indexes.last_mut().expect("an index was pushed");
                match key {
                    "UNIQUE" => x.unique = true,
                    "PRIMARY" => x.primary = true,
                    "INDEX-FIELD" => {
                        if let Some(f) = quoted(value) {
                            x.fields.push(f.to_lowercase());
                        }
                    }
                    _ => {}
                }
            }
            At::Nothing => {}
        }
    }
    Ok(tables)
}

/// Read a dump from disk (the Data Dictionary writes ISO-8859-15).
pub fn parse_df_file(path: &Path) -> Result<Vec<DfTable>, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    parse_df(&decode_to_utf8(&bytes).text)
}

fn non_empty(s: &str) -> Option<PropertyValue> {
    (!s.is_empty()).then(|| PropertyValue::from(s))
}

/// The dump of one database as records of the `erp` model. Run after the
/// cross-reference import, which the labels enrich; on its own it still
/// describes the schema.
pub struct DfSource {
    records: Box<dyn Iterator<Item = Record> + Send + Sync>,
}

impl DfSource {
    /// `programs`: the `Program` sheets the graph has, so a trigger is
    /// linked to its procedure only when that program exists.
    pub fn from_tables(db: &str, tables: Vec<DfTable>, programs: BTreeSet<String>) -> Self {
        let db = db.to_lowercase();
        let decls = xref_ontology_records();
        let database = Record::Concept(Concept::new(ConceptId(0), "Database", &db));
        let per_table = tables.into_iter().flat_map(move |t| {
            let qualified = format!("{db}.{}", t.name);
            let primary: Vec<String> = t
                .indexes
                .iter()
                .find(|x| x.primary)
                .map(|x| x.fields.clone())
                .unwrap_or_default();
            let unique: Vec<String> = t
                .indexes
                .iter()
                .filter(|x| x.unique)
                .map(|x| x.fields.join(","))
                .collect();
            let mut table = Concept::new(ConceptId(0), "Table", &qualified)
                .with_property("db", PropertyValue::from(&db))
                .with_property("table", PropertyValue::from(&t.name))
                .with_property("primary_key", PropertyValue::list(primary))
                .with_property("unique_keys", PropertyValue::list(unique))
                .with_property("field_count", PropertyValue::Number(t.fields.len() as f64));
            if let Some(v) = non_empty(&t.description) {
                table = table.with_property("label", v);
                table.description = t.description.clone();
            }
            if let Some(v) = non_empty(&t.dump_name) {
                table = table.with_property("dump_name", v);
            }
            let mut out: Vec<Record> = Vec::with_capacity(2 + 2 * t.fields.len());
            out.push(Record::Concept(table));
            out.push(link(
                "in_database",
                "Table",
                &qualified,
                "Database",
                &db,
                1.0,
                None,
            ));
            for (event, proc_) in &t.triggers {
                if programs.contains(proc_) {
                    out.push(link(
                        "fires_on",
                        "Program",
                        proc_,
                        "Table",
                        &qualified,
                        1.0,
                        Some(("event", event.as_str())),
                    ));
                }
            }
            for f in &t.fields {
                let name = format!("{qualified}.{}", f.name);
                let mut c = Concept::new(ConceptId(0), "Field", &name)
                    .with_property("table", PropertyValue::from(&qualified))
                    .with_property("field", PropertyValue::from(&f.name))
                    .with_property("data_type", PropertyValue::from(&f.data_type));
                for (k, v) in [
                    ("label", &f.label),
                    ("column_label", &f.column_label),
                    ("help", &f.help),
                    ("format", &f.format),
                ] {
                    if let Some(v) = non_empty(v) {
                        c = c.with_property(k, v);
                    }
                }
                for (k, v) in [
                    ("decimals", f.decimals),
                    ("extent", f.extent),
                    ("order", f.order),
                ] {
                    if let Some(n) = v {
                        c = c.with_property(k, PropertyValue::Number(f64::from(n)));
                    }
                }
                // What the field means, for the prompt and the index: its
                // label, then the dictionary description when there is one.
                c.description = match (f.label.is_empty(), f.description.is_empty()) {
                    (false, false) => format!("{} — {}", f.label, f.description),
                    (false, true) => f.label.clone(),
                    (true, false) => f.description.clone(),
                    (true, true) => String::new(),
                };
                out.push(Record::Concept(c));
                out.push(link(
                    "has_field",
                    "Table",
                    &qualified,
                    "Field",
                    &name,
                    1.0,
                    None,
                ));
            }
            out
        });
        Self {
            records: Box::new(
                decls
                    .into_iter()
                    .chain(std::iter::once(database))
                    .chain(per_table),
            ),
        }
    }

    pub fn from_file(
        path: &Path,
        db: &str,
        programs: BTreeSet<String>,
    ) -> Result<(Self, usize), IngestError> {
        let tables = parse_df_file(path).map_err(IngestError::Source)?;
        let n = tables.len();
        Ok((Self::from_tables(db, tables, programs), n))
    }

    /// Take the `Program` sheets from the graph for the trigger links.
    pub fn from_graph(
        path: &Path,
        db: &str,
        graph: &ontology_graph::OntologyGraph,
    ) -> Result<(Self, usize), IngestError> {
        let programs = graph
            .concepts_of_type("Program", false)
            .into_iter()
            .map(|c| c.name)
            .collect();
        Self::from_file(path, db, programs)
    }
}

fn link(
    ty: &str,
    st: &str,
    sn: &str,
    tt: &str,
    tn: &str,
    weight: f32,
    property: Option<(&str, &str)>,
) -> Record {
    let mut properties = ahash::AHashMap::default();
    if let Some((k, v)) = property {
        properties.insert(k.to_string(), PropertyValue::from(v));
    }
    Record::NamedRelation {
        relation_type: ty.into(),
        source_type: st.into(),
        source_name: sn.into(),
        target_type: tt.into(),
        target_name: tn.into(),
        weight,
        properties,
    }
}

#[async_trait]
impl Source for DfSource {
    async fn next(&mut self) -> Result<Option<Record>, IngestError> {
        Ok(self.records.next())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = r#"ADD TABLE "emp"
  AREA "DataArea"
  DESCRIPTION "Salariés"
  DUMP-NAME "emp"
  TABLE-TRIGGER "DELETE" OVERRIDE PROCEDURE "trig\td-emp.p" CRC "?"
  TABLE-TRIGGER "WRITE" OVERRIDE PROCEDURE "trig\tw-emp.p" CRC "?"

ADD FIELD "cono" OF "emp" AS integer
  FORMAT ">>>>>>"
  INITIAL "0"
  LABEL "Numéro de société"
  POSITION 2
  MAX-WIDTH 4
  COLUMN-LABEL "Sté"
  ORDER 10

ADD FIELD "shortname" OF "emp" AS character
  DESCRIPTION "Utilisé pour recherche ""alpha"""
  FORMAT "x(50)"
  LABEL "Intitulé court"
  HELP "Saisir le nom"
  DECIMALS 2
  EXTENT 3
  ORDER 20

ADD INDEX "empno" ON "emp"
  AREA "IndexData"
  UNIQUE
  PRIMARY
  INDEX-FIELD "cono" ASCENDING
  INDEX-FIELD "empid" ASCENDING

ADD INDEX "empname" ON "emp"
  AREA "IndexData"
  INDEX-FIELD "shortname" ASCENDING

ADD SEQUENCE "doc_seq"
  INITIAL 0
  INCREMENT 1

.
PSC
cpstream=ISO8859-15
.
0000012345
"#;

    #[test]
    fn parses_tables_fields_indexes_and_triggers() {
        let tables = parse_df(DUMP).unwrap();
        assert_eq!(tables.len(), 1);
        let t = &tables[0];
        assert_eq!(t.name, "emp");
        assert_eq!(t.description, "Salariés");
        assert_eq!(
            t.triggers,
            vec![
                ("DELETE".to_string(), "trig/td-emp.p".to_string()),
                ("WRITE".to_string(), "trig/tw-emp.p".to_string())
            ]
        );
        assert_eq!(t.fields.len(), 2);
        let cono = &t.fields[0];
        assert_eq!(
            (cono.name.as_str(), cono.data_type.as_str()),
            ("cono", "integer")
        );
        assert_eq!(cono.label, "Numéro de société");
        assert_eq!(cono.column_label, "Sté");
        assert_eq!(cono.format, ">>>>>>");
        assert_eq!(cono.order, Some(10));
        let shortname = &t.fields[1];
        assert_eq!(shortname.description, r#"Utilisé pour recherche "alpha""#);
        assert_eq!(shortname.help, "Saisir le nom");
        assert_eq!((shortname.decimals, shortname.extent), (Some(2), Some(3)));
        assert_eq!(t.indexes.len(), 2);
        assert!(t.indexes[0].unique && t.indexes[0].primary);
        assert_eq!(t.indexes[0].fields, ["cono", "empid"]);
        assert!(!t.indexes[1].unique);
    }

    #[test]
    fn a_field_of_an_undeclared_table_is_an_error() {
        let err = parse_df("ADD FIELD \"x\" OF \"ghost\" AS integer\n").unwrap_err();
        assert!(err.contains("ghost"), "{err}");
        assert!(parse_df("").unwrap().is_empty());
    }

    #[test]
    fn quoted_strings_resolve_doubled_quotes() {
        assert_eq!(
            quoted(r#"LABEL "a ""b"" c""#).as_deref(),
            Some(r#"a "b" c"#)
        );
        assert_eq!(all_quoted(r#""x" OF "y" AS z"#), ["x", "y"]);
        assert_eq!(quoted("none"), None);
    }
}
