// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Write-ahead API (`STORAGE.md` R8, `PERFORMANCE.md` R1/R2): every
//! mutation splits into a pure `prepare_*` / `preview_*` half and an
//! `apply_*` half, ids are born in `prepare_*`, and the one-piece forms are
//! exactly the composition of the two.

mod hardening_common;
use hardening_common::*;

use ahash::AHashMap;
use ontology_graph::{
    Action, ActionId, ActionPatch, Concept, ConceptId, ConceptPatch, GraphError, PropertyValue,
    Relation, RelationId, RelationPatch, Rule, RuleId, RulePatch, MAX_CONCEPT_ID,
};

// ---------------------------------------------------------------------------
// prepare_concept / apply_prepared_concept
// ---------------------------------------------------------------------------

/// R8: a prepared-but-unapplied concept is invisible to every read path.
#[test]
fn prepared_concept_is_invisible_until_applied() {
    let g = graph();
    let mut c = Concept::new(ConceptId(0), "Person", "Alice");
    g.prepare_concept(&mut c).unwrap();
    let id = c.id;
    assert!(matches!(
        g.get_concept(id),
        Err(GraphError::UnknownConcept(_))
    ));
    assert!(g.find_by_name("Person", "alice").is_none());
    assert_eq!(g.list_concepts_page(None, None, 0, 10, true, true).0, 0);
    assert_eq!(
        g.list_concepts_page(Some("Person"), None, 0, 10, true, false)
            .0,
        0
    );
    assert_eq!(
        g.list_concepts_page(None, Some("ali"), 0, 10, true, true).0,
        0
    );
    assert!(g.concepts_of_type("Person", true).is_empty());
    assert!(g.all_concepts().is_empty());
    assert!(g.incident_relation_ids(id).is_err());
    // The id is consumed: the next prepare gets a later one.
    let mut d = Concept::new(ConceptId(0), "Person", "Bob");
    g.prepare_concept(&mut d).unwrap();
    assert!(d.id > id);
    // Applying out of order is fine; ids stay as allocated.
    assert_eq!(g.apply_prepared_concept(d).unwrap(), ConceptId(2));
    assert_eq!(g.apply_prepared_concept(c).unwrap(), ConceptId(1));
    assert_eq!(g.concept_count(), 2);
}

/// D6: a concept id at or above 2^32 is refused where it is born, whether
/// supplied explicitly or reached by allocation after observing a big id.
#[test]
fn concept_ids_beyond_the_storage_limit_are_refused_at_prepare() {
    let g = graph();
    let mut too_big = Concept::new(ConceptId(MAX_CONCEPT_ID + 1), "Person", "Big");
    assert!(matches!(
        g.prepare_concept(&mut too_big),
        Err(GraphError::ConceptIdOutOfRange(id)) if id == ConceptId(MAX_CONCEPT_ID + 1)
    ));
    assert_eq!(g.concept_count(), 0);
    // A refused explicit id must not poison the allocator: the next fresh
    // concept still gets the first free id.
    assert_eq!(concept(&g, "Person", "First"), ConceptId(1));
    let p = concept(&g, "Paper", "P");
    // The largest representable id is fine…
    let mut edge = Concept::new(ConceptId(MAX_CONCEPT_ID), "Person", "Edge");
    g.prepare_concept(&mut edge).unwrap();
    g.apply_prepared_concept(edge).unwrap();
    assert!(ConceptId(MAX_CONCEPT_ID).fits_storage());
    // …but the allocator now sits past the limit (the format's hard
    // ceiling, D6), and the next fresh concept is refused rather than
    // silently persisted unrepresentably.
    let mut next = Concept::new(ConceptId(0), "Person", "Next");
    assert!(matches!(
        g.prepare_concept(&mut next),
        Err(GraphError::ConceptIdOutOfRange(_))
    ));
    assert_eq!(g.concept_count(), 3);
    // The edge id itself is a valid relation endpoint.
    let mut r = Relation::new(RelationId(0), "authored", ConceptId(MAX_CONCEPT_ID), p);
    g.prepare_relation(&mut r).unwrap();
    assert_ne!(r.id, RelationId(0));
}

