// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Model serialization: every `PropertyValue` shape survives a JSON round
//! trip inside a `Concept`, patches deserialize with absent fields, and
//! `indexable_text` only exposes textual properties.

use ahash::AHashMap;
use ontology_graph::{
    Action, ActionPatch, Concept, ConceptId, ConceptPatch, PropertyValue, Relation, RelationId,
    Rule, RuleId,
};

fn every_variant() -> AHashMap<String, PropertyValue> {
    let mut m = AHashMap::new();
    m.insert(
        "text".into(),
        PropertyValue::Text("héllo \"quoted\" \\ /".into()),
    );
    m.insert("empty".into(), PropertyValue::Text(String::new()));
    m.insert("int_like".into(), PropertyValue::Number(42.0));
    m.insert("float".into(), PropertyValue::Number(-0.125));
    m.insert("big".into(), PropertyValue::Number(1e300));
    m.insert("yes".into(), PropertyValue::Bool(true));
    m.insert("no".into(), PropertyValue::Bool(false));
    m.insert("empty_list".into(), PropertyValue::List(vec![]));
    m.insert(
        "mixed".into(),
        PropertyValue::List(vec![
            PropertyValue::Text("a".into()),
            PropertyValue::Number(1.0),
            PropertyValue::Bool(false),
        ]),
    );
    m.insert(
        "nested".into(),
        PropertyValue::List(vec![
            PropertyValue::List(vec![PropertyValue::Number(1.0), PropertyValue::Number(2.0)]),
            PropertyValue::List(vec![PropertyValue::List(vec![PropertyValue::Text(
                "deep".into(),
            )])]),
            PropertyValue::List(vec![]),
        ]),
    );
    m
}

/// Every `PropertyValue` variant — including empty, mixed and nested lists
/// — round-trips through JSON inside a `Concept` unchanged.
#[test]
fn concept_round_trips_every_property_value_variant() {
    let mut c = Concept::new(ConceptId(7), "Person", "Ada").with_description("desc");
    c.properties = every_variant();
    let js = serde_json::to_string(&c).unwrap();
    let back: Concept = serde_json::from_str(&js).unwrap();
    assert_eq!(back.id, c.id);
    assert_eq!(back.concept_type, c.concept_type);
    assert_eq!(back.name, c.name);
    assert_eq!(back.description, c.description);
    assert_eq!(back.properties, c.properties);
    // Ids serialize transparently as bare integers.
    assert!(js.contains("\"id\":7"));
    // A second round trip yields the same values (map order is not part of
    // the contract, so compare the decoded form, not the bytes).
    let again: Concept = serde_json::from_str(&serde_json::to_string(&back).unwrap()).unwrap();
    assert_eq!(again.properties, c.properties);
}

/// The untagged representation maps JSON scalars to the expected variant:
/// integers and floats to `Number`, booleans to `Bool`, strings to `Text`,
/// arrays to `List` — and a numeric string stays `Text`.
#[test]
fn property_value_untagged_decoding_is_unambiguous() {
    let decode = |s: &str| serde_json::from_str::<PropertyValue>(s).unwrap();
    assert_eq!(decode("3"), PropertyValue::Number(3.0));
    assert_eq!(decode("-3.5"), PropertyValue::Number(-3.5));
    assert_eq!(decode("true"), PropertyValue::Bool(true));
    assert_eq!(decode("\"true\""), PropertyValue::Text("true".into()));
    assert_eq!(decode("\"3\""), PropertyValue::Text("3".into()));
    assert_eq!(decode("[]"), PropertyValue::List(vec![]));
    assert_eq!(
        decode("[[1],[\"x\",false]]"),
        PropertyValue::List(vec![
            PropertyValue::List(vec![PropertyValue::Number(1.0)]),
            PropertyValue::List(vec![
                PropertyValue::Text("x".into()),
                PropertyValue::Bool(false)
            ]),
        ])
    );
    assert!(serde_json::from_str::<PropertyValue>("null").is_err());
    assert!(serde_json::from_str::<PropertyValue>("{\"a\":1}").is_err());
}

