// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Bulk-load mode (`OntologyGraph::begin_bulk` / `end_bulk`,
//! `STORAGE-PLAN.md` phase 4 item 4): the same sequence of mutations, run
//! once in normal mode and once in bulk mode, must leave two graphs whose
//! every observable view is identical — primary lookups, sorted listings,
//! per-type listings, trigram search, relation listings, rules, actions,
//! traversal — and the generations must have moved (R2) so no cached page
//! survives the load.

use ontology_graph::{
    Action, ActionId, ActionType, Cardinality, Concept, ConceptId, ConceptType, Ontology,
    OntologyGraph, PropertyValue, Relation, RelationId, RelationType, Rule, RuleId, RuleType,
    TraversalSpec,
};
use std::sync::Arc;

fn ontology() -> Ontology {
    let mut o = Ontology::new();
    for (name, parent) in [
        ("Person", None),
        ("Employee", Some("Person")),
        ("Company", None),
    ] {
        o.add_concept_type(ConceptType {
            name: name.into(),
            parent: parent.map(String::from),
            ..Default::default()
        });
    }
    o.add_relation_type(RelationType {
        name: "knows".into(),
        domain: "Person".into(),
        range: "Person".into(),
        symmetric: true,
        cardinality: Cardinality::ManyToMany,
        ..Default::default()
    })
    .unwrap();
    o.add_relation_type(RelationType {
        name: "works_for".into(),
        domain: "Person".into(),
        range: "Company".into(),
        cardinality: Cardinality::ManyToMany,
        ..Default::default()
    })
    .unwrap();
    o.add_rule_type(RuleType {
        name: "audit".into(),
        when: String::new(),
        then: String::new(),
        applies_to: vec!["Company".into()],
        strict: false,
        description: String::new(),
    })
    .unwrap();
    o.add_action_type(ActionType {
        name: "notify".into(),
        subject: "Person".into(),
        object: Some("Company".into()),
        parameters: vec![],
        effect: String::new(),
        description: String::new(),
    })
    .unwrap();
    o
}

/// xorshift64*: deterministic, dependency-free.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

const NAMES: &[&str] = &[
    "alice", "bob", "carol", "dave", "erin", "frank", "grace", "heidi", "ivan", "judy", "mallory",
    "oscar", "peggy", "trent", "victor", "walter", "élodie", "zoë", "Émile", "Ñandú",
];

/// One scripted mutation, replayed identically on both graphs.
#[derive(Clone, Debug)]
enum Op {
    Upsert(Concept),
    Rename(ConceptId, String),
    RemoveConcept(ConceptId),
    Relation(Relation),
    RelationExact(Relation),
    RemoveRelation(RelationId),
    Rule(Rule),
    Action(Action),
    RemoveRule(RuleId),
}

/// A deterministic script exercising every mutation kind: explicit ids,
/// upserts that rename, deletes with cascades, symmetric pairs, exact
/// relations, rules and actions, interleaved.
fn script(seed: u64, n: usize) -> Vec<Op> {
    let mut rng = Rng(seed ^ 0x9E37_79B9_7F4A_7C15);
    let mut ops = Vec::new();
    let mut live: Vec<ConceptId> = Vec::new();
    let mut companies: Vec<ConceptId> = Vec::new();
    let mut next_c = 1u64;
    let mut next_r = 1u64;
    let mut next_rule = 1u64;
    let mut next_action = 1u64;
    for i in 0..n {
        match rng.below(10) {
            0..=3 => {
                let ty = match rng.below(3) {
                    0 => "Person",
                    1 => "Employee",
                    _ => "Company",
                };
                let name = format!(
                    "{} {}-{}",
                    NAMES[rng.below(NAMES.len() as u64) as usize],
                    NAMES[rng.below(NAMES.len() as u64) as usize],
                    next_c
                );
                let mut c = Concept::new(ConceptId(next_c), ty, name)
                    .with_description(format!("concept number {i}"));
                c.properties
                    .insert("n".into(), PropertyValue::Number(i as f64));
                if ty == "Company" {
                    companies.push(ConceptId(next_c));
                }
                live.push(ConceptId(next_c));
                next_c += 1;
                ops.push(Op::Upsert(c));
            }
            4 if !live.is_empty() => {
                let id = live[rng.below(live.len() as u64) as usize];
                ops.push(Op::Rename(
                    id,
                    format!(
                        "{} renamed-{}",
                        NAMES[rng.below(NAMES.len() as u64) as usize],
                        i
                    ),
                ));
            }
            5 if live.len() > 2 => {
                let idx = rng.below(live.len() as u64) as usize;
                let id = live.swap_remove(idx);
                companies.retain(|c| *c != id);
                ops.push(Op::RemoveConcept(id));
            }
            6 | 7 if live.len() > 1 => {
                let a = live[rng.below(live.len() as u64) as usize];
                let b = live[rng.below(live.len() as u64) as usize];
                if a != b {
                    // `knows` is symmetric: the normal path materializes an
                    // inverse with a fresh id, so both graphs allocate the
                    // same ids as long as the script is identical.
                    let mut r = Relation::new(RelationId(0), "knows", a, b);
                    r.weight = (i % 7) as f32 / 7.0;
                    if rng.below(2) == 0 {
                        ops.push(Op::Relation(r));
                    } else {
                        r.id = RelationId(1_000_000 + next_r);
                        next_r += 1;
                        ops.push(Op::RelationExact(r));
                    }
                }
            }
            8 if !companies.is_empty() && !live.is_empty() => {
                let c = companies[rng.below(companies.len() as u64) as usize];
                ops.push(Op::Rule(Rule {
                    id: RuleId(next_rule),
                    rule_type: "audit".into(),
                    name: format!("audit {c}"),
                    when: "always".into(),
                    then: "log".into(),
                    applies_to: vec![c],
                    strict: false,
                    description: String::new(),
                    properties: Default::default(),
                }));
                if rng.below(3) == 0 {
                    ops.push(Op::RemoveRule(RuleId(next_rule)));
                }
                next_rule += 1;
                let p = live[rng.below(live.len() as u64) as usize];
                ops.push(Op::Action(Action {
                    id: ActionId(next_action),
                    action_type: "notify".into(),
                    name: format!("notify {p}"),
                    subject: p,
                    object: Some(c),
                    parameters: Default::default(),
                    effect: String::new(),
                    description: String::new(),
                }));
                next_action += 1;
            }
            _ => {
                if rng.below(2) == 0 && next_r > 1 {
                    ops.push(Op::RemoveRelation(RelationId(
                        1_000_000 + rng.below(next_r),
                    )));
                }
            }
        }
    }
    ops
}

