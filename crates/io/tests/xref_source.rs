// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Integration tests for [`XrefSource`]: importing a directory of
//! `.xref.xml` fixtures end to end through `ingest_records`, and checking
//! the shape of the resulting graph (types, sheets, links, weights).

mod hardening_common;

use hardening_common::{xref_fixtures_dir as fixtures_dir, xref_roots as roots};
use ontology_graph::{Ontology, OntologyGraph, PropertyValue};
use ontology_io::{ingest_records, ProgramKind, XrefFile, XrefSource};
use ontology_storage::FlakyStore;

#[tokio::test]
async fn imports_the_fixture_directory_into_an_empty_graph() {
    let (mut source, parsed) = XrefSource::from_path(&fixtures_dir(), &roots(), Some(2)).unwrap();

    assert_eq!(parsed.errors.len(), 2, "errors: {:?}", parsed.errors);
    let mut error_names: Vec<String> = parsed
        .errors
        .iter()
        .map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    error_names.sort();
    assert_eq!(error_names, ["empty.xref.xml", "truncated.xref.xml"]);

    let graph = OntologyGraph::with_arc(Ontology::new());
    let store = FlakyStore::new();
    ingest_records(&mut source, &graph, Some(&store))
        .await
        .unwrap();

    let ontology = graph.ontology();
    let xref_concept_types = [
        "Module", "Program", "Include", "Class", "Database", "Table", "Field",
    ];
    assert_eq!(ontology.concept_types.len(), xref_concept_types.len());
    for ty in xref_concept_types {
        let ct = ontology
            .concept_types
            .get(ty)
            .unwrap_or_else(|| panic!("missing concept type {ty}"));
        assert_eq!(ct.ns.as_deref(), Some("erp"), "ns of {ty}");
    }
    assert_eq!(ontology.relation_types.len(), 12);
    assert_eq!(ontology.rule_types.len(), 3);

    assert!(graph.find_by_name("Table", "msdb.sdva").is_some());
    assert!(graph.find_by_name("Table", "msdb.planning").is_some());
    assert!(graph.find_by_name("Field", "msdb.sdva.nste").is_some());
    assert!(graph
        .find_by_name("Program", "net/sd/gui/planning.cls")
        .is_some());
}

#[tokio::test]
async fn a_run_target_without_a_compiled_xref_becomes_a_missing_program() {
    let (mut source, parsed) = XrefSource::from_path(
        &fixtures_dir().join("adm_window.xref.xml"),
        &roots(),
        Some(1),
    )
    .unwrap();
    assert!(parsed.errors.is_empty(), "errors: {:?}", parsed.errors);

    let graph = OntologyGraph::with_arc(Ontology::new());
    let store = FlakyStore::new();
    ingest_records(&mut source, &graph, Some(&store))
        .await
        .unwrap();

    let window_id = graph
        .find_by_name("Program", "gd/w-items.w")
        .expect("the window program exists");
    let missing_id = graph
        .find_by_name("Program", "gd/d-versions.w")
        .expect("the uncompiled RUN target becomes a missing program");

    let window = graph.get_concept(window_id).unwrap();
    assert_eq!(
        window.properties.get("missing"),
        Some(&PropertyValue::Bool(false))
    );

    let missing = graph.get_concept(missing_id).unwrap();
    assert_eq!(
        missing.properties.get("missing"),
        Some(&PropertyValue::Bool(true))
    );

    let runs: Vec<_> = graph
        .outgoing_typed(window_id, &["runs".to_string()])
        .into_iter()
        .filter(|r| r.target == missing_id)
        .collect();
    assert_eq!(
        runs.len(),
        1,
        "exactly one `runs` edge to the missing program"
    );
}

