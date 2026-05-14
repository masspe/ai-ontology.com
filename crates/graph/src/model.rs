// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

use ahash::AHashMap;
use serde::{Deserialize, Serialize};

use crate::id::{ActionId, ConceptId, RelationId, RuleId};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PropertyValue {
    Text(String),
    Number(f64),
    Bool(bool),
    List(Vec<PropertyValue>),
}

impl PropertyValue {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            PropertyValue::Text(t) => Some(t),
            _ => None,
        }
    }
}

pub type Property = (String, PropertyValue);

/// A node in the graph. `name` is a human-readable label, unique per
/// concept type, used as the natural-language anchor in RAG retrieval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Concept {
    pub id: ConceptId,
    pub concept_type: String,
    pub name: String,
    /// Optional free-text description; embedded by the index for vector search.
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub properties: AHashMap<String, PropertyValue>,
}

impl Concept {
    pub fn new(id: ConceptId, concept_type: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id,
            concept_type: concept_type.into(),
            name: name.into(),
            description: String::new(),
            properties: AHashMap::new(),
        }
    }
    pub fn with_description(mut self, d: impl Into<String>) -> Self {
        self.description = d.into();
        self
    }
    pub fn with_property(mut self, k: impl Into<String>, v: PropertyValue) -> Self {
        self.properties.insert(k.into(), v);
        self
    }

    /// Concatenated text used for indexing (lexical + vector).
    pub fn indexable_text(&self) -> String {
        let mut s = String::with_capacity(self.name.len() + self.description.len() + 32);
        s.push_str(&self.name);
        if !self.description.is_empty() {
            s.push_str(". ");
            s.push_str(&self.description);
        }
        for (k, v) in &self.properties {
            if let PropertyValue::Text(t) = v {
                s.push_str(". ");
                s.push_str(k);
                s.push_str(": ");
                s.push_str(t);
            }
        }
        s
    }
}

/// Partial update to an existing concept. Each `Some` field replaces the
/// corresponding field; `None` leaves it untouched. `concept_type` is
/// intentionally absent — changing it would invalidate incident edges.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConceptPatch {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// When `Some`, replaces the property map wholesale. To merge instead,
    /// fetch the concept, modify the map, and pass the merged result.
    #[serde(default)]
    pub properties: Option<AHashMap<String, PropertyValue>>,
}

/// A directed, typed edge between two concepts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relation {
    pub id: RelationId,
    pub relation_type: String,
    pub source: ConceptId,
    pub target: ConceptId,
    /// Provenance / weight attached to the edge.
    #[serde(default)]
    pub weight: f32,
    #[serde(default)]
    pub properties: AHashMap<String, PropertyValue>,
}

impl Relation {
    pub fn new(
        id: RelationId,
        relation_type: impl Into<String>,
        source: ConceptId,
        target: ConceptId,
    ) -> Self {
        Self {
            id: id,
            relation_type: relation_type.into(),
            source,
            target,
            weight: 1.0,
            properties: AHashMap::new(),
        }
    }
}

/// A runtime instance of a [`crate::schema::RuleType`]. Carries the live
/// `when` / `then` expressions plus a free-form property bag for any
/// metadata (severity, owner, ticket id, …) the host application wants
/// to attach.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub id: RuleId,
    /// Name of the [`crate::schema::RuleType`] this rule conforms to.
    pub rule_type: String,
    /// Human-readable name, unique per `rule_type`.
    pub name: String,
    #[serde(default)]
    pub when: String,
    #[serde(default)]
    pub then: String,
    /// Concrete concept ids this rule scopes to. Empty means global.
    #[serde(default)]
    pub applies_to: Vec<ConceptId>,
    #[serde(default)]
    pub strict: bool,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub properties: AHashMap<String, PropertyValue>,
}

impl Rule {
    pub fn new(
        id: RuleId,
        rule_type: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            id,
            rule_type: rule_type.into(),
            name: name.into(),
            when: String::new(),
            then: String::new(),
            applies_to: Vec::new(),
            strict: false,
            description: String::new(),
            properties: AHashMap::new(),
        }
    }
}

/// Partial update to an existing rule. Each `Some` field replaces the
/// corresponding field; `None` leaves it untouched. `rule_type` is
/// intentionally absent — changing it would invalidate type validation
/// already applied at insert time.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RulePatch {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub when: Option<String>,
    #[serde(default)]
    pub then: Option<String>,
    #[serde(default)]
    pub applies_to: Option<Vec<ConceptId>>,
    #[serde(default)]
    pub strict: Option<bool>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub properties: Option<AHashMap<String, PropertyValue>>,
}

/// A runtime instance of a [`crate::schema::ActionType`]. Records a
/// concrete invocation (or invocation template) including the subject /
/// object concept ids and any captured parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    pub id: ActionId,
    /// Name of the [`crate::schema::ActionType`] this action conforms to.
    pub action_type: String,
    /// Human-readable name, unique per `action_type`.
    pub name: String,
    pub subject: ConceptId,
    #[serde(default)]
    pub object: Option<ConceptId>,
    #[serde(default)]
    pub parameters: AHashMap<String, PropertyValue>,
    #[serde(default)]
    pub effect: String,
    #[serde(default)]
    pub description: String,
}

impl Action {
    pub fn new(
        id: ActionId,
        action_type: impl Into<String>,
        name: impl Into<String>,
        subject: ConceptId,
    ) -> Self {
        Self {
            id,
            action_type: action_type.into(),
            name: name.into(),
            subject,
            object: None,
            parameters: AHashMap::new(),
            effect: String::new(),
            description: String::new(),
        }
    }
}

/// Partial update to an existing action. Each `Some` field replaces the
/// corresponding field; `None` leaves it untouched. `action_type` is
/// intentionally absent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActionPatch {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub subject: Option<ConceptId>,
    /// Replaces `object`. Use `Some(None)` to clear, `Some(Some(id))` to set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<Option<ConceptId>>,
    #[serde(default)]
    pub parameters: Option<AHashMap<String, PropertyValue>>,
    #[serde(default)]
    pub effect: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Partial update to an existing relation. Only edge metadata is mutable;
/// `source`, `target`, and `relation_type` are immutable because they
/// underpin the materialized adjacency index.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RelationPatch {
    #[serde(default)]
    pub weight: Option<f32>,
    #[serde(default)]
    pub properties: Option<AHashMap<String, PropertyValue>>,
}