/// Optional fields default when absent, so records written before a field
/// existed keep loading (`STORAGE.md` §5: replay tolerance).
#[test]
fn entities_deserialize_with_optional_fields_absent() {
    let c: Concept =
        serde_json::from_str(r#"{"id":1,"concept_type":"Person","name":"Ada"}"#).unwrap();
    assert_eq!(c.description, "");
    assert!(c.properties.is_empty());
    let r: Relation =
        serde_json::from_str(r#"{"id":1,"relation_type":"knows","source":1,"target":2}"#).unwrap();
    assert_eq!(r.weight, 0.0, "serde default, not Relation::new's 1.0");
    assert!(r.properties.is_empty());
    let rule: Rule = serde_json::from_str(r#"{"id":1,"rule_type":"r","name":"n"}"#).unwrap();
    assert!(rule.applies_to.is_empty() && !rule.strict && rule.when.is_empty());
    let a: Action =
        serde_json::from_str(r#"{"id":1,"action_type":"a","name":"n","subject":3}"#).unwrap();
    assert_eq!(a.object, None);
    assert!(a.parameters.is_empty());
    // Patches: `{}` touches nothing; ActionPatch distinguishes absent
    // object (`None`) from an explicit clear (`Some(None)`).
    let p: ConceptPatch = serde_json::from_str("{}").unwrap();
    assert!(p.name.is_none() && p.description.is_none() && p.properties.is_none());
    let ap: ActionPatch = serde_json::from_str("{}").unwrap();
    assert_eq!(ap.object, None);
    let ap: ActionPatch = serde_json::from_str(r#"{"object":null}"#).unwrap();
    assert_eq!(ap.object, Some(None));
    let ap: ActionPatch = serde_json::from_str(r#"{"object":9}"#).unwrap();
    assert_eq!(ap.object, Some(Some(ConceptId(9))));
    assert!(!serde_json::to_string(&ActionPatch::default())
        .unwrap()
        .contains("object"));
}

/// Relation and Rule/Action round trips preserve every field.
#[test]
fn relation_rule_and_action_round_trip() {
    let mut r = Relation::new(RelationId(3), "knows", ConceptId(1), ConceptId(2));
    r.weight = 0.75;
    r.properties = every_variant();
    let back: Relation = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
    assert_eq!(
        (back.id, back.source, back.target, back.weight),
        (r.id, r.source, r.target, r.weight)
    );
    assert_eq!(back.properties, r.properties);

    let mut rule = Rule::new(RuleId(2), "must_review", "r");
    rule.applies_to = vec![ConceptId(1), ConceptId(2)];
    rule.strict = true;
    rule.when = "w".into();
    rule.then = "t".into();
    rule.properties = every_variant();
    let back: Rule = serde_json::from_str(&serde_json::to_string(&rule).unwrap()).unwrap();
    assert_eq!(back.applies_to, rule.applies_to);
    assert!(back.strict);
    assert_eq!(back.properties, rule.properties);

    let mut a = Action::new(Default::default(), "sign", "s", ConceptId(1));
    a.object = Some(ConceptId(2));
    a.parameters = every_variant();
    let back: Action = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
    assert_eq!(back.object, Some(ConceptId(2)));
    assert_eq!(back.parameters, a.parameters);
}

/// `indexable_text` carries name, description and *textual* properties
/// only — numbers, booleans and lists never leak into the lexical index.
#[test]
fn indexable_text_exposes_only_textual_properties() {
    let mut c = Concept::new(ConceptId(1), "Person", "Ada");
    assert_eq!(c.indexable_text(), "Ada");
    c.description = "mathematician".into();
    assert_eq!(c.indexable_text(), "Ada. mathematician");
    c.properties = every_variant();
    let text = c.indexable_text();
    assert!(text.starts_with("Ada. mathematician"));
    assert!(text.contains("text: héllo"));
    assert!(text.contains(". empty: "));
    assert!(
        !text.contains("42") && !text.contains("true") && !text.contains("deep"),
        "{text}"
    );
}
