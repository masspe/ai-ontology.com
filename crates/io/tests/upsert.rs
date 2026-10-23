// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! `IngestOptions { upsert: true }`: a re-import updates what changed,
//! journals nothing for what did not, and keeps every id; `delete_concepts`
//! journals the cascade before the graph changes; an xref directory
//! imported twice is the same graph, and a prune deletes exactly what the
//! source no longer says — sheets and links — and refuses a mass delete.

mod hardening_common;

use hardening_common::{
    kinds, person_in as person, person_ontology as ontology, xref_fixtures_dir as fixtures_dir,
    xref_roots as roots, VecSource, UPSERT,
};
use ontology_graph::{ConceptId, Ontology, OntologyGraph, PropertyValue};
use ontology_io::{
    delete_concepts, ingest_records, ingest_records_with, parse_dir, IngestError, IngestOptions,
    Record, XrefFile, XrefSource,
};
use ontology_storage::{FlakyStore, Store};

fn knows(a: &str, b: &str, weight: f32) -> Record {
    hardening_common::named_relation("knows", a, b, weight)
}

fn near(a: &str, b: &str, weight: f32) -> Record {
    hardening_common::named_relation("near", a, b, weight)
}

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

/// The xref directory imported twice is one graph, not two.
#[tokio::test]
async fn an_xref_directory_reimports_idempotently() {
    let graph = OntologyGraph::with_arc(Ontology::new());
    let store = FlakyStore::new();
    let (mut src, _) = XrefSource::from_path(&fixtures_dir(), &roots(), Some(2)).unwrap();
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
}

const PRUNE: IngestOptions = IngestOptions {
    upsert: true,
    prune: true,
    merge_properties: false,
    prune_max_fraction: 0.25,
};

fn parsed_files() -> Vec<XrefFile> {
    parse_dir(&fixtures_dir(), &roots(), Some(2)).unwrap().files
}

fn programs_flagged(graph: &OntologyGraph, missing: bool) -> usize {
    graph
        .concepts_of_type("Program", false)
        .into_iter()
        .filter(|c| c.properties.get("missing") == Some(&PropertyValue::Bool(missing)))
        .count()
}

/// A synthetic compile: `n` programs in module `m`, each reading table
/// `appdb.t<i % tables>` on field `f`, program 0 also running program 1.
fn corpus(n: usize, tables: usize) -> Vec<XrefFile> {
    (0..n)
        .map(|i| {
            let mut f = XrefFile {
                program: format!("m/p{i}.p"),
                kind: Some(ontology_io::ProgramKind::Procedure),
                ..Default::default()
            };
            let mut use_ = ontology_io::TableUse {
                reads: 2,
                ..Default::default()
            };
            use_.fields_read.insert("f".into());
            f.tables.insert(format!("appdb.t{}", i % tables), use_);
            if i == 0 {
                f.runs.insert("m/p1.p".into(), 1);
            }
            f
        })
        .collect()
}

/// A recompile without one program: a re-import with prune deletes that
/// program and its links, nothing else; the never-compiled (`missing`)
/// programs the fixtures run are re-emitted by the source and stay.
#[tokio::test]
async fn prune_retires_the_program_whose_file_is_gone() {
    let graph = OntologyGraph::with_arc(Ontology::new());
    let store = FlakyStore::new();
    let files = parsed_files();
    let mut src = XrefSource::from_files(files.clone());
    ingest_records_with(&mut src, &graph, Some(&store), PRUNE)
        .await
        .unwrap();
    let (c1, r1) = (graph.concept_count(), graph.relation_count());
    let missing_before = programs_flagged(&graph, true);
    assert!(
        missing_before > 0,
        "the fixtures run programs they do not define"
    );

    // Drop a program that runs nothing: the `missing` run targets are
    // still emitted by their other callers and must stay.
    let idx = files.iter().position(|f| f.runs.is_empty()).unwrap();
    let gone = files[idx].program.clone();
    let gone_id = graph.find_by_name("Program", &gone).unwrap();
    let gone_links = graph.incident_relation_ids(gone_id).unwrap().len();
    assert!(gone_links > 0);
    let remaining: Vec<XrefFile> = files
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != idx)
        .map(|(_, f)| f.clone())
        .collect();
    let mut src = XrefSource::from_files(remaining);
    let stats = ingest_records_with(&mut src, &graph, Some(&store), PRUNE)
        .await
        .unwrap();
    assert!(stats.pruned_concepts >= 1, "{stats:?}");
    assert!(graph.get_concept(gone_id).is_err());
    assert_eq!(programs_flagged(&graph, true), missing_before);
    assert!(graph.concept_count() < c1 && graph.relation_count() < r1);

    // The journal says the same.
    let fresh = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&fresh).await.unwrap();
    assert_eq!(
        (fresh.concept_count(), fresh.relation_count()),
        (graph.concept_count(), graph.relation_count())
    );
}