/// `apply_prepared_concept` cannot be used to change a concept's type, even
/// when `prepare_concept` is skipped (defensive re-check, H5).
#[test]
fn apply_prepared_concept_defends_the_name_index_without_prepare() {
    let g = graph();
    let alice = concept(&g, "Person", "Alice");
    // Direct apply with a colliding name and a fresh id: refused.
    let err = g.apply_prepared_concept(Concept::new(ConceptId(77), "Person", "alice"));
    assert!(matches!(err, Err(GraphError::DuplicateConcept(_, _))));
    assert_eq!(g.find_by_name("Person", "alice"), Some(alice));
    assert!(g.get_concept(ConceptId(77)).is_err());
    // Direct apply with an unrelated fresh id registers it and raises the
    // watermark past it.
    g.apply_prepared_concept(Concept::new(ConceptId(77), "Person", "Zed"))
        .unwrap();
    let mut next = Concept::new(ConceptId(0), "Person", "Next");
    g.prepare_concept(&mut next).unwrap();
    assert_eq!(next.id, ConceptId(78));
}

// ---------------------------------------------------------------------------
// prepare_relation / apply_prepared_relation / insert_relation_exact
// ---------------------------------------------------------------------------

/// `prepare_relation` performs every check `add_relation` would — endpoint
/// existence, schema, cardinality, functional — and rejects without
/// allocating or touching adjacency.
#[test]
fn prepare_relation_rejects_without_side_effects() {
    let g = graph();
    let p1 = concept(&g, "Paper", "P1");
    let p2 = concept(&g, "Paper", "P2");
    let t1 = concept(&g, "Topic", "T1");
    let t2 = concept(&g, "Topic", "T2");
    relation(&g, "about", p1, t1);
    let gen = g.relations_generation();
    let cases: Vec<(&str, Relation)> = vec![
        (
            "unknown type",
            Relation::new(RelationId(0), "loves", p1, t2),
        ),
        (
            "second outbound of many-to-one",
            Relation::new(RelationId(0), "about", p1, t2),
        ),
        (
            "range violated",
            Relation::new(RelationId(0), "about", p1, p2),
        ),
        (
            "unknown source",
            Relation::new(RelationId(0), "cites", ConceptId(999), p2),
        ),
        (
            "unknown target",
            Relation::new(RelationId(0), "cites", p1, ConceptId(999)),
        ),
    ];
    for (label, mut r) in cases {
        let res = g.prepare_relation(&mut r);
        assert!(res.is_err(), "{label} must be refused");
        assert_eq!(r.id, RelationId(0), "{label}: no id allocated");
    }
    assert_eq!(g.relation_count(), 1);
    assert_eq!(g.outgoing(p1).len(), 1);
    assert_eq!(g.incoming(t2).len(), 0);
    assert_eq!(g.relations_generation(), gen);
    // Specific variants for the interesting ones.
    let mut r = Relation::new(RelationId(0), "about", p1, t2);
    assert!(matches!(
        g.prepare_relation(&mut r),
        Err(GraphError::CardinalityViolation { ref relation, concept }) if relation == "about" && concept == p1
    ));
    let mut r = Relation::new(RelationId(0), "loves", p1, t2);
    assert!(matches!(
        g.prepare_relation(&mut r),
        Err(GraphError::UnknownRelationType(t)) if t == "loves"
    ));
    // A many-to-one relation to a *different* source is fine.
    relation(&g, "about", p2, t1);
}

/// A caller-supplied relation id that collides with a live relation is
/// reassigned by `prepare_relation` (export re-ingest), never overwritten.
#[test]
fn prepare_relation_reassigns_a_colliding_explicit_id() {
    let g = graph();
    let a = concept(&g, "Person", "A");
    let p = concept(&g, "Paper", "P");
    let q = concept(&g, "Paper", "Q");
    let first = relation(&g, "authored", a, p);
    let mut clash = Relation::new(first, "authored", a, q);
    g.prepare_relation(&mut clash).unwrap();
    assert_ne!(clash.id, first);
    let id = g.apply_prepared_relation(clash).unwrap();
    assert_eq!(g.get_relation(first).unwrap().target, p, "original intact");
    assert_eq!(g.get_relation(id).unwrap().target, q);
    // A free explicit id is kept and observed.
    let mut explicit = Relation::new(RelationId(500), "cites", p, q);
    g.prepare_relation(&mut explicit).unwrap();
    assert_eq!(explicit.id, RelationId(500));
    g.apply_prepared_relation(explicit).unwrap();
    assert!(relation(&g, "cites", q, p) > RelationId(500));
}

