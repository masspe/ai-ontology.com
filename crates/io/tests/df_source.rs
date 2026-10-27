// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! The schema dump on top of the cross-reference: labels become the
//! sheets' descriptions, keys land on the tables, a table the code never
//! touches still exists, a trigger links to its program when the program
//! is there — and a later cross-reference re-import keeps all of it.

mod hardening_common;

use hardening_common::{xref_fixtures_dir, xref_roots, UPSERT};
use ontology_graph::{Ontology, OntologyGraph, PropertyValue};
use ontology_io::{ingest_records_with, DfSource, IngestOptions, XrefSource};
use ontology_storage::FlakyStore;
use std::collections::BTreeSet;
use std::path::PathBuf;

fn df_fixture() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/df/appdb.df"
    ))
}

const MERGE: IngestOptions = IngestOptions {
    merge_properties: true,
    ..UPSERT
};

async fn graph_with_xref() -> (std::sync::Arc<OntologyGraph>, FlakyStore) {
    let graph = OntologyGraph::with_arc(Ontology::new());
    let store = FlakyStore::new();
    let (mut src, _) = XrefSource::from_path(&xref_fixtures_dir(), &xref_roots(), Some(2)).unwrap();
    ingest_records_with(&mut src, &graph, Some(&store), MERGE)
        .await
        .unwrap();
    (graph, store)
}

fn text(c: &ontology_graph::Concept, key: &str) -> String {
    match c.properties.get(key) {
        Some(PropertyValue::Text(t)) => t.clone(),
        other => panic!("{key}: {other:?}"),
    }
}

#[tokio::test]
async fn the_dump_describes_the_tables_and_fields_the_code_touches() {
    let (graph, store) = graph_with_xref().await;
    let payvar = graph
        .find_by_name("Table", "appdb.payvar")
        .expect("from the xref");
    assert_eq!(graph.get_concept(payvar).unwrap().description, "");
    let cono = graph
        .find_by_name("Field", "appdb.payvar.cono")
        .expect("from the xref");

    let (mut src, tables) = DfSource::from_graph(&df_fixture(), "appdb", &graph).unwrap();
    assert_eq!(tables, 2);
    let stats = ingest_records_with(&mut src, &graph, Some(&store), MERGE)
        .await
        .unwrap();
    assert!(stats.updated >= 2, "{stats:?}");

    let t = graph.get_concept(payvar).unwrap();
    assert_eq!(
        t.description, "Variables de salaire",
        "label → description, French retrieval"
    );
    assert_eq!(text(&t, "label"), "Variables de salaire");
    assert_eq!(
        text(&t, "db"),
        "appdb",
        "the cross-reference's properties survive the merge"
    );
    assert_eq!(
        t.properties["primary_key"],
        PropertyValue::list(["cono", "period"])
    );
    assert_eq!(
        t.properties["unique_keys"],
        PropertyValue::list(["cono,period"])
    );

    let f = graph.get_concept(cono).unwrap();
    assert_eq!(f.description, "Numéro de société");
    assert_eq!(text(&f, "data_type"), "integer");
    assert_eq!(text(&f, "column_label"), "Sté");
    assert_eq!(
        text(&f, "table"),
        "appdb.payvar",
        "kept from the cross-reference"
    );
    let libelle = graph
        .find_by_name("Field", "appdb.payvar.libelle")
        .expect("new field");
    let l = graph.get_concept(libelle).unwrap();
    assert_eq!(l.description, "Libellé — Texte libre affiché sur la fiche");
    assert_eq!(text(&l, "help"), "Saisir un libellé");
    assert!(graph
        .relation_between(payvar, libelle, "has_field")
        .is_some());
}

#[tokio::test]
async fn a_table_the_code_never_touches_still_exists_and_the_trigger_links_when_its_program_does() {
    let (graph, store) = graph_with_xref().await;
    // A program for the trigger, as the cross-reference would have created it.
    let mut pre = DfSource::from_tables("appdb", vec![], BTreeSet::new());
    ingest_records_with(&mut pre, &graph, Some(&store), MERGE)
        .await
        .unwrap();
    let before = graph.concept_count();

    let (mut src, _) = DfSource::from_graph(&df_fixture(), "appdb", &graph).unwrap();
    ingest_records_with(&mut src, &graph, Some(&store), MERGE)
        .await
        .unwrap();
    let nouvelle = graph
        .find_by_name("Table", "appdb.nouvelle")
        .expect("schema-only table");
    assert_eq!(
        graph.get_concept(nouvelle).unwrap().description,
        "Table que le code ne touche pas"
    );
    assert!(graph.find_by_name("Field", "appdb.nouvelle.id").is_some());
    assert!(graph.concept_count() > before);
    // No `trig/tw-payvar.p` program in the fixtures: no dangling trigger link,
    // and no error either.
    let payvar = graph.find_by_name("Table", "appdb.payvar").unwrap();
    assert!(graph
        .incoming_typed(payvar, &["fires_on".into()])
        .is_empty());

    // With the program present, the trigger is linked.
    let graph2 = OntologyGraph::with_arc(Ontology::new());
    let store2 = FlakyStore::new();
    let programs: BTreeSet<String> = ["trig/tw-payvar.p".to_string()].into_iter().collect();
    let mut decl = XrefSource::from_files(vec![ontology_io::XrefFile {
        program: "trig/tw-payvar.p".into(),
        kind: Some(ontology_io::ProgramKind::Procedure),
        ..Default::default()
    }]);
    ingest_records_with(&mut decl, &graph2, Some(&store2), MERGE)
        .await
        .unwrap();
    let tables = ontology_io::parse_df_file(&df_fixture()).unwrap();
    let mut src = DfSource::from_tables("appdb", tables, programs);
    ingest_records_with(&mut src, &graph2, Some(&store2), MERGE)
        .await
        .unwrap();
    let p = graph2.find_by_name("Program", "trig/tw-payvar.p").unwrap();
    let t = graph2.find_by_name("Table", "appdb.payvar").unwrap();
    let rid = graph2
        .relation_between(p, t, "fires_on")
        .expect("trigger link");
    assert_eq!(
        graph2.get_relation(rid).unwrap().properties["event"],
        PropertyValue::Text("WRITE".into())
    );
}

