// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Documents attached to sheets by key: a help page becomes the screen's
//! description and text, a page about nothing the graph knows becomes a
//! document of its own (or is counted), and a re-import changes nothing.

mod hardening_common;

use hardening_common::{VecSource, UPSERT};
use ontology_graph::{Concept, ConceptId, ConceptType, Ontology, OntologyGraph, PropertyValue};
use ontology_io::{ingest_records_with, DocOptions, DocSource, IngestOptions, Record};
use ontology_storage::FlakyStore;
use regex::Regex;
use std::path::PathBuf;

fn docs_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/docs"))
}

const MERGE: IngestOptions = IngestOptions {
    merge_properties: true,
    ..UPSERT
};

/// A graph with two screens, declared with their own properties only.
async fn screens() -> (std::sync::Arc<OntologyGraph>, FlakyStore) {
    let graph = OntologyGraph::with_arc(Ontology::new());
    let store = FlakyStore::new();
    let mut c1 = Concept::new(ConceptId(0), "Screen", "gl100");
    c1.description = "gl100 - Grand livre".into();
    c1 = c1.with_property("title", PropertyValue::from("gl100 - Grand livre"));
    let c2 = Concept::new(ConceptId(0), "Screen", "GL101");
    let mut src = VecSource(std::collections::VecDeque::from(vec![
        Record::ConceptTypeDecl(ConceptType {
            name: "Screen".into(),
            properties: Some(vec!["title".into()]),
            ..Default::default()
        }),
        Record::Concept(c1),
        Record::Concept(c2),
    ]));
    ingest_records_with(&mut src, &graph, Some(&store), MERGE)
        .await
        .unwrap();
    (graph, store)
}

fn text(c: &Concept, key: &str) -> String {
    match c.properties.get(key) {
        Some(PropertyValue::Text(t)) => t.clone(),
        other => panic!("{key}: {other:?}"),
    }
}

fn opts() -> DocOptions {
    let mut o = DocOptions::new("Screen");
    o.key = Some(Regex::new(r"\(([a-z]{2}\d{3}[a-z0-9]*)\)").unwrap());
    o.property = "help".into();
    o
}

#[tokio::test]
async fn a_help_page_becomes_the_screens_description_and_text() {
    let (graph, store) = screens().await;
    let (mut src, report) = DocSource::from_graph(&docs_dir(), &opts(), &graph).unwrap();
    assert_eq!(
        (report.files, report.attached, report.unmatched),
        (5, 2, 3),
        "{report:?}"
    );
    let stats = ingest_records_with(&mut src, &graph, Some(&store), MERGE)
        .await
        .unwrap();
    assert_eq!(stats.updated, 1, "{stats:?}");

    let id = graph.find_by_name("Screen", "gl100").unwrap();
    let c = graph.get_concept(id).unwrap();
    assert!(
        c.description.starts_with(
            "1 - Paramétrage GL (gl100). Cet écran permet de paramétrer le grand livre"
        ),
        "{}",
        c.description
    );
    assert!(
        !c.description.contains("Guide de l'utilisateur"),
        "the page header is not the summary"
    );
    let help = text(&c, "help");
    assert!(
        help.contains("l'onglet Cumuls affiche les soldes"),
        "{help}"
    );
    assert!(!help.contains("drex_file_name"), "scripts are not text");
    // A second page about the same screen: its text follows, one sheet.
    assert!(help.contains("chaque journal porte une devise"), "{help}");
    assert!(help.find("grand livre").unwrap() < help.find("Journaux").unwrap());
    assert_eq!(
        text(&c, "title"),
        "gl100 - Grand livre",
        "the screen keeps its own properties"
    );
    // The type now declares the property, next to its own.
    let props = graph
        .with_ontology(|o| o.concept_types["Screen"].properties.clone())
        .unwrap_or_default();
    assert!(
        props.contains(&"help".to_string()) && props.contains(&"title".to_string()),
        "{props:?}"
    );
    // No sheet for the chapter page and the text file: nothing invented.
    assert_eq!(graph.concept_count(), 2);
}

#[tokio::test]
async fn without_a_key_the_file_name_is_the_sheet_and_the_rest_can_become_documents() {
    let (graph, store) = screens().await;
    let mut o = DocOptions::new("Screen");
    o.fallback_type = Some("Document".into());
    o.chunk_chars = 0;
    let (mut src, report) = DocSource::from_graph(&docs_dir(), &o, &graph).unwrap();
    assert_eq!((report.attached, report.unmatched), (1, 4), "{report:?}");
    ingest_records_with(&mut src, &graph, Some(&store), MERGE)
        .await
        .unwrap();
    // `gl101.txt` found `GL101` whatever the case; its text is the default property.
    let id = graph.find_by_name("Screen", "gl101").unwrap();
    let c = graph.get_concept(id).unwrap();
    assert!(
        c.description
            .starts_with("Paramétrage des genres de frais."),
        "{}",
        c.description
    );
    assert!(text(&c, "doc").contains("comptabilité analytique"));
    // The two pages about no screen are documents named by their title.
    let cal = graph
        .find_by_name("Document", "2. Création d'un calendrier")
        .expect("chapter page as a document");
    assert!(graph
        .get_concept(cal)
        .unwrap()
        .description
        .contains("jours ouvrés"));
    assert!(graph
        .find_by_name("Document", "1 - Paramétrage GL (gl100)")
        .is_some());
    // The same title in another chapter: a second document, named apart.
    let twin = graph
        .find_by_name(
            "Document",
            "2. Création d'un calendrier — 9__creation_d_un_calendrier",
        )
        .expect("the twin page keeps its own sheet");
    assert!(graph
        .get_concept(twin)
        .unwrap()
        .description
        .contains("livraisons"));
}

#[tokio::test]
async fn a_reimport_changes_nothing() {
    let (graph, store) = screens().await;
    let (mut src, _) = DocSource::from_graph(&docs_dir(), &opts(), &graph).unwrap();
    ingest_records_with(&mut src, &graph, Some(&store), MERGE)
        .await
        .unwrap();
    let written = store.records_written();
    let (mut src, _) = DocSource::from_graph(&docs_dir(), &opts(), &graph).unwrap();
    let stats = ingest_records_with(&mut src, &graph, Some(&store), MERGE)
        .await
        .unwrap();
    assert_eq!(
        (stats.concepts, stats.updated, stats.ontology_updates),
        (0, 0, 0),
        "{stats:?}"
    );
    assert_eq!(store.records_written(), written);
}

#[test]
fn a_missing_folder_is_an_error() {
    let err = match DocSource::from_dir(&docs_dir().join("nope"), &opts(), &Default::default(), &[])
    {
        Err(e) => e,
        Ok(_) => panic!("a missing folder read"),
    };
    assert!(err.to_string().contains("nope"), "{err}");
}