/// The materialized inverse of a symmetric relation is a stored relation
/// with its own id, so a cascade and `incident_relation_ids` see both.
#[test]
fn symmetric_relation_is_two_stored_relations_seen_by_incident_ids() {
    let g = graph();
    let a = concept(&g, "Person", "A");
    let b = concept(&g, "Person", "B");
    let id = relation(&g, "knows", a, b);
    assert_eq!(g.relation_count(), 2);
    let from_a = g.incident_relation_ids(a).unwrap();
    let from_b = g.incident_relation_ids(b).unwrap();
    assert_eq!(from_a.len(), 2);
    assert_eq!(from_a, from_b);
    assert!(from_a.contains(&id));
    let inverse = *from_a.iter().find(|r| **r != id).unwrap();
    let inv = g.get_relation(inverse).unwrap();
    assert_eq!((inv.source, inv.target), (b, a));
    assert_eq!(inv.weight, 1.0);
    // Removing the forward edge leaves the inverse (each direction is its
    // own record); removing the concept removes both.
    g.remove_relation(id).unwrap();
    assert_eq!(g.relation_count(), 1);
    assert_eq!(g.incident_relation_ids(a).unwrap(), vec![inverse]);
    let removed = g.remove_concept(a).unwrap();
    assert_eq!(removed, vec![inverse]);
    assert_eq!(g.relation_count(), 0);
    assert!(g.outgoing(b).is_empty() && g.incoming(b).is_empty());
}

/// A symmetric self-loop materializes no inverse (it would be the same
/// edge) and is counted once in the cascade.
#[test]
fn symmetric_self_loop_has_no_inverse() {
    let g = graph();
    let a = concept(&g, "Person", "A");
    let id = relation(&g, "knows", a, a);
    assert_eq!(g.relation_count(), 1);
    assert_eq!(g.incident_relation_ids(a).unwrap(), vec![id]);
    assert_eq!(g.remove_concept(a).unwrap(), vec![id]);
    assert_eq!(g.relation_count(), 0);
}

/// `insert_relation_exact` is the compaction replay path: it keeps the id,
/// skips cardinality checks and never materializes an inverse — and it
/// still requires both endpoints and a free, non-zero id.
#[test]
fn insert_relation_exact_bypasses_cardinality_but_not_endpoints() {
    let g = graph();
    let p = concept(&g, "Paper", "P");
    let t1 = concept(&g, "Topic", "T1");
    let t2 = concept(&g, "Topic", "T2");
    let a = concept(&g, "Person", "A");
    let b = concept(&g, "Person", "B");
    // Two `about` from the same paper would violate ManyToOne live; the
    // exact path accepts what the disk says.
    g.insert_relation_exact(Relation::new(RelationId(10), "about", p, t1))
        .unwrap();
    g.insert_relation_exact(Relation::new(RelationId(11), "about", p, t2))
        .unwrap();
    assert_eq!(g.outgoing(p).len(), 2);
    // Symmetric: one direction only.
    g.insert_relation_exact(Relation::new(RelationId(12), "knows", a, b))
        .unwrap();
    assert_eq!(g.relation_count(), 3);
    assert!(g.outgoing(b).is_empty(), "no inverse materialized");
    assert_eq!(g.incoming(b).len(), 1);
    // Refusals leave nothing behind.
    let gen = g.relations_generation();
    assert!(matches!(
        g.insert_relation_exact(Relation::new(RelationId(0), "cites", p, p)),
        Err(GraphError::NotPrepared("relation"))
    ));
    assert!(matches!(
        g.insert_relation_exact(Relation::new(RelationId(12), "cites", p, p)),
        Err(GraphError::RelationExists(RelationId(12)))
    ));
    assert!(matches!(
        g.insert_relation_exact(Relation::new(RelationId(13), "cites", ConceptId(999), p)),
        Err(GraphError::UnknownConcept(ConceptId(999)))
    ));
    assert!(matches!(
        g.insert_relation_exact(Relation::new(RelationId(13), "cites", p, ConceptId(999))),
        Err(GraphError::UnknownConcept(ConceptId(999)))
    ));
    assert_eq!(g.relation_count(), 3);
    assert_eq!(
        g.relations_generation(),
        gen,
        "a refused insert bumps nothing"
    );
    // Fast paths see exact relations like any other.
    let (total, page) = g.list_relations_page(Some(p), None, Some("about"), 0, 10, true);
    assert_eq!(total, 2);
    assert_eq!(
        page.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![RelationId(10), RelationId(11)]
    );
    // Watermark moved past the exact ids.
    assert!(relation(&g, "cites", p, p) > RelationId(12));
}

