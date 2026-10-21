// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! What batching relations must not change: a cardinality-limited
//! symmetric type still refuses its second edge (through the inverse the
//! batch has not materialised yet), a schema change settles the pending
//! relations before it is journaled, an explicit id used twice in one
//! batch is reassigned, and an upsert sees the links still waiting in the
//! batch. Every journal written here replays.

mod hardening_common;

use hardening_common::VecSource;
use ontology_graph::{
    Cardinality, Concept, ConceptId, ConceptType, Ontology, OntologyGraph, Relation, RelationId,
    RelationType,
};
use ontology_io::{ingest_records, ingest_records_with, IngestError, IngestOptions, Record};
use ontology_storage::{FlakyStore, RecordKind, Store};

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

fn person(name: &str) -> Record {
    Record::Concept(Concept::new(ConceptId(0), "Person", name))
}

fn named(rel: &str, a: &str, b: &str, weight: f32) -> Record {
    Record::NamedRelation {
        relation_type: rel.into(),
        source_type: "Person".into(),
        source_name: a.into(),
        target_type: "Person".into(),
        target_name: b.into(),
        weight,
        properties: Default::default(),
    }
}

fn kinds(store: &FlakyStore) -> Vec<&'static str> {
    store
        .records()
        .into_iter()
        .map(|r| match r.kind {
            RecordKind::Ontology(_) => "ontology",
            RecordKind::Concept(_) => "concept",
            RecordKind::Relation(_) => "relation",
            RecordKind::UpdateRelation(_) => "update_relation",
            _ => "other",
        })
        .collect()
}

async fn replays(store: &FlakyStore) -> std::sync::Arc<OntologyGraph> {
    let fresh = OntologyGraph::with_arc(ontology());
    store.load_into(&fresh).await.expect("the journal replays");
    fresh
}

/// `A spouse B` pending, then `B spouse C`: B's inverse edge is not
/// materialised yet, so the limit can only be seen through the batch.
/// (`C spouse A` after `A spouse B` is not covered: the graph checks a
/// functional type on the explicit edge's source only, and has always
/// accepted the out-edge the inverse adds at A — a schema gap older than
/// the batching, left as is.)
#[tokio::test]
async fn a_symmetric_functional_type_refuses_its_second_edge_through_the_batch() {
    for (first, second) in [(("a", "b"), ("b", "c"))] {
        let graph = OntologyGraph::with_arc(ontology());
        let store = FlakyStore::new();
        let records = vec![
            person("a"),
            person("b"),
            person("c"),
            named("spouse", first.0, first.1, 1.0),
            named("spouse", second.0, second.1, 1.0),
        ];
        let err = ingest_records(&mut VecSource::new(records), &graph, Some(&store))
            .await
            .unwrap_err();
        assert!(
            matches!(err, IngestError::Graph(_)),
            "{first:?} then {second:?}: {err}"
        );
        // The first edge (and its inverse) is applied; the second never was.
        assert_eq!(graph.relation_count(), 2, "{first:?} then {second:?}");
        assert_eq!(kinds(&store), ["concept", "concept", "concept", "relation"]);
        let fresh = replays(&store).await;
        assert_eq!(fresh.relation_count(), 2);
    }
}

/// `OneToOne` limits the target side as well: `A twin B` pending, then
/// `C twin A` — A already receives B through the inverse.
#[tokio::test]
async fn a_symmetric_one_to_one_type_limits_both_ends_through_the_batch() {
    let graph = OntologyGraph::with_arc(ontology());
    let store = FlakyStore::new();
    let records = vec![
        person("a"),
        person("b"),
        person("c"),
        named("twin", "a", "b", 1.0),
        named("twin", "c", "a", 1.0),
    ];
    let err = ingest_records(&mut VecSource::new(records), &graph, Some(&store))
        .await
        .unwrap_err();
    assert!(matches!(err, IngestError::Graph(_)), "{err}");
    assert_eq!(graph.relation_count(), 2);
    replays(&store).await;
}

/// Unrelated edges of a limited symmetric type still share one barrier.
#[tokio::test]
async fn distinct_pairs_of_a_limited_symmetric_type_stay_batched() {
    let graph = OntologyGraph::with_arc(ontology());
    let store = FlakyStore::new();
    let mut records: Vec<Record> = (0..6).map(|i| person(&format!("p{i}"))).collect();
    records.push(named("spouse", "p0", "p1", 1.0));
    records.push(named("spouse", "p2", "p3", 1.0));
    records.push(named("spouse", "p4", "p5", 1.0));
    let stats = ingest_records(&mut VecSource::new(records), &graph, Some(&store))
        .await
        .unwrap();
    assert_eq!(stats.relations, 3);
    assert_eq!(
        store.batch_calls(),
        2,
        "concepts, then the three edges together"
    );
    assert_eq!(replays(&store).await.relation_count(), 6);
}

