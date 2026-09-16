// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Schema guards: `Ontology` construction rules and the transition matrix
//! enforced by `extend_ontology` / `check_ontology` (`STORAGE.md` §10.10,
//! R13, decision D3).

mod hardening_common;
use hardening_common::*;

use ontology_graph::{
    Action, Cardinality, Concept, ConceptId, ConceptType, GraphError, Ontology, OntologyGraph,
    RelationType, Rule, DEFAULT_NS, MAX_NS_LEN,
};

// ---------------------------------------------------------------------------
// Ontology-level construction rules
// ---------------------------------------------------------------------------

/// A relation type may only reference declared concept types, on both ends.
#[test]
fn relation_type_with_unknown_domain_or_range_is_rejected() {
    let mut o = Ontology::new();
    o.add_concept_type(ct("Person", None, None));
    assert!(matches!(
        o.add_relation_type(rt("r", "Ghost", "Person")),
        Err(GraphError::UnknownConceptType(t)) if t == "Ghost"
    ));
    assert!(matches!(
        o.add_relation_type(rt("r", "Person", "Ghost")),
        Err(GraphError::UnknownConceptType(t)) if t == "Ghost"
    ));
    assert!(
        o.relation_types.is_empty(),
        "a rejected type is not registered"
    );
    o.add_relation_type(rt("r", "Person", "Person")).unwrap();
    assert!(o.relation_type("r").is_ok());
}

/// `inverse_of` must name a relation type; an empty name or a self-inverse
/// on a non-symmetric type is refused at declaration time.
#[test]
fn inverse_of_rejects_empty_and_non_symmetric_self_reference() {
    let mut o = Ontology::new();
    o.add_concept_type(ct("A", None, None));
    let mut empty = rt("r", "A", "A");
    empty.inverse_of = Some(String::new());
    assert!(matches!(
        o.add_relation_type(empty),
        Err(GraphError::UnknownRelationType(_))
    ));
    let mut selfish = rt("r", "A", "A");
    selfish.inverse_of = Some("r".into());
    assert!(matches!(
        o.add_relation_type(selfish),
        Err(GraphError::UnknownRelationType(_))
    ));
    // A symmetric type is legitimately its own inverse.
    let mut sym = rt_symmetric("s", "A", "A");
    sym.inverse_of = Some("s".into());
    o.add_relation_type(sym).unwrap();
    o.validate_inverses().unwrap();
}

/// Rule and action types can only scope to declared concept types.
#[test]
fn rule_and_action_types_require_known_concept_types() {
    let mut o = Ontology::new();
    o.add_concept_type(ct("Paper", None, None));
    assert!(matches!(
        o.add_rule_type(rule_type("r", &["Paper", "Ghost"])),
        Err(GraphError::UnknownConceptType(t)) if t == "Ghost"
    ));
    assert!(o.rule_type("r").is_none());
    o.add_rule_type(rule_type("r", &["Paper"])).unwrap();
    o.add_rule_type(rule_type("global", &[])).unwrap();

    assert!(matches!(
        o.add_action_type(action_type("a", "Ghost", None)),
        Err(GraphError::UnknownConceptType(t)) if t == "Ghost"
    ));
    assert!(matches!(
        o.add_action_type(action_type("a", "Paper", Some("Ghost"))),
        Err(GraphError::UnknownConceptType(t)) if t == "Ghost"
    ));
    assert!(o.action_type("a").is_none());
    o.add_action_type(action_type("a", "Paper", Some("Paper")))
        .unwrap();
    o.add_action_type(action_type("b", "Paper", None)).unwrap();
}

/// `disjoint_with` is symmetrised whichever side declares it, and never
/// duplicated when both sides do.
#[test]
fn disjoint_with_is_symmetrised_in_both_declaration_orders() {
    // Declared on the first type, second type added later.
    let mut o = Ontology::new();
    let mut person = ct("Person", None, None);
    person.disjoint_with = vec!["Robot".into()];
    o.add_concept_type(person);
    o.add_concept_type(ct("Robot", None, None));
    assert_eq!(
        o.concept_type("Robot").unwrap().disjoint_with,
        vec!["Person"]
    );
    assert_eq!(
        o.concept_type("Person").unwrap().disjoint_with,
        vec!["Robot"]
    );

    // Declared on the second type, first type already present.
    let mut o = Ontology::new();
    o.add_concept_type(ct("Person", None, None));
    let mut robot = ct("Robot", None, None);
    robot.disjoint_with = vec!["Person".into()];
    o.add_concept_type(robot);
    assert_eq!(
        o.concept_type("Person").unwrap().disjoint_with,
        vec!["Robot"]
    );

    // Declared on both: no duplicates.
    let mut o = Ontology::new();
    let mut person = ct("Person", None, None);
    person.disjoint_with = vec!["Robot".into()];
    let mut robot = ct("Robot", None, None);
    robot.disjoint_with = vec!["Person".into()];
    o.add_concept_type(person);
    o.add_concept_type(robot);
    assert_eq!(
        o.concept_type("Person").unwrap().disjoint_with,
        vec!["Robot"]
    );
    assert_eq!(
        o.concept_type("Robot").unwrap().disjoint_with,
        vec!["Person"]
    );
}