// ---------------------------------------------------------------------------
// preview_* / apply_*_update
// ---------------------------------------------------------------------------

/// A description-only patch leaves name, properties and every name-derived
/// index untouched, and still bumps the generation (R2).
#[test]
fn description_only_concept_patch_keeps_name_indexes_and_bumps_gen() {
    let g = graph();
    let mut props = AHashMap::new();
    props.insert("email".to_string(), PropertyValue::Text("a@b".into()));
    let mut c = Concept::new(ConceptId(0), "Person", "Alice");
    c.properties = props.clone();
    let alice = g.upsert_concept(c).unwrap();
    let gen = g.concepts_generation();
    let patch = ConceptPatch {
        description: Some("now described".into()),
        ..Default::default()
    };
    let previewed = g.preview_concept_update(alice, &patch).unwrap();
    assert_eq!(previewed.properties, props);
    assert_eq!(g.concepts_generation(), gen, "preview is pure");
    let applied = g.apply_concept_update(previewed).unwrap();
    assert_eq!(applied.description, "now described");
    assert_eq!(applied.properties, props);
    assert_eq!(g.find_by_name("Person", "Alice"), Some(alice));
    assert_eq!(
        names(&g.list_concepts_page(None, Some("ali"), 0, 10, true, true).1),
        vec!["Alice"]
    );
    assert_eq!(g.concepts_generation(), gen + 1);
}

/// Renaming onto one's own name (a case change) is not a collision; the
/// sorted index and the name index both follow.
#[test]
fn concept_rename_to_a_different_case_of_itself_is_allowed() {
    let g = graph();
    let alice = concept(&g, "Person", "alice");
    let patch = ConceptPatch {
        name: Some("Alice".into()),
        ..Default::default()
    };
    let c = g.update_concept(alice, patch).unwrap();
    assert_eq!(c.name, "Alice");
    assert_eq!(g.find_by_name("Person", "ALICE"), Some(alice));
    assert_eq!(
        names(
            &g.list_concepts_page(Some("Person"), None, 0, 10, true, false)
                .1
        ),
        vec!["Alice"]
    );
    assert_eq!(
        names(&g.list_concepts_page(None, Some("lic"), 0, 10, true, true).1),
        vec!["Alice"]
    );
}

/// `apply_concept_update` on an unknown id, or with a colliding rename
/// smuggled past preview, is refused and leaves every index intact.
#[test]
fn apply_concept_update_refuses_unknown_ids_and_smuggled_collisions() {
    let g = graph();
    let alice = concept(&g, "Person", "Alice");
    let bob = concept(&g, "Person", "Bob");
    let mut ghost = Concept::new(ConceptId(999), "Person", "Ghost");
    ghost.description = "x".into();
    assert!(matches!(
        g.apply_concept_update(ghost),
        Err(GraphError::UnknownConcept(ConceptId(999)))
    ));
    let mut renamed = g.get_concept(bob).unwrap();
    renamed.name = "ALICE".into();
    assert!(matches!(
        g.apply_concept_update(renamed),
        Err(GraphError::DuplicateConcept(_, _))
    ));
    assert_eq!(g.find_by_name("Person", "alice"), Some(alice));
    assert_eq!(g.find_by_name("Person", "bob"), Some(bob));
    assert_eq!(
        names(
            &g.list_concepts_page(Some("Person"), None, 0, 10, true, false)
                .1
        ),
        vec!["Alice", "Bob"]
    );
}

