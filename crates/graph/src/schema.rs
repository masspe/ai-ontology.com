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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    /// Properties that every instance of this type must define.
    #[serde(default)]
    pub required_properties: Vec<String>,
    /// Sibling concept types that an instance of this type cannot also share
    /// a (type, lowercase-name) identity with. Auto-symmetrised at insert.
    #[serde(default)]
    pub disjoint_with: Vec<String>,
}

/// An edge type in the ontology, e.g. `authored`, `treats`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    #[serde(default)]
    pub transitive: bool,
    /// Name of the relation type whose adjacency is the converse of this one.
    /// Traversals expose virtual edges in both directions when set.
    #[serde(default)]
    pub inverse_of: Option<String>,
    /// Hard ≤1 outgoing of this type per source, independent of cardinality.
    #[serde(default)]
    pub functional: bool,
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
    #[serde(skip)]
    descendants_cache: std::sync::OnceLock<AHashMap<String, Vec<String>>>,
}

impl Ontology {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_concept_type(&mut self, mut ct: ConceptType) {
        // Auto-symmetrise disjoint_with both ways: pull in back-edges from
        // any existing sibling that already names us, and push forward-edges
        // into our declared siblings.
        let name = ct.name.clone();
        for (other_name, other) in self.concept_types.iter() {
            if other.disjoint_with.iter().any(|n| n == &name)
                && !ct.disjoint_with.iter().any(|n| n == other_name)
            {
                ct.disjoint_with.push(other_name.clone());
            }
        }
        let siblings = ct.disjoint_with.clone();
        self.concept_types.insert(ct.name.clone(), ct);
        for other in siblings {
            if let Some(o) = self.concept_types.get_mut(&other) {
                if !o.disjoint_with.iter().any(|n| n == &name) {
                    o.disjoint_with.push(name.clone());
                }
            }
        }
        self.invalidate_caches();
    }

    pub fn add_relation_type(&mut self, rt: RelationType) -> GraphResult<()> {
        if !self.concept_types.contains_key(&rt.domain) {
            return Err(GraphError::UnknownConceptType(rt.domain));
        }
        if !self.concept_types.contains_key(&rt.range) {
            return Err(GraphError::UnknownConceptType(rt.range));
        }
        if let Some(inv) = &rt.inverse_of {
            if inv.is_empty() {
                return Err(GraphError::UnknownRelationType(inv.clone()));
            }
            if inv == &rt.name && !rt.symmetric {
                return Err(GraphError::UnknownRelationType(inv.clone()));
            }
        }
        self.relation_types.insert(rt.name.clone(), rt);
        self.invalidate_caches();
        Ok(())
    }

    /// Cross-check every `inverse_of` reference. Run after all relation types
    /// are loaded, since inverses may be declared out-of-order.
    pub fn validate_inverses(&self) -> GraphResult<()> {
        for rt in self.relation_types.values() {
            if let Some(inv_name) = &rt.inverse_of {
                let inv = self
                    .relation_types
                    .get(inv_name)
                    .ok_or_else(|| GraphError::UnknownRelationType(inv_name.clone()))?;
                if inv.domain != rt.range || inv.range != rt.domain {
                    return Err(GraphError::SchemaViolation {
                        relation: format!("inverse_of {}", rt.name),
                        source_type: inv.domain.clone(),
                        target_type: inv.range.clone(),
                        expected_domain: rt.range.clone(),
                        expected_range: rt.domain.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    /// All concept type names that are `ancestor` or transitively inherit
    /// from it. Includes `ancestor` itself. Empty slice if unknown.
    pub fn descendants(&self, ancestor: &str) -> Vec<String> {
        let cache = self
            .descendants_cache
            .get_or_init(|| self.build_descendants_index());
        cache.get(ancestor).cloned().unwrap_or_default()
    }

    fn build_descendants_index(&self) -> AHashMap<String, Vec<String>> {
        let mut children: AHashMap<&str, Vec<&str>> = AHashMap::new();
        for ct in self.concept_types.values() {
            if let Some(p) = &ct.parent {
                children.entry(p.as_str()).or_default().push(ct.name.as_str());
            }
        }
        let mut out: AHashMap<String, Vec<String>> = AHashMap::new();
        for name in self.concept_types.keys() {
            let mut acc = vec![name.clone()];
            let mut stack: Vec<&str> = children.get(name.as_str()).cloned().unwrap_or_default();
            while let Some(c) = stack.pop() {
                acc.push(c.to_string());
                if let Some(grand) = children.get(c) {
                    stack.extend(grand);
                }
            }
            acc.sort();
            acc.dedup();
            out.insert(name.clone(), acc);
        }
        out
    }

    fn invalidate_caches(&mut self) {
        self.descendants_cache = std::sync::OnceLock::new();
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
        self.invalidate_caches();
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
        self.invalidate_caches();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_inverses_happy_and_sad() {
        let mut o = Ontology::new();
        o.add_concept_type(ConceptType { name: "A".into(), ..Default::default() });
        o.add_concept_type(ConceptType { name: "B".into(), ..Default::default() });
        o.add_relation_type(RelationType {
            name: "fwd".into(),
            domain: "A".into(),
            range: "B".into(),
            ..Default::default()
        })
        .unwrap();
        o.add_relation_type(RelationType {
            name: "bwd".into(),
            domain: "B".into(),
            range: "A".into(),
            inverse_of: Some("fwd".into()),
            ..Default::default()
        })
        .unwrap();
        o.validate_inverses().unwrap();

        // Sad path: inverse points at non-existent type.
        let mut bad = o.clone();
        bad.relation_types.get_mut("bwd").unwrap().inverse_of = Some("nope".into());
        assert!(bad.validate_inverses().is_err());

        // Sad path: domain/range don't actually mirror.
        let mut bad2 = o.clone();
        bad2.relation_types.get_mut("bwd").unwrap().range = "B".into();
        assert!(bad2.validate_inverses().is_err());
    }
}
