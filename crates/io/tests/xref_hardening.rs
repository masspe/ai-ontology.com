// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! `.xref.xml` reader at its edges: a `.cls` unit with `INVOKE`/`NEW`/
//! `SEARCH` and a temp-table reference, a `.w` unit with ADM framework
//! calls, a program with includes, field-level aggregation with ignored
//! contexts, a truncated file, an empty document, determinism across two
//! parses and the effect of `roots` on the normalised program path.

use ontology_io::{parse_xref_file, ProgramKind};
use std::path::Path;

fn fixture(name: &str) -> &'static Path {
    Path::new(match name {
        "class_planning" => {
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/xref/class_planning.xref.xml"
            )
        }
        "adm_window" => {
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/xref/adm_window.xref.xml"
            )
        }
        "includes" => {
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/xref/includes.xref.xml"
            )
        }
        "fields" => {
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/xref/fields.xref.xml"
            )
        }
        "truncated" => {
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/xref/truncated.xref.xml"
            )
        }
        "empty" => concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/xref/empty.xref.xml"
        ),
        other => panic!("unknown fixture {other}"),
    })
}

/// A `.cls` unit: the `CLASS` reference names the type, `INVOKE`/`NEW` on
/// the same class are summed into one `invokes` entry, a `SEARCH` on a
/// temp-table (`Type:member` identifier) is not a table, and a `SEARCH` on
/// a real `db.table` is.
#[test]
fn cls_unit_aggregates_class_and_skips_temp_table_search() {
    let f = parse_xref_file(fixture("class_planning"), &[]).unwrap();
    assert_eq!(f.kind, Some(ProgramKind::Class));
    assert_eq!(f.class.as_deref(), Some("Net.sd.Gui.Planning"));
    assert_eq!(f.invokes["Net.Class.Config"], 3);
    assert_eq!(
        f.tables.len(),
        1,
        "the temp-table SEARCH must not become a table: {:?}",
        f.tables
    );
    assert!(f.tables.contains_key("msdb.planning"));
}

/// A `.w` unit driven by the ADM: bare `RUN` targets (internal procedures /
/// framework entry points) are lower-cased into `calls`, a `RUN` of a real
/// `.w` file goes to `runs`, and `RUN VALUE(...)` is a dynamic run.
#[test]
fn w_unit_splits_adm_calls_file_runs_and_dynamic_runs() {
    let f = parse_xref_file(fixture("adm_window"), &[]).unwrap();
    assert_eq!(f.kind, Some(ProgramKind::Window));
    assert_eq!(f.calls.len(), 3, "{:?}", f.calls);
    assert!(f.calls.contains_key("dispatch"));
    assert!(f.calls.contains_key("get-attribute"));
    assert!(f.calls.contains_key("get-link-handle"));
    assert!(f.runs.contains_key("gd/d-versions.w"));
    assert_eq!(f.dynamic_runs, 1);
}

/// `program` is set from the first `<Source>` only; the three further
/// `<Source>` elements (the includes themselves) never overwrite it. The
/// matching `INCLUDE` references are normalised regardless of path style.
#[test]
fn program_is_the_first_source_and_includes_are_normalised() {
    let roots = vec!["D:\\Mediasoft\\App\\MS.ERP2".to_string()];
    let f = parse_xref_file(fixture("includes"), &roots).unwrap();
    assert_eq!(f.program, "sd/main.p");
    assert_eq!(
        f.includes,
        [
            "net/web/inc/webconfig.i",
            "net/sd/inc/ds/salselect.i",
            "ba/connect.i",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
        "{:?}",
        f.includes
    );
}

/// Field-level references aggregate per table: repeated `ACCESS` on the
/// same field count once in the set but once per occurrence in `reads`,
/// and a context that is not a `db.table` (a class property, a parameter
/// signature) is ignored rather than creating a bogus table.
#[test]
fn field_aggregation_ignores_non_table_contexts() {
    let f = parse_xref_file(fixture("fields"), &[]).unwrap();
    assert_eq!(f.tables.len(), 1, "{:?}", f.tables);
    let t = &f.tables["msdb.sdva"];
    assert_eq!(t.reads, 3);
    assert_eq!(t.updates, 1);
    assert_eq!(
        t.fields_read,
        ["nste", "peri"].into_iter().map(String::from).collect()
    );
    assert_eq!(
        t.fields_updated,
        ["montant"].into_iter().map(String::from).collect()
    );
}

/// A file cut off mid-`<Reference>` is a clean parse error, not a panic.
#[test]
fn truncated_file_is_an_error_not_a_panic() {
    let err = parse_xref_file(fixture("truncated"), &[]).unwrap_err();
    assert!(err.contains("xml parse"), "{err}");
}

/// A well-formed document with no `<Source>` at all is rejected by name.
#[test]
fn empty_document_is_rejected_for_missing_source() {
    let err = parse_xref_file(fixture("empty"), &[]).unwrap_err();
    assert!(err.contains("no <Source>"), "{err}");
}

/// Parsing the same file twice yields equal summaries: no hidden
/// non-determinism (hash-map iteration order, etc.) leaks into `XrefFile`.
#[test]
fn parsing_is_deterministic() {
    let a = parse_xref_file(fixture("fields"), &[]).unwrap();
    let b = parse_xref_file(fixture("fields"), &[]).unwrap();
    assert_eq!(a, b);
}

/// `roots` strips the configured application root from the program path;
/// without a matching root the full (lower-cased, forward-slashed) path is
/// kept instead.
#[test]
fn roots_control_how_the_program_path_is_normalised() {
    let roots = vec!["D:\\Mediasoft\\App\\MS.ERP2".to_string()];
    let f = parse_xref_file(fixture("class_planning"), &roots).unwrap();
    assert_eq!(f.program, "net/sd/gui/planning.cls");

    let f = parse_xref_file(fixture("class_planning"), &[]).unwrap();
    assert_eq!(
        f.program,
        "d:/mediasoft/app/ms.erp2/net/sd/gui/planning.cls"
    );
}
