// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Security labels and a reader's visibility: labels live on the sheet
//! and survive an update; a traversal never crosses a hidden sheet (so
//! nothing about it is inferable from a path through it); a listing
//! neither lists nor counts one; the page cache is kept per reader.

use ontology_graph::{
    Concept, ConceptId, ConceptPatch, ConceptType, Ontology, OntologyGraph, Relation, RelationType,
    TraversalSpec, Visibility,
};
use std::sync::Arc;

fn ontology() -> Ontology {
    let mut o = Ontology::new();
    o.add_concept_type(ConceptType {
        name: "Topic".into(),
        ..Default::default()
    });
    o.add_relation_type(RelationType {
        name: "related_to".into(),
        domain: "Topic".into(),
        range: "Topic".into(),
        ..Default::default()
    })
    .unwrap();
    o.add_relation_type(RelationType {
        name: "part_of".into(),
        domain: "Topic".into(),
        range: "Topic".into(),
        transitive: true,
        ..Default::default()
    })
    .unwrap();
    o
}

fn topic(g: &OntologyGraph, name: &str, labels: &[&str]) -> ConceptId {
    g.upsert_concept(Concept::new(ConceptId(0), "Topic", name).with_labels(labels.iter().copied()))
        .unwrap()
}

fn link(g: &OntologyGraph, rel: &str, s: ConceptId, t: ConceptId) {
    g.add_relation(Relation {
        id: Default::default(),
        relation_type: rel.into(),
        source: s,
        target: t,
        weight: 1.0,
        properties: Default::default(),
    })
    .unwrap();
}

/// A reader with these clearances: sees a sheet when its labels are all
/// among them.
fn reader(g: &Arc<OntologyGraph>, key: u64, clearances: &[&str]) -> Visibility {
    let hidden: std::collections::HashSet<ConceptId> = g
        .labelled_concepts()
        .into_iter()
        .filter(|(_, labels)| !labels.iter().all(|l| clearances.contains(&l.as_str())))
        .map(|(id, _)| id)
        .collect();
    Visibility::new(key, move |id| !hidden.contains(&id))
}

