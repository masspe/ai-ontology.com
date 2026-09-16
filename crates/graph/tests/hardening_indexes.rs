// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Derived-index invariants (`PERFORMANCE.md` R2, R5, R6, §4.2–4.5): every
//! mutation keeps every index consistent with the primary maps, listings
//! come out in the documented order, and generations move exactly once per
//! mutation and never on reads.

mod hardening_common;
use hardening_common::*;

use ontology_graph::{Concept, ConceptId, ConceptPatch, OntologyGraph, Relation, RelationId};

/// Reference listing of concepts in the R6 order `(concept_type, name, id)`,
/// computed from the primary map with a brute-force filter.
fn brute_force_concepts(
    g: &OntologyGraph,
    concept_type: Option<&str>,
    needle: Option<&str>,
    include_subtypes: bool,
) -> Vec<Concept> {
    let onto = g.ontology();
    let mut all: Vec<Concept> = g
        .all_concepts()
        .into_iter()
        .filter(|c| match concept_type {
            None => true,
            Some(t) if include_subtypes => onto.is_subtype(&c.concept_type, t),
            Some(t) => c.concept_type == t,
        })
        .filter(|c| needle.is_none_or(|n| c.name.to_lowercase().contains(n)))
        .collect();
    // R6: `(concept_type, name, id)` globally; with a type filter the
    // buckets of the accepted types are merged on `(name, id)`.
    if concept_type.is_some() {
        all.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    } else {
        all.sort_by(|a, b| {
            a.concept_type
                .cmp(&b.concept_type)
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.id.cmp(&b.id))
        });
    }
    all
}

fn ids(page: &[Concept]) -> Vec<ConceptId> {
    page.iter().map(|c| c.id).collect()
}

/// Check every listing path against the brute force for one filter.
fn assert_listing_matches(
    g: &OntologyGraph,
    concept_type: Option<&str>,
    needle: Option<&str>,
    include_subtypes: bool,
    context: &str,
) {
    let expected = brute_force_concepts(g, concept_type, needle, include_subtypes);
    let (total, page) = g.list_concepts_page(concept_type, needle, 0, 1000, true, include_subtypes);
    assert_eq!(total, expected.len(), "{context}: total");
    assert_eq!(ids(&page), ids(&expected), "{context}: order/content");
    // Pages of 2 stitched together equal the full listing, whichever way
    // `total` is counted.
    for track_total in [true, false] {
        let mut stitched: Vec<ConceptId> = Vec::new();
        let mut offset = 0;
        loop {
            let (t, p) = g.list_concepts_page(
                concept_type,
                needle,
                offset,
                2,
                track_total,
                include_subtypes,
            );
            if track_total {
                assert_eq!(t, expected.len(), "{context}: exact total on every page");
            } else {
                assert!(
                    t <= expected.len(),
                    "{context}: lower bound {t} above the truth"
                );
                if !p.is_empty() {
                    assert!(
                        t >= offset + p.len(),
                        "{context}: lower bound {t} below offset + page"
                    );
                }
            }
            if p.is_empty() {
                break;
            }
            stitched.extend(ids(&p));
            offset += 2;
        }
        assert_eq!(
            stitched,
            ids(&expected),
            "{context}: stitched pages (track_total={track_total})"
        );
    }
}

fn all_filters(g: &OntologyGraph, context: &str) {
    for ct in [
        None,
        Some("Person"),
        Some("Researcher"),
        Some("Paper"),
        Some("Topic"),
        Some("Nope"),
    ] {
        for needle in [
            None,
            Some("ada"),
            Some("pap"),
            Some("a"),
            Some("zzz"),
            Some("é"),
        ] {
            for sub in [false, true] {
                assert_listing_matches(
                    g,
                    ct,
                    needle,
                    sub,
                    &format!("{context} ct={ct:?} q={needle:?} sub={sub}"),
                );
            }
        }
    }
}

