// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Traversal: `shortest_path`, `expand` and `closure` respect their depth
//! and size bounds, treat cycles as finite, and honour direction and type
//! filters (`PERFORMANCE.md` §4.6).

mod hardening_common;
use hardening_common::*;

use ontology_graph::{ConceptId, Direction, GraphError, TraversalSpec};

/// Build a chain `c0 -cites-> c1 -cites-> … -cites-> c(n-1)` of papers.
fn chain(g: &ontology_graph::OntologyGraph, n: usize) -> Vec<ConceptId> {
    let ids: Vec<ConceptId> = (0..n)
        .map(|i| concept(g, "Paper", &format!("C{i}")))
        .collect();
    for w in ids.windows(2) {
        relation(g, "cites", w[0], w[1]);
    }
    ids
}

/// `shortest_path` is undirected: a purely asymmetric chain is found in
/// both directions, and every step's relation links the previous concept
/// to the step's concept.
#[test]
fn shortest_path_is_undirected_and_its_steps_are_contiguous() {
    let g = graph();
    let c = chain(&g, 4);
    let fwd = g.shortest_path(c[0], c[3], 10).unwrap().unwrap();
    let bwd = g.shortest_path(c[3], c[0], 10).unwrap().unwrap();
    assert_eq!(fwd.len(), 3);
    assert_eq!(bwd.len(), 3);
    for path in [&fwd, &bwd] {
        let mut prev = path.start.id;
        for step in &path.steps {
            let r = &step.relation;
            assert!(
                (r.source == prev && r.target == step.concept.id)
                    || (r.target == prev && r.source == step.concept.id),
                "step relation {r:?} does not join {prev} and {}",
                step.concept.id
            );
            prev = step.concept.id;
        }
        assert_eq!(prev, path.target);
    }
    assert_eq!(fwd.steps.last().unwrap().concept.id, c[3]);
    assert_eq!(bwd.steps.last().unwrap().concept.id, c[0]);
}

/// `max_depth` bounds the number of hops: a path of length n is found at
/// depth n and not at depth n-1; depth 0 only finds the trivial path.
#[test]
fn shortest_path_respects_max_depth_exactly() {
    let g = graph();
    let c = chain(&g, 4);
    assert!(g.shortest_path(c[0], c[3], 2).unwrap().is_none());
    assert_eq!(g.shortest_path(c[0], c[3], 3).unwrap().unwrap().len(), 3);
    assert!(g.shortest_path(c[0], c[1], 0).unwrap().is_none());
    assert_eq!(g.shortest_path(c[0], c[1], 1).unwrap().unwrap().len(), 1);
    assert!(g.shortest_path(c[2], c[2], 0).unwrap().unwrap().is_empty());
}

/// Unknown endpoints are errors, not `None`.
#[test]
fn shortest_path_reports_unknown_endpoints() {
    let g = graph();
    let a = concept(&g, "Paper", "A");
    assert!(matches!(
        g.shortest_path(a, ConceptId(999), 3),
        Err(GraphError::UnknownConcept(ConceptId(999)))
    ));
    assert!(matches!(
        g.shortest_path(ConceptId(999), a, 3),
        Err(GraphError::UnknownConcept(ConceptId(999)))
    ));
}

/// BFS finds the shortest route when a longer one also exists, and a cycle
/// does not prevent termination or inflate the path.
#[test]
fn shortest_path_prefers_the_short_route_through_a_cycle() {
    let g = graph();
    // Ring of 6 papers, plus a chord 0 -> 3.
    let ring: Vec<ConceptId> = (0..6)
        .map(|i| concept(&g, "Paper", &format!("R{i}")))
        .collect();
    for i in 0..6 {
        relation(&g, "cites", ring[i], ring[(i + 1) % 6]);
    }
    assert_eq!(
        g.shortest_path(ring[0], ring[3], 10)
            .unwrap()
            .unwrap()
            .len(),
        3
    );
    relation(&g, "cites", ring[0], ring[3]);
    assert_eq!(
        g.shortest_path(ring[0], ring[3], 10)
            .unwrap()
            .unwrap()
            .len(),
        1
    );
    // 4 -> 5 -> 0, or 4 -> 3 -> 0 through the chord: two hops either way.
    assert_eq!(
        g.shortest_path(ring[4], ring[0], 10)
            .unwrap()
            .unwrap()
            .len(),
        2
    );
    // Disconnected component within the depth bound → None, promptly.
    let lone = concept(&g, "Paper", "Lone");
    assert!(g.shortest_path(ring[0], lone, 100).unwrap().is_none());
}

