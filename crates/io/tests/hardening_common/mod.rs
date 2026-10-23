// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Helpers shared by the `hardening_*` integration tests: an in-memory
//! `Source`, unique Windows-safe temp dirs, and minimal ZIP / DOCX / XLSX
//! builders so the archive-based parsers can be exercised without binary
//! fixtures checked into the repository.

#![allow(dead_code)]

use async_trait::async_trait;
use ontology_graph::{
    Cardinality, Concept, ConceptId, ConceptType, Ontology, PropertyValue, RelationType,
};
use ontology_io::{IngestError, IngestOptions, Record, Source};
use ontology_storage::{FlakyStore, RecordKind};
use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct VecSource(pub VecDeque<Record>);

impl VecSource {
    pub fn new(records: Vec<Record>) -> Self {
        Self(records.into_iter().collect())
    }
}

#[async_trait]
impl Source for VecSource {
    async fn next(&mut self) -> Result<Option<Record>, IngestError> {
        Ok(self.0.pop_front())
    }
}

static TEMP_SEQ: AtomicUsize = AtomicUsize::new(0);

/// A fresh, empty directory unique to this process, thread and call. The
/// caller removes it (best effort) at the end of the test.
pub fn tempdir(label: &str) -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!(
        "ontology-io-{label}-{}-{seq}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// Build a ZIP archive in memory from `(entry name, bytes)` pairs, stored
/// uncompressed (the reader side does not care).
pub fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let cursor = std::io::Cursor::new(Vec::new());
    let mut w = zip::ZipWriter::new(cursor);
    let opts =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, bytes) in entries {
        w.start_file(*name, opts).unwrap();
        w.write_all(bytes).unwrap();
    }
    w.finish().unwrap().into_inner()
}

/// A minimal `.docx`: the given `word/document.xml` body wrapped in the
/// `w:document/w:body` envelope, plus the content-types part.
pub fn docx_bytes(body_xml: &str) -> Vec<u8> {
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body_xml}</w:body></w:document>"#
    );
    let content_types = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
    zip_bytes(&[
        ("[Content_Types].xml", content_types.as_bytes()),
        ("word/document.xml", document.as_bytes()),
    ])
}