/// R2: a relation update is a mutation of the relation set — the caches
/// keyed on `relations_generation` (query cache, HTTP ETag) must not serve
/// the old weight afterwards.
#[test]
fn relation_update_bumps_relations_generation_and_invalidates_pages() {
    let g = graph();
    let a = concept(&g, "Person", "A");
    let p = concept(&g, "Paper", "P");
    let rid = relation(&g, "authored", a, p);
    let (_, page) = g.list_relations_page(Some(a), None, None, 0, 10, true);
    assert_eq!(page[0].weight, 1.0);
    let gen = g.relations_generation();
    let mut props = AHashMap::new();
    props.insert("since".to_string(), PropertyValue::Number(2020.0));
    let updated = g
        .update_relation(
            rid,
            RelationPatch {
                weight: Some(0.5),
                properties: Some(props.clone()),
            },
        )
        .unwrap();
    assert_eq!(updated.weight, 0.5);
    assert_eq!(g.relations_generation(), gen + 1, "one mutation, one bump");
    let (_, page) = g.list_relations_page(Some(a), None, None, 0, 10, true);
    assert_eq!(page[0].weight, 0.5, "cached page must be invalidated");
    assert_eq!(page[0].properties, props);
    // Endpoints and type are immutable and ignored by apply (H6).
    let mut tampered = g.get_relation(rid).unwrap();
    tampered.source = p;
    tampered.target = a;
    tampered.relation_type = "cites".into();
    tampered.weight = 0.25;
    g.apply_relation_update(tampered).unwrap();
    let r = g.get_relation(rid).unwrap();
    assert_eq!(
        (r.source, r.target, r.relation_type.as_str()),
        (a, p, "authored")
    );
    assert_eq!(r.weight, 0.25);
    assert_eq!(g.outgoing(a).len(), 1);
    assert_eq!(g.outgoing(p).len(), 0);
    assert!(matches!(
        g.apply_relation_update(Relation::new(RelationId(999), "authored", a, p)),
        Err(GraphError::UnknownRelation(RelationId(999)))
    ));
}

// ---------------------------------------------------------------------------
// rules
// ---------------------------------------------------------------------------

/// Rules: prepare validates type and `applies_to`, apply is idempotent on
/// an explicit id (upsert), and the sorted listing follows a rename.
#[test]
fn rule_write_ahead_round_trip_and_sorted_listing() {
    let g = graph();
    let p = concept(&g, "Paper", "P");
    // Unknown rule type → refused (reported as an unknown type).
    let mut bad = Rule::new(RuleId(0), "nope", "r");
    assert!(g.prepare_rule(&mut bad).is_err());
    assert_eq!(bad.id, RuleId(0));
    // Unknown concept in applies_to → refused.
    let mut bad = Rule::new(RuleId(0), "must_review", "r");
    bad.applies_to = vec![ConceptId(999)];
    assert!(matches!(
        g.prepare_rule(&mut bad),
        Err(GraphError::UnknownConcept(ConceptId(999)))
    ));
    // Unprepared apply.
    assert!(matches!(
        g.apply_prepared_rule(Rule::new(RuleId(0), "must_review", "r")),
        Err(GraphError::NotPrepared("rule"))
    ));
    assert_eq!(g.rule_count(), 0);
    // Happy path: prepare allocates, nothing visible, apply inserts.
    let mut r = Rule::new(RuleId(0), "must_review", "zeta");
    r.applies_to = vec![p];
    g.prepare_rule(&mut r).unwrap();
    assert_eq!(r.id, RuleId(1));
    assert_eq!(g.rule_count(), 0);
    assert!(g.get_rule(RuleId(1)).is_err());
    g.apply_prepared_rule(r).unwrap();
    let alpha = g
        .upsert_rule(Rule::new(RuleId(0), "must_review", "alpha"))
        .unwrap();
    assert_eq!(alpha, RuleId(2));
    let (total, page) = g.list_rules_page(0, 10);
    assert_eq!(total, 2);
    assert_eq!(
        page.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        vec!["alpha", "zeta"]
    );
    // Rename through preview/apply reorders the listing.
    let patch = RulePatch {
        name: Some("omega".into()),
        strict: Some(true),
        ..Default::default()
    };
    let previewed = g.preview_rule_update(alpha, &patch).unwrap();
    assert_eq!(g.get_rule(alpha).unwrap().name, "alpha", "preview is pure");
    g.apply_rule_update(previewed).unwrap();
    let (_, page) = g.list_rules_page(0, 10);
    assert_eq!(
        page.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        vec!["omega", "zeta"]
    );
    assert!(g.get_rule(alpha).unwrap().strict);
    // A replacement applies_to with an unknown concept is refused at preview.
    let patch = RulePatch {
        applies_to: Some(vec![ConceptId(999)]),
        ..Default::default()
    };
    assert!(matches!(
        g.preview_rule_update(alpha, &patch),
        Err(GraphError::UnknownConcept(_))
    ));
    // Explicit-id upsert replaces in place and re-keys the listing.
    g.upsert_rule(Rule::new(alpha, "must_review", "aaa"))
        .unwrap();
    assert_eq!(g.rule_count(), 2);
    let (_, page) = g.list_rules_page(0, 1);
    assert_eq!(page[0].name, "aaa");
    // Removal and pagination bounds.
    g.remove_rule(alpha).unwrap();
    assert!(g.remove_rule(alpha).is_err());
    assert_eq!(g.list_rules_page(0, 10).0, 1);
    assert!(g.list_rules_page(5, 10).1.is_empty());
    assert_eq!(
        g.list_rules_page(5, 10).0,
        1,
        "total is exact regardless of offset"
    );
}

