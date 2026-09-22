// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! T1 (`STORAGE-PLAN.md` §8): walking a listing by cursor yields exactly
//! the offset listing, page after page, on every scan path (global sorted
//! set, one type bucket, k-way merge over subtypes, trigram search with and
//! without a type filter; relations globally, by adjacency and by type),
//! and a cursor stays valid when the graph changes under it.

use ontology_graph::{
    Concept, ConceptId, ConceptKey, ConceptType, Ontology, OntologyGraph, Relation, RelationId,
    RelationType,
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
    for (name, d, r) in [
        ("knows", "Person", "Person"),
        ("works_at", "Person", "Company"),
    ] {
        o.add_relation_type(RelationType {
            name: name.into(),
            domain: d.into(),
            range: r.into(),
            ..Default::default()
        })
        .unwrap();
    }
    o
}

fn graph() -> Arc<OntologyGraph> {
    let g = OntologyGraph::with_arc(ontology());
    // Names are unique per type (the graph refuses duplicates) but collide
    // across types, so the `(concept_type, name)` order is exercised.
    let names = ["ada", "alan", "bob", "carl", "grace", "linus", "eve", "zoe"];
    let mut people = Vec::new();
    for (i, n) in names.iter().enumerate() {
        let ty = if i % 2 == 1 { "Employee" } else { "Person" };
        people.push(
            g.upsert_concept(Concept::new(ConceptId(0), ty, *n))
                .unwrap(),
        );
    }
    let mut firms = Vec::new();
    for n in ["acme", "globex", "ada corp", "ada"] {
        firms.push(
            g.upsert_concept(Concept::new(ConceptId(0), "Company", n))
                .unwrap(),
        );
    }
    for i in 0..people.len() {
        for j in (i + 1)..people.len() {
            if (i + j) % 3 != 0 {
                g.add_relation(Relation::new(RelationId(0), "knows", people[i], people[j]))
                    .unwrap();
            }
        }
        g.add_relation(Relation::new(
            RelationId(0),
            "works_at",
            people[i],
            firms[i % 3],
        ))
        .unwrap();
    }
    g
}

fn key(c: &Concept) -> ConceptKey {
    (c.concept_type.clone(), c.name.clone(), c.id)
}

/// Every page by cursor, concatenated, with the final cursor.
fn walk_concepts(
    g: &OntologyGraph,
    ty: Option<&str>,
    needle: Option<&str>,
    subtypes: bool,
    limit: usize,
) -> (Vec<ConceptId>, usize) {
    let mut out = Vec::new();
    let mut cursor: Option<ConceptKey> = None;
    let mut pages = 0;
    loop {
        let (page, next) = g.list_concepts_after(ty, needle, cursor.as_ref(), limit, subtypes);
        pages += 1;
        assert!(page.len() <= limit);
        out.extend(page.iter().map(|c| c.id));
        match next {
            Some(n) => {
                assert_eq!(
                    &n,
                    &key(page.last().unwrap()),
                    "cursor = key of the last row"
                );
                cursor = Some(n);
            }
            None => {
                assert!(
                    page.len() < limit || page.is_empty(),
                    "a short page ends the walk"
                );
                return (out, pages);
            }
        }
        assert!(pages < 1000, "runaway walk");
    }
}

#[test]
fn cursor_walk_equals_offset_listing_on_every_scan_path() {
    let g = graph();
    let cases: [(Option<&str>, Option<&str>, bool); 8] = [
        (None, None, true),                  // global sorted set
        (Some("Person"), None, false),       // one bucket
        (Some("Person"), None, true),        // k-way merge Person + Employee
        (Some("Employee"), None, true),      // merge with a single bucket
        (None, Some("a"), true),             // trigram-less needle, global
        (Some("Person"), Some("a"), true),   // needle within a filter
        (None, Some("ada"), true),           // trigram intersection, global
        (Some("Person"), Some("ada"), true), // trigram intersection + filter
    ];
    for (ty, needle, subtypes) in cases {
        let (total, all) = g.list_concepts_page(ty, needle, 0, 10_000, true, subtypes);
        let expected: Vec<ConceptId> = all.iter().map(|c| c.id).collect();
        assert_eq!(total, expected.len());
        for limit in [1, 2, 3, 7, 100] {
            let (walked, _) = walk_concepts(&g, ty, needle, subtypes, limit);
            assert_eq!(
                walked, expected,
                "{ty:?} {needle:?} subtypes={subtypes} limit={limit}"
            );
        }
    }
    // The empty filter and a zero limit are harmless.
    let (page, next) = g.list_concepts_after(Some("Nope"), None, None, 5, true);
    assert!(page.is_empty() && next.is_none());
    let (page, next) = g.list_concepts_after(None, None, None, 0, true);
    assert!(page.is_empty() && next.is_none());
}

