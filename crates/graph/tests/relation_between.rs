// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! `relation_between` finds one edge by its endpoints and type without
//! cloning, and a symmetric relation's update reaches its materialised
//! inverse — the two graph facts an idempotent import relies on.

mod hardening_common;
use hardening_common::*;

use ontology_graph::{PropertyValue, RelationPatch};

#[test]
fn relation_between_finds_the_edge_by_endpoints_and_type_only() {
    let g = graph();
    let a = concept(&g, "Paper", "A");
    let b = concept(&g, "Paper", "B");
    let c = concept(&g, "Paper", "C");
    let ab = relation(&g, "cites", a, b);
    relation(&g, "cites", a, c);

    assert_eq!(g.relation_between(a, b, "cites"), Some(ab));
    assert_eq!(g.relation_between(b, a, "cites"), None, "direction matters");
    assert_eq!(g.relation_between(a, b, "knows"), None, "type matters");
    assert_eq!(g.relation_between(c, b, "cites"), None, "no such edge");
}

#[test]
fn updating_a_symmetric_relation_updates_its_inverse_too() {
    let g = graph();
    let p = concept(&g, "Person", "P");
    let q = concept(&g, "Person", "Q");
    let pq = relation(&g, "knows", p, q);
    let qp = g
        .relation_between(q, p, "knows")
        .expect("materialised inverse");
    assert_ne!(pq, qp);

    let mut props = ahash::AHashMap::default();
    props.insert("since".to_string(), PropertyValue::Text("2020".into()));
    g.update_relation(
        pq,
        RelationPatch {
            weight: Some(7.0),
            properties: Some(props.clone()),
        },
    )
    .unwrap();

    let inv = g.get_relation(qp).unwrap();
    assert_eq!(inv.weight, 7.0);
    assert_eq!(inv.properties, props);
    assert_eq!(g.relation_count(), 2, "no edge was added");
}

#[test]
fn updating_an_asymmetric_relation_touches_nothing_else() {
    let g = graph();
    let a = concept(&g, "Paper", "A");
    let b = concept(&g, "Paper", "B");
    let ab = relation(&g, "cites", a, b);
    let ba = relation(&g, "cites", b, a);
    g.update_relation(
        ab,
        RelationPatch {
            weight: Some(3.0),
            properties: None,
        },
    )
    .unwrap();
    assert_eq!(g.get_relation(ba).unwrap().weight, 1.0);
}
