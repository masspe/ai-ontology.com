// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Wire formats: the `Record` JSONL contract (kind tags, defaults, the
//! shipped example files) and the LLM review types (`OntologyProposal`,
//! `ApplyDecision`, `ApplyOutcome`, `ConflictKind`).

use ontology_io::{
    ApplyDecision, ApplyOutcome, ConflictInfo, ConflictKind, DecisionAction, LangTag,
    OntologyProposal, ProposalAction, ProposalConcept, ProposalConceptType, ProposalRelation,
    ProposalRelationType, ProposalRule, ProposalSource, Record,
};
use serde_json::{json, Value};

// --------------------------------------------------------------- Record --

/// The `kind` tag is snake_case and carries the variant's fields flat.
#[test]
fn record_json_uses_snake_case_kind_tags() {
    let v = serde_json::to_value(Record::FragmentTypeDecl {
        document_type: "Contract".into(),
    })
    .unwrap();
    assert_eq!(
        v,
        json!({"kind": "fragment_type_decl", "document_type": "Contract"})
    );

    let v = serde_json::to_value(Record::ConceptTypeDecl(ontology_graph::ConceptType {
        name: "Party".into(),
        ..Default::default()
    }))
    .unwrap();
    assert_eq!(v["kind"], "concept_type_decl");
    assert_eq!(v["name"], "Party");

    let v = serde_json::to_value(Record::NamedRelation {
        relation_type: "knows".into(),
        source_type: "Person".into(),
        source_name: "a".into(),
        target_type: "Person".into(),
        target_name: "b".into(),
        weight: 0.5,
    })
    .unwrap();
    assert_eq!(v["kind"], "named_relation");
    assert_eq!(v["weight"], 0.5);

    for (rec, kind) in [
        (
            Record::Ontology(ontology_graph::Ontology::new()),
            "ontology",
        ),
        (
            Record::Concept(ontology_graph::Concept::new(
                ontology_graph::ConceptId(0),
                "T",
                "n",
            )),
            "concept",
        ),
        (
            Record::Rule(ontology_graph::Rule::new(
                ontology_graph::RuleId(0),
                "rt",
                "r",
            )),
            "rule",
        ),
        (
            Record::Action(ontology_graph::Action::new(
                ontology_graph::ActionId(0),
                "at",
                "a",
                ontology_graph::ConceptId(1),
            )),
            "action",
        ),
    ] {
        assert_eq!(serde_json::to_value(rec).unwrap()["kind"], kind);
    }
}