/// A symmetric relation is reachable from either endpoint in one hop.
#[test]
fn shortest_path_crosses_symmetric_relations_from_both_sides() {
    let g = graph();
    let a = concept(&g, "Person", "A");
    let b = concept(&g, "Person", "B");
    relation(&g, "knows", a, b);
    assert_eq!(g.shortest_path(a, b, 1).unwrap().unwrap().len(), 1);
    assert_eq!(g.shortest_path(b, a, 1).unwrap().unwrap().len(), 1);
}

/// `expand` at depth 0 returns only the seeds (deduplicated, known ones)
/// and no relation; depth 1 and 2 grow the frontier by exactly one hop
/// each and record the depth at which each concept was reached.
#[test]
fn expand_depth_zero_one_two_on_a_chain() {
    let g = graph();
    let c = chain(&g, 4);
    let spec = |d: u32| TraversalSpec {
        max_depth: d,
        ..Default::default()
    };
    let sg0 = g.expand(&[c[0], c[0], ConceptId(999)], &spec(0));
    assert_eq!(sg0.concepts.len(), 1);
    assert!(sg0.relations.is_empty());
    assert_eq!(sg0.depth_of[&c[0]], 0);
    assert_eq!(sg0.seeds.len(), 3, "seeds are echoed as given");

    let sg1 = g.expand(&[c[0]], &spec(1));
    assert_eq!(sg1.concepts.len(), 2);
    assert_eq!(sg1.relations.len(), 1);
    assert_eq!(sg1.depth_of[&c[1]], 1);

    let sg2 = g.expand(&[c[0]], &spec(2));
    assert_eq!(sg2.concepts.len(), 3);
    assert_eq!(sg2.relations.len(), 2);
    assert_eq!(sg2.depth_of[&c[2]], 2);
    assert!(!sg2.depth_of.contains_key(&c[3]));

    // From the middle, depth 1 reaches both neighbours (undirected).
    let mid = g.expand(&[c[1]], &spec(1));
    assert_eq!(mid.concepts.len(), 3);
    let empty = g.expand(&[], &spec(2));
    assert!(empty.is_empty() && empty.relations.is_empty());
}

/// `expand` on a cycle terminates, visits each node once and emits each
/// relation once; `max_nodes` caps the concept count.
#[test]
fn expand_terminates_on_cycles_and_honours_max_nodes() {
    let g = graph();
    let ring: Vec<ConceptId> = (0..5)
        .map(|i| concept(&g, "Paper", &format!("R{i}")))
        .collect();
    for i in 0..5 {
        relation(&g, "cites", ring[i], ring[(i + 1) % 5]);
    }
    let sg = g.expand(
        &[ring[0]],
        &TraversalSpec {
            max_depth: 50,
            ..Default::default()
        },
    );
    assert_eq!(sg.concepts.len(), 5);
    assert_eq!(sg.relations.len(), 5);
    let mut ids: Vec<ConceptId> = sg.concepts.iter().map(|c| c.id).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 5);
    let capped = g.expand(
        &[ring[0]],
        &TraversalSpec {
            max_depth: 50,
            max_nodes: 3,
            ..Default::default()
        },
    );
    assert_eq!(capped.concepts.len(), 3);
    // Every emitted relation joins two concepts of the subgraph.
    let members: std::collections::HashSet<ConceptId> =
        capped.concepts.iter().map(|c| c.id).collect();
    for r in &capped.relations {
        assert!(
            members.contains(&r.source) && members.contains(&r.target),
            "{r:?}"
        );
    }
}

