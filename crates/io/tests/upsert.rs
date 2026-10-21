// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! `IngestOptions { upsert: true }`: a re-import updates what changed,
//! journals nothing for what did not, and keeps every id; `delete_concepts`
//! journals the cascade before the graph changes; an xref directory
//! imported twice is the same graph, and `retire_programs` removes only
//! the compiled programs that disappeared.

mod hardening_common;

use hardening_common::VecSource;
use ontology_graph::{
    Concept, ConceptId, ConceptType, Ontology, OntologyGraph, PropertyValue, RelationType,
};
use ontology_io::{
    delete_concepts, ingest_records, ingest_records_with, retire_programs, IngestError,
    IngestOptions, Record, XrefSource,
};
use ontology_storage::{FlakyStore, RecordKind, Store};
use std::collections::BTreeSet;

fn ontology() -> Ontology {
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
    o
}

fn person(name: &str, city: &str) -> Record {
    Record::Concept(
        Concept::new(ConceptId(0), "Person", name)
            .with_property("city", PropertyValue::Text(city.into())),
    )
}

fn knows(a: &str, b: &str, weight: f32) -> Record {
    Record::NamedRelation {
        relation_type: "knows".into(),
        source_type: "Person".into(),
        source_name: a.into(),
        target_type: "Person".into(),
        target_name: b.into(),
        weight,
        properties: Default::default(),
    }
}

fn near(a: &str, b: &str, weight: f32) -> Record {
    Record::NamedRelation {
        relation_type: "near".into(),
        source_type: "Person".into(),
        source_name: a.into(),
        target_type: "Person".into(),
        target_name: b.into(),
        weight,
        properties: Default::default(),
    }
}

fn kinds(store: &FlakyStore) -> Vec<String> {
    store
        .records()
        .into_iter()
        .map(|r| match r.kind {
            RecordKind::Concept(_) => "concept",
            RecordKind::Relation(_) => "relation",
            RecordKind::UpdateRelation(_) => "update_relation",
            RecordKind::DeleteConcept { .. } => "delete_concept",
            RecordKind::DeleteRelation { .. } => "delete_relation",
            _ => "other",
        })
        .map(str::to_string)
        .collect()
}

const UPSERT: IngestOptions = IngestOptions { upsert: true };

/// The same records twice: the second pass journals nothing, every id is
/// kept, and the stats say so.
#[tokio::test]
async fn an_identical_reimport_journals_nothing_and_keeps_ids() {
    let graph = OntologyGraph::with_arc(ontology());
    let store = FlakyStore::new();
    let records = || {
        vec![
            person("a", "Geneva"),
            person("b", "Sion"),
            knows("a", "b", 2.0),
        ]
    };

    let first = ingest_records_with(&mut VecSource::new(records()), &graph, Some(&store), UPSERT)
        .await
        .unwrap();
    assert_eq!(
        (first.concepts, first.relations, first.unchanged),
        (2, 1, 0)
    );
    let written = store.records_written();
    let a = graph.find_by_name("Person", "a").unwrap();

    let second = ingest_records_with(&mut VecSource::new(records()), &graph, Some(&store), UPSERT)
        .await
        .unwrap();
    assert_eq!(
        (second.concepts, second.relations, second.updated),
        (0, 0, 0)
    );
    assert_eq!(
        second.unchanged, 3,
        "two concepts and one relation untouched"
    );
    assert_eq!(store.records_written(), written, "nothing journaled");
    assert_eq!(graph.find_by_name("Person", "a"), Some(a));
    assert_eq!((graph.concept_count(), graph.relation_count()), (2, 1));
}

/// A changed property updates the sheet in place (same id, journaled as a
/// concept record with that id); a changed weight becomes one
/// `UpdateRelation` record, batched, not an added edge.
#[tokio::test]
async fn changed_records_are_updated_in_place() {
    let graph = OntologyGraph::with_arc(ontology());
    let store = FlakyStore::new();
    ingest_records_with(
        &mut VecSource::new(vec![
            person("a", "Geneva"),
            person("b", "Sion"),
            knows("a", "b", 2.0),
        ]),
        &graph,
        Some(&store),
        UPSERT,
    )
    .await
    .unwrap();
    let a = graph.find_by_name("Person", "a").unwrap();
    let before = store.batch_calls();

    let stats = ingest_records_with(
        &mut VecSource::new(vec![
            person("a", "Lausanne"),
            person("b", "Sion"),
            knows("a", "b", 5.0),
        ]),
        &graph,
        Some(&store),
        UPSERT,
    )
    .await
    .unwrap();
    assert_eq!(stats.updated, 2, "one concept, one relation");
    assert_eq!(stats.unchanged, 1, "b");
    assert_eq!(graph.find_by_name("Person", "a"), Some(a), "id kept");
    assert_eq!(
        graph.get_concept(a).unwrap().properties["city"],
        PropertyValue::Text("Lausanne".into())
    );
    let b = graph.find_by_name("Person", "b").unwrap();
    let rid = graph.relation_between(a, b, "knows").unwrap();
    assert_eq!(graph.get_relation(rid).unwrap().weight, 5.0);
    assert_eq!((graph.concept_count(), graph.relation_count()), (2, 1));
    assert_eq!(
        &kinds(&store)[3..],
        ["concept", "update_relation"],
        "the update is journaled as such"
    );
    assert_eq!(store.batch_calls(), before + 2, "one barrier per kind");
    assert_eq!(store.append_calls(), 0);

    // And the journal replays to the same state.
    let fresh = OntologyGraph::with_arc(ontology());
    store.load_into(&fresh).await.unwrap();
    assert_eq!(
        fresh.get_concept(a).unwrap().properties["city"],
        PropertyValue::Text("Lausanne".into())
    );
    assert_eq!(fresh.get_relation(rid).unwrap().weight, 5.0);
}