/// R2/R6: after every kind of concept mutation, all listing paths (global,
/// by type, by type with subtypes, trigram, short-needle scan) agree with a
/// brute-force filter of the primary map.
#[test]
fn concept_listings_agree_with_brute_force_after_every_mutation() {
    let g = graph();
    all_filters(&g, "empty");
    let ada = concept(&g, "Researcher", "Ada Lovelace");
    let alan = concept(&g, "Person", "alan turing");
    let paper = concept(&g, "Paper", "Paper on Papers");
    let _topic = concept(&g, "Topic", "Éléphant");
    let _short = concept(&g, "Person", "Al");
    all_filters(&g, "after inserts");
    // Rename (moves in the sorted index and re-keys the trigrams).
    g.update_concept(
        alan,
        ConceptPatch {
            name: Some("Zed Adams".into()),
            ..Default::default()
        },
    )
    .unwrap();
    all_filters(&g, "after rename");
    // Upsert with explicit id and a new name.
    g.upsert_concept(Concept::new(ada, "Researcher", "Augusta Ada King"))
        .unwrap();
    all_filters(&g, "after explicit-id rename");
    // Relations do not disturb concept listings.
    relation(&g, "authored", ada, paper);
    all_filters(&g, "after relation");
    // Delete, including a concept with incident relations.
    g.remove_concept(ada).unwrap();
    all_filters(&g, "after delete");
    // Re-insert under the freed name.
    concept(&g, "Researcher", "Ada Lovelace");
    all_filters(&g, "after reinsert");
    g.clear_instances();
    all_filters(&g, "after clear");
}

/// R6: the trigram fast path (needle ≥ 3 chars) returns the same page as
/// the linear scan would, for both values of `track_total`, including when
/// the ids were allocated in an order unrelated to the sort order.
#[test]
fn trigram_fast_path_pages_are_independent_of_track_total() {
    let g = graph();
    // Ids ascend while names descend, so id order ≠ sort order.
    for name in [
        "Alice Zed",
        "Alice Yves",
        "Alice Xu",
        "Alice Bob",
        "Alice Ann",
    ] {
        concept(&g, "Person", name);
    }
    concept(&g, "Paper", "Alice in Wonderland");
    let exact = g.list_concepts_page(None, Some("alice"), 0, 2, true, true);
    let lower = g.list_concepts_page(None, Some("alice"), 0, 2, false, true);
    assert_eq!(names(&exact.1), vec!["Alice in Wonderland", "Alice Ann"]);
    assert_eq!(
        names(&lower.1),
        names(&exact.1),
        "page must not depend on track_total"
    );
    assert_eq!(exact.0, 6);
    assert!(lower.0 >= 2);
    let (_, p2) = g.list_concepts_page(None, Some("alice"), 2, 2, false, true);
    assert_eq!(names(&p2), vec!["Alice Bob", "Alice Xu"]);
    let (_, p3) = g.list_concepts_page(Some("Person"), Some("alice"), 3, 5, false, false);
    assert_eq!(names(&p3), vec!["Alice Yves", "Alice Zed"]);
    // A needle with a trigram nobody has short-circuits to empty.
    let (t, p) = g.list_concepts_page(None, Some("alicq"), 0, 10, false, true);
    assert!(t == 0 && p.is_empty());
    // The needle is matched as a substring after the trigram intersection:
    // "ice a" shares every trigram with "Alice Ann" only in order.
    assert_eq!(
        names(
            &g.list_concepts_page(None, Some("ice a"), 0, 10, true, true)
                .1
        ),
        vec!["Alice Ann"]
    );
}

/// R6 with `include_subtypes`: the order across descendant types is
/// `(name, id)`, and it is the same whether the needle takes the linear
/// scan (< 3 chars) or the trigram fast path (≥ 3 chars).
#[test]
fn subtype_listing_order_is_the_same_with_and_without_a_needle() {
    let g = graph();
    // Chosen so that `(name, id)` order and `(concept_type, name)` order
    // disagree: the Researcher sorts before the Person by name.
    concept(&g, "Person", "Zoe Ann");
    concept(&g, "Researcher", "Ann Lee");
    concept(&g, "Person", "Bob");
    concept(&g, "Researcher", "Carl Sagan");
    let list = |needle: Option<&str>| {
        names(
            &g.list_concepts_page(Some("Person"), needle, 0, 10, true, true)
                .1,
        )
    };
    assert_eq!(list(None), vec!["Ann Lee", "Bob", "Carl Sagan", "Zoe Ann"]);
    assert_eq!(
        list(Some("n")),
        vec!["Ann Lee", "Carl Sagan", "Zoe Ann"],
        "linear scan"
    );
    assert_eq!(
        list(Some("ann")),
        vec!["Ann Lee", "Zoe Ann"],
        "trigram path"
    );
    assert_eq!(list(Some("an")), vec!["Ann Lee", "Carl Sagan", "Zoe Ann"]);
    // Pages through the trigram path with a lower-bound total stitch in
    // that same order.
    let (_, p0) = g.list_concepts_page(Some("Person"), Some("ann"), 0, 1, false, true);
    let (_, p1) = g.list_concepts_page(Some("Person"), Some("ann"), 1, 1, false, true);
    assert_eq!(
        [names(&p0), names(&p1)].concat(),
        vec!["Ann Lee", "Zoe Ann"]
    );
    // Without subtypes the bucket is a single type: `(name, id)` again.
    assert_eq!(
        names(
            &g.list_concepts_page(Some("Person"), Some("ann"), 0, 10, true, false)
                .1
        ),
        vec!["Zoe Ann"]
    );
    // Globally the order is `(concept_type, name, id)`: Person before Researcher.
    assert_eq!(
        names(&g.list_concepts_page(None, Some("ann"), 0, 10, true, true).1),
        vec!["Zoe Ann", "Ann Lee"]
    );
}