// ---------------------------------------------------------------------------
// actions
// ---------------------------------------------------------------------------

/// Actions: prepare validates type, subject and object; `Some(None)` on
/// the patch clears the object; the sorted listing follows a rename.
#[test]
fn action_write_ahead_round_trip_and_object_clearing() {
    let g = graph();
    let a = concept(&g, "Person", "A");
    let p = concept(&g, "Paper", "P");
    let mut bad = Action::new(ActionId(0), "nope", "s", a);
    assert!(g.prepare_action(&mut bad).is_err());
    let mut bad = Action::new(ActionId(0), "sign", "s", ConceptId(999));
    assert!(matches!(
        g.prepare_action(&mut bad),
        Err(GraphError::UnknownConcept(ConceptId(999)))
    ));
    let mut bad = Action::new(ActionId(0), "sign", "s", a);
    bad.object = Some(ConceptId(999));
    assert!(matches!(
        g.prepare_action(&mut bad),
        Err(GraphError::UnknownConcept(ConceptId(999)))
    ));
    assert!(matches!(
        g.apply_prepared_action(Action::new(ActionId(0), "sign", "s", a)),
        Err(GraphError::NotPrepared("action"))
    ));
    assert_eq!(g.action_count(), 0);

    let mut act = Action::new(ActionId(0), "sign", "zeta", a);
    act.object = Some(p);
    g.prepare_action(&mut act).unwrap();
    assert_eq!(act.id, ActionId(1));
    assert_eq!(g.action_count(), 0);
    g.apply_prepared_action(act).unwrap();
    let alpha = g
        .upsert_action(Action::new(ActionId(0), "sign", "alpha", a))
        .unwrap();
    let (total, page) = g.list_actions_page(0, 10);
    assert_eq!(total, 2);
    assert_eq!(
        page.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(),
        vec!["alpha", "zeta"]
    );

    // Clear the object of the first, set it on the second, rename the second.
    let patch = ActionPatch {
        object: Some(None),
        ..Default::default()
    };
    let cleared = g.update_action(ActionId(1), patch).unwrap();
    assert_eq!(cleared.object, None);
    let patch = ActionPatch {
        name: Some("omega".into()),
        object: Some(Some(p)),
        subject: Some(a),
        ..Default::default()
    };
    let previewed = g.preview_action_update(alpha, &patch).unwrap();
    assert_eq!(
        g.get_action(alpha).unwrap().name,
        "alpha",
        "preview is pure"
    );
    g.apply_action_update(previewed).unwrap();
    let (_, page) = g.list_actions_page(0, 10);
    assert_eq!(
        page.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(),
        vec!["omega", "zeta"]
    );
    assert_eq!(g.get_action(alpha).unwrap().object, Some(p));
    // Unknown subject / object in a patch are refused at preview.
    let patch = ActionPatch {
        subject: Some(ConceptId(999)),
        ..Default::default()
    };
    assert!(matches!(
        g.preview_action_update(alpha, &patch),
        Err(GraphError::UnknownConcept(_))
    ));
    let patch = ActionPatch {
        object: Some(Some(ConceptId(999))),
        ..Default::default()
    };
    assert!(matches!(
        g.preview_action_update(alpha, &patch),
        Err(GraphError::UnknownConcept(_))
    ));
    g.remove_action(alpha).unwrap();
    assert!(g.remove_action(alpha).is_err());
    assert_eq!(g.list_actions_page(0, 10).0, 1);
}

