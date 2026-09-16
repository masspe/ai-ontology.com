// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Source adapters at their edges: JSONL framing (blank lines, CRLF, BOM,
//! comments, malformed lines), CSV failure modes and encodings, the triple
//! parser, text-directory filtering, `.docx` routing, and the XLSX reader
//! on hand-built workbooks.

mod hardening_common;

use hardening_common::{docx_bytes, tempdir, xlsx_bytes, Cell};
use ontology_graph::{ConceptType, Ontology, OntologyGraph, PropertyValue, RelationType};
use ontology_io::{
    ingest_records, spreadsheet_to_text, CsvSource, IngestError, JsonlSource, Record, Source,
    TextDocumentSource, TripleSource, XlsxSource,
};
use std::path::{Path, PathBuf};

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

// `IngestError` is the crate's own (large) error type; the helper mirrors
// `Source::next` and must return it unboxed for `matches!` on variants.
#[allow(clippy::result_large_err)]
async fn drain<S: Source>(src: &mut S) -> Result<Vec<Record>, IngestError> {
    let mut out = Vec::new();
    while let Some(r) = src.next().await? {
        out.push(r);
    }
    Ok(out)
}

fn concept_names(recs: &[Record], concept_type: &str) -> Vec<String> {
    recs.iter()
        .filter_map(|r| match r {
            Record::Concept(c) if c.concept_type == concept_type => Some(c.name.clone()),
            _ => None,
        })
        .collect()
}

fn people_ontology() -> Ontology {
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
    o
}

fn concept_line(name: &str) -> String {
    format!(
        r#"{{"kind":"concept","id":0,"concept_type":"Person","name":"{name}","description":"","properties":{{}}}}"#
    )
}

// ---------------------------------------------------------------- JSONL --

