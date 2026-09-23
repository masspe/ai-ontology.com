// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! P1, graph side (`STORAGE.md` §6.2 / §8.2, `STORAGE-PLAN.md` §7.2): a
//! concept slot keeps type and name in memory and drops its payload once
//! the store has sealed the partition holding it; every read of a full
//! concept then goes through the attached [`PayloadSource`]. Without
//! `set_loc` (P0) nothing changes and the source is never called.

use ontology_graph::{
    Cardinality, Concept, ConceptId, ConceptPatch, ConceptType, GraphError, Loc, Ontology,
    OntologyGraph, PayloadSource, PropertyValue, Relation, RelationId, RelationType, TraversalSpec,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

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
    o
}

/// In-memory stand-in for the segment store's reader.
#[derive(Default)]
struct Fake {
    records: Mutex<HashMap<Loc, Concept>>,
    hits: AtomicUsize,
    detached: AtomicBool,
}

impl Fake {
    fn put(&self, loc: Loc, c: Concept) {
        self.records.lock().unwrap().insert(loc, c);
    }
    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

impl PayloadSource for Fake {
    fn read(&self, loc: Loc) -> Result<Concept, String> {
        self.hits.fetch_add(1, Ordering::SeqCst);
        if self.detached.load(Ordering::SeqCst) {
            return Err("detached".into());
        }
        self.records
            .lock()
            .unwrap()
            .get(&loc)
            .cloned()
            .ok_or_else(|| format!("no record at {loc:?}"))
    }
}

fn loc(ns_id: u16, partition: u32, offset: u64) -> Loc {
    Loc {
        ns_id,
        partition,
        offset,
    }
}

/// `Concept` has no `PartialEq` and its property map iterates in a random
/// order; the JSON value (sorted object keys) is a stable comparison.
fn json(c: &Concept) -> serde_json::Value {
    serde_json::to_value(c).unwrap()
}

fn person(id: u64, name: &str) -> Concept {
    Concept::new(ConceptId(id), "Person", name)
        .with_description(format!("about {name}"))
        .with_property("n", PropertyValue::Number(id as f64))
        .with_property("tag", PropertyValue::Text(name.to_uppercase()))
}

fn unavailable(r: Result<Concept, GraphError>) -> bool {
    matches!(r, Err(GraphError::PayloadUnavailable(..)))
}

// 1. P0 unchanged: no `set_loc`, everything resident, source never asked.
#[test]
fn p0_keeps_every_payload_resident_and_never_calls_the_source() {
    let g = OntologyGraph::new(ontology());
    let fake = Arc::new(Fake::default());
    g.set_payload_source(fake.clone());
    for i in 1..=20 {
        g.upsert_concept(person(i, &format!("p{i}"))).unwrap();
    }
    g.add_relation(Relation::new(
        RelationId(0),
        "knows",
        ConceptId(1),
        ConceptId(2),
    ))
    .unwrap();
    assert_eq!(g.resident_payloads(), g.concept_count());
    assert_eq!(g.partition_sealed(0, 0), 0);

    let c = g.get_concept(ConceptId(3)).unwrap();
    assert_eq!(c.description, "about p3");
    assert_eq!(g.all_concepts().len(), 20);
    assert_eq!(g.list_concepts_page(None, None, 0, 100, true, true).0, 20);
    assert_eq!(g.concepts_of_type("Person", true).len(), 20);
    let sg = g.expand(&[ConceptId(1)], &TraversalSpec::default());
    assert_eq!(sg.concepts.len(), 2);
    assert_eq!(fake.hits(), 0);
}

// 2. Eviction: reads go through the source; a wrong record or a failing
//    source surfaces as `PayloadUnavailable`; no source → stays resident.
#[test]
fn evicted_concept_is_read_through_the_source() {
    let g = OntologyGraph::new(ontology());
    let id = g.upsert_concept(person(0, "alice")).unwrap();
    let before = g.get_concept(id).unwrap();

    // No source attached: `resident = false` cannot be honoured.
    g.set_loc(id, loc(1, 1, 0), false);
    assert_eq!(g.resident_payloads(), 1);
    assert_eq!(json(&g.get_concept(id).unwrap()), json(&before));

    let fake = Arc::new(Fake::default());
    fake.put(loc(1, 1, 0), before.clone());
    g.set_payload_source(fake.clone());
    g.set_loc(id, loc(1, 1, 0), false);
    assert_eq!(g.resident_payloads(), 0);
    assert_eq!(g.concept_count(), 1);

    let after = g.get_concept(id).unwrap();
    assert_eq!(json(&after), json(&before));
    assert_eq!(fake.hits(), 1);
    // Type and name never need the source.
    assert_eq!(g.find_by_name("Person", "ALICE"), Some(id));
    assert_eq!(fake.hits(), 1);

    // The record at the location belongs to someone else.
    fake.put(loc(1, 1, 0), person(99, "mallory"));
    assert!(unavailable(g.get_concept(id)));
    // Listings propagate instead of dropping the row.
    assert!(matches!(
        g.try_list_concepts_page(None, None, 0, 10, true, true),
        Err(GraphError::PayloadUnavailable(..))
    ));
    assert!(matches!(
        g.try_all_concepts(),
        Err(GraphError::PayloadUnavailable(..))
    ));
    assert!(matches!(
        g.try_concepts_of_type("Person", false),
        Err(GraphError::PayloadUnavailable(..))
    ));

    // The source itself fails.
    fake.put(loc(1, 1, 0), before.clone());
    fake.detached.store(true, Ordering::SeqCst);
    assert!(unavailable(g.get_concept(id)));
    fake.detached.store(false, Ordering::SeqCst);
    assert_eq!(json(&g.get_concept(id).unwrap()), json(&before));
}

// 3. Write-ahead: the location arrives before the concept and is attached
//    when the concept is applied.
#[test]
fn location_recorded_before_apply_is_attached_on_apply() {
    let g = OntologyGraph::new(ontology());
    let fake = Arc::new(Fake::default());
    g.set_payload_source(fake.clone());
    let c = person(42, "zoë");
    fake.put(loc(2, 7, 128), c.clone());

    g.set_loc(ConceptId(42), loc(2, 7, 128), true);
    assert_eq!(g.concept_count(), 0);
    g.upsert_concept(c.clone()).unwrap();
    assert_eq!(g.resident_payloads(), 1);

    assert_eq!(g.partition_sealed(2, 7), 1);
    assert_eq!(g.resident_payloads(), 0);
    assert_eq!(json(&g.get_concept(ConceptId(42)).unwrap()), json(&c));
    assert_eq!(fake.hits(), 1);
    // The stash was consumed: a later concept with a different id is not
    // affected, and a second seal has nothing left.
    assert_eq!(g.partition_sealed(2, 7), 0);
}

// 4. Seal: exactly the resident payloads of that partition are dropped.
#[test]
fn sealing_a_partition_drops_exactly_its_resident_payloads() {
    let g = OntologyGraph::new(ontology());
    let fake = Arc::new(Fake::default());
    g.set_payload_source(fake.clone());
    for i in 1..=10u64 {
        let c = person(i, &format!("p{i}"));
        let l = loc(3, if i <= 4 { 1 } else { 2 }, i * 64);
        fake.put(l, c.clone());
        g.upsert_concept(c).unwrap();
        g.set_loc(ConceptId(i), l, true);
    }
    assert_eq!(g.resident_payloads(), 10);

    assert_eq!(g.partition_sealed(3, 1), 4);
    assert_eq!(g.resident_payloads(), 6);
    assert_eq!(g.partition_sealed(3, 1), 0);
    // Another stream, same partition number: untouched.
    assert_eq!(g.partition_sealed(4, 2), 0);
    assert_eq!(g.resident_payloads(), 6);

    for i in 5..=10u64 {
        g.get_concept(ConceptId(i)).unwrap();
    }
    assert_eq!(fake.hits(), 0, "resident payloads never hit the source");
    for i in 1..=4u64 {
        assert_eq!(
            g.get_concept(ConceptId(i)).unwrap().description,
            format!("about p{i}")
        );
    }
    assert_eq!(fake.hits(), 4);

    assert_eq!(g.partition_sealed(3, 2), 6);
    assert_eq!(g.resident_payloads(), 0);
}

// 5. Update while on disk: read old, patch, resident again; a new
//    location evicts again and the source serves the patched record.
#[test]
fn updating_an_evicted_concept_reads_patches_and_becomes_resident() {
    let g = OntologyGraph::new(ontology());
    let fake = Arc::new(Fake::default());
    g.set_payload_source(fake.clone());
    let id = g.upsert_concept(person(0, "bob")).unwrap();
    let v1 = g.get_concept(id).unwrap();
    let (l1, l2) = (loc(1, 1, 0), loc(1, 2, 0));
    fake.put(l1, v1.clone());
    g.set_loc(id, l1, false);
    assert_eq!(g.resident_payloads(), 0);

    let v2 = g
        .update_concept(
            id,
            ConceptPatch {
                name: Some("robert".into()),
                description: Some("patched".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(fake.hits(), 1, "the old payload came from the source");
    assert_eq!(v2.name, "robert");
    assert_eq!(v2.description, "patched");
    assert_eq!(v2.properties.get("n"), v1.properties.get("n"));
    assert_eq!(g.resident_payloads(), 1);
    assert_eq!(json(&g.get_concept(id).unwrap()), json(&v2));
    assert_eq!(fake.hits(), 1);
    assert_eq!(g.find_by_name("Person", "robert"), Some(id));
    assert_eq!(g.find_by_name("Person", "bob"), None);

    fake.put(l2, v2.clone());
    g.set_loc(id, l2, false);
    assert_eq!(g.resident_payloads(), 0);
    assert_eq!(json(&g.get_concept(id).unwrap()), json(&v2));
    assert_eq!(fake.hits(), 2);
    // The old partition has nothing of ours any more.
    assert_eq!(g.partition_sealed(1, 1), 0);
    // Listing shows the new name and the patched payload.
    let (_, page) = g.list_concepts_page(None, Some("rob"), 0, 10, true, true);
    assert_eq!(page.len(), 1);
    assert_eq!(json(&page[0]), json(&v2));
}

// 6. Same script on a P0 graph and on a fully evicted P1 graph: every
//    listing and traversal is identical.

/// xorshift64*: deterministic, dependency-free.
struct Rng(u64);
impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D) % n.max(1)
    }
}

const NAMES: &[&str] = &[
    "alice", "bob", "carol", "dave", "erin", "frank", "grace", "heidi", "ivan", "judy", "élodie",
    "zoë",
];

#[derive(Clone)]
enum Op {
    Upsert(Concept),
    Rename(ConceptId, String),
    Remove(ConceptId),
    Relation(Relation),
}

fn script(seed: u64, n: usize) -> Vec<Op> {
    let mut rng = Rng(seed ^ 0x9E37_79B9_7F4A_7C15);
    let mut ops = Vec::new();
    let mut live: Vec<ConceptId> = Vec::new();
    let mut next = 1u64;
    for i in 0..n {
        match rng.below(8) {
            0..=3 => {
                let ty = ["Person", "Employee", "Company"][rng.below(3) as usize];
                let name = format!(
                    "{} {}-{next}",
                    NAMES[rng.below(12) as usize],
                    NAMES[rng.below(12) as usize]
                );
                let c = Concept::new(ConceptId(next), ty, name)
                    .with_description(format!("concept number {i}"))
                    .with_property("n", PropertyValue::Number(i as f64));
                live.push(ConceptId(next));
                next += 1;
                ops.push(Op::Upsert(c));
            }
            4 if !live.is_empty() => {
                let id = live[rng.below(live.len() as u64) as usize];
                ops.push(Op::Rename(
                    id,
                    format!("{} renamed-{i}", NAMES[rng.below(12) as usize]),
                ));
            }
            5 if live.len() > 2 => {
                let idx = rng.below(live.len() as u64) as usize;
                ops.push(Op::Remove(live.swap_remove(idx)));
            }
            _ if live.len() > 1 => {
                let a = live[rng.below(live.len() as u64) as usize];
                let b = live[rng.below(live.len() as u64) as usize];
                let rt = if rng.below(2) == 0 {
                    "knows"
                } else {
                    "works_for"
                };
                ops.push(Op::Relation(Relation::new(RelationId(0), rt, a, b)));
            }
            _ => {}
        }
    }
    ops
}

/// Outcomes as text: some ops fail (type mismatch on a relation, a
/// renamed-away name…) and must fail identically on both graphs.
fn run(g: &OntologyGraph, ops: &[Op]) -> Vec<String> {
    ops.iter()
        .map(|op| match op.clone() {
            Op::Upsert(c) => format!("{:?}", g.upsert_concept(c).map_err(|e| e.to_string())),
            Op::Rename(id, name) => format!(
                "{:?}",
                g.update_concept(
                    id,
                    ConceptPatch {
                        name: Some(name),
                        ..Default::default()
                    },
                )
                .map(|c| c.name)
                .map_err(|e| e.to_string())
            ),
            Op::Remove(id) => format!("{:?}", g.remove_concept(id).map_err(|e| e.to_string())),
            Op::Relation(r) => format!("{:?}", g.add_relation(r).map_err(|e| e.to_string())),
        })
        .collect()
}

fn views(g: &OntologyGraph) -> Vec<(&'static str, String)> {
    let js = |page: &[Concept]| format!("{:?}", page.iter().map(json).collect::<Vec<_>>());
    let mut v = Vec::new();
    let (total, all) = g.list_concepts_page(None, None, 0, 10_000, true, true);
    v.push(("all_sorted", format!("{total} {}", js(&all))));
    for ty in ["Person", "Employee", "Company"] {
        for sub in [false, true] {
            let (t, page) = g.list_concepts_page(Some(ty), None, 0, 10_000, true, sub);
            v.push(("by_type", format!("{ty}/{sub} {t} {}", js(&page))));
            v.push(("of_type", js(&g.concepts_of_type(ty, sub))));
        }
    }
    for needle in ["ali", "ren", "xyz", "-1", "zoë"] {
        let (t, page) = g.list_concepts_page(None, Some(needle), 0, 10_000, true, true);
        v.push(("search", format!("{needle} {t} {}", js(&page))));
        let (t, page) = g.list_concepts_page(Some("Person"), Some(needle), 2, 5, false, true);
        v.push(("search_paged", format!("{needle} {t} {}", js(&page))));
    }
    let mut after = None;
    let mut walked = Vec::new();
    loop {
        let (page, next) = g.list_concepts_after(None, None, after.as_ref(), 7, true);
        walked.extend(page.iter().map(json));
        match next {
            Some(k) => after = Some(k),
            None => break,
        }
    }
    v.push(("cursor_walk", format!("{walked:?}")));
    let mut every = g.all_concepts();
    every.sort_by_key(|c| c.id);
    v.push(("all_concepts", js(&every)));
    for c in all.iter().take(15) {
        let sg = g.expand(
            &[c.id],
            &TraversalSpec {
                max_depth: 2,
                ..Default::default()
            },
        );
        let mut nodes = sg.concepts;
        nodes.sort_by_key(|c| c.id);
        v.push((
            "expand",
            format!("{} {} {}", c.id, js(&nodes), sg.relations.len()),
        ));
    }
    v
}

#[test]
fn fully_evicted_graph_lists_and_traverses_like_p0() {
    for seed in [1u64, 7, 42] {
        let ops = script(seed, 400);
        let p0 = OntologyGraph::new(ontology());
        let out0 = run(&p0, &ops);

        let p1 = OntologyGraph::new(ontology());
        let out1 = run(&p1, &ops);
        assert_eq!(out0, out1, "seed {seed}: outcomes differ");

        let fake = Arc::new(Fake::default());
        for c in p1.all_concepts() {
            let l = loc(1, (c.id.0 % 4) as u32, c.id.0 * 100);
            fake.put(l, c.clone());
            p1.set_loc(c.id, l, false);
        }
        p1.set_payload_source(fake.clone());
        // `resident = false` before the source existed kept them resident;
        // sealing the four partitions evicts every one.
        assert_eq!(p1.resident_payloads(), p1.concept_count());
        let dropped: usize = (0..4).map(|p| p1.partition_sealed(1, p)).sum();
        assert_eq!(dropped, p1.concept_count());
        assert_eq!(p1.resident_payloads(), 0);

        let (a, b) = (views(&p0), views(&p1));
        assert_eq!(a.len(), b.len());
        for ((ka, va), (kb, vb)) in a.iter().zip(b.iter()) {
            assert_eq!(ka, kb);
            assert_eq!(va, vb, "seed {seed}: view `{ka}` differs between P0 and P1");
        }
        assert!(fake.hits() > 0);
        assert_eq!(p1.resident_payloads(), 0, "reads must not re-hydrate");
    }
}

// 7. Removing an evicted concept: side index clean, cascade intact.
#[test]
fn removing_an_evicted_concept_cleans_up_and_cascades() {
    let g = OntologyGraph::new(ontology());
    let fake = Arc::new(Fake::default());
    g.set_payload_source(fake.clone());
    let a = g.upsert_concept(person(0, "a")).unwrap();
    let b = g.upsert_concept(person(0, "b")).unwrap();
    let c = g.upsert_concept(person(0, "c")).unwrap();
    let ab = g
        .add_relation(Relation::new(RelationId(0), "knows", a, b))
        .unwrap();
    let ca = g
        .add_relation(Relation::new(RelationId(0), "knows", c, a))
        .unwrap();
    assert_eq!(g.relation_count(), 4, "two symmetric pairs");

    // `a` resident with a location (in the seal index), then evicted.
    fake.put(loc(1, 1, 0), g.get_concept(a).unwrap());
    g.set_loc(a, loc(1, 1, 0), true);
    g.set_loc(a, loc(1, 1, 0), false);
    assert_eq!(g.resident_payloads(), 2);

    let removed = g.remove_concept(a).unwrap();
    assert_eq!(removed.len(), 4);
    assert!(removed.contains(&ab) && removed.contains(&ca));
    assert_eq!(g.relation_count(), 0);
    assert!(matches!(
        g.get_concept(a),
        Err(GraphError::UnknownConcept(_))
    ));
    assert_eq!(g.concept_count(), 2);
    assert_eq!(g.resident_payloads(), 2);
    assert_eq!(fake.hits(), 0, "removal never needs the payload");

    // Nothing of `a` is left for the seal to find.
    assert_eq!(g.partition_sealed(1, 1), 0);
    assert_eq!(
        g.incident_relation_ids(b).unwrap(),
        Vec::<RelationId>::new()
    );
    assert_eq!(
        g.incident_relation_ids(c).unwrap(),
        Vec::<RelationId>::new()
    );

    // The id can be reused by an explicit upsert without a stale location.
    g.upsert_concept(Concept::new(a, "Person", "a again"))
        .unwrap();
    assert_eq!(g.resident_payloads(), 3);
    assert_eq!(g.partition_sealed(1, 1), 0);
    assert_eq!(g.get_concept(a).unwrap().name, "a again");
    assert_eq!(fake.hits(), 0);
}

// `clear_instances` forgets pending locations and the seal index too.
#[test]
fn clear_instances_resets_p1_state() {
    let g = OntologyGraph::new(ontology());
    let fake = Arc::new(Fake::default());
    g.set_payload_source(fake.clone());
    g.set_loc(ConceptId(5), loc(1, 1, 0), true);
    let id = g.upsert_concept(person(0, "x")).unwrap();
    g.set_loc(id, loc(1, 1, 8), true);
    g.clear_instances();
    assert_eq!(g.resident_payloads(), 0);
    assert_eq!(g.partition_sealed(1, 1), 0);
    // Id 5 after the reset: no stashed location gets attached.
    for _ in 0..5 {
        g.upsert_concept(person(0, &format!("n{}", g.concept_count())))
            .unwrap();
    }
    assert_eq!(g.partition_sealed(1, 1), 0);
    assert_eq!(g.resident_payloads(), 5);
    assert_eq!(fake.hits(), 0);
}
