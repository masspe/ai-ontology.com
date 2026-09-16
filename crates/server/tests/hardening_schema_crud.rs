// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! The HTTP contract of the schema (`PUT /ontology`) and of the CRUD
//! routes: status codes for every refusal, pagination, ETag / 304
//! (`PERFORMANCE.md` §4.8) and the writer lock under concurrency.

mod hardening_common;

use axum::body::Body;
use hardening_common::*;
use http::{header, Request, StatusCode};
use ontology_graph::{Concept, ConceptId, ConceptType, Ontology, OntologyGraph};
use ontology_storage::RecordKind;
use serde_json::{json, Value};

async fn put_ontology(app: &axum::Router, o: &Ontology) -> (StatusCode, Value) {
    call(
        app,
        "PUT",
        "/ontology",
        Some(serde_json::to_value(o).unwrap()),
    )
    .await
}

/// Every schema guard of `check_ontology` is a 400 carrying the guard's own
/// message, and the refused schema never reaches the store.
#[tokio::test]
async fn put_ontology_maps_each_schema_guard_to_400_with_its_reason() {
    let (app, graph, store) = flaky_app();
    let a = create_topic(&app, "A").await;
    let b = create_topic(&app, "B").await;
    relate(&app, "related_to", a, b).await; // 2 stored directions
    create_rule(&app, "r1", &[a]).await;
    create_action(&app, "act1", a).await;
    let before = fingerprint(&graph);
    let written = store.records_written();

    let mut cases: Vec<(&str, Ontology, &str)> = Vec::new();
    {
        let mut o = ontology();
        o.concept_type_mut("Topic").ns = Some("meta".into());
        cases.push((
            "reserved domain",
            o,
            "concept type `Topic` declares invalid domain `meta`",
        ));
    }
    {
        let mut o = ontology();
        o.concept_type_mut("Topic").ns = Some("Not Valid".into());
        cases.push((
            "invalid domain",
            o,
            "concept type `Topic` declares invalid domain `Not Valid`",
        ));
    }
    {
        let mut o = ontology();
        o.concept_types.remove("Topic");
        cases.push((
            "concept type in use",
            o,
            "concept type `Topic` still has 2 instance(s)",
        ));
    }
    {
        let mut o = ontology();
        o.concept_type_mut("Topic").ns = Some("other".into());
        cases.push((
            "domain move with instances",
            o,
            "concept type `Topic` cannot move from domain `default` to `other`: 2 instance(s) exist",
        ));
    }
    {
        let mut o = ontology();
        o.relation_types.get_mut("related_to").unwrap().domain = "Tag".into();
        cases.push((
            "relation type reshaped with instances",
            o,
            "relation type `related_to` cannot change domain/range: 2 relation(s) exist",
        ));
    }
    {
        let mut o = ontology();
        o.relation_types.remove("related_to");
        cases.push((
            "relation type in use",
            o,
            "relation type `related_to` still has 2 instance(s)",
        ));
    }
    {
        let mut o = ontology();
        o.rule_types.remove("must_review");
        cases.push((
            "rule type in use",
            o,
            "rule type `must_review` still has 1 instance(s)",
        ));
    }
    {
        let mut o = ontology();
        o.action_types.remove("archive");
        cases.push((
            "action type in use",
            o,
            "action type `archive` still has 1 instance(s)",
        ));
    }

    for (label, candidate, expected) in cases {
        let (st, v) = put_ontology(&app, &candidate).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{label}: {v}");
        let msg = error_text(&v);
        assert!(msg.contains(expected), "{label}: got `{msg}`");
    }
    assert_eq!(
        store.records_written(),
        written,
        "no refused schema journaled"
    );
    assert_eq!(fingerprint(&graph), before);
}

/// A valid extension is journaled as exactly one `Ontology` record and is
/// what `GET /ontology` serves afterwards.
#[tokio::test]
async fn put_ontology_valid_extension_is_journaled_once_as_an_ontology_record() {
    let (app, graph, store) = flaky_app();
    create_topic(&app, "A").await;
    let records = store.records().len();

    let mut o = ontology();
    o.add_concept_type(ConceptType {
        name: "Extra".into(),
        ..Default::default()
    });
    let (st, v) = put_ontology(&app, &o).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(v["concept_types"]["Extra"].is_object(), "echoed back: {v}");

    let tail = &store.records()[records..];
    assert_eq!(tail.len(), 1);
    assert!(matches!(tail[0].kind, RecordKind::Ontology(_)));
    let (_, served) = get(&app, "/ontology").await;
    assert!(served["concept_types"]["Extra"].is_object());
    assert_eq!(graph.ontology().concept_types.len(), 3);
}