#[test]
fn labels_live_on_the_sheet_and_survive_updates() {
    let g = OntologyGraph::with_arc(ontology());
    let a = topic(&g, "Payroll", &["payroll", "ch"]);
    let b = topic(&g, "Public", &[]);
    assert_eq!(g.get_concept(a).unwrap().labels, vec!["payroll", "ch"]);
    assert!(g.get_concept(b).unwrap().labels.is_empty());
    assert_eq!(
        g.labelled_concepts(),
        vec![(a, vec!["payroll".to_string(), "ch".to_string()])]
    );

    // A patch without labels keeps them; one with labels replaces them.
    g.update_concept(
        a,
        ConceptPatch {
            description: Some("salaries".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(g.get_concept(a).unwrap().labels, vec!["payroll", "ch"]);
    g.update_concept(
        a,
        ConceptPatch {
            labels: Some(vec!["hr".into()]),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(g.get_concept(a).unwrap().labels, vec!["hr"]);
    g.update_concept(
        a,
        ConceptPatch {
            labels: Some(vec![]),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(g.labelled_concepts().is_empty());
    // JSON keeps the field only when set.
    let json = serde_json::to_string(&g.get_concept(b).unwrap()).unwrap();
    assert!(!json.contains("labels"), "{json}");
    let json = serde_json::to_string(&Concept::new(ConceptId(1), "Topic", "x").with_labels(["a"]))
        .unwrap();
    assert!(json.contains("\"labels\":[\"a\"]"), "{json}");
}

#[test]
fn a_traversal_never_crosses_a_hidden_sheet() {
    let g = OntologyGraph::with_arc(ontology());
    // a — secret — c, and a — b (public) at depth 1.
    let a = topic(&g, "a", &[]);
    let secret = topic(&g, "secret", &["payroll"]);
    let c = topic(&g, "c", &[]);
    let b = topic(&g, "b", &[]);
    link(&g, "related_to", a, secret);
    link(&g, "related_to", secret, c);
    link(&g, "related_to", a, b);

    let everyone = TraversalSpec {
        max_depth: 3,
        ..Default::default()
    };
    let full = g.expand(&[a], &everyone);
    let ids = |s: &ontology_graph::Subgraph| {
        let mut v: Vec<ConceptId> = s.concepts.iter().map(|c| c.id).collect();
        v.sort();
        v
    };
    assert_eq!(ids(&full), {
        let mut v = vec![a, secret, c, b];
        v.sort();
        v
    });

    let outsider = reader(&g, 1, &[]);
    let limited = g.expand(
        &[a],
        &TraversalSpec {
            visible: Some(outsider.clone()),
            ..everyone.clone()
        },
    );
    assert_eq!(ids(&limited), {
        let mut v = vec![a, b];
        v.sort();
        v
    });
    assert!(
        limited
            .relations
            .iter()
            .all(|r| r.source != secret && r.target != secret),
        "no edge touches the hidden sheet"
    );
    // A hidden seed is dropped too.
    let none = g.expand(
        &[secret],
        &TraversalSpec {
            visible: Some(outsider.clone()),
            ..everyone.clone()
        },
    );
    assert!(none.concepts.is_empty() && none.relations.is_empty());
    // A cleared reader sees everything.
    let insider = reader(&g, 2, &["payroll"]);
    let all = g.expand(
        &[a],
        &TraversalSpec {
            visible: Some(insider),
            ..everyone
        },
    );
    assert_eq!(ids(&all), ids(&full));

    // The shortest path a→c runs through the secret: none for the outsider,
    // and a hidden end is "no path", not an error.
    assert!(g.shortest_path(a, c, 4).unwrap().is_some());
    assert!(g
        .shortest_path_where(a, c, 4, Some(&outsider))
        .unwrap()
        .is_none());
    assert!(g
        .shortest_path_where(a, secret, 4, Some(&outsider))
        .unwrap()
        .is_none());
    assert!(g
        .shortest_path_where(a, b, 4, Some(&outsider))
        .unwrap()
        .is_some());

    // The transitive closure stops at a hidden sheet as well.
    let p1 = topic(&g, "p1", &[]);
    let p2 = topic(&g, "p2", &["payroll"]);
    let p3 = topic(&g, "p3", &[]);
    link(&g, "part_of", p1, p2);
    link(&g, "part_of", p2, p3);
    assert_eq!(g.closure(p1, "part_of", 3).unwrap().len(), 2);
    let outsider = reader(&g, 3, &[]);
    assert!(g
        .closure_where(p1, "part_of", 3, Some(&outsider))
        .unwrap()
        .is_empty());
}

#[test]
fn a_listing_neither_lists_nor_counts_a_hidden_sheet() {
    let g = OntologyGraph::with_arc(ontology());
    let names = ["alpha", "beta", "gamma", "delta"];
    let ids: Vec<ConceptId> = names
        .iter()
        .map(|n| topic(&g, n, if *n == "beta" { &["payroll"] } else { &[] }))
        .collect();
    let outsider = reader(&g, 7, &[]);
    let insider = reader(&g, 8, &["payroll"]);

    let (total, page) = g
        .try_list_concepts_page(Some("Topic"), None, 0, 10, true, true)
        .unwrap();
    assert_eq!((total, page.len()), (4, 4));
    let (total, page) = g
        .try_list_concepts_page_for(Some("Topic"), None, 0, 10, true, true, Some(&outsider))
        .unwrap();
    assert_eq!(total, 3);
    assert!(page.iter().all(|c| c.name != "beta"), "{page:?}");
    // Offsets count visible rows only: the second visible row is `delta`
    // (sorted by name: alpha, delta, gamma).
    let (_, page) = g
        .try_list_concepts_page_for(Some("Topic"), None, 1, 1, false, true, Some(&outsider))
        .unwrap();
    assert_eq!(page[0].name, "delta");
    // The name search path (the candidate intersection) filters too.
    let (total, page) = g
        .try_list_concepts_page_for(None, Some("ta"), 0, 10, true, true, Some(&outsider))
        .unwrap();
    assert_eq!(total, 1, "{page:?}");
    assert_eq!(page[0].name, "delta");
    // Cursor listing.
    let (page, _) = g
        .try_list_concepts_after_for(Some("Topic"), None, None, 10, true, Some(&outsider))
        .unwrap();
    assert_eq!(page.len(), 3);

    // The page cache is per reader: the insider's page is not the outsider's.
    let (total, _) = g
        .try_list_concepts_page_for(Some("Topic"), None, 0, 10, true, true, Some(&insider))
        .unwrap();
    assert_eq!(total, 4);
    let (total, _) = g
        .try_list_concepts_page_for(Some("Topic"), None, 0, 10, true, true, Some(&outsider))
        .unwrap();
    assert_eq!(total, 3);
    // Labelling a sheet bumps the generation: the cached page is dropped.
    g.update_concept(
        ids[0],
        ConceptPatch {
            labels: Some(vec!["payroll".into()]),
            ..Default::default()
        },
    )
    .unwrap();
    let outsider = reader(&g, 7, &[]);
    let (total, _) = g
        .try_list_concepts_page_for(Some("Topic"), None, 0, 10, true, true, Some(&outsider))
        .unwrap();
    assert_eq!(total, 2);
    assert_eq!(format!("{outsider:?}"), "Visibility(7)");
}
