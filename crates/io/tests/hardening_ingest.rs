// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! `ingest_records` write-ahead contract, the parts `write_ahead.rs` does
//! not pin: exact barrier boundaries, `Relation` / `Rule` / `Action`
//! records settling the concept batch, the error a deferred
//! `NamedRelation` reports, `Record::Ontology` validated-then-journaled
//! (`STORAGE.md` §10.10), rule / action redeclarations, deferred fragment
//! types, and the rollback of every un-journaled declaration kind.

mod hardening_common;

use hardening_common::VecSource;
use ontology_graph::{
    Action, ActionId, ActionType, Concept, ConceptId, ConceptType, Ontology, OntologyGraph,
    Relation, RelationId, RelationType, Rule, RuleId, RuleType,
};
use ontology_io::{
    fragment_relation_name, fragment_type_name, ingest_records, IngestError, Record,
    INGEST_BATCH_SIZE,
};
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
    o.add_rule_type(rule_type("")).unwrap();
    o.add_action_type(action_type("")).unwrap();
    o
}

fn rule_type(description: &str) -> RuleType {
    RuleType {
        name: "must_know_someone".into(),
        when: "a Person exists".into(),
        then: "it knows at least one Person".into(),
        applies_to: vec!["Person".into()],
        strict: false,
        description: description.into(),
    }
}

fn action_type(effect: &str) -> ActionType {
    ActionType {
        name: "introduce".into(),
        subject: "Person".into(),
        object: Some("Person".into()),
        parameters: vec!["date".into()],
        effect: effect.into(),
        description: String::new(),
    }
}

fn person(name: &str) -> Record {
    Record::Concept(Concept::new(ConceptId(0), "Person", name))
}

fn person_with_id(id: u64, name: &str) -> Record {
    Record::Concept(Concept::new(ConceptId(id), "Person", name))
}