/// Disjointness is enforced across both orders of insertion, and only on
/// the (type, lowercase name) identity — a different name is fine.
#[test]
fn disjoint_types_are_enforced_in_both_directions_case_insensitively() {
    let mut o = Ontology::new();
    let mut person = ct("Person", None, None);
    person.disjoint_with = vec!["Robot".into()];
    o.add_concept_type(person);
    o.add_concept_type(ct("Robot", None, None));
    let g = OntologyGraph::new(o);
    concept(&g, "Robot", "Alice");
    let err = g.upsert_concept(Concept::new(ConceptId(0), "Person", "ALICE"));
    assert!(
        matches!(err, Err(GraphError::DisjointTypeViolation { ref type_a, ref type_b }) if type_a == "Person" && type_b == "Robot"),
        "{err:?}"
    );
    concept(&g, "Person", "Bob");
    assert_eq!(g.concept_count(), 2);
}

/// `is_subtype` follows the whole parent chain and is reflexive, even for
/// types the ontology does not know.
#[test]
fn is_subtype_is_reflexive_and_transitive() {
    let mut o = Ontology::new();
    o.add_concept_type(ct("Animal", None, None));
    o.add_concept_type(ct("Mammal", Some("Animal"), None));
    o.add_concept_type(ct("Dog", Some("Mammal"), None));
    assert!(o.is_subtype("Dog", "Dog"));
    assert!(o.is_subtype("Dog", "Mammal"));
    assert!(o.is_subtype("Dog", "Animal"));
    assert!(!o.is_subtype("Animal", "Dog"));
    assert!(!o.is_subtype("Mammal", "Dog"));
    assert!(
        o.is_subtype("Unknown", "Unknown"),
        "reflexive even if unknown"
    );
    assert!(!o.is_subtype("Unknown", "Animal"));
    assert!(!o.is_subtype("Dog", "Unknown"));
}

/// The descendants cache is invalidated by every schema edit, so a type
/// added after the first lookup is visible to the next one.
#[test]
fn descendants_cache_follows_schema_edits() {
    let mut o = Ontology::new();
    o.add_concept_type(ct("Animal", None, None));
    assert_eq!(o.descendants("Animal"), vec!["Animal".to_string()]);
    o.add_concept_type(ct("Dog", Some("Animal"), None));
    assert_eq!(
        o.descendants("Animal"),
        vec!["Animal".to_string(), "Dog".into()]
    );
    o.concept_type_mut("Dog").parent = None;
    assert_eq!(o.descendants("Animal"), vec!["Animal".to_string()]);
    o.concept_types.remove("Animal");
    // A raw edit of the map does not invalidate; merge/add do.
    o.merge_concept_type(ct("Cat", None, None));
    assert!(o.descendants("Animal").is_empty());
}