/// Apply the script through the public API; errors are expected for some
/// ops (a relation whose endpoint was deleted, a removed relation id) and
/// must be *the same* on both graphs.
fn run(graph: &OntologyGraph, ops: &[Op]) -> Vec<String> {
    let mut outcomes = Vec::with_capacity(ops.len());
    for op in ops {
        let r: Result<String, String> = match op.clone() {
            Op::Upsert(c) => graph
                .upsert_concept(c)
                .map(|id| id.to_string())
                .map_err(|e| e.to_string()),
            Op::Rename(id, name) => graph
                .update_concept(
                    id,
                    ontology_graph::ConceptPatch {
                        name: Some(name),
                        ..Default::default()
                    },
                )
                .map(|c| c.name)
                .map_err(|e| e.to_string()),
            Op::RemoveConcept(id) => graph
                .remove_concept(id)
                .map(|v| format!("{v:?}"))
                .map_err(|e| e.to_string()),
            Op::Relation(r) => graph
                .add_relation(r)
                .map(|id| id.to_string())
                .map_err(|e| e.to_string()),
            Op::RelationExact(r) => graph
                .insert_relation_exact(r)
                .map(|id| id.to_string())
                .map_err(|e| e.to_string()),
            Op::RemoveRelation(id) => graph
                .remove_relation(id)
                .map(|_| "ok".into())
                .map_err(|e| e.to_string()),
            Op::Rule(rule) => graph
                .upsert_rule(rule)
                .map(|id| id.to_string())
                .map_err(|e| e.to_string()),
            Op::Action(a) => graph
                .upsert_action(a)
                .map(|id| id.to_string())
                .map_err(|e| e.to_string()),
            Op::RemoveRule(id) => graph
                .remove_rule(id)
                .map(|_| "ok".into())
                .map_err(|e| e.to_string()),
        };
        outcomes.push(format!("{r:?}"));
    }
    outcomes
}