/// Pagination arithmetic: `offset` past the end yields an empty page with
/// an exact total, `limit = 0` yields nothing, `track_total = false` gives
/// `offset + page.len()` as the lower bound (§4.3).
#[test]
fn pagination_edges_offset_past_end_zero_limit_and_lower_bound_total() {
    let g = graph();
    for i in 0..5 {
        concept(&g, "Person", &format!("P{i}"));
    }
    let (t, p) = g.list_concepts_page(None, None, 10, 3, true, true);
    assert!(t == 5 && p.is_empty());
    assert_eq!(
        g.list_concepts_page(None, None, 10, 3, false, true).1.len(),
        0
    );
    assert_eq!(
        g.list_concepts_page(None, None, 0, 0, true, true).1.len(),
        0
    );
    assert_eq!(g.list_concepts_page(None, None, 0, 0, true, true).0, 5);
    let (t, p) = g.list_concepts_page(None, None, 1, 2, false, true);
    assert_eq!(p.len(), 2);
    assert_eq!(t, 3, "lower bound is offset + page.len()");
    let (t, p) = g.list_concepts_page(None, None, 3, 10, false, true);
    assert_eq!(p.len(), 2);
    assert_eq!(t, 5, "the last page reaches the exact total");
    // R17: a huge `offset` / `limit` must neither overflow nor pre-allocate.
    assert_eq!(
        g.list_concepts_page(None, None, usize::MAX, 1, true, true)
            .1
            .len(),
        0
    );
    assert_eq!(
        g.list_concepts_page(None, Some("p"), usize::MAX, usize::MAX, false, true)
            .1
            .len(),
        0
    );
    assert_eq!(
        g.list_concepts_page(None, None, 0, usize::MAX, true, true)
            .1
            .len(),
        5
    );
    assert_eq!(
        g.list_concepts_page(Some("Person"), Some("p0"), 0, usize::MAX, false, false)
            .1
            .len(),
        1
    );
    assert_eq!(
        g.list_concepts_page(None, Some("p0p"), usize::MAX, usize::MAX, false, true)
            .1
            .len(),
        0
    );
    // Same for relations.
    let a = concept(&g, "Person", "A");
    let b = concept(&g, "Person", "B");
    relation(&g, "knows", a, b);
    let (t, p) = g.list_relations_page(None, None, None, 5, 3, true);
    assert!(t == 2 && p.is_empty());
    assert_eq!(
        g.list_relations_page(Some(a), None, None, 0, 0, true)
            .1
            .len(),
        0
    );
    // `source = a` matches only `a -> b`; the materialized inverse has
    // `source = b`. An offset past the single match still reports it.
    assert_eq!(g.list_relations_page(Some(a), None, None, 1, 5, false).0, 1);
    assert_eq!(
        g.list_relations_page(None, None, None, 0, usize::MAX, true)
            .1
            .len(),
        2
    );
    assert_eq!(
        g.list_relations_page(Some(a), None, Some("knows"), 0, usize::MAX, false)
            .1
            .len(),
        1
    );
    assert_eq!(
        g.list_relations_page(Some(a), None, None, usize::MAX, usize::MAX, false)
            .1
            .len(),
        0
    );
}