/// One spreadsheet cell for [`xlsx_bytes`].
#[derive(Clone, Debug)]
pub enum Cell {
    Str(&'static str),
    Num(f64),
    Bool(bool),
    Empty,
}

fn column_letters(mut idx: usize) -> String {
    let mut s = String::new();
    loop {
        s.insert(0, (b'A' + (idx % 26) as u8) as char);
        if idx < 26 {
            break;
        }
        idx = idx / 26 - 1;
    }
    s
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// A minimal `.xlsx` workbook: one worksheet per `(name, rows)` entry,
/// strings inline (no shared-string table), numbers and booleans typed.
/// An empty `rows` produces a sheet with no cells at all.
pub fn xlsx_bytes(sheets: &[(&str, Vec<Vec<Cell>>)]) -> Vec<u8> {
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();

    let mut content_types = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>"#,
    );
    let mut workbook = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>"#,
    );
    let mut rels = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );

    for (i, (name, rows)) in sheets.iter().enumerate() {
        let n = i + 1;
        content_types.push_str(&format!(
            r#"<Override PartName="/xl/worksheets/sheet{n}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#
        ));
        workbook.push_str(&format!(
            r#"<sheet name="{}" sheetId="{n}" r:id="rId{n}"/>"#,
            xml_escape(name)
        ));
        rels.push_str(&format!(
            r#"<Relationship Id="rId{n}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{n}.xml"/>"#
        ));

        let mut sheet = String::from(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
        );
        for (r, row) in rows.iter().enumerate() {
            sheet.push_str(&format!(r#"<row r="{}">"#, r + 1));
            for (c, cell) in row.iter().enumerate() {
                let reference = format!("{}{}", column_letters(c), r + 1);
                match cell {
                    Cell::Empty => {}
                    Cell::Str(s) => sheet.push_str(&format!(
                        r#"<c r="{reference}" t="inlineStr"><is><t>{}</t></is></c>"#,
                        xml_escape(s)
                    )),
                    Cell::Num(v) => {
                        sheet.push_str(&format!(r#"<c r="{reference}"><v>{v}</v></c>"#))
                    }
                    Cell::Bool(b) => sheet.push_str(&format!(
                        r#"<c r="{reference}" t="b"><v>{}</v></c>"#,
                        u8::from(*b)
                    )),
                }
            }
            sheet.push_str("</row>");
        }
        sheet.push_str("</sheetData></worksheet>");
        entries.push((format!("xl/worksheets/sheet{n}.xml"), sheet.into_bytes()));
    }
    content_types.push_str("</Types>");
    workbook.push_str("</sheets></workbook>");
    rels.push_str("</Relationships>");

    let root_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;

    entries.push(("[Content_Types].xml".into(), content_types.into_bytes()));
    entries.push(("_rels/.rels".into(), root_rels.as_bytes().to_vec()));
    entries.push(("xl/workbook.xml".into(), workbook.into_bytes()));
    entries.push(("xl/_rels/workbook.xml.rels".into(), rels.into_bytes()));

    let borrowed: Vec<(&str, &[u8])> = entries
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    zip_bytes(&borrowed)
}

/// The checked-in `.xref.xml` fixtures.
pub fn xref_fixtures_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/xref"))
}

/// One `.xref.xml` fixture, by base name (without the extension).
pub fn xref_fixture(name: &str) -> PathBuf {
    xref_fixtures_dir().join(format!("{name}.xref.xml"))
}

/// The application root the xref fixtures were compiled under, as the
/// `roots` argument of [`parse_xref_file`](ontology_io::parse_xref_file) /
/// [`parse_dir`](ontology_io::parse_dir).
pub fn xref_roots() -> Vec<String> {
    vec!["D:\\Vendor\\App\\ERP2".to_string()]
}

/// The checked-in ABL (`.p`/`.w`/`.i`) fixtures.
pub fn abl_fixtures_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/abl"))
}

/// The Person/knows test ontology shared by the ingest/upsert/batching
/// hardening tests: `knows` (plain), `near` (symmetric), `spouse`
/// (symmetric + functional) and `twin` (symmetric + one-to-one). A test that
/// pins the exact shape of the ontology (a relation-type count, a
/// `Record::Ontology` payload) keeps its own local copy instead.
pub fn person_ontology() -> Ontology {
    let mut o = Ontology::new();
    o.add_concept_type(ConceptType {
        name: "Person".into(),
        ..Default::default()
    });
    o.add_relation_type(RelationType {
        name: "knows".into(),
        domain: "Person".into(),
        range: "Person".into(),
        ..Default::default()
    })
    .unwrap();
    o.add_relation_type(RelationType {
        name: "near".into(),
        domain: "Person".into(),
        range: "Person".into(),
        symmetric: true,
        ..Default::default()
    })
    .unwrap();
    o.add_relation_type(RelationType {
        name: "spouse".into(),
        domain: "Person".into(),
        range: "Person".into(),
        symmetric: true,
        functional: true,
        ..Default::default()
    })
    .unwrap();
    o.add_relation_type(RelationType {
        name: "twin".into(),
        domain: "Person".into(),
        range: "Person".into(),
        symmetric: true,
        cardinality: Cardinality::OneToOne,
        ..Default::default()
    })
    .unwrap();
    o
}

/// A bare `Person` concept.
pub fn person(name: &str) -> Record {
    Record::Concept(Concept::new(ConceptId(0), "Person", name))
}

/// A `Person` concept with a `city` property.
pub fn person_in(name: &str, city: &str) -> Record {
    Record::Concept(
        Concept::new(ConceptId(0), "Person", name)
            .with_property("city", PropertyValue::Text(city.into())),
    )
}

/// A `NamedRelation` record between two `Person` concepts.
pub fn named_relation(relation_type: &str, source: &str, target: &str, weight: f32) -> Record {
    Record::NamedRelation {
        relation_type: relation_type.into(),
        source_type: "Person".into(),
        source_name: source.into(),
        target_type: "Person".into(),
        target_name: target.into(),
        weight,
        properties: Default::default(),
    }
}

/// `upsert: true`, otherwise default — the option most ingest/upsert tests
/// run under.
pub const UPSERT: IngestOptions = IngestOptions {
    upsert: true,
    prune: false,
    merge_properties: false,
    prune_max_fraction: 0.25,
};

/// The kind label of each record journaled to `store`, in order.
pub fn kinds(store: &FlakyStore) -> Vec<String> {
    store
        .records()
        .into_iter()
        .map(|r| match r.kind {
            RecordKind::Ontology(_) => "ontology",
            RecordKind::Concept(_) => "concept",
            RecordKind::Relation(_) => "relation",
            RecordKind::UpdateRelation(_) => "update_relation",
            RecordKind::DeleteConcept(_) => "delete_concept",
            RecordKind::DeleteRelation(_) => "delete_relation",
            RecordKind::Rule(_) => "rule",
            RecordKind::Action(_) => "action",
            _ => "other",
        })
        .map(str::to_string)
        .collect()
}
