// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! `export_graph` → `JsonlSink` → `JsonlSource` → `ingest_records` into a
//! fresh graph reproduces the schema (domains included), concept ids,
//! relations (symmetric ones once), rules and actions.

mod hardening_common;

use hardening_common::tempdir;
use ontology_graph::{
    Action, ActionId, ActionType, Cardinality, Concept, ConceptId, ConceptType, Ontology,
    OntologyGraph, PropertyValue, Relation, RelationId, RelationType, Rule, RuleId, RuleType,
};
use ontology_io::{export_graph, ingest_records, JsonlSink, JsonlSource};

fn ontology() -> Ontology {
    let mut o = Ontology::new();
    o.add_concept_type(ConceptType {
        name: "Person".into(),
        ns: Some("parties".into()),
        description: "an individual".into(),
        ..Default::default()
    });
    o.add_concept_type(ConceptType {
        name: "Employee".into(),
        parent: Some("Person".into()),
        ..Default::default()
    });
    o.add_concept_type(ConceptType {
        name: "Contract".into(),
        ns: Some("contrats".into()),
        required_properties: vec!["ref".into()],
        ..Default::default()
    });
    o.add_relation_type(RelationType {
        name: "signed".into(),
        domain: "Person".into(),
        range: "Contract".into(),
        cardinality: Cardinality::ManyToMany,
        ..Default::default()
    })
    .unwrap();
    o.add_relation_type(RelationType {
        name: "knows".into(),
        domain: "Person".into(),
        range: "Person".into(),
        symmetric: true,
        ..Default::default()
    })
    .unwrap();
    o.add_rule_type(RuleType {
        name: "must_be_signed".into(),
        when: "a Contract exists".into(),
        then: "someone signed it".into(),
        applies_to: vec!["Contract".into()],
        strict: true,
        description: String::new(),
    })
    .unwrap();
    o.add_action_type(ActionType {
        name: "sign".into(),
        subject: "Person".into(),
        object: Some("Contract".into()),
        parameters: vec!["date".into()],
        effect: "creates a signed edge".into(),
        description: String::new(),
    })
    .unwrap();
    o
}

