// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! `ingest_records` write-ahead contract: concepts are journaled in batches
//! under one barrier, schema declarations produce a single `Ontology`
//! record placed before the first instance, intra-batch conflicts are
//! rejected before anything is durable, a failing store leaves the graph
//! untouched, and whatever was journaled replays to the same graph.

use async_trait::async_trait;
use ontology_graph::{Concept, ConceptId, ConceptType, Ontology, OntologyGraph, RelationType};
use ontology_io::{ingest_records, IngestError, Record, Source, INGEST_BATCH_SIZE};
use ontology_storage::{FlakyStore, RecordKind, Store};
use std::collections::VecDeque;
use std::sync::Arc;

struct VecSource(VecDeque<Record>);

impl VecSource {
    fn new(records: Vec<Record>) -> Self {
        Self(records.into_iter().collect())
    }
}

#[async_trait]
impl Source for VecSource {
    async fn next(&mut self) -> Result<Option<Record>, IngestError> {
        Ok(self.0.pop_front())
    }
}

fn ontology() -> Ontology {
    let mut o = Ontology::new();
    o.add_concept_type(ConceptType {
        name: "Person".into(),
        ..Default::default()
    });
    o.add_concept_type(ConceptType {
        name: "Robot".into(),
        disjoint_with: vec!["Person".into()],
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

fn person(name: &str) -> Record {
    Record::Concept(Concept::new(ConceptId(0), "Person", name))
}

fn kinds(store: &FlakyStore) -> Vec<&'static str> {
    store
        .records()
        .iter()
        .map(|r| match r.kind {
            RecordKind::Ontology(_) => "ontology",
            RecordKind::Concept(_) => "concept",
            RecordKind::Relation(_) => "relation",
            RecordKind::Rule(_) => "rule",
            RecordKind::Action(_) => "action",
            _ => "other",
        })
        .collect()
}

#[tokio::test]
async fn consecutive_concepts_are_journaled_in_batches_under_one_barrier() {
    let graph = OntologyGraph::with_arc(ontology());
    let store = FlakyStore::new();
    let n = INGEST_BATCH_SIZE * 2 + 40;
    let records: Vec<Record> = (0..n).map(|i| person(&format!("p{i}"))).collect();

    let stats = ingest_records(&mut VecSource::new(records), &graph, Some(&store))
        .await
        .unwrap();

    assert_eq!(stats.concepts as usize, n);
    assert_eq!(graph.concept_count(), n);
    assert_eq!(store.records_written() as usize, n);
    assert_eq!(store.batch_calls(), 3, "256 + 256 + 40 → three barriers");
    assert_eq!(store.append_calls(), 0, "no per-record appends");
}

#[tokio::test]
async fn type_declarations_produce_one_ontology_record_before_the_first_instance() {
    let graph = OntologyGraph::with_arc(Ontology::new());
    let store = FlakyStore::new();
    let mut records = vec![
        Record::ConceptTypeDecl(ConceptType {
            name: "Person".into(),
            ..Default::default()
        }),
        Record::ConceptTypeDecl(ConceptType {
            name: "City".into(),
            ..Default::default()
        }),
        Record::RelationTypeDecl(RelationType {
            name: "lives_in".into(),
            domain: "Person".into(),
            range: "City".into(),
            ..Default::default()
        }),
    ];
    records.extend((0..5).map(|i| person(&format!("p{i}"))));
    records.push(Record::Concept(Concept::new(
        ConceptId(0),
        "City",
        "Geneva",
    )));
    records.push(Record::NamedRelation {
        relation_type: "lives_in".into(),
        source_type: "Person".into(),
        source_name: "p0".into(),
        target_type: "City".into(),
        target_name: "Geneva".into(),
        weight: 1.0,
    });

    let stats = ingest_records(&mut VecSource::new(records), &graph, Some(&store))
        .await
        .unwrap();
    assert_eq!(stats.ontology_updates, 3);
    assert_eq!(stats.concepts, 6);
    assert_eq!(stats.relations, 1);

    let k = kinds(&store);
    let ontology_records = k.iter().filter(|k| **k == "ontology").count();
    assert_eq!(ontology_records, 1, "three declarations, one record: {k:?}");
    assert_eq!(
        k[0], "ontology",
        "the schema precedes every instance: {k:?}"
    );
    assert_eq!(k.last(), Some(&"relation"));

    // The single record carries the *complete* schema.
    let onto = store
        .records()
        .into_iter()
        .find_map(|r| match r.kind {
            RecordKind::Ontology(o) => Some(o),
            _ => None,
        })
        .unwrap();
    assert!(onto.concept_types.contains_key("Person"));
    assert!(onto.concept_types.contains_key("City"));
    assert!(onto.relation_types.contains_key("lives_in"));
}

#[tokio::test]
async fn duplicate_names_inside_one_batch_are_rejected_before_anything_is_durable() {
    let graph = OntologyGraph::with_arc(ontology());
    let store = FlakyStore::new();
    let records = vec![person("Alice"), person("Bob"), person("alice")];

    let err = ingest_records(&mut VecSource::new(records), &graph, Some(&store))
        .await
        .expect_err("duplicate must be rejected");
    assert!(err.to_string().contains("duplicate"), "{err}");
    assert_eq!(
        store.records_written(),
        0,
        "the batch never reached the store"
    );
    assert_eq!(graph.concept_count(), 0, "nothing applied");

    // Replaying the (empty) journal is trivially consistent — and had the
    // batch been journaled, replay would have failed on the duplicate.
    let fresh = OntologyGraph::with_arc(ontology());
    store.load_into(&fresh).await.unwrap();
    assert_eq!(fresh.concept_count(), 0);
}

#[tokio::test]
async fn disjoint_types_are_enforced_inside_one_batch() {
    let graph = OntologyGraph::with_arc(ontology());
    let store = FlakyStore::new();
    let records = vec![
        person("R2D2"),
        Record::Concept(Concept::new(ConceptId(0), "Robot", "r2d2")),
    ];
    let err = ingest_records(&mut VecSource::new(records), &graph, Some(&store))
        .await
        .expect_err("disjoint violation must be rejected");
    assert!(err.to_string().contains("disjoint"), "{err}");
    assert_eq!(store.records_written(), 0);
    assert_eq!(graph.concept_count(), 0);
}

#[tokio::test]
async fn a_failing_store_leaves_the_graph_untouched() {
    let graph = OntologyGraph::with_arc(ontology());
    let store = FlakyStore::new();
    store.set_failing(true);
    let records: Vec<Record> = (0..10).map(|i| person(&format!("p{i}"))).collect();

    let err = ingest_records(&mut VecSource::new(records), &graph, Some(&store))
        .await
        .expect_err("store failure must surface");
    assert!(matches!(err, IngestError::Store(_)), "{err}");
    assert_eq!(graph.concept_count(), 0, "memory never runs ahead of disk");
    assert_eq!(graph.concepts_generation(), 0);

    // A relation whose append fails likewise leaves nothing behind.
    store.set_failing(false);
    ingest_records(
        &mut VecSource::new(vec![person("a"), person("b")]),
        &graph,
        Some(&store),
    )
    .await
    .unwrap();
    store.set_failing(true);
    let rel_gen = graph.relations_generation();
    let err = ingest_records(
        &mut VecSource::new(vec![Record::NamedRelation {
            relation_type: "knows".into(),
            source_type: "Person".into(),
            source_name: "a".into(),
            target_type: "Person".into(),
            target_name: "b".into(),
            weight: 1.0,
        }]),
        &graph,
        Some(&store),
    )
    .await
    .expect_err("store failure must surface");
    assert!(matches!(err, IngestError::Store(_)));
    assert_eq!(graph.relation_count(), 0);
    assert_eq!(graph.relations_generation(), rel_gen);
}

#[tokio::test]
async fn the_journal_replays_to_the_same_graph() {
    let graph = OntologyGraph::with_arc(Ontology::new());
    let store = FlakyStore::new();
    let mut records = vec![Record::Ontology(ontology())];
    records.extend((0..300).map(|i| person(&format!("p{i}"))));
    records.push(Record::NamedRelation {
        relation_type: "knows".into(),
        source_type: "Person".into(),
        source_name: "p1".into(),
        target_type: "Person".into(),
        target_name: "p299".into(),
        weight: 0.5,
    });
    // A named relation whose endpoints arrive *after* it is deferred, then
    // resolved once the batch holding them is flushed.
    records.insert(
        1,
        Record::NamedRelation {
            relation_type: "knows".into(),
            source_type: "Person".into(),
            source_name: "p10".into(),
            target_type: "Person".into(),
            target_name: "p20".into(),
            weight: 1.0,
        },
    );

    let stats = ingest_records(&mut VecSource::new(records), &graph, Some(&store))
        .await
        .unwrap();
    assert_eq!(stats.concepts, 300);
    assert_eq!(stats.relations, 2);

    let fresh = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&fresh).await.unwrap();
    assert_eq!(fresh.concept_count(), 300);
    assert_eq!(fresh.relation_count(), 2);
    assert_eq!(fresh.ontology().concept_types.len(), 2);
    // Same ids on both sides: the journaled record carries the allocated id.
    let p1 = graph.find_by_name("Person", "p1").unwrap();
    assert_eq!(fresh.find_by_name("Person", "p1"), Some(p1));
}

#[tokio::test]
async fn explicit_ids_upsert_across_and_within_batches() {
    // Re-ingesting an export (concepts carry their ids) must be idempotent.
    let graph = OntologyGraph::with_arc(ontology());
    let store = FlakyStore::new();
    let export: Vec<Record> = (1..=20)
        .map(|i| Record::Concept(Concept::new(ConceptId(i), "Person", format!("p{i}"))))
        .collect();
    ingest_records(&mut VecSource::new(export.clone()), &graph, Some(&store))
        .await
        .unwrap();
    ingest_records(&mut VecSource::new(export), &graph, Some(&store))
        .await
        .unwrap();
    assert_eq!(graph.concept_count(), 20);
    // Same entity twice in one stream (a rename mid-export): last one wins.
    let twice = vec![
        Record::Concept(Concept::new(ConceptId(7), "Person", "p7")),
        Record::Concept(Concept::new(ConceptId(7), "Person", "p7-renamed")),
    ];
    ingest_records(&mut VecSource::new(twice), &graph, Some(&store))
        .await
        .unwrap();
    assert_eq!(graph.concept_count(), 20);
    assert_eq!(graph.get_concept(ConceptId(7)).unwrap().name, "p7-renamed");

    let fresh = OntologyGraph::with_arc(ontology());
    store.load_into(&fresh).await.unwrap();
    assert_eq!(fresh.concept_count(), 20);
    assert_eq!(fresh.get_concept(ConceptId(7)).unwrap().name, "p7-renamed");
}

#[tokio::test]
async fn ingest_without_a_store_still_applies_everything() {
    let graph = OntologyGraph::with_arc(ontology());
    let records: Vec<Record> = (0..INGEST_BATCH_SIZE + 3)
        .map(|i| person(&format!("p{i}")))
        .collect();
    let stats = ingest_records(&mut VecSource::new(records), &graph, None)
        .await
        .unwrap();
    assert_eq!(stats.concepts as usize, INGEST_BATCH_SIZE + 3);
    assert_eq!(graph.concept_count(), INGEST_BATCH_SIZE + 3);
    drop(Arc::clone(&graph));
}