fn named(rel: &str, src: &str, tgt: &str) -> Record {
    Record::NamedRelation {
        relation_type: rel.into(),
        source_type: "Person".into(),
        source_name: src.into(),
        target_type: "Person".into(),
        target_name: tgt.into(),
        weight: 1.0,
    }
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

/// Exactly `INGEST_BATCH_SIZE` consecutive concepts fit under one barrier;
/// one more concept opens a second batch of size one.
#[tokio::test]
async fn a_batch_of_exactly_256_costs_one_barrier_and_257_costs_two() {
    for (n, expected_barriers) in [(INGEST_BATCH_SIZE, 1), (INGEST_BATCH_SIZE + 1, 2)] {
        let graph = OntologyGraph::with_arc(ontology());
        let store = FlakyStore::new();
        let records: Vec<Record> = (0..n).map(|i| person(&format!("p{i}"))).collect();
        let stats = ingest_records(&mut VecSource::new(records), &graph, Some(&store))
            .await
            .unwrap();
        assert_eq!(stats.concepts as usize, n);
        assert_eq!(store.records_written() as usize, n);
        assert_eq!(store.batch_calls(), expected_barriers, "n = {n}");
        assert_eq!(store.append_calls(), 0, "concepts never go one by one");
        assert_eq!(graph.concept_count(), n);
    }
}

/// A `Relation` record (explicit endpoint ids) settles the pending concepts
/// under one barrier before it is journaled itself, and its weight and
/// allocated id are what reaches the store.
#[tokio::test]
async fn a_relation_record_settles_the_pending_concepts_before_it_is_journaled() {
    let graph = OntologyGraph::with_arc(ontology());
    let store = FlakyStore::new();
    let mut rel = Relation::new(RelationId(0), "knows", ConceptId(1), ConceptId(2));
    rel.weight = 0.25;
    let records = vec![
        person_with_id(1, "a"),
        person_with_id(2, "b"),
        person_with_id(3, "c"),
        Record::Relation(rel),
        person_with_id(4, "d"),
        person_with_id(5, "e"),
    ];
    let stats = ingest_records(&mut VecSource::new(records), &graph, Some(&store))
        .await
        .unwrap();
    assert_eq!((stats.concepts, stats.relations), (5, 1));
    assert_eq!(
        kinds(&store),
        vec!["concept", "concept", "concept", "relation", "concept", "concept"]
    );
    assert_eq!(store.batch_calls(), 2, "the relation split the concept run");
    assert_eq!(store.append_calls(), 1);
    let journaled = store
        .records()
        .into_iter()
        .find_map(|r| match r.kind {
            RecordKind::Relation(r) => Some(r),
            _ => None,
        })
        .unwrap();
    assert_ne!(
        journaled.id,
        RelationId(0),
        "the journaled record carries the allocated id"
    );
    assert_eq!(journaled.weight, 0.25);
    let live = graph.get_relation(journaled.id).unwrap();
    assert_eq!(
        (live.source, live.target, live.weight),
        (ConceptId(1), ConceptId(2), 0.25)
    );
}

/// A `NamedRelation` whose endpoint never shows up fails with `UnknownNamed`
/// naming the endpoint that is actually missing — source or target — and
/// the concepts that did arrive stay durable.
#[tokio::test]
async fn a_deferred_named_relation_names_the_missing_endpoint() {
    // Missing target.
    let graph = OntologyGraph::with_arc(ontology());
    let store = FlakyStore::new();
    let records = vec![named("knows", "alice", "ghost"), person("alice")];
    let err = ingest_records(&mut VecSource::new(records), &graph, Some(&store))
        .await
        .unwrap_err();
    match &err {
        IngestError::UnknownNamed { concept_type, name } => {
            assert_eq!((concept_type.as_str(), name.as_str()), ("Person", "ghost"));
        }
        other => panic!("expected UnknownNamed, got {other}"),
    }
    assert!(err.to_string().contains("`ghost`"), "{err}");
    assert_eq!(graph.concept_count(), 1, "alice was applied");
    assert_eq!(store.records_written(), 1, "alice is durable");
    assert_eq!(graph.relation_count(), 0);

    // Missing source.
    let graph = OntologyGraph::with_arc(ontology());
    let records = vec![person("bob"), named("knows", "nobody", "bob")];
    let err = ingest_records(&mut VecSource::new(records), &graph, None)
        .await
        .unwrap_err();
    match err {
        IngestError::UnknownNamed { concept_type, name } => {
            assert_eq!((concept_type.as_str(), name.as_str()), ("Person", "nobody"));
        }
        other => panic!("expected UnknownNamed, got {other}"),
    }
}

/// A `NamedRelation` between two concepts still waiting in the batch
/// resolves within the same call: the batch is settled first, then the
/// relation is journaled — one barrier, one append.
#[tokio::test]
async fn a_named_relation_resolves_endpoints_sitting_in_the_pending_batch() {
    let graph = OntologyGraph::with_arc(ontology());
    let store = FlakyStore::new();
    let records = vec![person("a"), person("b"), named("knows", "a", "b")];
    let stats = ingest_records(&mut VecSource::new(records), &graph, Some(&store))
        .await
        .unwrap();
    assert_eq!(stats.relations, 1);
    assert_eq!(kinds(&store), vec!["concept", "concept", "relation"]);
    assert_eq!((store.batch_calls(), store.append_calls()), (1, 1));
}

/// `Record::Ontology` replaces the whole schema: it settles the pending
/// batch, is journaled before any instance that follows, and the types it
/// does not carry are gone from the live ontology.
#[tokio::test]
async fn an_ontology_record_replaces_the_schema_and_precedes_the_instances_after_it() {
    let graph = OntologyGraph::with_arc(ontology());
    let store = FlakyStore::new();
    let mut replacement = Ontology::new();
    replacement.add_concept_type(ConceptType {
        name: "Person".into(),
        ..Default::default()
    });
    replacement.add_concept_type(ConceptType {
        name: "City".into(),
        ns: Some("geo".into()),
        ..Default::default()
    });
    let records = vec![
        person("a"),
        Record::Ontology(replacement),
        Record::Concept(Concept::new(ConceptId(0), "City", "Geneva")),
    ];
    let stats = ingest_records(&mut VecSource::new(records), &graph, Some(&store))
        .await
        .unwrap();
    assert_eq!((stats.concepts, stats.ontology_updates), (2, 1));
    assert_eq!(kinds(&store), vec!["concept", "ontology", "concept"]);
    let onto = graph.ontology();
    assert!(onto.concept_types.contains_key("City"));
    assert!(
        !onto.relation_types.contains_key("knows") && onto.rule_types.is_empty(),
        "replacement, not merge"
    );
    assert_eq!(onto.ns_of_type("City"), "geo");

    let fresh = OntologyGraph::with_arc(ontology());
    store.load_into(&fresh).await.unwrap();
    assert_eq!(fresh.ontology().namespaces(), onto.namespaces());
    assert!(fresh.find_by_name("City", "Geneva").is_some());
}

/// `STORAGE.md` §10.10 — a schema that would orphan instances is refused
/// **before** it is journaled: the store must never hold an ontology record
/// the graph rejected, or the next replay fails on it.
#[tokio::test]
async fn a_refused_ontology_record_never_reaches_the_store() {
    let graph = OntologyGraph::with_arc(ontology());
    let store = FlakyStore::new();
    ingest_records(&mut VecSource::new(vec![person("a")]), &graph, Some(&store))
        .await
        .unwrap();
    let written_before = store.records_written();

    // Drops `Person` while a Person instance exists.
    let mut orphaning = Ontology::new();
    orphaning.add_concept_type(ConceptType {
        name: "City".into(),
        ..Default::default()
    });
    let err = ingest_records(
        &mut VecSource::new(vec![Record::Ontology(orphaning)]),
        &graph,
        Some(&store),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, IngestError::Graph(_)), "{err}");
    assert_eq!(
        store.records_written(),
        written_before,
        "the refused schema is not on disk: {:?}",
        kinds(&store)
    );
    assert!(
        graph.ontology().concept_types.contains_key("Person"),
        "the live schema is unchanged"
    );
    // And the journal still replays.
    let fresh = OntologyGraph::with_arc(ontology());
    store.load_into(&fresh).await.unwrap();
    assert_eq!(fresh.concept_count(), 1);
}