/// Every observable view of a graph, as comparable text.
fn views(graph: &OntologyGraph) -> Vec<(&'static str, String)> {
    let mut v = Vec::new();
    let (total, all) = graph.list_concepts_page(None, None, 0, 10_000, true, true);
    v.push(("concepts_total", total.to_string()));
    v.push((
        "concepts_sorted",
        all.iter()
            .map(|c| {
                format!(
                    "{}|{}|{}|{}|{:?}",
                    c.concept_type,
                    c.name,
                    c.id,
                    c.description,
                    c.properties.get("n")
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
    ));
    for ty in ["Person", "Employee", "Company"] {
        for subtypes in [false, true] {
            let (t, page) = graph.list_concepts_page(Some(ty), None, 0, 10_000, true, subtypes);
            v.push((
                "by_type",
                format!(
                    "{ty}/{subtypes}: {t} {:?}",
                    page.iter().map(|c| c.id).collect::<Vec<_>>()
                ),
            ));
        }
    }
    for needle in ["ali", "ren", "ob ", "élo", "zoë", "xyz", "-1"] {
        let (t, page) = graph.list_concepts_page(None, Some(needle), 0, 10_000, true, true);
        v.push((
            "search",
            format!(
                "{needle}: {t} {:?}",
                page.iter().map(|c| c.id).collect::<Vec<_>>()
            ),
        ));
        let (t, page) = graph.list_concepts_page(Some("Person"), Some(needle), 3, 5, false, true);
        v.push((
            "search_paged",
            format!(
                "{needle}: {t} {:?}",
                page.iter().map(|c| c.id).collect::<Vec<_>>()
            ),
        ));
    }
    // Paged windows must stitch to the same whole.
    let mut stitched = Vec::new();
    let mut off = 0;
    loop {
        let (_, page) = graph.list_concepts_page(None, None, off, 7, false, true);
        if page.is_empty() {
            break;
        }
        stitched.extend(page.iter().map(|c| c.id));
        off += 7;
    }
    v.push(("stitched", format!("{stitched:?}")));
    let mut rels = graph.all_relations();
    rels.sort_by_key(|r| r.id);
    v.push((
        "relations",
        rels.iter()
            .map(|r| {
                format!(
                    "{}|{}|{}|{}|{}",
                    r.id, r.relation_type, r.source, r.target, r.weight
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
    ));
    v.push(("relation_count", graph.relation_count().to_string()));
    for c in all.iter().take(12) {
        v.push((
            "incident",
            format!("{}: {:?}", c.id, graph.incident_relation_ids(c.id).unwrap()),
        ));
        v.push((
            "find_by_name",
            format!(
                "{}: {:?}",
                c.id,
                graph.find_by_name(&c.concept_type, &c.name.to_uppercase())
            ),
        ));
        let sg = graph.expand(
            &[c.id],
            &TraversalSpec {
                max_depth: 2,
                ..Default::default()
            },
        );
        let mut ids: Vec<u64> = sg.concepts.iter().map(|c| c.id.0).collect();
        ids.sort_unstable();
        v.push(("expand", format!("{}: {ids:?}", c.id)));
    }
    let mut rules = graph.all_rules();
    rules.sort_by_key(|r| r.id);
    v.push((
        "rules",
        rules
            .iter()
            .map(|r| format!("{}|{}|{:?}", r.id, r.name, r.applies_to))
            .collect::<Vec<_>>()
            .join("\n"),
    ));
    let mut actions = graph.all_actions();
    actions.sort_by_key(|a| a.id);
    v.push((
        "actions",
        actions
            .iter()
            .map(|a| format!("{}|{}|{}|{:?}", a.id, a.name, a.subject, a.object))
            .collect::<Vec<_>>()
            .join("\n"),
    ));
    v
}

fn assert_same_views(normal: &OntologyGraph, bulk: &OntologyGraph) {
    let (a, b) = (views(normal), views(bulk));
    assert_eq!(a.len(), b.len());
    for ((ka, va), (kb, vb)) in a.iter().zip(b.iter()) {
        assert_eq!(ka, kb);
        assert_eq!(va, vb, "view `{ka}` differs between normal and bulk mode");
    }
}

/// The core contract: same script, same outcomes, identical views.
#[test]
fn bulk_mode_yields_exactly_the_same_graph_as_normal_mode() {
    for seed in [1u64, 7, 42, 2026] {
        let ops = script(seed, 600);
        let normal = OntologyGraph::new(ontology());
        let out_normal = run(&normal, &ops);

        let bulk = OntologyGraph::new(ontology());
        let guard = bulk.begin_bulk();
        assert!(bulk.is_bulk());
        let out_bulk = run(&bulk, &ops);
        let report = guard.finish();
        assert!(!bulk.is_bulk());

        assert_eq!(out_normal, out_bulk, "seed {seed}: outcomes differ");
        assert_same_views(&normal, &bulk);
        assert_eq!(report.concepts, bulk.concept_count());
        assert_eq!(report.relations, bulk.relation_count());
        assert_eq!(report.rules, bulk.all_rules().len());
        assert_eq!(report.actions, bulk.all_actions().len());
        assert!(report.trigrams > 0);
        assert!(report.total_ms >= 0.0);
    }
}

/// While the guard is alive the derived indexes are stale by design; the
/// primary lookups are live. `end_bulk` brings the derived views up to date
/// and bumps both generations exactly once.
#[test]
fn derived_indexes_are_rebuilt_once_and_generations_bump_once() {
    let g = OntologyGraph::new(ontology());
    let (cg0, rg0) = (g.concepts_generation(), g.relations_generation());
    let guard = g.begin_bulk();
    let a = g
        .upsert_concept(Concept::new(ConceptId(0), "Person", "Alice"))
        .unwrap();
    let b = g
        .upsert_concept(Concept::new(ConceptId(0), "Company", "Acme"))
        .unwrap();
    g.add_relation(Relation::new(RelationId(0), "works_for", a, b))
        .unwrap();
    // Primary lookups are live during the load…
    assert_eq!(g.concept_count(), 2);
    assert_eq!(g.find_by_name("Person", "alice"), Some(a));
    assert_eq!(g.incident_relation_ids(a).unwrap().len(), 1);
    // …the derived listing is not yet.
    let (total, _) = g.list_concepts_page(None, None, 0, 10, true, true);
    assert_eq!(total, 0, "sorted index untouched during bulk mode");
    let (_, hits) = g.list_concepts_page(None, Some("ali"), 0, 10, true, true);
    assert!(hits.is_empty(), "trigram index untouched during bulk mode");
    assert_eq!(
        g.concepts_generation(),
        cg0,
        "no per-mutation bump in bulk mode"
    );
    assert_eq!(g.relations_generation(), rg0);

    let report = guard.finish();
    assert_eq!(report.concepts, 2);
    assert_eq!(report.relations, 1);
    let (total, page) = g.list_concepts_page(None, None, 0, 10, true, true);
    assert_eq!(total, 2);
    assert_eq!(
        page.iter().map(|c| c.id).collect::<Vec<_>>(),
        vec![b, a],
        "(type, name) order: Company before Person"
    );
    let (_, hits) = g.list_concepts_page(None, Some("ali"), 0, 10, true, true);
    assert_eq!(hits.iter().map(|c| c.id).collect::<Vec<_>>(), vec![a]);
    assert_eq!(g.concepts_generation(), cg0 + 1, "exactly one bump");
    assert_eq!(g.relations_generation(), rg0 + 1);

    // Back in normal mode, mutations maintain the indexes again.
    g.upsert_concept(Concept::new(ConceptId(0), "Person", "Bob"))
        .unwrap();
    let (total, _) = g.list_concepts_page(None, None, 0, 10, true, true);
    assert_eq!(total, 3);
    assert_eq!(g.concepts_generation(), cg0 + 2);
}

/// Dropping the guard (an early return, a `?` in the loader) ends the mode
/// and rebuilds; finishing twice or ending outside bulk mode is harmless.
#[test]
fn guard_drop_rebuilds_and_end_bulk_is_idempotent() {
    fn failing_load(g: &OntologyGraph) -> Result<(), &'static str> {
        let _guard = g.begin_bulk();
        g.upsert_concept(Concept::new(ConceptId(0), "Person", "Alice"))
            .unwrap();
        Err("simulated replay failure")
    }
    let g = OntologyGraph::new(ontology());
    assert!(failing_load(&g).is_err());
    assert!(!g.is_bulk(), "the guard ended the mode on drop");
    let (total, _) = g.list_concepts_page(None, None, 0, 10, true, true);
    assert_eq!(total, 1, "what was loaded before the failure is indexed");

    let gen = g.concepts_generation();
    assert_eq!(g.end_bulk(), Default::default(), "no-op outside bulk mode");
    assert_eq!(g.concepts_generation(), gen, "no bump for a no-op");
}

/// Nested guards: only the outermost rebuilds; the inner one is inert.
#[test]
fn nested_guards_rebuild_once_at_the_outermost() {
    let g = OntologyGraph::new(ontology());
    let outer = g.begin_bulk();
    {
        let inner = g.begin_bulk();
        g.upsert_concept(Concept::new(ConceptId(0), "Person", "Alice"))
            .unwrap();
        let r = inner.finish();
        assert_eq!(r, Default::default(), "inner guard does not rebuild");
        assert!(g.is_bulk(), "still in bulk mode after the inner guard");
    }
    let (total, _) = g.list_concepts_page(None, None, 0, 10, true, true);
    assert_eq!(total, 0);
    let r = outer.finish();
    assert_eq!(r.concepts, 1);
    let (total, _) = g.list_concepts_page(None, None, 0, 10, true, true);
    assert_eq!(total, 1);
}

/// `clear_instances` inside a bulk load leaves nothing behind after rebuild.
#[test]
fn clear_inside_bulk_mode_rebuilds_empty_indexes() {
    let g = Arc::new(OntologyGraph::new(ontology()));
    let guard = g.begin_bulk();
    for i in 0..20 {
        g.upsert_concept(Concept::new(ConceptId(0), "Person", format!("p{i}")))
            .unwrap();
    }
    g.clear_instances();
    let r = guard.finish();
    assert_eq!(r.concepts, 0);
    assert_eq!(r.trigrams, 0);
    let (total, _) = g.list_concepts_page(None, Some("p1"), 0, 10, true, true);
    assert_eq!(total, 0);
}