#[tokio::test]
async fn link_weight_is_the_reference_count() {
    let (mut source, parsed) =
        XrefSource::from_path(&fixtures_dir().join("fields.xref.xml"), &roots(), Some(1)).unwrap();
    assert!(parsed.errors.is_empty(), "errors: {:?}", parsed.errors);

    let graph = OntologyGraph::with_arc(Ontology::new());
    let store = FlakyStore::new();
    ingest_records(&mut source, &graph, Some(&store))
        .await
        .unwrap();

    let program_id = graph
        .find_by_name("Program", "sd/main.p")
        .expect("the program sheet exists");
    let table_id = graph
        .find_by_name("Table", "msdb.sdva")
        .expect("the table sheet exists");

    let reads: Vec<_> = graph
        .outgoing_typed(program_id, &["reads".to_string()])
        .into_iter()
        .filter(|r| r.target == table_id)
        .collect();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].weight, 3.0, "2x ACCESS nste + 1x ACCESS peri");

    let updates: Vec<_> = graph
        .outgoing_typed(program_id, &["updates".to_string()])
        .into_iter()
        .filter(|r| r.target == table_id)
        .collect();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].weight, 1.0, "1x UPDATE montant");

    for field in ["msdb.sdva.nste", "msdb.sdva.peri"] {
        let field_id = graph
            .find_by_name("Field", field)
            .unwrap_or_else(|| panic!("missing field {field}"));
        let edges: Vec<_> = graph
            .outgoing_typed(program_id, &["reads_field".to_string()])
            .into_iter()
            .filter(|r| r.target == field_id)
            .collect();
        assert_eq!(edges.len(), 1, "reads_field edge to {field}");
    }

    let montant_id = graph
        .find_by_name("Field", "msdb.sdva.montant")
        .expect("missing field msdb.sdva.montant");
    let edges: Vec<_> = graph
        .outgoing_typed(program_id, &["updates_field".to_string()])
        .into_iter()
        .filter(|r| r.target == montant_id)
        .collect();
    assert_eq!(edges.len(), 1, "updates_field edge to msdb.sdva.montant");
}

#[tokio::test]
async fn class_names_differing_only_by_case_are_one_sheet() {
    let mut a = XrefFile {
        program: "sd/a.p".to_string(),
        kind: Some(ProgramKind::Procedure),
        ..Default::default()
    };
    a.invokes.insert("System.Object".to_string(), 2);

    let mut b = XrefFile {
        program: "sd/b.p".to_string(),
        kind: Some(ProgramKind::Procedure),
        ..Default::default()
    };
    b.invokes.insert("System.object".to_string(), 1);

    let mut source = XrefSource::from_files(vec![a, b]);

    let graph = OntologyGraph::with_arc(Ontology::new());
    let store = FlakyStore::new();
    ingest_records(&mut source, &graph, Some(&store))
        .await
        .unwrap();

    let id_upper = graph
        .find_by_name("Class", "System.Object")
        .expect("System.Object sheet exists");
    let id_lower = graph
        .find_by_name("Class", "System.object")
        .expect("case-insensitive lookup finds the same sheet");
    assert_eq!(id_upper, id_lower, "one sheet for both spellings");

    let (_, classes) = graph.list_concepts_page(Some("Class"), None, 0, 100, true, false);
    assert_eq!(classes.len(), 1, "exactly one Class concept");

    let a_id = graph.find_by_name("Program", "sd/a.p").unwrap();
    let b_id = graph.find_by_name("Program", "sd/b.p").unwrap();

    let a_invokes = graph.outgoing_typed(a_id, &["invokes".to_string()]);
    assert_eq!(a_invokes.len(), 1);
    assert_eq!(a_invokes[0].target, id_upper);
    assert_eq!(a_invokes[0].weight, 2.0);

    let b_invokes = graph.outgoing_typed(b_id, &["invokes".to_string()]);
    assert_eq!(b_invokes.len(), 1);
    assert_eq!(b_invokes[0].target, id_upper);
    assert_eq!(b_invokes[0].weight, 1.0);
}

#[tokio::test]
async fn every_program_belongs_to_exactly_one_module() {
    let (mut source, parsed) = XrefSource::from_path(&fixtures_dir(), &roots(), Some(2)).unwrap();
    assert_eq!(parsed.errors.len(), 2, "errors: {:?}", parsed.errors);

    let graph = OntologyGraph::with_arc(Ontology::new());
    let store = FlakyStore::new();
    ingest_records(&mut source, &graph, Some(&store))
        .await
        .unwrap();

    let (total, programs) = graph.list_concepts_page(Some("Program"), None, 0, 10_000, true, false);
    assert_eq!(total, programs.len(), "the page holds every Program");
    assert!(!programs.is_empty());

    for program in programs {
        let belongs_to = graph.outgoing_typed(program.id, &["belongs_to".to_string()]);
        assert_eq!(
            belongs_to.len(),
            1,
            "program {} has exactly one belongs_to edge",
            program.name
        );
        let module = graph.get_concept(belongs_to[0].target).unwrap();
        let expected = program.name.split_once('/').map_or("root", |(m, _)| m);
        assert_eq!(module.name, expected, "module of {}", program.name);
    }
}