/// A recompile where a program stopped touching a table other programs
/// still use: its `reads` and `reads_field` links go; the program, the
/// table and the other programs' links stay.
#[tokio::test]
async fn prune_retires_the_links_a_recompile_dropped() {
    let graph = OntologyGraph::with_arc(Ontology::new());
    let store = FlakyStore::new();
    let files = corpus(4, 1);
    let mut src = XrefSource::from_files(files.clone());
    ingest_records_with(&mut src, &graph, Some(&store), PRUNE)
        .await
        .unwrap();
    let p = graph.find_by_name("Program", "m/p2.p").unwrap();
    let t = graph.find_by_name("Table", "appdb.t0").unwrap();
    let links_before = graph.outgoing(p).len();
    let (c1, r1) = (graph.concept_count(), graph.relation_count());

    let mut changed = files.clone();
    changed[2].tables.clear();
    let mut src = XrefSource::from_files(changed);
    let stats = ingest_records_with(&mut src, &graph, Some(&store), PRUNE)
        .await
        .unwrap();
    assert_eq!(stats.pruned_concepts, 0, "{stats:?}");
    assert_eq!(stats.pruned_relations, 2, "reads + reads_field: {stats:?}");
    assert!(graph.get_concept(p).is_ok() && graph.get_concept(t).is_ok());
    assert_eq!(graph.outgoing(p).len(), links_before - 2);
    assert!(graph.relation_between(p, t, "reads").is_none());
    assert_eq!(graph.concept_count(), c1);
    assert_eq!(graph.relation_count(), r1 - 2);
    let other = graph.find_by_name("Program", "m/p3.p").unwrap();
    assert!(graph.relation_between(other, t, "reads").is_some());
}

/// A re-import under other identities would delete everything: refused
/// above the allowed share on a type big enough to judge, nothing
/// touched; forced through with the share raised.
#[tokio::test]
async fn prune_refuses_a_mass_delete_unless_forced() {
    let graph = OntologyGraph::with_arc(Ontology::new());
    let store = FlakyStore::new();
    let files = corpus(ontology_io::PRUNE_GUARD_MIN_SHEETS * 2, 2);
    let mut src = XrefSource::from_files(files.clone());
    ingest_records_with(&mut src, &graph, Some(&store), PRUNE)
        .await
        .unwrap();
    let (c1, r1) = (graph.concept_count(), graph.relation_count());
    let written = store.records_written();

    let mut src = XrefSource::from_files(files[..2].to_vec());
    let err = ingest_records_with(&mut src, &graph, Some(&store), PRUNE)
        .await
        .unwrap_err();
    assert!(
        matches!(err, IngestError::PruneRefused { ref concept_type, .. } if concept_type == "Program"),
        "{err}"
    );
    assert_eq!((graph.concept_count(), graph.relation_count()), (c1, r1));
    // The upserts that preceded the refusal stand (the module's
    // program_count changed); no delete was journaled.
    let deletes = kinds(&store)[written as usize..]
        .iter()
        .filter(|k| k.starts_with("delete"))
        .count();
    assert_eq!(deletes, 0, "{:?}", &kinds(&store)[written as usize..]);

    let forced = IngestOptions {
        prune_max_fraction: 1.0,
        ..PRUNE
    };
    let mut src = XrefSource::from_files(files[..2].to_vec());
    let stats = ingest_records_with(&mut src, &graph, Some(&store), forced)
        .await
        .unwrap();
    assert_eq!(stats.pruned_concepts as usize, files.len() - 2);
    assert_eq!(graph.concepts_of_type("Program", false).len(), 2);
}

/// The first import with prune into an empty graph prunes nothing.
#[tokio::test]
async fn prune_on_a_first_import_deletes_nothing() {
    let graph = OntologyGraph::with_arc(Ontology::new());
    let store = FlakyStore::new();
    let (mut src, _) = XrefSource::from_path(&fixtures_dir(), &roots(), Some(2)).unwrap();
    let stats = ingest_records_with(&mut src, &graph, Some(&store), PRUNE)
        .await
        .unwrap();
    assert_eq!((stats.pruned_concepts, stats.pruned_relations), (0, 0));
}