/// R2/R5: generations move exactly once per mutation, never on reads, and
/// only the family that changed moves — except `remove_concept`, which
/// cascades and therefore bumps both.
#[test]
fn generations_bump_once_per_mutation_and_never_on_reads() {
    let g = graph();
    let snap = |g: &OntologyGraph| (g.concepts_generation(), g.relations_generation());
    let (c0, r0) = snap(&g);
    let a = concept(&g, "Person", "A");
    assert_eq!(snap(&g), (c0 + 1, r0));
    let b = concept(&g, "Person", "B");
    let p = concept(&g, "Paper", "P");
    assert_eq!(snap(&g), (c0 + 3, r0));
    // Reads of every kind.
    g.list_concepts_page(None, None, 0, 10, true, true);
    g.list_concepts_page(Some("Person"), Some("a"), 0, 10, false, false);
    g.list_relations_page(Some(a), None, None, 0, 10, true);
    g.get_concept(a).unwrap();
    g.find_by_name("Person", "a");
    g.concepts_of_type("Person", true);
    g.incident_relation_ids(a).unwrap();
    g.ontology();
    g.check_ontology(&g.ontology()).unwrap();
    g.preview_concept_update(a, &ConceptPatch::default())
        .unwrap();
    let mut prepared = Concept::new(ConceptId(0), "Person", "Prepared");
    g.prepare_concept(&mut prepared).unwrap();
    let mut prel = Relation::new(RelationId(0), "authored", a, p);
    g.prepare_relation(&mut prel).unwrap();
    assert_eq!(snap(&g), (c0 + 3, r0), "reads and prepares bump nothing");
    // A rejected write bumps nothing either.
    assert!(g
        .upsert_concept(Concept::new(ConceptId(0), "Person", "a"))
        .is_err());
    assert!(g
        .add_relation(Relation::new(RelationId(0), "authored", p, a))
        .is_err());
    assert_eq!(snap(&g), (c0 + 3, r0));
    // Relation writes: one bump each, even when an inverse is materialized.
    let auth = g.apply_prepared_relation(prel).unwrap();
    assert_eq!(snap(&g), (c0 + 3, r0 + 1));
    let knows = relation(&g, "knows", a, b);
    assert_eq!(snap(&g), (c0 + 3, r0 + 2));
    g.insert_relation_exact(Relation::new(RelationId(900), "cites", p, p))
        .unwrap();
    assert_eq!(snap(&g), (c0 + 3, r0 + 3));
    g.update_relation(auth, Default::default()).unwrap();
    assert_eq!(snap(&g), (c0 + 3, r0 + 4));
    g.remove_relation(knows).unwrap();
    assert_eq!(snap(&g), (c0 + 3, r0 + 5));
    // Concept writes.
    g.update_concept(
        a,
        ConceptPatch {
            description: Some("d".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(snap(&g), (c0 + 4, r0 + 5));
    g.apply_prepared_concept(prepared).unwrap();
    assert_eq!(snap(&g), (c0 + 5, r0 + 5));
    // The cascade bumps both, once each, even with several relations.
    g.remove_concept(a).unwrap();
    assert_eq!(snap(&g), (c0 + 6, r0 + 6));
    // Schema edits are not instance mutations.
    g.extend_ontology(|o| {
        o.add_concept_type(ct("Venue", None, None));
        Ok(())
    })
    .unwrap();
    assert_eq!(snap(&g), (c0 + 6, r0 + 6));
    g.clear_instances();
    assert_eq!(snap(&g), (c0 + 7, r0 + 7));
}

/// R5: a cached page is served only while the generation is unchanged;
/// every concept mutation makes the next call rebuild from the indexes.
#[test]
fn list_cache_never_serves_a_page_older_than_the_last_mutation() {
    let g = graph();
    let a = concept(&g, "Person", "Alice");
    let (t, p) = g.list_concepts_page(Some("Person"), Some("ali"), 0, 10, true, true);
    assert_eq!((t, names(&p)), (1, vec!["Alice".to_string()]));
    concept(&g, "Person", "Alina");
    let (t, p) = g.list_concepts_page(Some("Person"), Some("ali"), 0, 10, true, true);
    assert_eq!(
        (t, names(&p)),
        (2, vec!["Alice".to_string(), "Alina".into()])
    );
    g.update_concept(
        a,
        ConceptPatch {
            description: Some("v2".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let (_, p) = g.list_concepts_page(Some("Person"), Some("ali"), 0, 10, true, true);
    assert_eq!(
        p[0].description, "v2",
        "in-place field changes reach cached pages"
    );
    g.remove_concept(a).unwrap();
    let (t, p) = g.list_concepts_page(Some("Person"), Some("ali"), 0, 10, true, true);
    assert_eq!((t, names(&p)), (1, vec!["Alina".to_string()]));
}

// ---------------------------------------------------------------------------
// list_relations_page fast paths vs brute force
// ---------------------------------------------------------------------------

fn brute_force_relations(
    g: &OntologyGraph,
    source: Option<ConceptId>,
    target: Option<ConceptId>,
    rt: Option<&str>,
) -> Vec<RelationId> {
    let mut all: Vec<RelationId> = g
        .all_relations()
        .into_iter()
        .filter(|r| source.is_none_or(|s| r.source == s))
        .filter(|r| target.is_none_or(|t| r.target == t))
        .filter(|r| rt.is_none_or(|t| r.relation_type == t))
        .map(|r| r.id)
        .collect();
    all.sort();
    all
}

fn assert_relation_paths_agree(g: &OntologyGraph, nodes: &[ConceptId], context: &str) {
    let types = [
        None,
        Some("authored"),
        Some("knows"),
        Some("cites"),
        Some("about"),
        Some("nope"),
    ];
    let mut node_opts: Vec<Option<ConceptId>> = vec![None, Some(ConceptId(9999))];
    node_opts.extend(nodes.iter().map(|n| Some(*n)));
    for s in &node_opts {
        for t in &node_opts {
            for rt in types {
                let expected = brute_force_relations(g, *s, *t, rt);
                let (total, page) = g.list_relations_page(*s, *t, rt, 0, 10_000, true);
                let got: Vec<RelationId> = page.iter().map(|r| r.id).collect();
                assert_eq!(
                    total,
                    expected.len(),
                    "{context} s={s:?} t={t:?} rt={rt:?}: total"
                );
                assert_eq!(
                    got, expected,
                    "{context} s={s:?} t={t:?} rt={rt:?}: ids (sorted by id)"
                );
                // Paging with a lower-bound total stitches to the same list.
                let mut stitched = Vec::new();
                let mut offset = 0;
                loop {
                    let (lb, p) = g.list_relations_page(*s, *t, rt, offset, 3, false);
                    assert!(lb >= offset.min(expected.len()) + p.len());
                    if p.is_empty() {
                        break;
                    }
                    stitched.extend(p.iter().map(|r| r.id));
                    offset += 3;
                }
                assert_eq!(
                    stitched, expected,
                    "{context} s={s:?} t={t:?} rt={rt:?}: stitched"
                );
            }
        }
    }
}

/// §4.4: every fast path of `list_relations_page` (by source, by target,
/// both, typed variants, full scan) agrees with a brute-force filter of
/// the primary map, through a deterministic pseudo-random sequence of
/// inserts and deletes — including symmetric inverses and self-loops.
#[test]
fn relation_listing_fast_paths_agree_with_brute_force_under_random_mutations() {
    let g = graph();
    let mut rng = Rng::new(0x5EED_2026);
    let persons: Vec<ConceptId> = (0..4)
        .map(|i| concept(&g, "Person", &format!("P{i}")))
        .collect();
    let papers: Vec<ConceptId> = (0..3)
        .map(|i| concept(&g, "Paper", &format!("D{i}")))
        .collect();
    let topics: Vec<ConceptId> = (0..2)
        .map(|i| concept(&g, "Topic", &format!("T{i}")))
        .collect();
    let mut nodes = persons.clone();
    nodes.extend(&papers);
    nodes.extend(&topics);
    let mut live: Vec<RelationId> = Vec::new();
    for step in 0..120 {
        let op = rng.below(10);
        if op < 6 || live.is_empty() {
            let rel = match rng.below(4) {
                0 => Relation::new(
                    RelationId(0),
                    "authored",
                    persons[rng.below(4)],
                    papers[rng.below(3)],
                ),
                1 => Relation::new(
                    RelationId(0),
                    "knows",
                    persons[rng.below(4)],
                    persons[rng.below(4)],
                ),
                2 => Relation::new(
                    RelationId(0),
                    "cites",
                    papers[rng.below(3)],
                    papers[rng.below(3)],
                ),
                _ => Relation::new(
                    RelationId(0),
                    "about",
                    papers[rng.below(3)],
                    topics[rng.below(2)],
                ),
            };
            // `about` is many-to-one, so some inserts are legitimately refused.
            if let Ok(id) = g.add_relation(rel) {
                live.push(id);
            }
        } else if op < 8 {
            let i = rng.below(live.len());
            let id = live.swap_remove(i);
            // The id may already be gone if it was a materialized inverse
            // removed through its concept; both outcomes are fine.
            let _ = g.remove_relation(id);
        } else if op == 8 && rng.chance(3) {
            // Remove and re-create a person: exercises the cascade.
            let i = rng.below(4);
            let old = persons[i];
            g.remove_concept(old).unwrap();
            let fresh = concept(&g, "Person", &format!("P{i}"));
            let mut refreshed = persons.clone();
            refreshed[i] = fresh;
            nodes = refreshed.clone();
            nodes.extend(&papers);
            nodes.extend(&topics);
            live.retain(|id| g.get_relation(*id).is_ok());
            return_persons(&mut nodes, &refreshed);
            // Continue with the refreshed handles.
            let g_ref = &g;
            assert_relation_paths_agree(g_ref, &nodes, &format!("step {step} after cascade"));
            continue;
        }
        live.retain(|id| g.get_relation(*id).is_ok());
        if step % 10 == 0 {
            assert_relation_paths_agree(&g, &nodes, &format!("step {step}"));
        }
    }
    assert_relation_paths_agree(&g, &nodes, "final");
    // Adjacency mirrors the primary map: every live relation is in exactly
    // one out-list and one in-list.
    for r in g.all_relations() {
        assert_eq!(
            g.outgoing(r.source).iter().filter(|x| x.id == r.id).count(),
            1
        );
        assert_eq!(
            g.incoming(r.target).iter().filter(|x| x.id == r.id).count(),
            1
        );
        assert_eq!(
            g.outgoing_typed(r.source, std::slice::from_ref(&r.relation_type))
                .iter()
                .filter(|x| x.id == r.id)
                .count(),
            1
        );
    }
    let degree_sum: usize = nodes.iter().map(|n| g.outgoing(*n).len()).sum();
    assert_eq!(
        degree_sum,
        g.relation_count(),
        "no dangling adjacency entries"
    );
}

fn return_persons(nodes: &mut [ConceptId], persons: &[ConceptId]) {
    nodes[..persons.len()].copy_from_slice(persons);
}

/// `remove_concept` scrubs the surviving endpoint's plain and typed
/// adjacency, so a later listing by that endpoint sees nothing stale.
#[test]
fn remove_concept_scrubs_the_surviving_endpoints_adjacency() {
    let g = graph();
    let a = concept(&g, "Person", "A");
    let b = concept(&g, "Person", "B");
    let p = concept(&g, "Paper", "P");
    relation(&g, "authored", a, p);
    relation(&g, "authored", b, p);
    relation(&g, "knows", a, b);
    let removed = g.remove_concept(a).unwrap();
    assert_eq!(removed.len(), 3, "authored + both directions of knows");
    assert_eq!(g.relation_count(), 1);
    assert_eq!(g.incoming(p).len(), 1);
    assert_eq!(g.incoming(p)[0].source, b);
    let out_b = g.outgoing(b);
    assert_eq!(
        out_b.len(),
        1,
        "b -> a inverse gone, b's own `authored` kept"
    );
    assert_eq!(
        (out_b[0].relation_type.as_str(), out_b[0].target),
        ("authored", p)
    );
    assert!(g.incoming(b).is_empty(), "a -> b gone");
    assert!(g.outgoing_typed(b, &["knows".to_string()]).is_empty());
    assert!(g.incoming_typed(b, &["knows".to_string()]).is_empty());
    assert_eq!(
        g.list_relations_page(None, Some(p), Some("authored"), 0, 10, true)
            .0,
        1
    );
    assert_eq!(
        g.list_relations_page(Some(b), None, Some("knows"), 0, 10, true)
            .0,
        0
    );
    assert_eq!(g.list_relations_page(Some(a), None, None, 0, 10, true).0, 0);
    assert!(g.remove_concept(a).is_err(), "second removal is an error");
    for id in removed {
        assert!(g.get_relation(id).is_err());
    }
}

/// `find_by_name` is case-insensitive and type-scoped; the same name may
/// exist under two non-disjoint types.
#[test]
fn find_by_name_is_case_insensitive_and_scoped_by_type() {
    let g = graph();
    let person = concept(&g, "Person", "Ada");
    let topic = concept(&g, "Topic", "ADA");
    assert_eq!(g.find_by_name("Person", "ada"), Some(person));
    assert_eq!(g.find_by_name("Person", "ADA"), Some(person));
    assert_eq!(g.find_by_name("Topic", "Ada"), Some(topic));
    assert_eq!(g.find_by_name("Researcher", "Ada"), None, "not inherited");
    assert!(
        g.upsert_concept(Concept::new(ConceptId(0), "Person", " Ada"))
            .is_ok(),
        "whitespace is significant"
    );
}