/// Direction and relation-type filters restrict what `expand` follows;
/// concept-type filters restrict what it emits, with subsumption on by
/// default.
#[test]
fn expand_direction_type_and_concept_filters() {
    let g = graph();
    let ada = concept(&g, "Researcher", "Ada");
    let bob = concept(&g, "Person", "Bob");
    let p = concept(&g, "Paper", "P");
    let t = concept(&g, "Topic", "T");
    relation(&g, "authored", ada, p);
    relation(&g, "knows", bob, ada);
    relation(&g, "about", p, t);
    let names_of = |sg: &ontology_graph::Subgraph| {
        let mut v: Vec<String> = sg.concepts.iter().map(|c| c.name.clone()).collect();
        v.sort();
        v
    };
    let out = g.expand(
        &[ada],
        &TraversalSpec {
            max_depth: 1,
            direction: Direction::Outgoing,
            ..Default::default()
        },
    );
    // Outgoing from Ada: authored → P, and the materialized inverse Ada → Bob.
    assert_eq!(names_of(&out), vec!["Ada", "Bob", "P"]);
    let inc = g.expand(
        &[p],
        &TraversalSpec {
            max_depth: 1,
            direction: Direction::Incoming,
            ..Default::default()
        },
    );
    assert_eq!(names_of(&inc), vec!["Ada", "P"]);
    let typed = g.expand(
        &[ada],
        &TraversalSpec {
            max_depth: 2,
            relation_types: vec!["authored".into(), "about".into()],
            ..Default::default()
        },
    );
    assert_eq!(names_of(&typed), vec!["Ada", "P", "T"]);
    assert!(typed.relations.iter().all(|r| r.relation_type != "knows"));
    let only_people = g.expand(
        &[p],
        &TraversalSpec {
            max_depth: 2,
            concept_types: vec!["Person".into()],
            ..Default::default()
        },
    );
    assert_eq!(
        names_of(&only_people),
        vec!["Ada", "Bob", "P"],
        "subtypes subsumed"
    );
    let strict = g.expand(
        &[p],
        &TraversalSpec {
            max_depth: 2,
            concept_types: vec!["Person".into()],
            subsume_concept_types: false,
            ..Default::default()
        },
    );
    assert_eq!(
        names_of(&strict),
        vec!["P"],
        "Ada is a Researcher, Bob is behind Ada"
    );
}

/// `closure` follows one relation type in both directions, excludes the
/// seed, stops at `max_depth` and terminates on cycles.
#[test]
fn closure_is_bounded_and_cycle_safe() {
    let g = graph();
    let c = chain(&g, 4);
    relation(&g, "cites", c[3], c[0]); // close the loop
    let a = concept(&g, "Person", "A");
    relation(&g, "authored", a, c[0]);
    let mut all = g.closure(c[0], "cites", 10).unwrap();
    all.sort();
    let mut expected = c[1..].to_vec();
    expected.sort();
    assert_eq!(
        all, expected,
        "seed excluded, other type ignored, cycle finite"
    );
    assert_eq!(
        g.closure(c[0], "cites", 0).unwrap(),
        Vec::<ConceptId>::new()
    );
    let one = g.closure(c[0], "cites", 1).unwrap();
    assert_eq!(one.len(), 2, "both directions: c1 (out) and c3 (in)");
    assert!(g.closure(c[0], "authored", 5).unwrap() == vec![a]);
    assert!(g.closure(c[0], "nope", 5).unwrap().is_empty());
    assert!(matches!(
        g.closure(ConceptId(999), "cites", 1),
        Err(GraphError::UnknownConcept(_))
    ));
}
