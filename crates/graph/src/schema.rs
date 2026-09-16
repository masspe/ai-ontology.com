// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

use ahash::{AHashMap, AHashSet};
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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Storage domain (`STORAGE.md` H13): the unit of retention, compaction,
    /// backup and rights. `None` inherits the parent's domain, or
    /// [`DEFAULT_NS`] for a root type. Identifier `[a-z0-9_-]{1,32}`. Named
    /// `ns`, never `domain` — `domain` is the relation-type source
    /// constraint (`STORAGE.md` §10.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ns: Option<String>,
}

/// Domain of every concept type that declares none (and has no parent).
pub const DEFAULT_NS: &str = "default";
/// Longest accepted domain identifier.
pub const MAX_NS_LEN: usize = 32;

/// `true` when `ns` is a well-formed domain identifier: 1 to 32 chars from
/// `[a-z0-9_-]`. Domain names become directory names on disk.
pub fn is_valid_ns(ns: &str) -> bool {
    !ns.is_empty()
        && ns.len() <= MAX_NS_LEN
        && !RESERVED_NS.contains(&ns)
        && ns
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// Names a domain can never take: `meta` is the schema/rules/actions stream
/// of the store (`STORAGE.md` D3).
pub const RESERVED_NS: &[&str] = &["meta"];

/// An edge type in the ontology, e.g. `authored`, `treats`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

    /// Register a type, or **refresh** an existing one without dropping what
    /// the declaration does not mention: an ingest declaration that only
    /// names a type (`@concept_type Contract`, a text document typed
    /// `Contract`) must not reset its domain, parent, properties or
    /// description. Fields the declaration does set win.
    ///
    /// Returns `false` when the merged type is identical to the registered
    /// one, so callers can skip journaling a schema that did not change
    /// (every text document re-declares its own type).
    pub fn merge_concept_type(&mut self, decl: ConceptType) -> bool {
        let merged = self.merged_concept_type(decl);
        if self.concept_types.get(&merged.name) == Some(&merged) {
            return false;
        }
        self.add_concept_type(merged);
        true
    }

    /// The type `merge_concept_type` would register for `decl`: the
    /// declaration completed with the registered type's unmentioned fields.
    pub fn merged_concept_type(&self, mut decl: ConceptType) -> ConceptType {
        if let Some(existing) = self.concept_types.get(&decl.name) {
            if decl.ns.is_none() {
                decl.ns = existing.ns.clone();
            }
            if decl.parent.is_none() {
                decl.parent = existing.parent.clone();
            }
            if decl.properties.is_none() {
                decl.properties = existing.properties.clone();
            }
            if decl.description.is_empty() {
                decl.description = existing.description.clone();
            }
            if decl.required_properties.is_empty() {
                decl.required_properties = existing.required_properties.clone();
            }
            if decl.disjoint_with.is_empty() {
                decl.disjoint_with = existing.disjoint_with.clone();
            }
        }
        decl
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
                children
                    .entry(p.as_str())
                    .or_default()
                    .push(ct.name.as_str());
            }
        }
        let mut out: AHashMap<String, Vec<String>> = AHashMap::new();
        for name in self.concept_types.keys() {
            let mut acc = vec![name.clone()];
            // `seen` bounds the walk: a parent cycle (refused by
            // `validate_hierarchy`, but the read path must never rely on
            // it) would otherwise loop forever.
            let mut seen: AHashSet<&str> = AHashSet::new();
            seen.insert(name.as_str());
            let mut stack: Vec<&str> = children.get(name.as_str()).cloned().unwrap_or_default();
            while let Some(c) = stack.pop() {
                if !seen.insert(c) {
                    continue;
                }
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

    /// The `parent` chains must be acyclic: a type that is its own
    /// ancestor has no effective domain, no finite descendant set and no
    /// subtype answer. Unknown parents are tolerated here (the chain simply
    /// stops), so out-of-order declarations keep working.
    pub fn validate_hierarchy(&self) -> GraphResult<()> {
        for start in self.concept_types.values() {
            let mut cursor = start.parent.as_deref();
            let mut hops = 0usize;
            while let Some(p) = cursor {
                if p == start.name {
                    return Err(GraphError::ParentCycle {
                        concept_type: start.name.clone(),
                    });
                }
                hops += 1;
                if hops > self.concept_types.len() {
                    // Only reachable through a cycle that does not pass
                    // through `start`; `start` is still on it.
                    return Err(GraphError::ParentCycle {
                        concept_type: start.name.clone(),
                    });
                }
                cursor = self
                    .concept_types
                    .get(p)
                    .and_then(|ct| ct.parent.as_deref());
            }
        }
        Ok(())
    }

    fn invalidate_caches(&mut self) {
        self.descendants_cache = std::sync::OnceLock::new();
    }

    pub fn concept_type(&self, name: &str) -> GraphResult<&ConceptType> {
        self.concept_types
            .get(name)
            .ok_or_else(|| GraphError::UnknownConceptType(name.to_string()))
    }

    /// Effective storage domain of a concept type: its own `ns`, else the
    /// nearest ancestor's, else [`DEFAULT_NS`]. Unknown types resolve to
    /// [`DEFAULT_NS`] too, so a caller can route without a second lookup;
    /// validate existence separately. Static per H13.
    pub fn ns_of_type(&self, name: &str) -> &str {
        let mut cursor = self.concept_types.get(name);
        let mut hops = 0;
        while let Some(ct) = cursor {
            if let Some(ns) = &ct.ns {
                return ns.as_str();
            }
            cursor = ct.parent.as_deref().and_then(|p| self.concept_types.get(p));
            hops += 1;
            if hops > self.concept_types.len() {
                break; // defensive: a parent cycle must not loop forever
            }
        }
        DEFAULT_NS
    }

    /// `(source_ns, target_ns)` of a relation type through its `domain` /
    /// `range` concept types (H14): a relation is routable before its
    /// endpoints are inspected.
    pub fn ns_of_relation_type(&self, name: &str) -> GraphResult<(&str, &str)> {
        let rt = self.relation_type(name)?;
        Ok((self.ns_of_type(&rt.domain), self.ns_of_type(&rt.range)))
    }

    /// Every distinct domain declared or inherited by the concept types,
    /// sorted. Always contains [`DEFAULT_NS`] if any type resolves to it.
    pub fn namespaces(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .concept_types
            .keys()
            .map(|t| self.ns_of_type(t).to_string())
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Domain rules (STORAGE-PLAN.md §5.1): every declared `ns` is a valid
    /// identifier, and a child type that declares a `ns` declares the same
    /// one as its parent — otherwise `?type=Parent&include_subtypes` would
    /// silently span domains.
    pub fn validate_namespaces(&self) -> GraphResult<()> {
        for ct in self.concept_types.values() {
            if let Some(ns) = &ct.ns {
                if !is_valid_ns(ns) {
                    return Err(GraphError::InvalidNamespace {
                        concept_type: ct.name.clone(),
                        ns: ns.clone(),
                    });
                }
                if let Some(parent) = &ct.parent {
                    if self.concept_types.contains_key(parent) {
                        let parent_ns = self.ns_of_type(parent);
                        if parent_ns != ns {
                            return Err(GraphError::NamespaceMismatch {
                                concept_type: ct.name.clone(),
                                ns: ns.clone(),
                                parent: parent.clone(),
                                parent_ns: parent_ns.to_string(),
                            });
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Mutable access to a concept type; caches are invalidated. Used to
    /// build schema variants (tests, tooling).
    pub fn concept_type_mut(&mut self, name: &str) -> &mut ConceptType {
        self.invalidate_caches();
        self.concept_types
            .get_mut(name)
            .unwrap_or_else(|| panic!("unknown concept type `{name}`"))
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
        let mut hops = 0usize;
        while let Some(ct) = cursor {
            match &ct.parent {
                Some(p) if p == ancestor => return true,
                Some(p) => cursor = self.concept_types.get(p),
                None => return false,
            }
            hops += 1;
            if hops > self.concept_types.len() {
                return false; // defensive: a parent cycle must not loop forever
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

    fn ct(name: &str, parent: Option<&str>, ns: Option<&str>) -> ConceptType {
        ConceptType {
            name: name.into(),
            parent: parent.map(str::to_string),
            ns: ns.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn ns_identifiers_are_validated() {
        assert!(is_valid_ns("default"));
        assert!(is_valid_ns("facturation-2026_v2"));
        assert!(!is_valid_ns(""));
        assert!(!is_valid_ns("meta"), "reserved for the schema stream");
        assert!(!is_valid_ns("Modele"));
        assert!(!is_valid_ns("a b"));
        assert!(!is_valid_ns("é"));
        assert!(!is_valid_ns(&"x".repeat(MAX_NS_LEN + 1)));
        assert!(is_valid_ns(&"x".repeat(MAX_NS_LEN)));
    }

    #[test]
    fn merge_keeps_what_a_bare_declaration_does_not_mention() {
        let mut o = Ontology::new();
        let mut full = ct("Contract", None, Some("contrats"));
        full.description = "a legal agreement".into();
        full.properties = Some(vec!["amount".into()]);
        o.add_concept_type(full);
        // A bare re-declaration (name only) changes nothing.
        o.merge_concept_type(ct("Contract", None, None));
        let c = o.concept_type("Contract").unwrap();
        assert_eq!(c.ns.as_deref(), Some("contrats"));
        assert_eq!(c.description, "a legal agreement");
        assert_eq!(c.properties.as_deref(), Some(&["amount".to_string()][..]));
        // Fields the declaration sets win.
        let mut renamed = ct("Contract", None, None);
        renamed.description = "updated".into();
        o.merge_concept_type(renamed);
        let c = o.concept_type("Contract").unwrap();
        assert_eq!(c.description, "updated");
        assert_eq!(c.ns.as_deref(), Some("contrats"));
        // An unknown type is simply added.
        o.merge_concept_type(ct("Invoice", None, Some("facturation")));
        assert_eq!(o.ns_of_type("Invoice"), "facturation");
    }

    #[test]
    fn ns_is_inherited_along_the_parent_chain() {
        let mut o = Ontology::new();
        o.add_concept_type(ct("Company", None, Some("parties")));
        o.add_concept_type(ct("Subsidiary", Some("Company"), None));
        o.add_concept_type(ct("Branch", Some("Subsidiary"), None));
        o.add_concept_type(ct("Invoice", None, None));
        assert_eq!(o.ns_of_type("Company"), "parties");
        assert_eq!(o.ns_of_type("Subsidiary"), "parties");
        assert_eq!(o.ns_of_type("Branch"), "parties");
        assert_eq!(o.ns_of_type("Invoice"), DEFAULT_NS);
        assert_eq!(o.ns_of_type("Unknown"), DEFAULT_NS);
        assert_eq!(
            o.namespaces(),
            vec!["default".to_string(), "parties".to_string()]
        );
        o.validate_namespaces().unwrap();
    }

    #[test]
    fn ns_of_relation_type_follows_domain_and_range() {
        let mut o = Ontology::new();
        o.add_concept_type(ct("Contract", None, Some("contrats")));
        o.add_concept_type(ct("Company", None, Some("parties")));
        o.add_relation_type(RelationType {
            name: "between".into(),
            domain: "Contract".into(),
            range: "Company".into(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            o.ns_of_relation_type("between").unwrap(),
            ("contrats", "parties")
        );
        assert!(o.ns_of_relation_type("nope").is_err());
    }

    #[test]
    fn a_child_cannot_leave_its_parents_domain() {
        let mut o = Ontology::new();
        o.add_concept_type(ct("Company", None, Some("parties")));
        o.add_concept_type(ct("Subsidiary", Some("Company"), Some("parties")));
        o.validate_namespaces().unwrap();
        o.add_concept_type(ct("Rogue", Some("Company"), Some("elsewhere")));
        assert!(matches!(
            o.validate_namespaces(),
            Err(GraphError::NamespaceMismatch { .. })
        ));
        let mut o = Ontology::new();
        o.add_concept_type(ct("Bad", None, Some("Not Valid")));
        assert!(matches!(
            o.validate_namespaces(),
            Err(GraphError::InvalidNamespace { .. })
        ));
    }

    #[test]
    fn ns_round_trips_through_json_and_is_omitted_when_unset() {
        let with = ct("A", None, Some("x"));
        let js = serde_json::to_string(&with).unwrap();
        assert!(js.contains("\"ns\":\"x\""));
        let back: ConceptType = serde_json::from_str(&js).unwrap();
        assert_eq!(back.ns.as_deref(), Some("x"));
        let without = ct("B", None, None);
        let js = serde_json::to_string(&without).unwrap();
        assert!(!js.contains("\"ns\""), "{js}");
        // Older ontology files without the field keep loading.
        let legacy: ConceptType = serde_json::from_str(r#"{"name":"C"}"#).unwrap();
        assert_eq!(legacy.ns, None);
    }

    #[test]
    fn validate_inverses_happy_and_sad() {
        let mut o = Ontology::new();
        o.add_concept_type(ConceptType {
            name: "A".into(),
            ..Default::default()
        });
        o.add_concept_type(ConceptType {
            name: "B".into(),
            ..Default::default()
        });
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