/// A parent cycle must neither hang the hierarchy helpers nor be accepted
/// as a schema (`STORAGE.md` §10.10: the schema is validated before it is
/// journaled, and a replay must terminate).
#[test]
fn parent_cycles_terminate_and_are_refused_by_check_ontology() {
    let mut cyclic = Ontology::new();
    cyclic.add_concept_type(ct("A", Some("B"), None));
    cyclic.add_concept_type(ct("B", Some("C"), None));
    cyclic.add_concept_type(ct("C", Some("A"), None));
    cyclic.add_concept_type(ct("Loner", None, None));
    // Read helpers terminate on a corrupt hierarchy.
    assert!(!cyclic.is_subtype("A", "Loner"));
    assert!(cyclic.is_subtype("A", "C"));
    let mut d = cyclic.descendants("A");
    d.sort();
    assert_eq!(d, vec!["A".to_string(), "B".into(), "C".into()]);
    assert_eq!(cyclic.ns_of_type("A"), DEFAULT_NS);
    assert!(matches!(
        cyclic.validate_hierarchy(),
        Err(GraphError::ParentCycle { .. })
    ));

    // The graph refuses to adopt such a schema, atomically.
    let g = graph();
    assert!(matches!(
        g.check_ontology(&cyclic),
        Err(GraphError::ParentCycle { .. })
    ));
    let err = g.extend_ontology(|o| {
        o.concept_type_mut("Person").parent = Some("Researcher".into());
        Ok(())
    });
    assert!(
        matches!(err, Err(GraphError::ParentCycle { ref concept_type }) if concept_type == "Person" || concept_type == "Researcher"),
        "{err:?}"
    );
    assert_eq!(g.ontology().concept_type("Person").unwrap().parent, None);
    // A self-parent is the degenerate cycle.
    let err = g.extend_ontology(|o| {
        o.concept_type_mut("Topic").parent = Some("Topic".into());
        Ok(())
    });
    assert!(
        matches!(err, Err(GraphError::ParentCycle { .. })),
        "{err:?}"
    );
    // A dangling parent is not a cycle: tolerated, resolves to the default
    // domain, and the chain simply stops there.
    g.extend_ontology(|o| {
        o.add_concept_type(ct("Orphan", Some("Nowhere"), None));
        Ok(())
    })
    .unwrap();
    assert_eq!(g.ns_of_type("Orphan"), DEFAULT_NS);
}

// ---------------------------------------------------------------------------
// merge_concept_type / merged_concept_type
// ---------------------------------------------------------------------------

fn full_type() -> ConceptType {
    ConceptType {
        name: "Contract".into(),
        properties: Some(vec!["amount".into()]),
        parent: None,
        description: "a legal agreement".into(),
        required_properties: vec!["amount".into()],
        disjoint_with: vec![],
        ns: Some("contrats".into()),
    }
}

/// `merge_concept_type` reports `false` only when nothing changes: a bare
/// re-declaration, or one that repeats the registered values.
#[test]
fn merge_concept_type_is_false_when_the_merged_type_is_identical() {
    let mut o = Ontology::new();
    assert!(o.merge_concept_type(full_type()), "first registration");
    assert!(!o.merge_concept_type(ct("Contract", None, None)), "bare");
    assert!(!o.merge_concept_type(full_type()), "verbatim repeat");
    let mut partial = ct("Contract", None, Some("contrats"));
    partial.description = "a legal agreement".into();
    assert!(!o.merge_concept_type(partial), "repeating some fields");
    assert_eq!(o.concept_type("Contract").unwrap(), &full_type());
}

/// Every field that a declaration sets differently makes the merge a
/// change, and only that field moves.
#[test]
fn merge_concept_type_is_true_for_each_field_that_differs() {
    let mut o = Ontology::new();
    o.add_concept_type(ct("Legal", None, Some("contrats")));
    o.add_concept_type(full_type());

    let mut decl = ct("Contract", None, None);
    decl.description = "updated".into();
    assert!(o.merge_concept_type(decl));
    assert_eq!(o.concept_type("Contract").unwrap().description, "updated");
    assert_eq!(
        o.concept_type("Contract").unwrap().ns.as_deref(),
        Some("contrats")
    );

    let mut decl = ct("Contract", None, None);
    decl.properties = Some(vec!["amount".into(), "party".into()]);
    assert!(o.merge_concept_type(decl));
    assert_eq!(
        o.concept_type("Contract").unwrap().properties.as_deref(),
        Some(&["amount".to_string(), "party".to_string()][..])
    );

    let mut decl = ct("Contract", None, None);
    decl.required_properties = vec!["party".into()];
    assert!(o.merge_concept_type(decl));
    assert_eq!(
        o.concept_type("Contract").unwrap().required_properties,
        vec!["party"]
    );

    assert!(o.merge_concept_type(ct("Contract", Some("Legal"), None)));
    assert_eq!(
        o.concept_type("Contract").unwrap().parent.as_deref(),
        Some("Legal")
    );

    assert!(o.merge_concept_type(ct("Contract", None, Some("legal"))));
    assert_eq!(
        o.concept_type("Contract").unwrap().ns.as_deref(),
        Some("legal")
    );

    let mut decl = ct("Contract", None, None);
    decl.disjoint_with = vec!["Legal".into()];
    assert!(o.merge_concept_type(decl));
    assert_eq!(
        o.concept_type("Contract").unwrap().disjoint_with,
        vec!["Legal"]
    );
    // …and the sibling got the back-edge through add_concept_type.
    assert_eq!(
        o.concept_type("Legal").unwrap().disjoint_with,
        vec!["Contract"]
    );
    // Everything not mentioned survived the whole sequence.
    assert_eq!(o.concept_type("Contract").unwrap().description, "updated");
}