/// Upsert of a symmetric relation refreshes the inverse edge as well.
#[tokio::test]
async fn a_symmetric_relation_upsert_reaches_the_inverse() {
    let graph = OntologyGraph::with_arc(ontology());
    let store = FlakyStore::new();
    let batch = |w| vec![person("a", "x"), person("b", "y"), near("a", "b", w)];
    ingest_records_with(
        &mut VecSource::new(batch(1.0)),
        &graph,
        Some(&store),
        UPSERT,
    )
    .await
    .unwrap();
    ingest_records_with(
        &mut VecSource::new(batch(9.0)),
        &graph,
        Some(&store),
        UPSERT,
    )
    .await
    .unwrap();
    let a = graph.find_by_name("Person", "a").unwrap();
    let b = graph.find_by_name("Person", "b").unwrap();
    let inv = graph.relation_between(b, a, "near").unwrap();
    assert_eq!(graph.get_relation(inv).unwrap().weight, 9.0);
    assert_eq!(
        graph.relation_count(),
        2,
        "original + inverse, nothing added"
    );
}

/// Without the option a duplicate name is still refused: a CSV loaded
/// twice by mistake must not silently overwrite.
#[tokio::test]
async fn without_upsert_a_duplicate_name_is_still_an_error() {
    let graph = OntologyGraph::with_arc(ontology());
    ingest_records(&mut VecSource::new(vec![person("a", "x")]), &graph, None)
        .await
        .unwrap();
    let err = ingest_records(&mut VecSource::new(vec![person("a", "y")]), &graph, None)
        .await
        .unwrap_err();
    assert!(matches!(err, IngestError::Graph(_)), "{err}");
}

/// `delete_concepts`: the cascade (concept + incident relations) is
/// journaled under one barrier, then removed; unknown ids are skipped.
#[tokio::test]
async fn delete_concepts_journals_the_cascade_then_removes() {
    let graph = OntologyGraph::with_arc(ontology());
    let store = FlakyStore::new();
    ingest_records(
        &mut VecSource::new(vec![
            person("a", "x"),
            person("b", "y"),
            person("c", "z"),
            knows("a", "b", 1.0),
            knows("c", "a", 1.0),
        ]),
        &graph,
        Some(&store),
    )
    .await
    .unwrap();
    let a = graph.find_by_name("Person", "a").unwrap();
    let before = store.batch_calls();
    let report = delete_concepts(&graph, Some(&store), &[a, ConceptId(9999)])
        .await
        .unwrap();
    assert_eq!((report.deleted, report.relations), (1, 2));
    assert_eq!(store.batch_calls(), before + 1);
    assert_eq!(
        &kinds(&store)[5..],
        ["delete_concept", "delete_relation", "delete_relation"]
    );
    assert_eq!((graph.concept_count(), graph.relation_count()), (2, 0));
    let fresh = OntologyGraph::with_arc(ontology());
    store.load_into(&fresh).await.unwrap();
    assert_eq!((fresh.concept_count(), fresh.relation_count()), (2, 0));
}

fn fixtures_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/xref"))
}

fn roots() -> Vec<String> {
    vec!["D:\\Mediasoft\\App\\MS.ERP2".to_string()]
}

/// The xref directory imported twice is one graph, not two; a program
/// that is no longer compiled is retired by `retire_programs`, and the
/// never-compiled (`missing`) ones stay.
#[tokio::test]
async fn an_xref_directory_reimports_idempotently_and_sync_retires_the_gone() {
    let graph = OntologyGraph::with_arc(Ontology::new());
    let store = FlakyStore::new();
    let (mut src, parsed) = XrefSource::from_path(&fixtures_dir(), &roots(), Some(2)).unwrap();
    let first = ingest_records_with(&mut src, &graph, Some(&store), UPSERT)
        .await
        .unwrap();
    let (c1, r1) = (graph.concept_count(), graph.relation_count());
    assert!(first.concepts > 0 && first.relations > 0);
    let written = store.records_written();

    let (mut src, _) = XrefSource::from_path(&fixtures_dir(), &roots(), Some(2)).unwrap();
    let second = ingest_records_with(&mut src, &graph, Some(&store), UPSERT)
        .await
        .unwrap();
    assert_eq!((graph.concept_count(), graph.relation_count()), (c1, r1));
    assert_eq!(
        (second.concepts, second.relations, second.updated),
        (0, 0, 0)
    );
    assert_eq!(
        store.records_written(),
        written,
        "an identical re-import journals nothing"
    );

    // Pretend one compiled program disappeared from the next compile.
    let mut keep: BTreeSet<String> = parsed.programs.clone();
    let gone = keep.pop_first().unwrap();
    let gone_id = graph.find_by_name("Program", &gone).unwrap();
    let missing_before = graph
        .concepts_of_type("Program", false)
        .into_iter()
        .filter(|c| c.properties.get("missing") == Some(&PropertyValue::Bool(true)))
        .count();
    assert!(
        missing_before > 0,
        "the fixtures run programs they do not define"
    );

    let report = retire_programs(&graph, Some(&store), &keep).await.unwrap();
    assert_eq!(report.deleted, 1, "{report:?}");
    assert!(graph.get_concept(gone_id).is_err());
    let missing_after = graph
        .concepts_of_type("Program", false)
        .into_iter()
        .filter(|c| c.properties.get("missing") == Some(&PropertyValue::Bool(true)))
        .count();
    assert_eq!(
        missing_after, missing_before,
        "missing programs are not retired"
    );
    assert_eq!(graph.concept_count(), c1 - 1);
}