/// Identical `RuleTypeDecl` / `ActionTypeDecl` / `RelationTypeDecl`
/// redeclarations neither journal the schema nor break the concept batch;
/// changed ones are applied and journaled as a single `Ontology` record.
#[tokio::test]
async fn identical_rule_and_action_redeclarations_are_no_ops_and_changed_ones_journal_once() {
    let graph = OntologyGraph::with_arc(ontology());
    let store = FlakyStore::new();
    let mut records = Vec::new();
    for chunk in 0..4 {
        records.extend((0..5).map(|i| person(&format!("p{chunk}-{i}"))));
        match chunk {
            0 => records.push(Record::RuleTypeDecl(rule_type(""))),
            1 => records.push(Record::ActionTypeDecl(action_type(""))),
            _ => records.push(Record::RelationTypeDecl(RelationType {
                name: "knows".into(),
                domain: "Person".into(),
                range: "Person".into(),
                ..Default::default()
            })),
        }
    }
    let stats = ingest_records(&mut VecSource::new(records), &graph, Some(&store))
        .await
        .unwrap();
    assert_eq!((stats.concepts, stats.ontology_updates), (20, 0));
    assert_eq!(
        store.batch_calls(),
        1,
        "no-op declarations keep the batch open"
    );
    assert!(!kinds(&store).contains(&"ontology"));

    // Changed declarations: both merged into one schema record, before the
    // next instance.
    let records = vec![
        Record::RuleTypeDecl(rule_type("now documented")),
        Record::ActionTypeDecl(action_type("creates a knows edge")),
        person("z"),
    ];
    let stats = ingest_records(&mut VecSource::new(records), &graph, Some(&store))
        .await
        .unwrap();
    assert_eq!(stats.ontology_updates, 2);
    let k = kinds(&store);
    assert_eq!(&k[20..], &["ontology", "concept"], "{k:?}");
    let onto = graph.ontology();
    assert_eq!(
        onto.rule_type("must_know_someone").unwrap().description,
        "now documented"
    );
    assert_eq!(
        onto.action_type("introduce").unwrap().effect,
        "creates a knows edge"
    );
    let journaled = store
        .records()
        .into_iter()
        .rev()
        .find_map(|r| match r.kind {
            RecordKind::Ontology(o) => Some(o),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        journaled.action_type("introduce").unwrap().effect,
        "creates a knows edge",
        "the single record carries both changes"
    );
}

/// Decision G — a `FragmentTypeDecl` seen before its document type is
/// deferred (not refused) and resolved once the type exists, inheriting its
/// domain. One whose document type never appears is dropped without error,
/// like any other dependent declaration.
#[tokio::test]
async fn a_fragment_type_declaration_waits_for_its_document_type() {
    let graph = OntologyGraph::with_arc(ontology());
    let store = FlakyStore::new();
    let records = vec![
        Record::FragmentTypeDecl {
            document_type: "Memo".into(),
        },
        Record::ConceptTypeDecl(ConceptType {
            name: "Memo".into(),
            ns: Some("notes".into()),
            ..Default::default()
        }),
        Record::Concept(Concept::new(ConceptId(0), "Memo", "m1")),
    ];
    ingest_records(&mut VecSource::new(records), &graph, Some(&store))
        .await
        .unwrap();
    let ftype = fragment_type_name("Memo");
    let frel = fragment_relation_name("Memo");
    let onto = graph.ontology();
    assert_eq!(onto.ns_of_type(&ftype), "notes", "inherited domain");
    let rt = onto.relation_type(&frel).unwrap();
    assert_eq!(
        (rt.domain.as_str(), rt.range.as_str()),
        (ftype.as_str(), "Memo")
    );
    assert!(matches!(
        rt.cardinality,
        ontology_graph::Cardinality::ManyToOne
    ));
    // The deferred declaration is journaled too: replay knows the type.
    let fresh = OntologyGraph::with_arc(ontology());
    store.load_into(&fresh).await.unwrap();
    assert_eq!(fresh.ontology().ns_of_type(&ftype), "notes");

    // Never-declared document type: no error, no type.
    let graph = OntologyGraph::with_arc(ontology());
    let stats = ingest_records(
        &mut VecSource::new(vec![Record::FragmentTypeDecl {
            document_type: "Nowhere".into(),
        }]),
        &graph,
        None,
    )
    .await
    .unwrap();
    assert_eq!(stats.ontology_updates, 0);
    assert!(!graph
        .ontology()
        .concept_types
        .contains_key(&fragment_type_name("Nowhere")));
}

/// R8 for the schema — when the store fails at the schema flush, every
/// un-journaled declaration kind (concept, relation, rule, action and
/// fragment types) is withdrawn from the live ontology, and a later ingest
/// of an instance of a withdrawn type is refused with nothing journaled.
#[tokio::test]
async fn rollback_withdraws_every_declaration_kind_and_their_instances_are_refused_afterwards() {
    let graph = OntologyGraph::with_arc(ontology());
    let store = FlakyStore::new();
    let baseline = graph.ontology();
    store.set_failing(true);
    let records = vec![
        Record::ConceptTypeDecl(ConceptType {
            name: "City".into(),
            ..Default::default()
        }),
        Record::RelationTypeDecl(RelationType {
            name: "likes".into(),
            domain: "Person".into(),
            range: "Person".into(),
            ..Default::default()
        }),
        Record::FragmentTypeDecl {
            document_type: "Person".into(),
        },
        Record::RuleTypeDecl(RuleType {
            name: "city_rule".into(),
            applies_to: vec!["City".into()],
            ..rule_type("")
        }),
        Record::ActionTypeDecl(ActionType {
            name: "visit".into(),
            subject: "Person".into(),
            object: Some("City".into()),
            ..action_type("")
        }),
        Record::Concept(Concept::new(ConceptId(0), "City", "Geneva")),
    ];
    let err = ingest_records(&mut VecSource::new(records), &graph, Some(&store))
        .await
        .unwrap_err();
    assert!(matches!(err, IngestError::Store(_)), "{err}");
    assert_eq!(store.records_written(), 0);
    let onto = graph.ontology();
    for (present, what) in [
        (onto.concept_types.contains_key("City"), "City"),
        (onto.relation_types.contains_key("likes"), "likes"),
        (
            onto.concept_types
                .contains_key(&fragment_type_name("Person")),
            "PersonFragment",
        ),
        (
            onto.relation_types
                .contains_key(&fragment_relation_name("Person")),
            "fragment_of_person",
        ),
        (onto.rule_type("city_rule").is_some(), "city_rule"),
        (onto.action_type("visit").is_some(), "visit"),
    ] {
        assert!(!present, "{what} must be rolled back");
    }
    assert_eq!(onto.concept_types.len(), baseline.concept_types.len());
    assert_eq!(onto.relation_types.len(), baseline.relation_types.len());

    // Instances of the withdrawn types are refused, before anything is
    // journaled, now that the store works again.
    store.set_failing(false);
    let err = ingest_records(
        &mut VecSource::new(vec![Record::Concept(Concept::new(
            ConceptId(0),
            "City",
            "Geneva",
        ))]),
        &graph,
        Some(&store),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, IngestError::Graph(_)), "{err}");
    assert!(err.to_string().contains("City"), "{err}");
    ingest_records(
        &mut VecSource::new(vec![person("a"), person("b")]),
        &graph,
        Some(&store),
    )
    .await
    .unwrap();
    let err = ingest_records(
        &mut VecSource::new(vec![named("likes", "a", "b")]),
        &graph,
        Some(&store),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, IngestError::Graph(_)), "{err}");
    assert_eq!(store.records_written(), 2, "only a and b are on disk");
    assert_eq!(graph.relation_count(), 0);
}