/// An unknown path id is a 404 (with the id in the message) for GET, PATCH
/// and DELETE on every entity; the store is never consulted. A non-numeric
/// id is a 400 from the path extractor.
#[tokio::test]
async fn unknown_ids_are_404_for_every_entity_and_verb() {
    let (app, graph, store) = flaky_app();
    let before = fingerprint(&graph);

    for entity in ["concepts", "relations", "rules", "actions"] {
        for method in ["GET", "PATCH", "DELETE"] {
            let body = (method == "PATCH").then(|| json!({}));
            let uri = format!("/{entity}/4242");
            let (st, v) = call(&app, method, &uri, body).await;
            assert_eq!(st, StatusCode::NOT_FOUND, "{method} {uri}: {v}");
            assert!(error_text(&v).contains("4242"), "{method} {uri}: {v}");
        }
    }
    for entity in ["concepts", "relations", "rules", "actions"] {
        let (st, _) = get(&app, &format!("/{entity}/not-a-number")).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{entity}: non-numeric id");
    }
    assert_eq!(store.append_calls(), 0);
    assert_eq!(store.batch_calls(), 0);
    assert_eq!(fingerprint(&graph), before);
}

/// Renaming onto a name already used by another concept of the same type
/// is a 400 (`duplicate concept name`), case-insensitively; renaming onto
/// one's own name with a different case is accepted. Only the accepted
/// rename is journaled.
#[tokio::test]
async fn patch_rename_onto_an_existing_name_is_400_and_journals_nothing() {
    let (app, graph, store) = flaky_app();
    let a = create_topic(&app, "Alpha").await;
    create_topic(&app, "Beta").await;
    let written = store.records_written();

    for clash in ["Beta", "beta", "BETA"] {
        let (st, v) = call(
            &app,
            "PATCH",
            &format!("/concepts/{a}"),
            Some(json!({ "name": clash })),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{clash}: {v}");
        assert!(
            error_text(&v).contains(&format!("duplicate concept name `{clash}`")),
            "{clash}: {v}"
        );
    }
    assert_eq!(store.records_written(), written);
    assert_eq!(graph.get_concept(ConceptId(a)).unwrap().name, "Alpha");

    let (st, v) = call(
        &app,
        "PATCH",
        &format!("/concepts/{a}"),
        Some(json!({ "name": "ALPHA" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["name"], "ALPHA");
    assert_eq!(store.records_written(), written + 1);
    assert_eq!(graph.find_by_name("Topic", "alpha"), Some(ConceptId(a)));
}

/// `POST /relations` refusals: cardinality is 422, a schema (domain/range)
/// violation, an unknown endpoint named in the body or an unknown relation
/// type are 400 — and none of them is journaled.
#[tokio::test]
async fn relation_refusals_have_their_status_and_journal_nothing() {
    let (app, graph, store) = flaky_app();
    let a = create_topic(&app, "A").await;
    let b = create_topic(&app, "B").await;
    let c = create_topic(&app, "C").await;
    let t = create(&app, "Tag", "T").await;
    relate(&app, "owned_by", a, b).await; // many-to-one: A already has an owner
    let before = fingerprint(&graph);
    let written = store.records_written();

    let (st, v) = call(&app, "POST", "/relations", Some(relation("owned_by", a, c))).await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert!(
        error_text(&v).contains("cardinality violated for relation `owned_by`"),
        "{v}"
    );

    let (st, v) = call(
        &app,
        "POST",
        "/relations",
        Some(relation("related_to", a, t)),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(error_text(&v).contains("violates schema"), "{v}");

    let (st, v) = call(
        &app,
        "POST",
        "/relations",
        Some(relation("related_to", a, 999)),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::BAD_REQUEST,
        "body reference, not a path id: {v}"
    );
    let msg = error_text(&v);
    assert!(
        msg.contains("unknown concept") && msg.contains("999"),
        "{v}"
    );

    let (st, v) = call(
        &app,
        "POST",
        "/relations",
        Some(relation("no_such_type", a, b)),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(
        error_text(&v).contains("not defined in the ontology"),
        "{v}"
    );

    assert_eq!(store.records_written(), written);
    assert_eq!(fingerprint(&graph), before);
}

/// Pins the current semantics of symmetric relations: creating one stores
/// two directions (the inverse has its own id), and `DELETE /relations/:id`
/// tombstones **only the addressed direction** — the inverse stays and
/// must be deleted on its own.
#[tokio::test]
async fn symmetric_relation_materializes_its_inverse_and_delete_removes_only_the_addressed_direction(
) {
    let (app, graph, store) = flaky_app();
    let a = create_topic(&app, "A").await;
    let b = create_topic(&app, "B").await;
    let r = relate(&app, "related_to", a, b).await;
    assert_eq!(graph.relation_count(), 2);

    let (st, v) = get(&app, &format!("/relations?source={b}")).await;
    assert_eq!(st, StatusCode::OK);
    let inverse = &v["relations"][0];
    assert_eq!(v["total"], 1);
    assert_eq!(inverse["relation_type"], "related_to");
    assert_eq!(inverse["source"], b);
    assert_eq!(inverse["target"], a);
    let inv = inverse["id"].as_u64().unwrap();
    assert_ne!(inv, r);
    let records = store.records().len();

    let (st, _) = call(&app, "DELETE", &format!("/relations/{r}"), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert_eq!(graph.relation_count(), 1, "the inverse direction remains");
    let (st, _) = get(&app, &format!("/relations/{r}")).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _) = get(&app, &format!("/relations/{inv}")).await;
    assert_eq!(st, StatusCode::OK);
    let tail = &store.records()[records..];
    assert_eq!(tail.len(), 1);
    assert!(matches!(tail[0].kind, RecordKind::DeleteRelation(id) if id.0 == r));

    let (st, _) = call(&app, "DELETE", &format!("/relations/{inv}"), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert_eq!(graph.relation_count(), 0);
}

fn seed(graph: &OntologyGraph, ty: &str, names: &[&str]) {
    for n in names {
        graph
            .upsert_concept(Concept::new(ConceptId(0), ty, *n))
            .unwrap();
    }
}

/// `GET /concepts`: stable `(type, name)` order, `limit`/`offset`, `type`
/// and `q` combined (case-insensitive), `track_total=false` reporting a
/// lower bound, and an offset past the end yielding an empty page with the
/// true total.
#[tokio::test]
async fn list_concepts_pagination_contract() {
    let (app, graph, _store) = flaky_app();
    seed(&graph, "Topic", &["t1", "t2", "t3", "t4", "t5", "t6", "t7"]);
    seed(&graph, "Tag", &["alpha", "beta", "gamma"]);
    let names = |v: &Value| -> Vec<String> {
        v["concepts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap().to_string())
            .collect()
    };

    let (st, v) = get(&app, "/concepts?limit=3").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["total"], 10);
    assert_eq!(
        names(&v),
        ["alpha", "beta", "gamma"],
        "Tag sorts before Topic"
    );

    let (_, v) = get(&app, "/concepts?offset=100").await;
    assert_eq!(v["total"], 10);
    assert_eq!(names(&v), Vec::<String>::new());

    // `beta` contains a `t`: the type filter is what excludes it.
    let (_, v) = get(&app, "/concepts?q=T").await;
    assert_eq!(v["total"], 8, "case-insensitive substring: {v}");
    let (_, v) = get(&app, "/concepts?type=Topic&q=T").await;
    assert_eq!(v["total"], 7);
    assert_eq!(names(&v).len(), 7);
    let (_, v) = get(&app, "/concepts?type=Topic&q=t1").await;
    assert_eq!(v["total"], 1);
    assert_eq!(names(&v), ["t1"]);
    let (_, v) = get(&app, "/concepts?type=Nope").await;
    assert_eq!(v["total"], 0);
    assert_eq!(names(&v), Vec::<String>::new());

    let (_, v) = get(&app, "/concepts?limit=2&track_total=false").await;
    assert_eq!(v["total"], 2, "lower bound = offset + page: {v}");
    assert_eq!(names(&v), ["alpha", "beta"]);
    let (_, v) = get(&app, "/concepts?offset=3&limit=2&track_total=false").await;
    assert_eq!(v["total"], 5);
    assert_eq!(names(&v), ["t1", "t2"]);
}

/// `limit` is capped at 5 000 whatever the client asks for.
#[tokio::test]
async fn list_concepts_limit_is_capped_at_5000() {
    let (app, graph, _store) = flaky_app();
    for i in 0..5_001u32 {
        graph
            .upsert_concept(Concept::new(ConceptId(0), "Topic", format!("n{i:05}")))
            .unwrap();
    }
    let (st, v) = get(&app, "/concepts?limit=99999").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["total"], 5_001);
    assert_eq!(v["concepts"].as_array().unwrap().len(), 5_000);
}

async fn etag_of(
    app: &axum::Router,
    uri: &str,
    if_none_match: Option<&str>,
) -> (StatusCode, String, usize) {
    let mut req = Request::builder().uri(uri);
    if let Some(tag) = if_none_match {
        req = req.header(header::IF_NONE_MATCH, tag);
    }
    let (st, headers, bytes) = send(app, req.body(Body::empty()).unwrap()).await;
    let etag = headers
        .get(header::ETAG)
        .map(|h| h.to_str().unwrap().to_string())
        .unwrap_or_default();
    (st, etag, bytes.len())
}

/// `PERFORMANCE.md` §4.8: the ETag is the family's generation (`W/"c<gen>"`
/// / `W/"r<gen>"`), identical across query strings; `If-None-Match` on the
/// current tag is a bodiless 304; a write to one family moves its tag and
/// leaves the other's alone.
#[tokio::test]
async fn etag_follows_the_generation_and_if_none_match_yields_304() {
    let (app, graph, _store) = flaky_app();
    let a = create_topic(&app, "A").await;
    let b = create_topic(&app, "B").await;
    relate(&app, "related_to", a, b).await;

    let (st, c_tag, _) = etag_of(&app, "/concepts", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(c_tag, format!("W/\"c{}\"", graph.concepts_generation()));
    let (st, r_tag, _) = etag_of(&app, "/relations", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(r_tag, format!("W/\"r{}\"", graph.relations_generation()));

    let (st, _, len) = etag_of(&app, "/concepts", Some(&c_tag)).await;
    assert_eq!(st, StatusCode::NOT_MODIFIED);
    assert_eq!(len, 0, "304 has no body");
    let (st, _, _) = etag_of(&app, "/concepts?type=Topic&limit=1", Some(&c_tag)).await;
    assert_eq!(
        st,
        StatusCode::NOT_MODIFIED,
        "tag does not depend on the query"
    );
    let (st, _, _) = etag_of(&app, "/relations", Some(&r_tag)).await;
    assert_eq!(st, StatusCode::NOT_MODIFIED);
    let (st, _, _) = etag_of(&app, "/concepts", Some("W/\"c0\"")).await;
    assert_eq!(st, StatusCode::OK, "stale tag is served in full");

    // A concept write moves the concept tag only.
    let (st, _) = call(
        &app,
        "PATCH",
        &format!("/concepts/{a}"),
        Some(json!({ "description": "d" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, c_tag2, _) = etag_of(&app, "/concepts", Some(&c_tag)).await;
    assert_eq!(st, StatusCode::OK);
    assert_ne!(c_tag2, c_tag);
    let (st, _, _) = etag_of(&app, "/relations", Some(&r_tag)).await;
    assert_eq!(
        st,
        StatusCode::NOT_MODIFIED,
        "relations untouched by a concept patch"
    );

    // A relation write moves the relation tag only.
    relate(&app, "owned_by", a, b).await;
    let (st, r_tag2, _) = etag_of(&app, "/relations", Some(&r_tag)).await;
    assert_eq!(st, StatusCode::OK);
    assert_ne!(r_tag2, r_tag);
    let (st, _, _) = etag_of(&app, "/concepts", Some(&c_tag2)).await;
    assert_eq!(st, StatusCode::NOT_MODIFIED);

    // A concept deletion cascades: both tags move.
    let (st, _) = call(&app, "DELETE", &format!("/concepts/{a}"), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, _, _) = etag_of(&app, "/concepts", Some(&c_tag2)).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _, _) = etag_of(&app, "/relations", Some(&r_tag2)).await;
    assert_eq!(st, StatusCode::OK);
}

/// The writer lock serializes `prepare → append → apply`: two concurrent
/// creations of the same name end with exactly one 200 and one 400, one
/// concept and one journaled record.
#[tokio::test]
async fn concurrent_creates_of_the_same_name_yield_exactly_one_success() {
    let (app, graph, store) = flaky_app();

    let (r1, r2) = tokio::join!(
        call(&app, "POST", "/concepts", Some(topic("Same"))),
        call(&app, "POST", "/concepts", Some(topic("same"))),
    );
    let mut statuses = [r1.0, r2.0];
    statuses.sort();
    assert_eq!(
        statuses,
        [StatusCode::OK, StatusCode::BAD_REQUEST],
        "{:?} / {:?}",
        r1.1,
        r2.1
    );
    let loser = if r1.0 == StatusCode::OK { &r2.1 } else { &r1.1 };
    assert!(
        error_text(loser).contains("duplicate concept name"),
        "{loser}"
    );
    assert_eq!(graph.concept_count(), 1);
    assert_eq!(store.records_written(), 1);
    assert!(store
        .records()
        .iter()
        .all(|r| matches!(r.kind, RecordKind::Concept(_))));

    // Distinct names racing all land, with distinct ids.
    let (x, y, z) = tokio::join!(
        call(&app, "POST", "/concepts", Some(topic("X"))),
        call(&app, "POST", "/concepts", Some(topic("Y"))),
        call(&app, "POST", "/concepts", Some(topic("Z"))),
    );
    let mut ids: Vec<u64> = [x, y, z]
        .iter()
        .map(|(st, v)| {
            assert_eq!(*st, StatusCode::OK, "{v}");
            v["id"].as_u64().unwrap()
        })
        .collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 3, "distinct ids");
    assert_eq!(graph.concept_count(), 4);
    assert_eq!(store.records_written(), 4);
}