/// `merged_concept_type` is pure and never lets an unset field of the
/// declaration erase a set field of the registered type.
#[test]
fn merged_concept_type_never_overwrites_set_fields_with_unset_ones() {
    let mut o = Ontology::new();
    o.add_concept_type(full_type());
    let before = o.concept_type("Contract").unwrap().clone();
    let merged = o.merged_concept_type(ct("Contract", None, None));
    assert_eq!(merged, full_type());
    assert_eq!(o.concept_type("Contract").unwrap(), &before, "pure");
    // An unknown type is returned as declared.
    let fresh = ct("Invoice", None, None);
    assert_eq!(o.merged_concept_type(fresh.clone()), fresh);
    assert!(o.concept_type("Invoice").is_err());
}

// ---------------------------------------------------------------------------
// extend_ontology / check_ontology transition matrix
// ---------------------------------------------------------------------------

/// Every malformed domain identifier is refused as a whole schema.
#[test]
fn invalid_namespaces_are_refused_with_the_offending_type_named() {
    let g = graph();
    let too_long = "x".repeat(MAX_NS_LEN + 1);
    let longest = "x".repeat(MAX_NS_LEN);
    for bad in [
        "meta",            // reserved (D3)
        "",                // empty
        "Contracts",       // uppercase
        "con tracts",      // space
        "contrats/2026",   // separator
        "été",             // non-ASCII
        too_long.as_str(), // too long
    ] {
        let err = g.extend_ontology(|o| {
            o.add_concept_type(ct("Fresh", None, Some(bad)));
            Ok(())
        });
        assert!(
            matches!(err, Err(GraphError::InvalidNamespace { ref concept_type, ref ns }) if concept_type == "Fresh" && ns == bad),
            "{bad:?}: {err:?}"
        );
        assert!(!g.ontology().concept_types.contains_key("Fresh"));
    }
    // The longest legal identifier is accepted.
    g.extend_ontology(|o| {
        o.add_concept_type(ct("Fresh", None, Some(longest.as_str())));
        Ok(())
    })
    .unwrap();
}

/// A child declaring a domain must declare its parent's effective domain,
/// including when the parent only inherits the default one.
#[test]
fn namespace_mismatch_is_refused_and_inheritance_is_not() {
    let g = graph();
    let err = g.extend_ontology(|o| {
        o.add_concept_type(ct("Student", Some("Person"), Some("campus")));
        Ok(())
    });
    assert!(
        matches!(
            err,
            Err(GraphError::NamespaceMismatch { ref concept_type, ref ns, ref parent, ref parent_ns })
                if concept_type == "Student" && ns == "campus" && parent == "Person" && parent_ns == DEFAULT_NS
        ),
        "{err:?}"
    );
    // Restating the inherited domain explicitly is fine.
    g.extend_ontology(|o| {
        o.add_concept_type(ct("Student", Some("Person"), Some(DEFAULT_NS)));
        Ok(())
    })
    .unwrap();
    // A child without its own domain inherits, at any depth.
    g.extend_ontology(|o| {
        o.add_concept_type(ct("Lab", None, Some("labs")));
        o.add_concept_type(ct("Team", Some("Lab"), None));
        o.add_concept_type(ct("SubTeam", Some("Team"), None));
        Ok(())
    })
    .unwrap();
    assert_eq!(g.ns_of_type("SubTeam"), "labs");
    let mut ns = g.ontology().namespaces();
    ns.sort();
    assert_eq!(ns, vec![DEFAULT_NS.to_string(), "labs".into()]);
}