/// `Rule` and `Action` instances flush the concept batch, are journaled
/// one by one after it, and replay with the same ids and references.
#[tokio::test]
async fn rule_and_action_instances_are_journaled_after_the_barrier_and_replay() {
    let graph = OntologyGraph::with_arc(ontology());
    let store = FlakyStore::new();
    let mut rule = Rule::new(RuleId(0), "must_know_someone", "r-a");
    rule.applies_to = vec![ConceptId(1)];
    rule.strict = true;
    let mut action = Action::new(ActionId(0), "introduce", "a-meets-b", ConceptId(1));
    action.object = Some(ConceptId(2));
    let records = vec![
        person_with_id(1, "a"),
        person_with_id(2, "b"),
        Record::Rule(rule),
        Record::Action(action),
    ];
    let stats = ingest_records(&mut VecSource::new(records), &graph, Some(&store))
        .await
        .unwrap();
    assert_eq!((stats.concepts, stats.rules, stats.actions), (2, 1, 1));
    assert_eq!(kinds(&store), vec!["concept", "concept", "rule", "action"]);
    assert_eq!((store.batch_calls(), store.append_calls()), (1, 2));

    let fresh = OntologyGraph::with_arc(ontology());
    store.load_into(&fresh).await.unwrap();
    assert_eq!((fresh.rule_count(), fresh.action_count()), (1, 1));
    let live = graph.all_rules().pop().unwrap();
    let replayed = fresh.get_rule(live.id).unwrap();
    assert_eq!(replayed.applies_to, vec![ConceptId(1)]);
    assert!(replayed.strict);
    let live = graph.all_actions().pop().unwrap();
    let replayed = fresh.get_action(live.id).unwrap();
    assert_eq!(
        (replayed.subject, replayed.object),
        (ConceptId(1), Some(ConceptId(2)))
    );
}