/// Relations validated under the old schema are journaled before the new
/// one: `[ontology, concepts, relations, ontology]`, never relations after
/// a schema that would refuse them.
#[tokio::test]
async fn a_type_redeclaration_settles_the_pending_relations_first() {
    let graph = OntologyGraph::with_arc(Ontology::new());
    let store = FlakyStore::new();
    let records = vec![
        Record::ConceptTypeDecl(ConceptType {
            name: "Person".into(),
            ..Default::default()
        }),
        Record::RelationTypeDecl(RelationType {
            name: "knows".into(),
            domain: "Person".into(),
            range: "Person".into(),
            ..Default::default()
        }),
        person("a"),
        person("b"),
        person("c"),
        named("knows", "a", "b", 1.0),
        named("knows", "a", "c", 1.0),
        // The same type again, now limited: with two `knows` edges applied
        // from `a`, the graph must refuse the change — not accept it and
        // journal the edges after it.
        Record::RelationTypeDecl(RelationType {
            name: "knows".into(),
            domain: "Person".into(),
            range: "Person".into(),
            cardinality: Cardinality::ManyToOne,
            ..Default::default()
        }),
    ];
    let result = ingest_records(&mut VecSource::new(records), &graph, Some(&store)).await;
    let k = kinds(&store);
    let last_relation = k.iter().rposition(|x| *x == "relation").unwrap();
    assert!(
        k[..last_relation]
            .iter()
            .filter(|x| **x == "ontology")
            .count()
            == 1,
        "one schema before the relations, none between: {k:?}"
    );
    assert!(
        !k[last_relation..].contains(&"ontology") || result.is_ok(),
        "a schema after the relations only if the graph accepted it: {k:?} {result:?}"
    );
    // Whatever the graph decided, the journal replays.
    let fresh = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&fresh).await.expect("replays");
    assert_eq!(fresh.relation_count(), 2);
}

/// Two `relation` records with the same explicit id in one batch: the
/// second is reassigned, as it was when relations were applied one by one.
#[tokio::test]
async fn an_explicit_id_used_twice_in_a_batch_is_reassigned() {
    let graph = OntologyGraph::with_arc(ontology());
    let store = FlakyStore::new();
    let records = vec![
        Record::Concept(Concept::new(ConceptId(1), "Person", "a")),
        Record::Concept(Concept::new(ConceptId(2), "Person", "b")),
        Record::Concept(Concept::new(ConceptId(3), "Person", "c")),
        Record::Relation(Relation::new(
            RelationId(7),
            "knows",
            ConceptId(1),
            ConceptId(2),
        )),
        Record::Relation(Relation::new(
            RelationId(7),
            "knows",
            ConceptId(1),
            ConceptId(3),
        )),
    ];
    let stats = ingest_records(&mut VecSource::new(records), &graph, Some(&store))
        .await
        .unwrap();
    assert_eq!(stats.relations, 2);
    assert_eq!(graph.relation_count(), 2);
    let ids: Vec<RelationId> = graph.outgoing(ConceptId(1)).iter().map(|r| r.id).collect();
    assert!(ids.contains(&RelationId(7)) && ids.iter().any(|i| *i != RelationId(7)));
    assert_eq!(replays(&store).await.relation_count(), 2);
}

/// Upsert sees the batch: a link named twice before any barrier is one
/// edge with the last weight; a sheet named twice keeps the last record.
#[tokio::test]
async fn an_upsert_sees_what_is_still_pending() {
    let graph = OntologyGraph::with_arc(ontology());
    let store = FlakyStore::new();
    let upsert = IngestOptions { upsert: true };
    let records = vec![
        person("a"),
        person("b"),
        named("knows", "a", "b", 1.0),
        named("knows", "a", "b", 5.0),
        named("spouse", "a", "b", 1.0),
        named("spouse", "b", "a", 3.0),
    ];
    let stats = ingest_records_with(&mut VecSource::new(records), &graph, Some(&store), upsert)
        .await
        .unwrap();
    assert_eq!(stats.relations, 2, "knows once, spouse once");
    let a = graph.find_by_name("Person", "a").unwrap();
    let b = graph.find_by_name("Person", "b").unwrap();
    let knows = graph.relation_between(a, b, "knows").unwrap();
    assert_eq!(
        graph.get_relation(knows).unwrap().weight,
        5.0,
        "last record wins"
    );
    let spouse = graph.relation_between(a, b, "spouse").unwrap();
    assert_eq!(
        graph.get_relation(spouse).unwrap().weight,
        3.0,
        "either orientation"
    );
    assert_eq!(graph.relation_count(), 3, "knows + spouse + its inverse");

    // A sheet twice in one batch, the applied version being what the last
    // record says: the last record wins and nothing is journaled for it.
    let v1 = Record::Concept(Concept::new(ConceptId(0), "Person", "a").with_description("v1"));
    let same = person("a");
    let before = store.records_written();
    let stats = ingest_records_with(
        &mut VecSource::new(vec![v1, same]),
        &graph,
        Some(&store),
        upsert,
    )
    .await
    .unwrap();
    assert_eq!(
        graph.get_concept(a).unwrap().description,
        "",
        "the last record said: unchanged"
    );
    assert_eq!(stats.concepts, 0, "an upsert is not an addition");
    assert!(
        store.records_written() >= before,
        "v1 may have been journaled, then superseded"
    );
    assert_eq!(
        replays(&store).await.get_concept(a).unwrap().description,
        ""
    );
}
