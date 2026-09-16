// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Payload codecs (`STORAGE.md` §7.1, `STORAGE-PLAN.md` phase 4).
//!
//! The container is codec-agnostic: every record header carries the codec
//! byte its payload was written with, so a store may hold JSON sealed
//! segments next to postcard ones and still read everything. The store's
//! *write* codec is `MANIFEST.codec`; it changes only through
//! [`crate::SegmentStore::compact_with_codec`], which rewrites the whole
//! store, so a given segment holds a single codec.
//!
//! # Why a mirror for the binary codec
//!
//! `ontology_graph::PropertyValue` is `#[serde(untagged)]`: that is the
//! shape the HTTP API and the JSON files expose and it must stay so. A
//! self-describing format (JSON) deserializes untagged enums by looking at
//! the value; `postcard` is not self-describing (no `deserialize_any`), so
//! it cannot. The binary codec therefore serializes a **tagged** mirror
//! ([`StoredValue`], [`StoredRecord`]) converted from and to the graph
//! types. The mirror is the on-disk contract of codec 1: its variant order
//! and field order are frozen — append, never reorder.

use ahash::AHashMap;
use ontology_graph::{
    Action, ActionId, Concept, ConceptId, Ontology, PropertyValue, Relation, RelationId, Rule,
    RuleId,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::log::RecordKind;

pub use crate::segment::CODEC_JSON;

/// `postcard` payloads over the tagged mirror below.
pub const CODEC_POSTCARD: u8 = 1;

/// Every codec this build can read and write, in id order.
pub const KNOWN_CODECS: &[u8] = &[CODEC_JSON, CODEC_POSTCARD];

#[derive(Debug, Error)]
pub enum CodecError {
    #[error("codec {0} not supported by this build")]
    Unknown(u8),
    #[error("encode ({codec}): {msg}")]
    Encode { codec: &'static str, msg: String },
    #[error("decode ({codec}): {msg}")]
    Decode { codec: &'static str, msg: String },
}

/// Human name of a codec id (`json`, `postcard`), or `?` when unknown.
pub fn codec_name(codec: u8) -> &'static str {
    match codec {
        CODEC_JSON => "json",
        CODEC_POSTCARD => "postcard",
        _ => "?",
    }
}

/// Parse a codec name or id as given on a command line.
pub fn parse_codec(s: &str) -> Option<u8> {
    match s.trim().to_ascii_lowercase().as_str() {
        "json" | "0" => Some(CODEC_JSON),
        "postcard" | "bin" | "binary" | "1" => Some(CODEC_POSTCARD),
        _ => None,
    }
}

pub fn is_known(codec: u8) -> bool {
    KNOWN_CODECS.contains(&codec)
}

/// Encode a record's payload with `codec`.
pub fn encode(codec: u8, kind: &RecordKind) -> Result<Vec<u8>, CodecError> {
    match codec {
        CODEC_JSON => serde_json::to_vec(kind).map_err(|e| CodecError::Encode {
            codec: "json",
            msg: e.to_string(),
        }),
        CODEC_POSTCARD => {
            postcard::to_allocvec(&StoredRecord::from(kind)).map_err(|e| CodecError::Encode {
                codec: "postcard",
                msg: e.to_string(),
            })
        }
        other => Err(CodecError::Unknown(other)),
    }
}

/// Decode a payload written with `codec` (taken from the record header).
pub fn decode(codec: u8, payload: &[u8]) -> Result<RecordKind, CodecError> {
    match codec {
        CODEC_JSON => serde_json::from_slice(payload).map_err(|e| CodecError::Decode {
            codec: "json",
            msg: e.to_string(),
        }),
        CODEC_POSTCARD => postcard::from_bytes::<StoredRecord>(payload)
            .map(RecordKind::from)
            .map_err(|e| CodecError::Decode {
                codec: "postcard",
                msg: e.to_string(),
            }),
        other => Err(CodecError::Unknown(other)),
    }
}

// ---------------------------------------------------------------------------
// Tagged mirror — the on-disk contract of codec 1. Frozen: append only.
// ---------------------------------------------------------------------------

/// Tagged twin of `PropertyValue` (which is untagged for JSON).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StoredValue {
    Text(String),
    Number(f64),
    Bool(bool),
    List(Vec<StoredValue>),
}

impl From<&PropertyValue> for StoredValue {
    fn from(v: &PropertyValue) -> Self {
        match v {
            PropertyValue::Text(t) => StoredValue::Text(t.clone()),
            PropertyValue::Number(n) => StoredValue::Number(*n),
            PropertyValue::Bool(b) => StoredValue::Bool(*b),
            PropertyValue::List(items) => StoredValue::List(items.iter().map(Self::from).collect()),
        }
    }
}

impl From<StoredValue> for PropertyValue {
    fn from(v: StoredValue) -> Self {
        match v {
            StoredValue::Text(t) => PropertyValue::Text(t),
            StoredValue::Number(n) => PropertyValue::Number(n),
            StoredValue::Bool(b) => PropertyValue::Bool(b),
            StoredValue::List(items) => {
                PropertyValue::List(items.into_iter().map(PropertyValue::from).collect())
            }
        }
    }
}

/// Properties as a sorted list of pairs: deterministic bytes for identical
/// content (a hash map iterates in arbitrary order).
type StoredProps = Vec<(String, StoredValue)>;

fn props_out(m: &AHashMap<String, PropertyValue>) -> StoredProps {
    let mut v: StoredProps = m
        .iter()
        .map(|(k, v)| (k.clone(), StoredValue::from(v)))
        .collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

fn props_in(v: StoredProps) -> AHashMap<String, PropertyValue> {
    v.into_iter()
        .map(|(k, v)| (k, PropertyValue::from(v)))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredConcept {
    pub id: u64,
    pub concept_type: String,
    pub name: String,
    pub description: String,
    pub properties: StoredProps,
}

impl From<&Concept> for StoredConcept {
    fn from(c: &Concept) -> Self {
        Self {
            id: c.id.0,
            concept_type: c.concept_type.clone(),
            name: c.name.clone(),
            description: c.description.clone(),
            properties: props_out(&c.properties),
        }
    }
}

impl From<StoredConcept> for Concept {
    fn from(c: StoredConcept) -> Self {
        Concept {
            id: ConceptId(c.id),
            concept_type: c.concept_type,
            name: c.name,
            description: c.description,
            properties: props_in(c.properties),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredRelation {
    pub id: u64,
    pub relation_type: String,
    pub source: u64,
    pub target: u64,
    pub weight: f32,
    pub properties: StoredProps,
}

impl From<&Relation> for StoredRelation {
    fn from(r: &Relation) -> Self {
        Self {
            id: r.id.0,
            relation_type: r.relation_type.clone(),
            source: r.source.0,
            target: r.target.0,
            weight: r.weight,
            properties: props_out(&r.properties),
        }
    }
}

impl From<StoredRelation> for Relation {
    fn from(r: StoredRelation) -> Self {
        Relation {
            id: RelationId(r.id),
            relation_type: r.relation_type,
            source: ConceptId(r.source),
            target: ConceptId(r.target),
            weight: r.weight,
            properties: props_in(r.properties),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredRule {
    pub id: u64,
    pub rule_type: String,
    pub name: String,
    pub when: String,
    pub then: String,
    pub applies_to: Vec<u64>,
    pub strict: bool,
    pub description: String,
    pub properties: StoredProps,
}

impl From<&Rule> for StoredRule {
    fn from(r: &Rule) -> Self {
        Self {
            id: r.id.0,
            rule_type: r.rule_type.clone(),
            name: r.name.clone(),
            when: r.when.clone(),
            then: r.then.clone(),
            applies_to: r.applies_to.iter().map(|c| c.0).collect(),
            strict: r.strict,
            description: r.description.clone(),
            properties: props_out(&r.properties),
        }
    }
}

impl From<StoredRule> for Rule {
    fn from(r: StoredRule) -> Self {
        Rule {
            id: RuleId(r.id),
            rule_type: r.rule_type,
            name: r.name,
            when: r.when,
            then: r.then,
            applies_to: r.applies_to.into_iter().map(ConceptId).collect(),
            strict: r.strict,
            description: r.description,
            properties: props_in(r.properties),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredAction {
    pub id: u64,
    pub action_type: String,
    pub name: String,
    pub subject: u64,
    pub object: Option<u64>,
    pub parameters: StoredProps,
    pub effect: String,
    pub description: String,
}

impl From<&Action> for StoredAction {
    fn from(a: &Action) -> Self {
        Self {
            id: a.id.0,
            action_type: a.action_type.clone(),
            name: a.name.clone(),
            subject: a.subject.0,
            object: a.object.map(|c| c.0),
            parameters: props_out(&a.parameters),
            effect: a.effect.clone(),
            description: a.description.clone(),
        }
    }
}

impl From<StoredAction> for Action {
    fn from(a: StoredAction) -> Self {
        Action {
            id: ActionId(a.id),
            action_type: a.action_type,
            name: a.name,
            subject: ConceptId(a.subject),
            object: a.object.map(ConceptId),
            parameters: props_in(a.parameters),
            effect: a.effect,
            description: a.description,
        }
    }
}

/// Twin of [`RecordKind`]. The `Ontology` schema carries no untagged type,
/// so it is stored as is (postcard handles its maps and options).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StoredRecord {
    Ontology(Ontology),
    Concept(StoredConcept),
    Relation(StoredRelation),
    UpdateRelation(StoredRelation),
    UpdateConcept(StoredConcept),
    DeleteConcept(u64),
    DeleteRelation(u64),
    Rule(StoredRule),
    Action(StoredAction),
    DeleteRule(u64),
    DeleteAction(u64),
    RelationExact(StoredRelation),
}

impl From<&RecordKind> for StoredRecord {
    fn from(k: &RecordKind) -> Self {
        match k {
            RecordKind::Ontology(o) => StoredRecord::Ontology(o.clone()),
            RecordKind::Concept(c) => StoredRecord::Concept(c.into()),
            RecordKind::Relation(r) => StoredRecord::Relation(r.into()),
            RecordKind::UpdateRelation(r) => StoredRecord::UpdateRelation(r.into()),
            RecordKind::UpdateConcept(c) => StoredRecord::UpdateConcept(c.into()),
            RecordKind::DeleteConcept(id) => StoredRecord::DeleteConcept(id.0),
            RecordKind::DeleteRelation(id) => StoredRecord::DeleteRelation(id.0),
            RecordKind::Rule(r) => StoredRecord::Rule(r.into()),
            RecordKind::Action(a) => StoredRecord::Action(a.into()),
            RecordKind::DeleteRule(id) => StoredRecord::DeleteRule(id.0),
            RecordKind::DeleteAction(id) => StoredRecord::DeleteAction(id.0),
            RecordKind::RelationExact(r) => StoredRecord::RelationExact(r.into()),
        }
    }
}

impl From<StoredRecord> for RecordKind {
    fn from(s: StoredRecord) -> Self {
        match s {
            StoredRecord::Ontology(o) => RecordKind::Ontology(o),
            StoredRecord::Concept(c) => RecordKind::Concept(c.into()),
            StoredRecord::Relation(r) => RecordKind::Relation(r.into()),
            StoredRecord::UpdateRelation(r) => RecordKind::UpdateRelation(r.into()),
            StoredRecord::UpdateConcept(c) => RecordKind::UpdateConcept(c.into()),
            StoredRecord::DeleteConcept(id) => RecordKind::DeleteConcept(ConceptId(id)),
            StoredRecord::DeleteRelation(id) => RecordKind::DeleteRelation(RelationId(id)),
            StoredRecord::Rule(r) => RecordKind::Rule(r.into()),
            StoredRecord::Action(a) => RecordKind::Action(a.into()),
            StoredRecord::DeleteRule(id) => RecordKind::DeleteRule(RuleId(id)),
            StoredRecord::DeleteAction(id) => RecordKind::DeleteAction(ActionId(id)),
            StoredRecord::RelationExact(r) => RecordKind::RelationExact(r.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn concept() -> Concept {
        let mut c = Concept::new(ConceptId(7), "Invoice", "INV-1001")
            .with_description("Q1 milestone — automation pilot");
        c.properties
            .insert("amount_eur".into(), PropertyValue::Number(18000.0));
        c.properties
            .insert("paid".into(), PropertyValue::Bool(false));
        c.properties.insert(
            "tags".into(),
            PropertyValue::List(vec![
                PropertyValue::Text("q1".into()),
                PropertyValue::List(vec![PropertyValue::Number(1.5)]),
            ]),
        );
        c
    }

    #[test]
    fn names_and_ids_round_trip() {
        for &c in KNOWN_CODECS {
            assert_eq!(parse_codec(codec_name(c)), Some(c));
            assert_eq!(parse_codec(&c.to_string()), Some(c));
            assert!(is_known(c));
        }
        assert_eq!(parse_codec("bincode"), None);
        assert!(!is_known(7));
        assert_eq!(codec_name(7), "?");
    }

    #[test]
    fn postcard_round_trips_every_property_value_shape() {
        let kind = RecordKind::Concept(concept());
        let bytes = encode(CODEC_POSTCARD, &kind).unwrap();
        let back = decode(CODEC_POSTCARD, &bytes).unwrap();
        let RecordKind::Concept(c) = back else {
            panic!("kind changed");
        };
        let orig = concept();
        assert_eq!(c.id, orig.id);
        assert_eq!(c.name, orig.name);
        assert_eq!(c.description, orig.description);
        assert_eq!(c.properties.len(), orig.properties.len());
        assert!(matches!(c.properties["amount_eur"], PropertyValue::Number(n) if n == 18000.0));
        assert!(matches!(c.properties["paid"], PropertyValue::Bool(false)));
        assert!(matches!(&c.properties["tags"], PropertyValue::List(items) if items.len() == 2));
    }

    #[test]
    fn postcard_bytes_are_deterministic_and_smaller_than_json() {
        let kind = RecordKind::Concept(concept());
        let a = encode(CODEC_POSTCARD, &kind).unwrap();
        let b = encode(CODEC_POSTCARD, &kind).unwrap();
        assert_eq!(a, b, "sorted properties give identical bytes");
        let json = encode(CODEC_JSON, &kind).unwrap();
        assert!(
            a.len() < json.len(),
            "postcard {} vs json {}",
            a.len(),
            json.len()
        );
    }

    #[test]
    fn f64_bits_survive_postcard_exactly() {
        for v in [
            -1.8854965612953506e-119_f64,
            1e-310,
            f64::MAX,
            f64::MIN_POSITIVE,
            -0.0,
            18000.5,
        ] {
            let mut c = Concept::new(ConceptId(1), "T", "n");
            c.properties.insert("x".into(), PropertyValue::Number(v));
            let kind = RecordKind::Concept(c);
            let bytes = encode(CODEC_POSTCARD, &kind).unwrap();
            let RecordKind::Concept(back) = decode(CODEC_POSTCARD, &bytes).unwrap() else {
                panic!()
            };
            let PropertyValue::Number(got) = back.properties["x"] else {
                panic!()
            };
            assert_eq!(got.to_bits(), v.to_bits(), "{v:e} -> {got:e}");
            // And the JSON view used as the reference by the property tests.
            let j1 = serde_json::to_value(&kind).unwrap();
            let j2 = serde_json::to_value(RecordKind::Concept(back)).unwrap();
            assert_eq!(j1, j2, "json view of {v:e}");
        }
    }

    #[test]
    fn unknown_codec_is_refused_on_both_sides() {
        let kind = RecordKind::DeleteConcept(ConceptId(3));
        assert!(matches!(encode(9, &kind), Err(CodecError::Unknown(9))));
        assert!(matches!(decode(9, b"x"), Err(CodecError::Unknown(9))));
    }

    #[test]
    fn a_json_payload_is_not_mistaken_for_postcard() {
        let kind = RecordKind::DeleteConcept(ConceptId(3));
        let json = encode(CODEC_JSON, &kind).unwrap();
        // Decoding JSON bytes as postcard must fail cleanly, never panic or
        // yield a plausible-looking record.
        match decode(CODEC_POSTCARD, &json) {
            Err(CodecError::Decode {
                codec: "postcard", ..
            }) => {}
            Ok(RecordKind::DeleteConcept(id)) => assert_ne!(id, ConceptId(3)),
            other => panic!("unexpected: {other:?}"),
        }
    }
}