/// A `Rule` / `Action` whose referenced concept never arrives is an error,
/// not a silently dropped instance: the stats must never claim less than
/// the source contained without saying so.
#[tokio::test]
async fn an_unresolvable_rule_or_action_is_an_error_not_a_silent_drop() {
    let graph = OntologyGraph::with_arc(ontology());
    let store = FlakyStore::new();
    let mut rule = Rule::new(RuleId(0), "must_know_someone", "dangling");
    rule.applies_to = vec![ConceptId(99)];
    let err = ingest_records(
        &mut VecSource::new(vec![person("a"), Record::Rule(rule)]),
        &graph,
        Some(&store),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, IngestError::Graph(_)), "{err}");
    assert!(
        err.to_string().contains("99"),
        "names the missing concept: {err}"
    );
    assert_eq!(graph.rule_count(), 0);
    assert_eq!(
        kinds(&store),
        vec!["concept"],
        "a is durable, the rule is not"
    );

    let action = Action::new(ActionId(0), "introduce", "dangling", ConceptId(42));
    let err = ingest_records(
        &mut VecSource::new(vec![Record::Action(action)]),
        &graph,
        Some(&store),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, IngestError::Graph(_)), "{err}");
    assert!(err.to_string().contains("42"), "{err}");
    assert_eq!(graph.action_count(), 0);

    // An unknown rule / action *type* is reported as such.
    let orphan = Rule::new(RuleId(0), "no_such_rule_type", "x");
    let err = ingest_records(
        &mut VecSource::new(vec![Record::Rule(orphan)]),
        &graph,
        None,
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("no_such_rule_type"), "{err}");
}