/// Moving a type to another domain is refused while it has instances,
/// whether the move is direct or inherited through a parent.
#[test]
fn namespace_change_with_instances_is_refused_even_when_inherited() {
    let g = graph();
    concept(&g, "Researcher", "Ada");
    // Direct move of the type with the instance.
    let err = g.extend_ontology(|o| {
        o.concept_type_mut("Person").ns = Some("people".into());
        o.concept_type_mut("Researcher").ns = Some("labs".into());
        Ok(())
    });
    assert!(
        matches!(err, Err(GraphError::NamespaceMismatch { .. })),
        "{err:?}"
    );
    // Consistent move of parent and child: the child has an instance.
    let err = g.extend_ontology(|o| {
        o.concept_type_mut("Person").ns = Some("people".into());
        Ok(())
    });
    assert!(
        matches!(
            err,
            Err(GraphError::NamespaceChangeWithInstances { ref concept_type, ref from, ref to, instances: 1 })
                if concept_type == "Researcher" && from == DEFAULT_NS && to == "people"
        ),
        "{err:?}"
    );
    assert_eq!(g.ns_of_type("Researcher"), DEFAULT_NS);
    // Types without instances still move freely.
    g.extend_ontology(|o| {
        o.concept_type_mut("Topic").ns = Some("topics".into());
        Ok(())
    })
    .unwrap();
    assert_eq!(g.ns_of_type("Topic"), "topics");
}

/// Removing a type is refused for each of the four families while it has
/// instances, with the exact count, and allowed once it has none.
#[test]
fn type_in_use_matrix_for_concepts_relations_rules_and_actions() {
    let g = graph();
    let a = concept(&g, "Person", "A");
    let b = concept(&g, "Person", "B");
    let p = concept(&g, "Paper", "P");
    relation(&g, "authored", a, p);
    relation(&g, "authored", b, p);
    relation(&g, "knows", a, b); // symmetric: two stored relations
    let mut rule = Rule::new(Default::default(), "must_review", "r");
    rule.applies_to = vec![p];
    g.upsert_rule(rule).unwrap();
    let mut act = Action::new(Default::default(), "sign", "s", a);
    act.object = Some(p);
    g.upsert_action(act).unwrap();

    let remove_concept_type = |name: &str| {
        g.extend_ontology(|o| {
            o.concept_types.remove(name);
            Ok(())
        })
    };
    assert!(matches!(
        remove_concept_type("Person"),
        Err(GraphError::TypeInUse { kind: "concept", ref name, instances: 2 }) if name == "Person"
    ));
    assert!(matches!(
        remove_concept_type("Paper"),
        Err(GraphError::TypeInUse {
            kind: "concept",
            instances: 1,
            ..
        })
    ));
    assert!(matches!(
        g.extend_ontology(|o| {
            o.relation_types.remove("authored");
            Ok(())
        }),
        Err(GraphError::TypeInUse { kind: "relation", ref name, instances: 2 }) if name == "authored"
    ));
    assert!(matches!(
        g.extend_ontology(|o| {
            o.relation_types.remove("knows");
            Ok(())
        }),
        Err(GraphError::TypeInUse {
            kind: "relation",
            instances: 2,
            ..
        })
    ));
    assert!(matches!(
        g.extend_ontology(|o| {
            o.rule_types.remove("must_review");
            Ok(())
        }),
        Err(GraphError::TypeInUse {
            kind: "rule",
            instances: 1,
            ..
        })
    ));
    assert!(matches!(
        g.extend_ontology(|o| {
            o.action_types.remove("sign");
            Ok(())
        }),
        Err(GraphError::TypeInUse {
            kind: "action",
            instances: 1,
            ..
        })
    ));
    // Unused types go: Researcher, cites, about (no instances), and an
    // unused rule/action type added then dropped.
    g.extend_ontology(|o| {
        o.concept_types.remove("Researcher");
        o.relation_types.remove("cites");
        o.relation_types.remove("about");
        o.add_rule_type(rule_type("tmp", &[])).unwrap();
        o.add_action_type(action_type("tmp", "Person", None))
            .unwrap();
        Ok(())
    })
    .unwrap();
    g.extend_ontology(|o| {
        o.rule_types.remove("tmp");
        o.action_types.remove("tmp");
        Ok(())
    })
    .unwrap();
    // Once every instance is gone, the guarded removals succeed.
    g.clear_instances();
    g.extend_ontology(|o| {
        o.concept_types.remove("Topic");
        o.relation_types.remove("authored");
        o.relation_types.remove("knows");
        o.rule_types.remove("must_review");
        o.action_types.remove("sign");
        o.concept_types.remove("Paper");
        o.concept_types.remove("Person");
        Ok(())
    })
    .unwrap();
    assert!(g.ontology().concept_types.is_empty());
}