// ---------------------------------------------------------------------------
// clear_instances
// ---------------------------------------------------------------------------

/// `clear_instances` empties every primary map and every derived index,
/// bumps both generations, keeps the schema and restarts ids at 1.
#[test]
fn clear_instances_resets_everything_but_the_schema() {
    let g = graph();
    let a = concept(&g, "Person", "A");
    let b = concept(&g, "Researcher", "B");
    let p = concept(&g, "Paper", "Paper one");
    relation(&g, "authored", a, p);
    relation(&g, "knows", a, b);
    let mut rule = Rule::new(RuleId(0), "must_review", "r");
    rule.applies_to = vec![p];
    g.upsert_rule(rule).unwrap();
    g.upsert_action(Action::new(ActionId(0), "sign", "s", a))
        .unwrap();
    // Warm the caches.
    g.list_concepts_page(None, None, 0, 10, true, true);
    g.list_concepts_page(None, Some("pap"), 0, 10, true, true);
    g.list_relations_page(Some(a), None, None, 0, 10, true);
    let (cg, rg) = (g.concepts_generation(), g.relations_generation());
    let schema_before = g.ontology();

    g.clear_instances();

    assert_eq!(g.concept_count(), 0);
    assert_eq!(g.relation_count(), 0);
    assert_eq!(g.rule_count(), 0);
    assert_eq!(g.action_count(), 0);
    assert!(g.all_concepts().is_empty() && g.all_relations().is_empty());
    assert!(g.all_rules().is_empty() && g.all_actions().is_empty());
    assert!(g.find_by_name("Person", "a").is_none());
    assert!(g.get_concept(a).is_err() && g.get_relation(RelationId(1)).is_err());
    let (t, page) = g.list_concepts_page(None, None, 0, 10, true, true);
    assert!(t == 0 && page.is_empty());
    assert_eq!(
        g.list_concepts_page(Some("Person"), None, 0, 10, true, true)
            .0,
        0
    );
    assert_eq!(
        g.list_concepts_page(Some("Person"), None, 0, 10, true, false)
            .0,
        0
    );
    assert_eq!(
        g.list_concepts_page(None, Some("pap"), 0, 10, true, true).0,
        0
    );
    assert_eq!(g.list_relations_page(Some(a), None, None, 0, 10, true).0, 0);
    assert_eq!(g.list_relations_page(None, None, None, 0, 10, true).0, 0);
    assert_eq!(g.list_rules_page(0, 10).0, 0);
    assert_eq!(g.list_actions_page(0, 10).0, 0);
    assert!(g.concepts_of_type("Person", true).is_empty());
    assert!(g.outgoing(a).is_empty() && g.incoming(p).is_empty());
    assert!(g.outgoing_typed(a, &["knows".to_string()]).is_empty());
    assert!(g.concepts_generation() > cg && g.relations_generation() > rg);
    assert_eq!(
        g.ontology().concept_types.len(),
        schema_before.concept_types.len()
    );
    assert_eq!(
        g.ontology().relation_types.len(),
        schema_before.relation_types.len()
    );

    // Ids restart at 1 for every family and the old names are free again.
    assert_eq!(concept(&g, "Person", "A"), ConceptId(1));
    let p2 = concept(&g, "Paper", "Paper one");
    assert_eq!(p2, ConceptId(2));
    assert_eq!(relation(&g, "authored", ConceptId(1), p2), RelationId(1));
    assert_eq!(
        g.upsert_rule(Rule::new(RuleId(0), "must_review", "r"))
            .unwrap(),
        RuleId(1)
    );
    assert_eq!(
        g.upsert_action(Action::new(ActionId(0), "sign", "s", ConceptId(1)))
            .unwrap(),
        ActionId(1)
    );
    // No stale adjacency survived under the reused ids.
    assert_eq!(g.outgoing(ConceptId(1)).len(), 1);
    assert_eq!(g.incoming(ConceptId(2)).len(), 1);
    assert_eq!(
        g.list_relations_page(Some(ConceptId(1)), None, None, 0, 10, true)
            .0,
        1
    );
}