/// The whole graph — ontology with `ns`, concepts with their ids and
/// properties, relations (each symmetric pair once), rules and actions —
/// comes back identical from its JSONL export.
#[tokio::test]
async fn export_then_reingest_preserves_schema_with_ns_rules_and_actions() {
    let dir = tempdir("export-full");
    let path = dir.join("graph.jsonl");

    let g1 = OntologyGraph::with_arc(ontology());
    let alice = g1
        .upsert_concept(
            Concept::new(ConceptId(0), "Person", "Alice")
                .with_property("email", PropertyValue::Text("a@x".into())),
        )
        .unwrap();
    let bob = g1
        .upsert_concept(Concept::new(ConceptId(0), "Employee", "Bob"))
        .unwrap();
    let c1 = g1
        .upsert_concept(
            Concept::new(ConceptId(0), "Contract", "C-1")
                .with_property("ref", PropertyValue::Text("2026/1".into()))
                .with_property("amount", PropertyValue::Number(1_200.5)),
        )
        .unwrap();
    g1.add_relation(Relation::new(RelationId(0), "signed", alice, c1))
        .unwrap();
    g1.add_relation(Relation::new(RelationId(0), "knows", alice, bob))
        .unwrap();
    assert_eq!(g1.relation_count(), 3, "signed + knows + its inverse");
    let mut rule = Rule::new(RuleId(0), "must_be_signed", "C-1 must be signed");
    rule.applies_to = vec![c1];
    rule.strict = true;
    rule.when = "C-1 exists".into();
    let rule_id = g1.upsert_rule(rule).unwrap();
    let mut action = Action::new(ActionId(0), "sign", "alice signs C-1", alice);
    action.object = Some(c1);
    action
        .parameters
        .insert("date".into(), PropertyValue::Text("2026-01-01".into()));
    action.effect = "signed".into();
    let action_id = g1.upsert_action(action).unwrap();

    let mut sink = JsonlSink::create(&path).await.unwrap();
    let stats = export_graph(&g1, &mut sink).await.unwrap();
    assert_eq!(
        (stats.concepts, stats.relations, stats.rules, stats.actions),
        (3, 2, 1, 1),
        "the symmetric pair is exported once"
    );

    // Order on the wire: schema first, then all concepts, then the rest.
    let text = std::fs::read_to_string(&path).unwrap();
    let kinds: Vec<String> = text
        .lines()
        .map(|l| {
            serde_json::from_str::<serde_json::Value>(l).unwrap()["kind"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(kinds[0], "ontology");
    let last_concept = kinds.iter().rposition(|k| k == "concept").unwrap();
    let first_other = kinds
        .iter()
        .position(|k| matches!(k.as_str(), "relation" | "rule" | "action"))
        .unwrap();
    assert!(last_concept < first_other, "{kinds:?}");
    assert_eq!(kinds.len(), 1 + 3 + 2 + 1 + 1);

    let g2 = OntologyGraph::with_arc(Ontology::new());
    let mut src = JsonlSource::open(&path).await.unwrap();
    let stats = ingest_records(&mut src, &g2, None).await.unwrap();
    assert_eq!(
        (
            stats.ontology_updates,
            stats.concepts,
            stats.relations,
            stats.rules,
            stats.actions
        ),
        (1, 3, 2, 1, 1)
    );

    // Schema, domains included (AHashMap order is irrelevant as JSON values).
    assert_eq!(
        serde_json::to_value(g1.ontology()).unwrap(),
        serde_json::to_value(g2.ontology()).unwrap()
    );
    let o2 = g2.ontology();
    assert_eq!(o2.ns_of_type("Person"), "parties");
    assert_eq!(o2.ns_of_type("Employee"), "parties", "inherited via parent");
    assert_eq!(o2.ns_of_type("Contract"), "contrats");

    // Same ids, same properties.
    assert_eq!(g2.find_by_name("Person", "Alice"), Some(alice));
    assert_eq!(g2.find_by_name("Employee", "Bob"), Some(bob));
    assert_eq!(g2.find_by_name("Contract", "C-1"), Some(c1));
    assert_eq!(
        g2.get_concept(c1).unwrap().properties.get("amount"),
        Some(&PropertyValue::Number(1_200.5))
    );

    // Relations: the inverse is materialized again, once.
    assert_eq!(g2.relation_count(), 3);
    assert_eq!(g2.outgoing(bob).len(), 1, "bob knows alice (inverse)");
    assert!(g2
        .outgoing(alice)
        .iter()
        .any(|r| r.relation_type == "signed" && r.target == c1));

    // Rules and actions, by id.
    let r2 = g2.get_rule(rule_id).unwrap();
    assert_eq!(r2.applies_to, vec![c1]);
    assert!(r2.strict);
    assert_eq!(r2.when, "C-1 exists");
    let a2 = g2.get_action(action_id).unwrap();
    assert_eq!((a2.subject, a2.object), (alice, Some(c1)));
    assert_eq!(
        a2.parameters.get("date"),
        Some(&PropertyValue::Text("2026-01-01".into()))
    );
    assert_eq!(a2.effect, "signed");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Exporting an empty graph yields exactly one `ontology` line, which a
/// fresh graph ingests back to the same (empty) schema.
#[tokio::test]
async fn an_empty_graph_exports_its_schema_alone() {
    let dir = tempdir("export-empty");
    let path = dir.join("empty.jsonl");
    let g1 = OntologyGraph::with_arc(ontology());
    let mut sink = JsonlSink::create(&path).await.unwrap();
    let stats = export_graph(&g1, &mut sink).await.unwrap();
    assert_eq!(
        (stats.concepts, stats.relations, stats.rules, stats.actions),
        (0, 0, 0, 0)
    );
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().count(), 1);

    let g2 = OntologyGraph::with_arc(Ontology::new());
    let mut src = JsonlSource::open(&path).await.unwrap();
    ingest_records(&mut src, &g2, None).await.unwrap();
    assert_eq!(
        serde_json::to_value(g1.ontology()).unwrap(),
        serde_json::to_value(g2.ontology()).unwrap()
    );
    let _ = std::fs::remove_dir_all(&dir);
}
