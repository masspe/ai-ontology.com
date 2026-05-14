// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

use ahash::AHashMap;
use serde::{Deserialize, Serialize};

use crate::error::{GraphError, GraphResult};

/// Cardinality constraint for a relation type.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cardinality {
    OneToOne,
    OneToMany,
    ManyToOne,
    #[default]
    ManyToMany,
}

/// A node type in the ontology, e.g. `Person`, `Paper`, `Drug`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConceptType {
    pub name: String,
    /// Names of properties that may appear on instances of this type.
    /// `None` means open-world (any property allowed).
    #[serde(default)]
    pub properties: Option<Vec<String>>,
    /// Optional parent concept type — instances of `name` are also instances
    /// of any ancestor type. Enables simple subtyping for relation domains.
    #[serde(default)]
    pub parent: Option<String>,
    /// Human-readable description used by the RAG prompt builder.
    #[serde(default)]
    pub description: String,
}

/// An edge type in the ontology, e.g. `authored`, `treats`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationType {
    pub name: String,
    pub domain: String,
    pub range: String,
    #[serde(default)]
    pub cardinality: Cardinality,
    #[serde(default)]
    pub symmetric: bool,
    #[serde(default)]
    pub description: String,
}

/// A declarative rule attached to the ontology, e.g.
/// `"every Invoice must reference a Contract"`.
///
/// Rules are intentionally free-form (textual `when` / `then`) so they can
/// describe both hard constraints and softer inference hints used by the
/// RAG prompt. They are not evaluated by the graph engine — they ship to
/// the LLM as part of the ontology context so the model is aware of the
/// domain's expected invariants.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleType {
    pub name: String,
    /// Condition under which the rule applies (free text).
    #[serde(default)]
    pub when: String,
    /// Consequence / expectation when the condition holds (free text).
    #[serde(default)]
    pub then: String,
    /// Optional set of concept types this rule scopes to. Empty / absent
    /// means the rule is global.
    #[serde(default)]
    pub applies_to: Vec<String>,
    /// `true` if violations are hard errors; `false` for advisory rules.
    #[serde(default)]
    pub strict: bool,
    #[serde(default)]
    pub description: String,
}

/// A named action that can be performed on or by an instance of a concept
/// type, e.g. `"sign(Contract)"` or `"issue(Invoice)"`. Like [`RuleType`],
/// actions are declarative metadata surfaced to the LLM — the graph does
/// not execute them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionType {
    pub name: String,
    /// Concept type that performs / receives the action.
    pub subject: String,
    /// Optional concept type the action targets (e.g. `sign` on `Contract`
    /// by a `Person` would have `subject = "Person"`, `object = "Contract"`).
    #[serde(default)]
    pub object: Option<String>,
    /// Named parameters expected by the action.
    #[serde(default)]
    pub parameters: Vec<String>,
    /// Free-text effect description (what changes after the action runs).
    #[serde(default)]
    pub effect: String,
    #[serde(default)]
    pub description: String,
}

/// The full ontology: concept types, relation types, rules and actions.
///
/// `rule_types` and `action_types` default to empty so older ontology JSON
/// files that only declare concepts and relations keep deserializing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Ontology {
    pub concept_types: AHashMap<String, ConceptType>,
    pub relation_types: AHashMap<String, RelationType>,
    #[serde(default)]
    pub rule_types: AHashMap<String, RuleType>,
    #[serde(default)]
    pub action_types: AHashMap<String, ActionType>,
}

impl Ontology {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_concept_type(&mut self, ct: ConceptType) {
        self.concept_types.insert(ct.name.clone(), ct);
    }

    pub fn add_relation_type(&mut self, rt: RelationType) -> GraphResult<()> {
        if !self.concept_types.contains_key(&rt.domain) {
            return Err(GraphError::UnknownConceptType(rt.domain));
        }
        if !self.concept_types.contains_key(&rt.range) {
            return Err(GraphError::UnknownConceptType(rt.range));
        }
        self.relation_types.insert(rt.name.clone(), rt);
        Ok(())
    }

    pub fn concept_type(&self, name: &str) -> GraphResult<&ConceptType> {
        self.concept_types
            .get(name)
            .ok_or_else(|| GraphError::UnknownConceptType(name.to_string()))
    }

    pub fn relation_type(&self, name: &str) -> GraphResult<&RelationType> {
        self.relation_types
            .get(name)
            .ok_or_else(|| GraphError::UnknownRelationType(name.to_string()))
    }

    /// Register a rule. Any concept type listed in `applies_to` must be
    /// defined in the ontology, otherwise the rule is rejected.
    pub fn add_rule_type(&mut self, rule: RuleType) -> GraphResult<()> {
        for ct in &rule.applies_to {
            if !self.concept_types.contains_key(ct) {
                return Err(GraphError::UnknownConceptType(ct.clone()));
            }
        }
        self.rule_types.insert(rule.name.clone(), rule);
        Ok(())
    }

    /// Register an action. `subject` and (when present) `object` must be
    /// known concept types.
    pub fn add_action_type(&mut self, action: ActionType) -> GraphResult<()> {
        if !self.concept_types.contains_key(&action.subject) {
            return Err(GraphError::UnknownConceptType(action.subject.clone()));
        }
        if let Some(obj) = &action.object {
            if !self.concept_types.contains_key(obj) {
                return Err(GraphError::UnknownConceptType(obj.clone()));
            }
        }
        self.action_types.insert(action.name.clone(), action);
        Ok(())
    }

    pub fn rule_type(&self, name: &str) -> Option<&RuleType> {
        self.rule_types.get(name)
    }

    pub fn action_type(&self, name: &str) -> Option<&ActionType> {
        self.action_types.get(name)
    }

    /// Returns true if `child` is `ancestor`, or transitively inherits from it.
    pub fn is_subtype(&self, child: &str, ancestor: &str) -> bool {
        if child == ancestor {
            return true;
        }
        let mut cursor = self.concept_types.get(child);
        while let Some(ct) = cursor {
            match &ct.parent {
                Some(p) if p == ancestor => return true,
                Some(p) => cursor = self.concept_types.get(p),
                None => return false,
            }
        }
        false
    }

    /// Validate that a relation linking the two concept types is permitted.
    pub fn validate_edge(
        &self,
        relation: &str,
        source_type: &str,
        target_type: &str,
    ) -> GraphResult<()> {
        let rt = self.relation_type(relation)?;
        if !self.is_subtype(source_type, &rt.domain) || !self.is_subtype(target_type, &rt.range) {
            return Err(GraphError::SchemaViolation {
                relation: relation.to_string(),
                source_type: source_type.to_string(),
                target_type: target_type.to_string(),
                expected_domain: rt.domain.clone(),
                expected_range: rt.range.clone(),
            });
        }
        Ok(())
    }
}