/// Framing noise is not data: blank lines, CRLF endings, `#` comments, a
/// trailing newline and a leading UTF-8 BOM are all skipped.
#[tokio::test]
async fn jsonl_tolerates_blank_lines_crlf_comments_trailing_newline_and_a_bom() {
    let dir = tempdir("jsonl-framing");
    let content = format!(
        "\u{FEFF}{}\r\n\r\n# a comment\r\n   \r\n{}\r\n",
        concept_line("Alice"),
        concept_line("Bob")
    );
    let path = write(&dir, "in.jsonl", content.as_bytes());
    let mut src = JsonlSource::open(&path).await.unwrap();
    let recs = drain(&mut src).await.unwrap();
    assert_eq!(concept_names(&recs, "Person"), vec!["Alice", "Bob"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A malformed line fails with its 1-based position in the file, after the
/// good records before it were delivered; an unknown `kind` is malformed.
#[tokio::test]
async fn a_malformed_jsonl_line_is_reported_by_its_line_number() {
    let dir = tempdir("jsonl-malformed");
    let content = format!(
        "{}\n\n# comment\n{{not json\n{}\n",
        concept_line("Alice"),
        concept_line("Bob")
    );
    let path = write(&dir, "bad.jsonl", content.as_bytes());
    let mut src = JsonlSource::open(&path).await.unwrap();
    assert!(matches!(src.next().await, Ok(Some(Record::Concept(c))) if c.name == "Alice"));
    let err = src.next().await.unwrap_err();
    assert!(matches!(err, IngestError::Source(_)), "{err}");
    assert!(err.to_string().contains("jsonl: line 4:"), "{err}");

    let path = write(&dir, "kind.jsonl", br#"{"kind":"widget","name":"x"}"#);
    let mut src = JsonlSource::open(&path).await.unwrap();
    let err = src.next().await.unwrap_err();
    assert!(err.to_string().contains("line 1"), "{err}");
    assert!(err.to_string().contains("widget"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ------------------------------------------------------------------ CSV --

/// The header must carry a `name` column; the error says so.
#[tokio::test]
async fn csv_without_a_name_header_fails_clearly() {
    let dir = tempdir("csv-noname");
    let path = write(&dir, "x.csv", b"id,label\n1,x\n");
    let mut src = CsvSource::open(&path, "Person").await.unwrap();
    let err = src.next().await.unwrap_err();
    assert!(
        err.to_string().contains("expected a `name` column"),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Wrong column counts and empty names are loud errors at the offending
/// row; rows before it are delivered. Record-level quoting (a newline
/// inside quotes) is out of scope and fails the same loud way rather than
/// producing a garbled concept.
#[tokio::test]
async fn csv_rows_with_a_wrong_column_count_or_an_empty_name_are_errors() {
    let dir = tempdir("csv-rows");
    let path = write(&dir, "short.csv", b"name,role\nAlice,boss\nBob\n");
    let mut src = CsvSource::open(&path, "Person").await.unwrap();
    assert!(matches!(src.next().await, Ok(Some(Record::Concept(c))) if c.name == "Alice"));
    let err = src.next().await.unwrap_err();
    assert!(
        err.to_string().contains("row has 1 columns, expected 2"),
        "{err}"
    );

    let path = write(&dir, "empty.csv", b"name,role\n,boss\n");
    let mut src = CsvSource::open(&path, "Person").await.unwrap();
    let err = src.next().await.unwrap_err();
    assert!(err.to_string().contains("empty `name` cell"), "{err}");

    let path = write(&dir, "multiline.csv", b"name,role\n\"Al\nice\",boss\n");
    let mut src = CsvSource::open(&path, "Person").await.unwrap();
    let err = src.next().await.unwrap_err();
    assert!(err.to_string().contains("columns"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Excel-on-Windows output imports as is: Windows-1252 bytes with CRLF, a
/// case-insensitive `Name` header, `Description` mapped to the description,
/// and a UTF-8 BOM in front of the header.
#[tokio::test]
async fn csv_decodes_windows_1252_crlf_and_a_bom_and_matches_headers_case_insensitively() {
    let dir = tempdir("csv-encoding");
    let path = write(
        &dir,
        "win.csv",
        b"Name,Description,city\r\nZo\xEB,caf\xE9 owner,Gen\xE8ve\r\n",
    );
    let mut src = CsvSource::open(&path, "Person").await.unwrap();
    let recs = drain(&mut src).await.unwrap();
    let Record::Concept(c) = &recs[0] else {
        panic!("concept expected");
    };
    assert_eq!(c.name, "Zoë");
    assert_eq!(c.description, "café owner");
    assert_eq!(
        c.properties.get("city"),
        Some(&PropertyValue::Text("Genève".into()))
    );
    assert!(!c.properties.contains_key("Description"));

    let mut bom = vec![0xEF, 0xBB, 0xBF];
    bom.extend_from_slice(b"name,x\r\nA,1\r\n");
    let path = write(&dir, "bom.csv", &bom);
    let mut src = CsvSource::open(&path, "Person").await.unwrap();
    let recs = drain(&mut src).await.unwrap();
    assert_eq!(concept_names(&recs, "Person"), vec!["A"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Blank lines are skipped and empty cells produce no property, so a
/// sparse row does not carry empty-string properties into the graph.
#[tokio::test]
async fn csv_skips_blank_lines_and_keeps_empty_cells_out_of_properties() {
    let dir = tempdir("csv-sparse");
    let path = write(&dir, "sparse.csv", b"name,a,b\n\nx,,2\n\n");
    let mut src = CsvSource::open(&path, "Person").await.unwrap();
    let recs = drain(&mut src).await.unwrap();
    assert_eq!(recs.len(), 1);
    let Record::Concept(c) = &recs[0] else {
        panic!("concept expected");
    };
    assert!(!c.properties.contains_key("a"));
    assert_eq!(
        c.properties.get("b"),
        Some(&PropertyValue::Text("2".into()))
    );
    assert_eq!(c.description, "");
    let _ = std::fs::remove_dir_all(&dir);
}

// -------------------------------------------------------------- Triples --

/// Comments (full-line and inline) and blank lines are skipped, each
/// endpoint concept is emitted once and before the relations that need it,
/// and the whole thing ingests.
#[tokio::test]
async fn triples_skip_comments_and_blank_lines_and_emit_each_concept_once() {
    let dir = tempdir("triples-ok");
    let path = write(
        &dir,
        "t.txt",
        b"# people\nPerson:Alice knows Person:Bob   # inline comment\n   \n\nPerson:Bob knows Person:Carol\nPerson:Alice knows Person:Carol\n",
    );
    let mut src = TripleSource::open(&path).await.unwrap();
    let recs = drain(&mut src).await.unwrap();
    assert_eq!(
        concept_names(&recs, "Person"),
        vec!["Alice", "Bob", "Carol"],
        "each concept once, in order of first appearance"
    );
    let relations = recs
        .iter()
        .filter(|r| matches!(r, Record::NamedRelation { .. }))
        .count();
    assert_eq!(relations, 3);
    // Every relation comes after both of its endpoints.
    let pos = |name: &str| {
        recs.iter()
            .position(|r| matches!(r, Record::Concept(c) if c.name == name))
            .unwrap()
    };
    for (i, r) in recs.iter().enumerate() {
        if let Record::NamedRelation {
            source_name,
            target_name,
            ..
        } = r
        {
            assert!(pos(source_name) < i && pos(target_name) < i);
        }
    }

    let graph = OntologyGraph::with_arc(people_ontology());
    let mut src = TripleSource::open(&path).await.unwrap();
    let stats = ingest_records(&mut src, &graph, None).await.unwrap();
    assert_eq!((stats.concepts, stats.relations), (3, 3));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A line without three tokens, or an endpoint without `Type:Name`, is an
/// error quoting the offending input.
#[tokio::test]
async fn malformed_triple_lines_are_errors_naming_the_offending_input() {
    let dir = tempdir("triples-bad");
    let path = write(&dir, "two.txt", b"Person:Alice knows\n");
    let mut src = TripleSource::open(&path).await.unwrap();
    let err = src.next().await.unwrap_err();
    assert!(
        err.to_string().contains("expected `Subj predicate Obj`"),
        "{err}"
    );
    assert!(err.to_string().contains("Person:Alice knows"), "{err}");

    let path = write(&dir, "endpoint.txt", b"Alice knows Person:Bob\n");
    let mut src = TripleSource::open(&path).await.unwrap();
    let err = src.next().await.unwrap_err();
    assert!(
        err.to_string().contains("malformed endpoint `Alice`"),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ------------------------------------------------------- Text documents --

/// `from_dir` keeps regular files whose extension matches (case
/// insensitively), skips the rest and sub-directories, names each document
/// by its file stem and types it with the configured `--text-type`.
#[tokio::test]
async fn text_dir_ingestion_filters_by_extension_and_types_every_document() {
    let dir = tempdir("text-dir");
    write(&dir, "a.txt", b"Alpha body");
    write(&dir, "B.MD", b"Beta body");
    write(&dir, "c.csv", b"name\nskip");
    write(&dir, "noext", b"skip too");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    write(&dir.join("sub"), "d.txt", b"nested, skipped");

    let mut src = TextDocumentSource::from_dir("Memo", &dir, &["txt", "md"])
        .await
        .unwrap();
    let recs = drain(&mut src).await.unwrap();
    let mut names = concept_names(&recs, "Memo");
    names.sort();
    assert_eq!(names, vec!["B", "a"]);
    assert!(recs
        .iter()
        .all(|r| !matches!(r, Record::Concept(c) if c.concept_type != "Memo")));
    assert!(recs
        .iter()
        .any(|r| matches!(r, Record::ConceptTypeDecl(ct) if ct.name == "Memo")));
    let alpha = recs
        .iter()
        .find_map(|r| match r {
            Record::Concept(c) if c.name == "a" => Some(c),
            _ => None,
        })
        .unwrap();
    assert_eq!(alpha.description, "Alpha body");

    let graph = OntologyGraph::with_arc(Ontology::new());
    let mut src = TextDocumentSource::from_dir("Memo", &dir, &["txt", "md"])
        .await
        .unwrap();
    let stats = ingest_records(&mut src, &graph, None).await.unwrap();
    assert_eq!(stats.concepts, 2);
    assert_eq!(graph.concepts_of_type("Memo", false).len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A `.docx` goes through the Word extractor (paragraphs joined by
/// newlines); a `.docx` that is not a ZIP is an error naming the file.
#[tokio::test]
async fn text_source_reads_docx_files_and_rejects_a_docx_that_is_not_a_zip() {
    let dir = tempdir("text-docx");
    let good = write(
        &dir,
        "memo.docx",
        &docx_bytes("<w:p><w:r><w:t>Hello</w:t></w:r></w:p><w:p><w:r><w:t>World</w:t></w:r></w:p>"),
    );
    let mut src = TextDocumentSource::from_files("Memo", [&good]);
    let recs = drain(&mut src).await.unwrap();
    let memo = recs
        .iter()
        .find_map(|r| match r {
            Record::Concept(c) if c.name == "memo" => Some(c),
            _ => None,
        })
        .unwrap();
    assert_eq!(memo.description, "Hello\nWorld");

    let bad = write(&dir, "bad.docx", b"this is not a zip archive");
    let mut src = TextDocumentSource::from_files("Memo", [&bad]);
    let err = src.next().await.unwrap_err();
    assert!(matches!(err, IngestError::Source(_)), "{err}");
    assert!(err.to_string().contains("docx extract"), "{err}");
    assert!(err.to_string().contains("bad.docx"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `with_chunk_chars` drives fragmentation per file; 0 keeps documents
/// whole.
#[tokio::test]
async fn text_source_chunk_chars_controls_fragmentation() {
    let dir = tempdir("text-chunk");
    let body = (0..5)
        .map(|i| format!("{i}{}", "w".repeat(69)))
        .collect::<Vec<_>>()
        .join("\n\n");
    let path = write(&dir, "big.txt", body.as_bytes());

    let mut src = TextDocumentSource::from_files("Memo", [&path]).with_chunk_chars(100);
    let recs = drain(&mut src).await.unwrap();
    assert!(recs.iter().any(
        |r| matches!(r, Record::FragmentTypeDecl { document_type } if document_type == "Memo")
    ));
    assert_eq!(concept_names(&recs, "MemoFragment").len(), 5);

    let mut src = TextDocumentSource::from_files("Memo", [&path]).with_chunk_chars(0);
    let recs = drain(&mut src).await.unwrap();
    assert!(concept_names(&recs, "MemoFragment").is_empty());
    assert_eq!(recs.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

// ----------------------------------------------------------------- XLSX --

/// Rows without a name (or entirely empty) are skipped; numeric cells
/// become `Number`, booleans `Bool`, `description` maps to the
/// description, and a blank header column is ignored.
#[test]
fn xlsx_rows_without_a_name_are_skipped_and_cells_are_typed() {
    let dir = tempdir("xlsx-rows");
    let bytes = xlsx_bytes(&[(
        "Items",
        vec![
            vec![
                Cell::Str("name"),
                Cell::Str("amount"),
                Cell::Str("active"),
                Cell::Str("Description"),
                Cell::Empty,
            ],
            vec![
                Cell::Str("A"),
                Cell::Num(12.5),
                Cell::Bool(true),
                Cell::Str("first"),
                Cell::Str("under a blank header"),
            ],
            vec![
                Cell::Empty,
                Cell::Num(3.0),
                Cell::Bool(false),
                Cell::Str("no name"),
                Cell::Empty,
            ],
            vec![
                Cell::Empty,
                Cell::Empty,
                Cell::Empty,
                Cell::Empty,
                Cell::Empty,
            ],
            vec![
                Cell::Str("B"),
                Cell::Empty,
                Cell::Empty,
                Cell::Empty,
                Cell::Empty,
            ],
        ],
    )]);
    let path = write(&dir, "items.xlsx", &bytes);
    let mut src = XlsxSource::open(&path, "Item").unwrap();
    let recs = futures::executor::block_on(drain(&mut src)).unwrap();
    assert_eq!(concept_names(&recs, "Item"), vec!["A", "B"]);
    let Record::Concept(a) = &recs[0] else {
        panic!("concept expected");
    };
    assert_eq!(
        a.properties.get("amount"),
        Some(&PropertyValue::Number(12.5))
    );
    assert_eq!(a.properties.get("active"), Some(&PropertyValue::Bool(true)));
    assert_eq!(a.description, "first");
    assert_eq!(a.properties.len(), 2, "{:?}", a.properties);
    let Record::Concept(b) = &recs[1] else {
        panic!("concept expected");
    };
    assert!(b.properties.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// An empty first sheet and a header without `name` are errors, not empty
/// imports.
#[test]
fn xlsx_empty_sheet_and_missing_name_header_are_errors() {
    let dir = tempdir("xlsx-errors");
    let path = write(&dir, "empty.xlsx", &xlsx_bytes(&[("Empty", vec![])]));
    let Err(err) = XlsxSource::open(&path, "Item") else {
        panic!("an empty sheet must be an error");
    };
    assert!(err.to_string().contains("empty sheet"), "{err}");

    let path = write(
        &dir,
        "noname.xlsx",
        &xlsx_bytes(&[(
            "S",
            vec![
                vec![Cell::Str("id"), Cell::Str("x")],
                vec![Cell::Num(1.0), Cell::Str("y")],
            ],
        )]),
    );
    let Err(err) = XlsxSource::open(&path, "Item") else {
        panic!("a header without `name` must be an error");
    };
    assert!(
        err.to_string().contains("expected a `name` header column"),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `spreadsheet_to_text` walks every sheet in order, skips sheets without
/// rows, names blank headers by position and renders numbers plainly.
#[test]
fn spreadsheet_to_text_flattens_every_sheet_and_names_blank_headers_by_position() {
    let dir = tempdir("xlsx-text");
    let bytes = xlsx_bytes(&[
        (
            "Clients",
            vec![
                vec![Cell::Str("name"), Cell::Empty, Cell::Str("active")],
                vec![Cell::Str("Acme"), Cell::Num(42.0), Cell::Bool(true)],
                vec![Cell::Empty, Cell::Empty, Cell::Empty],
            ],
        ),
        ("Empty", vec![]),
        (
            "Notes",
            vec![vec![Cell::Str("k")], vec![Cell::Str("v & w")]],
        ),
    ]);
    let path = write(&dir, "multi.xlsx", &bytes);
    let text = spreadsheet_to_text(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines,
        vec![
            "# Sheet: Clients",
            "name: Acme; col2: 42; active: true",
            "",
            "# Sheet: Notes",
            "k: v & w",
        ],
        "{text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
