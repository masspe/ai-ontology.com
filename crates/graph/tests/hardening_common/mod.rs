// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Shared fixtures for the `hardening_*` integration tests. Not a test
//! target itself (Cargo only picks up files directly under `tests/`).

#![allow(dead_code)]

use ontology_graph::{
    ActionType, Cardinality, Concept, ConceptId, ConceptType, Ontology, OntologyGraph, Relation,
    RelationId, RelationType, RuleType,
};

/// A concept type with an optional parent and an optional storage domain.
pub fn ct(name: &str, parent: Option<&str>, ns: Option<&str>) -> ConceptType {
    ConceptType {
        name: name.into(),
        parent: parent.map(str::to_string),
        ns: ns.map(str::to_string),
        ..Default::default()
    }
}

/// A many-to-many, non-symmetric relation type.
pub fn rt(name: &str, domain: &str, range: &str) -> RelationType {
    RelationType {
        name: name.into(),
        domain: domain.into(),
        range: range.into(),
        ..Default::default()
    }
}

pub fn rt_card(name: &str, domain: &str, range: &str, cardinality: Cardinality) -> RelationType {
    RelationType {
        cardinality,
        ..rt(name, domain, range)
    }
}

pub fn rt_symmetric(name: &str, domain: &str, range: &str) -> RelationType {
    RelationType {
        symmetric: true,
        ..rt(name, domain, range)
    }
}

pub fn rule_type(name: &str, applies_to: &[&str]) -> RuleType {
    RuleType {
        name: name.into(),
        when: String::new(),
        then: String::new(),
        applies_to: applies_to.iter().map(|s| s.to_string()).collect(),
        strict: false,
        description: String::new(),
    }
}

pub fn action_type(name: &str, subject: &str, object: Option<&str>) -> ActionType {
    ActionType {
        name: name.into(),
        subject: subject.into(),
        object: object.map(str::to_string),
        parameters: Vec::new(),
        effect: String::new(),
        description: String::new(),
    }
}

/// The reference ontology of these tests:
///
/// * `Person` (root, default domain), `Researcher` (child of `Person`),
///   `Paper`, `Topic`;
/// * `authored: Person -> Paper` (many-to-many),
///   `knows: Person <-> Person` (symmetric),
///   `cites: Paper -> Paper`,
///   `about: Paper -> Topic` (many-to-one: a paper has at most one topic);
/// * rule type `must_review` scoped to `Paper`;
/// * action type `sign` performed by a `Person` on a `Paper`.
pub fn reference_ontology() -> Ontology {
    let mut o = Ontology::new();
    o.add_concept_type(ct("Person", None, None));
    o.add_concept_type(ct("Researcher", Some("Person"), None));
    o.add_concept_type(ct("Paper", None, None));
    o.add_concept_type(ct("Topic", None, None));
    o.add_relation_type(rt("authored", "Person", "Paper"))
        .unwrap();
    o.add_relation_type(rt_symmetric("knows", "Person", "Person"))
        .unwrap();
    o.add_relation_type(rt("cites", "Paper", "Paper")).unwrap();
    o.add_relation_type(rt_card("about", "Paper", "Topic", Cardinality::ManyToOne))
        .unwrap();
    o.add_rule_type(rule_type("must_review", &["Paper"]))
        .unwrap();
    o.add_action_type(action_type("sign", "Person", Some("Paper")))
        .unwrap();
    o
}

pub fn graph() -> OntologyGraph {
    OntologyGraph::new(reference_ontology())
}

/// Insert a fresh concept (id allocated by the graph).
pub fn concept(g: &OntologyGraph, concept_type: &str, name: &str) -> ConceptId {
    g.upsert_concept(Concept::new(ConceptId(0), concept_type, name))
        .unwrap_or_else(|e| panic!("insert {concept_type}/{name}: {e}"))
}

/// Insert a fresh relation (id allocated by the graph).
pub fn relation(g: &OntologyGraph, relation_type: &str, s: ConceptId, t: ConceptId) -> RelationId {
    g.add_relation(Relation::new(RelationId(0), relation_type, s, t))
        .unwrap_or_else(|e| panic!("relation {relation_type} {s}->{t}: {e}"))
}

/// Names of a concept page, in the order returned.
pub fn names(page: &[Concept]) -> Vec<String> {
    page.iter().map(|c| c.name.clone()).collect()
}

/// Small deterministic generator (xorshift64*), so the randomized tests are
/// reproducible without pulling `proptest` into this crate's dependencies.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.max(1))
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform-ish value in `0..n` (`n > 0`).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
    pub fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in) == 0
    }
}
