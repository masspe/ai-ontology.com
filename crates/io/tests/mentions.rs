// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Entity linking: the sheets a text names become its links — by name,
//! by alias, counted, never to itself, never when the alias is ambiguous.

mod hardening_common;

use hardening_common::{VecSource, UPSERT};
use ontology_graph::{Concept, ConceptId, ConceptType, Ontology, OntologyGraph, PropertyValue};
use ontology_io::{ingest_records_with, IngestOptions, MentionIndex, MentionSource, Record};
use ontology_storage::FlakyStore;
use std::collections::VecDeque;

const MERGE: IngestOptions = IngestOptions {
    merge_properties: true,
    ..UPSERT
};

fn ty(name: &str, props: &[&str]) -> Record {
    Record::ConceptTypeDecl(ConceptType {
        name: name.into(),
        properties: Some(props.iter().map(|s| s.to_string()).collect()),
        ..Default::default()
    })
}

fn sheet(ty: &str, name: &str, description: &str, help: Option<&str>) -> Record {
    let mut c = Concept::new(ConceptId(0), ty, name);
    c.description = description.into();
    if let Some(h) = help {
        c = c.with_property("help", PropertyValue::from(h));
    }
    Record::Concept(c)
}

/// Screens, programs, tables, and documents that talk about them.
async fn world() -> (std::sync::Arc<OntologyGraph>, FlakyStore) {
    let graph = OntologyGraph::with_arc(Ontology::new());
    let store = FlakyStore::new();
    let mut src = VecSource(VecDeque::from(vec![
        ty("Screen", &["help"]),
        ty("Program", &[]),
        ty("Table", &[]),
        ty("Document", &[]),
        sheet("Screen", "gl100", "Grand livre", Some("Voir aussi gl101 pour les journaux.")),
        sheet("Screen", "gl101", "Journaux", None),
        sheet("Program", "gl/w-gl100.w", "", None),
        sheet("Program", "pr/w-gl100.w", "", None),
        sheet("Program", "gl/b-gl100.w", "", None),
        sheet("Table", "appdb.emp", "Salariés", None),
        sheet(
            "Document",
            "Ticket 1234",
            "Erreur dans GL100 à l'ouverture ; GL100 plante après b-gl100.w. La table appdb.emp est vide. Voir w-gl100.",
            None,
        ),
        sheet("Document", "Ticket 1235", "Rien de connu ici (cat, dog).", None),
    ]));
    ingest_records_with(&mut src, &graph, Some(&store), MERGE)
        .await
        .unwrap();
    (graph, store)
}

#[tokio::test]
async fn a_text_is_linked_to_the_sheets_it_names_with_counts_and_aliases() {
    let (graph, store) = world().await;
    let to = [
        "Screen".to_string(),
        "Program".to_string(),
        "Table".to_string(),
    ];
    let index = MentionIndex::from_graph(&graph, &to);
    // `w-gl100.w` / `w-gl100` could be two programs: not an alias; `b-gl100` is.
    assert!(index.find("w-gl100").is_empty());
    assert_eq!(
        index.find("B-GL100.W b-gl100")[&("Program".into(), "gl/b-gl100.w".into())],
        2
    );

    let (mut src, report) =
        MentionSource::from_graph(&graph, &["Document".into(), "Screen".into()], &to);
    assert_eq!((report.scanned, report.targets), (4, 4), "{report:?}");
    let stats = ingest_records_with(&mut src, &graph, Some(&store), MERGE)
        .await
        .unwrap();
    assert_eq!(stats.relations, 4, "{stats:?}");

    let ticket = graph.find_by_name("Document", "Ticket 1234").unwrap();
    let gl100 = graph.find_by_name("Screen", "gl100").unwrap();
    let rid = graph
        .relation_between(ticket, gl100, "mentions_document_screen")
        .expect("ticket → screen");
    assert_eq!(graph.get_relation(rid).unwrap().weight, 2.0, "GL100 twice");
    let prog = graph.find_by_name("Program", "gl/b-gl100.w").unwrap();
    assert!(graph
        .relation_between(ticket, prog, "mentions_document_program")
        .is_some());
    let emp = graph.find_by_name("Table", "appdb.emp").unwrap();
    assert!(graph
        .relation_between(ticket, emp, "mentions_document_table")
        .is_some());
    // A screen's help names another screen; never itself.
    let gl101 = graph.find_by_name("Screen", "gl101").unwrap();
    assert!(graph
        .relation_between(gl100, gl101, "mentions_screen_screen")
        .is_some());
    assert!(graph
        .relation_between(gl100, gl100, "mentions_screen_screen")
        .is_none());
    let other = graph.find_by_name("Document", "Ticket 1235").unwrap();
    assert!(graph
        .outgoing_typed(other, &["mentions_document_screen".into()])
        .is_empty());

    // Again: nothing to write.
    let written = store.records_written();
    let (mut src, _) =
        MentionSource::from_graph(&graph, &["Document".into(), "Screen".into()], &to);
    let stats = ingest_records_with(&mut src, &graph, Some(&store), MERGE)
        .await
        .unwrap();
    assert_eq!((stats.relations, stats.updated), (0, 0), "{stats:?}");
    assert_eq!(store.records_written(), written);
}

#[test]
fn short_names_are_not_indexed() {
    let graph = OntologyGraph::with_arc(Ontology::new());
    let index = MentionIndex::from_graph(&graph, &["Nothing".into()]);
    assert!(index.is_empty());
    assert_eq!(index.len(), 0);
}