#[test]
fn a_cursor_survives_deletions_and_insertions_around_it() {
    let g = graph();
    let (p1, next) = g.list_concepts_after(None, None, None, 4, true);
    let cursor = next.unwrap();
    let (_, rest_before) = g.list_concepts_page(None, None, 4, 10_000, true, true);
    let rest_before: Vec<ConceptId> = rest_before.iter().map(|c| c.id).collect();

    // Delete the very concept the cursor names, plus one before it and one
    // after it; insert two concepts, one sorting before the cursor and one
    // after.
    g.remove_concept(p1[3].id).unwrap();
    g.remove_concept(p1[0].id).unwrap();
    let removed_after = rest_before[1];
    g.remove_concept(removed_after).unwrap();
    let before = g
        .upsert_concept(Concept::new(ConceptId(0), "Company", "aaa first"))
        .unwrap();
    let after = g
        .upsert_concept(Concept::new(ConceptId(0), "Person", "zzz last"))
        .unwrap();

    let (walked, _) = {
        let mut out = Vec::new();
        let mut c = Some(cursor.clone());
        while let Some(k) = c {
            let (page, next) = g.list_concepts_after(None, None, Some(&k), 3, true);
            out.extend(page.iter().map(|x| x.id));
            c = next;
        }
        (out, ())
    };
    let expected: Vec<ConceptId> = rest_before
        .iter()
        .copied()
        .filter(|id| *id != removed_after)
        .chain(std::iter::once(after))
        .collect();
    assert_eq!(
        walked, expected,
        "continues strictly after the (deleted) cursor key"
    );
    assert!(
        !walked.contains(&before),
        "an insertion before the cursor is not revisited"
    );
    assert!(!walked.contains(&p1[3].id));
}

#[test]
fn relations_walk_by_cursor_on_every_path() {
    let g = graph();
    let people: Vec<ConceptId> = g
        .list_concepts_page(Some("Person"), None, 0, 100, true, true)
        .1
        .iter()
        .map(|c| c.id)
        .collect();
    let hub = people[0];
    let cases: [(Option<ConceptId>, Option<ConceptId>, Option<&str>); 5] = [
        (None, None, None),               // global id order
        (Some(hub), None, None),          // adjacency (out)
        (None, Some(hub), None),          // adjacency (in)
        (Some(hub), None, Some("knows")), // typed adjacency
        (None, None, Some("works_at")),   // type filter on the global scan
    ];
    for (src, tgt, rt) in cases {
        let (total, all) = g.list_relations_page(src, tgt, rt, 0, 10_000, true);
        let expected: Vec<RelationId> = all.iter().map(|r| r.id).collect();
        assert_eq!(total, expected.len());
        for limit in [1, 2, 5, 100] {
            let mut out = Vec::new();
            let mut cursor: Option<RelationId> = None;
            loop {
                let (page, next) = g.list_relations_after(src, tgt, rt, cursor, limit);
                out.extend(page.iter().map(|r| r.id));
                if let Some(n) = next {
                    assert_eq!(n, page.last().unwrap().id);
                    cursor = Some(n);
                } else {
                    break;
                }
            }
            assert_eq!(out, expected, "{src:?} {tgt:?} {rt:?} limit={limit}");
        }
    }
    // Deleting the cursor relation does not break the walk.
    let (p1, next) = g.list_relations_after(None, None, None, None, 3);
    let cursor = next.unwrap();
    g.remove_relation(p1[2].id).unwrap();
    let (p2, _) = g.list_relations_after(None, None, None, Some(cursor), 3);
    assert!(p2.iter().all(|r| r.id > cursor));
    assert_eq!(p2.len(), 3);
}