/// A `named_relation` without `weight` (the shape of the shipped example
/// files) defaults to 1.0; an explicit weight is kept; an unknown kind is
/// rejected.
#[test]
fn named_relation_weight_defaults_to_one_and_unknown_kinds_are_rejected() {
    let line = r#"{"kind":"named_relation","relation_type":"between","source_type":"Contract","source_name":"C-1","target_type":"Company","target_name":"Acme"}"#;
    match serde_json::from_str::<Record>(line).unwrap() {
        Record::NamedRelation { weight, .. } => assert_eq!(weight, 1.0),
        other => panic!("{other:?}"),
    }
    let line = r#"{"kind":"named_relation","relation_type":"between","source_type":"Contract","source_name":"C-1","target_type":"Company","target_name":"Acme","weight":0.3}"#;
    match serde_json::from_str::<Record>(line).unwrap() {
        Record::NamedRelation { weight, .. } => assert_eq!(weight, 0.3),
        other => panic!("{other:?}"),
    }
    assert!(serde_json::from_str::<Record>(r#"{"kind":"widget","name":"x"}"#).is_err());
    assert!(
        serde_json::from_str::<Record>(r#"{"name":"x"}"#).is_err(),
        "kind is mandatory"
    );
}

/// Every line of the shipped `examples/finance` JSONL files is a valid
/// `Record`, so the demo seed matches the wire contract this crate reads.
#[test]
fn the_finance_example_jsonl_files_parse_as_records() {
    for file in ["seed.jsonl", "relations.jsonl"] {
        let path = format!(
            "{}/../../examples/finance/{file}",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(&path).unwrap();
        let mut n = 0;
        for (i, line) in text.lines().enumerate() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            serde_json::from_str::<Record>(t).unwrap_or_else(|e| panic!("{file}:{}: {e}", i + 1));
            n += 1;
        }
        assert!(n > 0, "{file} has records");
    }
}

// ------------------------------------------------------------- Proposal --

fn conflict() -> ConflictInfo {
    ConflictInfo {
        kind: ConflictKind::Exists {
            existing_id: "C#7".into(),
            existing_display: "Person:Alice".into(),
        },
        summary: "already exists".into(),
    }
}

fn full_proposal() -> OntologyProposal {
    OntologyProposal {
        source: Some(ProposalSource {
            name: "memo.docx".into(),
            kind: "application/vnd.openxmlformats-officedocument.wordprocessingml.document".into(),
            encoding: "UTF-8".into(),
            had_bom: true,
            provider: "anthropic".into(),
            model: "test-model".into(),
        }),
        language: Some(LangTag {
            code: "fr".into(),
            script: "Latin".into(),
            confidence: 0.9,
        }),
        concept_types: vec![ProposalConceptType {
            client_ref: "ct1".into(),
            name: "Person".into(),
            description: "a human".into(),
            properties: vec!["email".into()],
            parent: None,
            confidence: 0.8,
            conflict: Some(conflict()),
        }],
        relation_types: vec![ProposalRelationType {
            client_ref: "rt1".into(),
            name: "knows".into(),
            domain: "Person".into(),
            range: "Person".into(),
            symmetric: true,
            description: String::new(),
            confidence: 0.7,
            conflict: None,
        }],
        concepts: vec![ProposalConcept {
            client_ref: "c1".into(),
            concept_type: "Person".into(),
            name: "Alice".into(),
            description: String::new(),
            properties: vec![("email".into(), "a@x".into())],
            evidence: Some("Alice signed".into()),
            confidence: 0.95,
            conflict: None,
        }],
        relations: vec![ProposalRelation {
            client_ref: "r1".into(),
            relation_type: "knows".into(),
            source_ref: "c1".into(),
            target_ref: "Person:Bob".into(),
            weight: Some(0.5),
            evidence: None,
            confidence: 0.6,
            conflict: Some(ConflictInfo {
                kind: ConflictKind::DanglingRef {
                    missing_ref: "Person:Bob".into(),
                },
                summary: "unknown".into(),
            }),
        }],
        rules: vec![ProposalRule {
            client_ref: "ru1".into(),
            rule_type: "must".into(),
            name: "must_sign".into(),
            when: "w".into(),
            then: "t".into(),
            applies_to: vec!["c1".into()],
            strict: true,
            description: String::new(),
            evidence: None,
            confidence: 0.5,
            conflict: None,
        }],
        actions: vec![ProposalAction {
            client_ref: "a1".into(),
            action_type: "sign".into(),
            name: "alice_signs".into(),
            subject_ref: "c1".into(),
            object_ref: None,
            parameters: vec![("date".into(), "2026-01-01".into())],
            effect: "signed".into(),
            description: String::new(),
            evidence: None,
            confidence: 0.4,
            conflict: None,
        }],
    }
}

/// A fully populated proposal survives a JSON round-trip byte-for-byte (as
/// a JSON value), and an empty object deserializes to the defaults.
#[test]
fn ontology_proposal_round_trips_and_an_empty_object_is_all_defaults() {
    let p = full_proposal();
    let v1 = serde_json::to_value(&p).unwrap();
    let back: OntologyProposal = serde_json::from_value(v1.clone()).unwrap();
    let v2 = serde_json::to_value(&back).unwrap();
    assert_eq!(v1, v2);
    assert_eq!(v1["language"]["code"], "fr");
    assert_eq!(v1["concept_types"][0]["conflict"]["kind"]["kind"], "exists");
    assert_eq!(
        v1["relations"][0]["conflict"]["kind"]["kind"],
        "dangling_ref"
    );

    let empty: OntologyProposal = serde_json::from_str("{}").unwrap();
    assert!(empty.source.is_none() && empty.language.is_none());
    assert!(
        empty.concept_types.is_empty()
            && empty.relation_types.is_empty()
            && empty.concepts.is_empty()
            && empty.relations.is_empty()
            && empty.rules.is_empty()
            && empty.actions.is_empty()
    );
    assert_eq!(empty.iter_refs().count(), 0);
}

/// Each item only needs its identity fields; everything else defaults
/// (confidence 0, no conflict, no evidence, empty bags).
#[test]
fn proposal_items_need_only_their_identity_fields() {
    let c: ProposalConcept =
        serde_json::from_value(json!({"client_ref": "x", "concept_type": "T", "name": "n"}))
            .unwrap();
    assert_eq!(c.confidence, 0.0);
    assert!(c.conflict.is_none() && c.evidence.is_none());
    assert!(c.properties.is_empty() && c.description.is_empty());
    assert!(
        serde_json::from_value::<ProposalConcept>(json!({"client_ref": "x", "name": "n"})).is_err(),
        "concept_type is mandatory"
    );

    let rt: ProposalRelationType = serde_json::from_value(
        json!({"client_ref": "r", "name": "knows", "domain": "A", "range": "B"}),
    )
    .unwrap();
    assert!(!rt.symmetric);

    let src: ProposalSource = serde_json::from_value(json!({"name": "f.txt"})).unwrap();
    assert!(!src.had_bom && src.encoding.is_empty() && src.provider.is_empty());
}

/// `DecisionAction` and `ApplyOutcome` use snake_case tags; a decision
/// without an action is rejected rather than defaulted.
#[test]
fn decision_actions_and_apply_outcomes_use_snake_case_tags() {
    for (text, expected) in [
        ("merge", DecisionAction::Merge),
        ("create_new", DecisionAction::CreateNew),
        ("skip", DecisionAction::Skip),
    ] {
        let d: ApplyDecision =
            serde_json::from_value(json!({"client_ref": "c1", "action": text})).unwrap();
        assert_eq!(d.action, expected);
        assert_eq!(serde_json::to_value(&d).unwrap()["action"], text);
    }
    assert!(serde_json::from_value::<ApplyDecision>(json!({"client_ref": "c1"})).is_err());
    assert!(
        serde_json::from_value::<ApplyDecision>(json!({"client_ref": "c1", "action": "Merge"}))
            .is_err(),
        "tags are case-sensitive"
    );

    assert_eq!(
        serde_json::to_value(ApplyOutcome::Created { id: "7".into() }).unwrap(),
        json!({"status": "created", "id": "7"})
    );
    assert_eq!(
        serde_json::to_value(ApplyOutcome::Skipped).unwrap(),
        json!({"status": "skipped"})
    );
    assert_eq!(
        serde_json::to_value(ApplyOutcome::Failed {
            error: "boom".into()
        })
        .unwrap(),
        json!({"status": "failed", "error": "boom"})
    );
    let v = serde_json::to_value(ConflictKind::TypeMismatch {
        existing_type: "Company".into(),
        existing_id: "C#3".into(),
    })
    .unwrap();
    assert_eq!(
        v,
        json!({"kind": "type_mismatch", "existing_type": "Company", "existing_id": "C#3"})
    );
    let _: Value = v;
}

/// `iter_refs` walks types first, then instances, each group in insertion
/// order — the order the apply step resolves references in.
#[test]
fn iter_refs_walks_types_then_instances_in_declaration_order() {
    let p = full_proposal();
    let refs: Vec<&str> = p.iter_refs().collect();
    assert_eq!(refs, vec!["ct1", "rt1", "c1", "r1", "ru1", "a1"]);
}