/// A relation type with instances can neither change `domain` nor `range`
/// (`STORAGE.md` §10.10). Other fields (description, cardinality,
/// symmetric, transitive) are not guarded by §10.10 and stay editable.
#[test]
fn relation_type_change_with_instances_guards_domain_and_range_only() {
    let g = graph();
    let a = concept(&g, "Person", "A");
    let p = concept(&g, "Paper", "P");
    relation(&g, "authored", a, p);
    for (label, edit) in [
        (
            "domain",
            Box::new(|r: &mut RelationType| r.domain = "Paper".into())
                as Box<dyn Fn(&mut RelationType)>,
        ),
        (
            "range",
            Box::new(|r: &mut RelationType| r.range = "Person".into()),
        ),
    ] {
        let err = g.extend_ontology(|o| {
            let mut r = o.relation_type("authored").unwrap().clone();
            edit(&mut r);
            o.relation_types.insert(r.name.clone(), r);
            Ok(())
        });
        assert!(
            matches!(
                err,
                Err(GraphError::RelationTypeChangeWithInstances { ref relation_type, instances: 1 })
                    if relation_type == "authored"
            ),
            "{label}: {err:?}"
        );
    }
    assert_eq!(
        g.ontology().relation_type("authored").unwrap().domain,
        "Person"
    );
    g.extend_ontology(|o| {
        let mut r = o.relation_type("authored").unwrap().clone();
        r.description = "who wrote what".into();
        r.cardinality = Cardinality::OneToMany;
        r.transitive = true;
        o.relation_types.insert(r.name.clone(), r);
        Ok(())
    })
    .unwrap();
    let after = g.ontology().relation_type("authored").unwrap().clone();
    assert_eq!(after.cardinality, Cardinality::OneToMany);
    assert!(after.transitive);
    // Widening the domain to a supertype is still a domain change.
    let g2 = graph();
    let r = concept(&g2, "Researcher", "R");
    let p2 = concept(&g2, "Paper", "P");
    g2.extend_ontology(|o| o.add_relation_type(rt("reviewed", "Researcher", "Paper")))
        .unwrap();
    relation(&g2, "reviewed", r, p2);
    assert!(matches!(
        g2.extend_ontology(|o| {
            o.relation_types.get_mut("reviewed").unwrap().domain = "Person".into();
            Ok(())
        }),
        Err(GraphError::RelationTypeChangeWithInstances { .. })
    ));
}

/// `check_ontology` and `extend_ontology` give the same verdict, and the
/// pure form never touches the live schema.
#[test]
fn check_ontology_is_the_pure_twin_of_extend_ontology() {
    let g = graph();
    concept(&g, "Person", "A");
    let mut candidate = g.ontology();
    candidate.concept_types.remove("Person");
    let checked = g.check_ontology(&candidate);
    let extended = g.extend_ontology(|o| {
        o.concept_types.remove("Person");
        Ok(())
    });
    assert!(matches!(checked, Err(GraphError::TypeInUse { .. })));
    assert!(matches!(extended, Err(GraphError::TypeInUse { .. })));
    assert!(g.ontology().concept_types.contains_key("Person"));

    let mut ok = g.ontology();
    ok.add_concept_type(ct("Venue", None, Some("venues")));
    g.check_ontology(&ok).unwrap();
    assert!(
        !g.ontology().concept_types.contains_key("Venue"),
        "check_ontology applies nothing"
    );
    // A schema identical to the live one is always accepted.
    g.check_ontology(&g.ontology()).unwrap();
}

/// A schema edit is applied atomically: either every change of the closure
/// lands, or none does — including when the closure itself succeeds but a
/// later guard fails.
#[test]
fn extend_ontology_applies_all_or_nothing() {
    let g = graph();
    concept(&g, "Paper", "P");
    let before = g.ontology();
    let err = g.extend_ontology(|o| {
        o.add_concept_type(ct("Venue", None, None)); // fine on its own
        o.concept_types.remove("Paper"); // refused: in use
        Ok(())
    });
    assert!(matches!(err, Err(GraphError::TypeInUse { .. })));
    let after = g.ontology();
    assert!(!after.concept_types.contains_key("Venue"));
    assert!(after.concept_types.contains_key("Paper"));
    assert_eq!(
        after.concept_types.len(),
        before.concept_types.len(),
        "nothing from a refused closure leaks"
    );
}