#[tokio::test]
async fn a_cross_reference_reimport_after_the_dump_keeps_the_labels() {
    let (graph, store) = graph_with_xref().await;
    let (mut src, _) = DfSource::from_graph(&df_fixture(), "appdb", &graph).unwrap();
    ingest_records_with(&mut src, &graph, Some(&store), MERGE)
        .await
        .unwrap();
    let payvar = graph.find_by_name("Table", "appdb.payvar").unwrap();

    let (mut again, _) =
        XrefSource::from_path(&xref_fixtures_dir(), &xref_roots(), Some(2)).unwrap();
    let stats = ingest_records_with(&mut again, &graph, Some(&store), MERGE)
        .await
        .unwrap();
    let t = graph.get_concept(payvar).unwrap();
    assert_eq!(t.description, "Variables de salaire", "{stats:?}");
    assert_eq!(text(&t, "label"), "Variables de salaire");
    assert_eq!(
        t.properties["primary_key"],
        PropertyValue::list(["cono", "period"])
    );

    // And the dump itself re-imports as a no-op.
    let written = store.records_written();
    let (mut src, _) = DfSource::from_graph(&df_fixture(), "appdb", &graph).unwrap();
    let stats = ingest_records_with(&mut src, &graph, Some(&store), MERGE)
        .await
        .unwrap();
    assert_eq!(
        (stats.concepts, stats.relations, stats.updated),
        (0, 0, 0),
        "{stats:?}"
    );
    assert_eq!(store.records_written(), written);
}

/// Without merge an upsert replaces: that is still what a CSV loaded
/// twice should do.
#[tokio::test]
async fn without_merge_an_upsert_replaces_the_properties() {
    let (graph, store) = graph_with_xref().await;
    let (mut src, _) = DfSource::from_graph(&df_fixture(), "appdb", &graph).unwrap();
    ingest_records_with(&mut src, &graph, Some(&store), MERGE)
        .await
        .unwrap();
    let (mut again, _) =
        XrefSource::from_path(&xref_fixtures_dir(), &xref_roots(), Some(2)).unwrap();
    ingest_records_with(&mut again, &graph, Some(&store), UPSERT)
        .await
        .unwrap();
    let payvar = graph.find_by_name("Table", "appdb.payvar").unwrap();
    let t = graph.get_concept(payvar).unwrap();
    assert!(!t.properties.contains_key("label"), "replaced, not merged");
    assert_eq!(t.description, "");
}

/// A cross-reference sync after the dump: the tables and fields no
/// program touches are the dump's, so the sync keeps them when told the
/// schema types are shared — and would take them away otherwise.
#[tokio::test]
async fn a_cross_reference_sync_keeps_the_dumps_fields_when_the_types_are_shared() {
    let keep = IngestOptions {
        prune: true,
        prune_max_fraction: 1.0,
        prune_keep: &[
            "Database",
            "Table",
            "Field",
            "has_field",
            "in_database",
            "fires_on",
        ],
        ..MERGE
    };
    let (graph, store) = graph_with_xref().await;
    let (mut src, _) = DfSource::from_graph(&df_fixture(), "appdb", &graph).unwrap();
    ingest_records_with(&mut src, &graph, Some(&store), MERGE)
        .await
        .unwrap();
    let nouvelle = graph.find_by_name("Table", "appdb.nouvelle").unwrap();
    let id_field = graph.find_by_name("Field", "appdb.nouvelle.id").unwrap();
    let (sheets, links) = (graph.concept_count(), graph.relation_count());

    let (mut again, _) =
        XrefSource::from_path(&xref_fixtures_dir(), &xref_roots(), Some(2)).unwrap();
    let stats = ingest_records_with(&mut again, &graph, Some(&store), keep)
        .await
        .unwrap();
    assert_eq!(
        (stats.pruned_concepts, stats.pruned_relations),
        (0, 0),
        "{stats:?}"
    );
    assert!(graph.get_concept(nouvelle).is_ok() && graph.get_concept(id_field).is_ok());
    assert_eq!(
        (graph.concept_count(), graph.relation_count()),
        (sheets, links)
    );

    // Without the keep list the same sync removes what the dump added.
    let blunt = IngestOptions {
        prune_keep: &[],
        ..keep
    };
    let (mut again, _) =
        XrefSource::from_path(&xref_fixtures_dir(), &xref_roots(), Some(2)).unwrap();
    let stats = ingest_records_with(&mut again, &graph, Some(&store), blunt)
        .await
        .unwrap();
    assert!(stats.pruned_concepts > 0, "{stats:?}");
    assert!(graph.get_concept(nouvelle).is_err());
}