/// A duplicate name arriving after a barrier is rejected alone: the durable
/// prefix stays applied and replays, nothing of the failing batch lands.
#[tokio::test]
async fn a_duplicate_after_a_barrier_keeps_the_durable_prefix() {
    let graph = OntologyGraph::with_arc(ontology());
    let store = FlakyStore::new();
    let mut records: Vec<Record> = (0..INGEST_BATCH_SIZE)
        .map(|i| person(&format!("p{i}")))
        .collect();
    records.push(person("fresh"));
    records.push(person("P0")); // case-insensitive duplicate of p0
    let err = ingest_records(&mut VecSource::new(records), &graph, Some(&store))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("duplicate"), "{err}");
    assert_eq!(graph.concept_count(), INGEST_BATCH_SIZE);
    assert_eq!(store.records_written() as usize, INGEST_BATCH_SIZE);
    assert!(
        graph.find_by_name("Person", "fresh").is_none(),
        "the batch holding the duplicate is dropped whole"
    );
    let fresh = OntologyGraph::with_arc(ontology());
    store.load_into(&fresh).await.unwrap();
    assert_eq!(fresh.concept_count(), INGEST_BATCH_SIZE);
}

/// A concept of an undeclared type is refused at staging: nothing is
/// journaled, nothing applied, the batch before it is settled cleanly.
#[tokio::test]
async fn a_concept_of_an_unknown_type_is_refused_before_it_is_staged() {
    let graph = OntologyGraph::with_arc(ontology());
    let store = FlakyStore::new();
    let records = vec![
        person("a"),
        Record::Concept(Concept::new(ConceptId(0), "Dragon", "Smaug")),
    ];
    let err = ingest_records(&mut VecSource::new(records), &graph, Some(&store))
        .await
        .unwrap_err();
    assert!(matches!(err, IngestError::Graph(_)), "{err}");
    assert!(err.to_string().contains("Dragon"), "{err}");
    assert_eq!(
        store.records_written(),
        0,
        "the pending batch was dropped with the error"
    );
    assert_eq!(graph.concept_count(), 0);
}
