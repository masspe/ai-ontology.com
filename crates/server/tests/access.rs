// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Labels and clearances on the API: a sheet whose labels exceed the
//! caller's clearances does not exist for them on any read path
//! (sheet, listing, links, path, subgraph, retrieval, question), cannot
//! be written to, and no label beyond one's clearances can be attached.
//! Administrators see everything; without a login, so does everyone.

mod hardening_common;

use axum::body::{to_bytes, Body};
use axum::Router;
use hardening_common::*;
use http::{Request, StatusCode};
use ontology_graph::{ConceptType, Ontology, OntologyGraph, RelationType};
use ontology_server::auth::{UserAuth, UserStore};
use ontology_server::{build_router_with_config, JwtAuth, RouterConfig};
use ontology_storage::{MemoryStore, Store};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

const SECRET: &[u8] = b"test-secret-for-the-built-in-auth";

fn tempdir(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "ontology-access-{tag}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

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
    o
}

fn app(users_file: Option<PathBuf>) -> Router {
    let graph = OntologyGraph::with_arc(ontology());
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let state = state_with(store, graph);
    build_router_with_config(
        state,
        RouterConfig {
            bearer_token: None,
            jwt: users_file
                .as_ref()
                .map(|_| JwtAuth::from_secret(SECRET.to_vec())),
            rate_limit: None,
            users: users_file.map(|f| UserAuth {
                store: Arc::new(UserStore::open(f).expect("users file")),
                allow_signup: true,
            }),
            web_dir: None,
            audit: None,
        },
    )
}

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let req = match body {
        Some(b) => req
            .header("content-type", "application/json")
            .body(Body::from(b.to_string()))
            .unwrap(),
        None => req.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1 << 22).await.unwrap();
    let v = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (status, v)
}

/// Three sheets and three links: public — hr — secret, public — secret.
struct World {
    app: Router,
    admin: String,
    bob: String,
    carol: String,
    bob_id: String,
    public: u64,
    hr: u64,
    secret: u64,
    rel_public_hr: u64,
    rel_public_secret: u64,
}

async fn world(tag: &str) -> World {
    let dir = tempdir(tag);
    let app = app(Some(dir.join("users.json")));
    let creds =
        |email: &str, name: &str| json!({ "email": email, "name": name, "password": "Passw0rd!x" });
    let (st, v) = call(
        &app,
        "POST",
        "/auth/signup",
        None,
        Some(creds("alice@example.com", "Alice")),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let admin = v["token"].as_str().unwrap().to_string();
    let mut ids = Vec::new();
    for (email, name) in [("bob@example.com", "Bob"), ("carol@example.com", "Carol")] {
        let (st, v) = call(
            &app,
            "POST",
            "/auth/users",
            Some(&admin),
            Some(creds(email, name)),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{v}");
        ids.push(v["user"]["id"].as_str().unwrap().to_string());
    }
    let (st, v) = call(
        &app,
        "PATCH",
        &format!("/auth/users/{}", ids[0]),
        Some(&admin),
        Some(json!({ "clearances": ["HR", " hr "] })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["user"]["clearances"], json!(["hr", "hr"]));
    let login = |email: &'static str| {
        let app = app.clone();
        async move {
            let (st, v) = call(
                &app,
                "POST",
                "/auth/login",
                None,
                Some(json!({ "email": email, "password": "Passw0rd!x" })),
            )
            .await;
            assert_eq!(st, StatusCode::OK, "{v}");
            v["token"].as_str().unwrap().to_string()
        }
    };
    let bob = login("bob@example.com").await;
    let carol = login("carol@example.com").await;

    let sheet = |name: &'static str, labels: Vec<&'static str>| {
        let (app, admin) = (app.clone(), admin.clone());
        async move {
            let (st, v) = call(
                &app,
                "POST",
                "/concepts",
                Some(&admin),
                Some(json!({ "id": 0, "concept_type": "Topic", "name": name, "description": format!("about {name}"), "labels": labels })),
            )
            .await;
            assert_eq!(st, StatusCode::OK, "{v}");
            v["id"].as_u64().unwrap()
        }
    };
    let public = sheet("public-sheet", vec![]).await;
    let hr = sheet("hr-sheet", vec!["hr"]).await;
    let secret = sheet("secret-sheet", vec!["hr", "secret"]).await;
    let link = |s: u64, t: u64| {
        let (app, admin) = (app.clone(), admin.clone());
        async move {
            let (st, v) = call(
                &app,
                "POST",
                "/relations",
                Some(&admin),
                Some(json!({ "id": 0, "relation_type": "related_to", "source": s, "target": t, "weight": 1.0 })),
            )
            .await;
            assert_eq!(st, StatusCode::OK, "{v}");
            v["id"].as_u64().unwrap()
        }
    };
    let rel_public_hr = link(public, hr).await;
    let _rel_hr_secret = link(hr, secret).await;
    let rel_public_secret = link(public, secret).await;
    World {
        app,
        admin,
        bob,
        carol,
        bob_id: ids[0].clone(),
        public,
        hr,
        secret,
        rel_public_hr,
        rel_public_secret,
    }
}

fn names(v: &Value) -> Vec<String> {
    let mut out: Vec<String> = v
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|c| c["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

#[tokio::test]
async fn a_hidden_sheet_does_not_exist_on_any_read_path() {
    let w = world("reads").await;
    let (app, bob) = (&w.app, w.bob.as_str());

    // The sheet itself: 404, like an id never allocated.
    let (st, _) = call(
        app,
        "GET",
        &format!("/concepts/{}", w.secret),
        Some(bob),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _) = call(app, "GET", &format!("/concepts/{}", w.hr), Some(bob), None).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = call(
        app,
        "GET",
        &format!("/concepts/{}", w.secret),
        Some(&w.admin),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "the administrator sees everything");

    // Listings: neither listed nor counted; the name search too.
    let (st, v) = call(app, "GET", "/concepts?type=Topic", Some(bob), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(names(&v["concepts"]), vec!["hr-sheet", "public-sheet"]);
    assert_eq!(v["total"], 2);
    let (_, v) = call(app, "GET", "/concepts?q=secret", Some(bob), None).await;
    assert_eq!(names(&v["concepts"]), Vec::<String>::new());
    let (_, v) = call(app, "GET", "/concepts", Some(&w.carol), None).await;
    assert_eq!(
        names(&v["concepts"]),
        vec!["public-sheet"],
        "no clearances at all"
    );
    let (_, v) = call(app, "GET", "/concepts", Some(&w.admin), None).await;
    assert_eq!(v["total"], 3);

    // Links: one hidden end hides the link.
    let (_, v) = call(app, "GET", "/relations", Some(bob), None).await;
    let ids: Vec<u64> = v["relations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids, vec![w.rel_public_hr]);
    assert_eq!(v["total"], 1);
    let (st, _) = call(
        app,
        "GET",
        &format!("/relations/{}", w.rel_public_secret),
        Some(bob),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _) = call(
        app,
        "GET",
        &format!("/relations/{}", w.rel_public_hr),
        Some(bob),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);

    // A path through or to a hidden sheet is no path.
    let path = |to: &'static str, token: &str| {
        let (app, token) = (app.clone(), token.to_string());
        async move {
            let (st, v) = call(
                &app,
                "POST",
                "/path",
                Some(&token),
                Some(json!({ "from_type": "Topic", "from_name": "public-sheet", "to_type": "Topic", "to_name": to, "max_depth": 4 })),
            )
            .await;
            assert_eq!(st, StatusCode::OK, "{v}");
            v["found"].as_bool().unwrap()
        }
    };
    assert!(path("secret-sheet", &w.admin).await);
    assert!(!path("secret-sheet", bob).await);
    assert!(path("hr-sheet", bob).await);

    // The subgraph page, retrieval and the question never carry it.
    let (st, v) = call(
        app,
        "POST",
        "/subgraph",
        Some(bob),
        Some(json!({ "seed_concept_ids": [w.public], "expansion_depth": 3, "limit": 50 })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(!v.to_string().contains("secret-sheet"), "{v}");
    assert!(v.to_string().contains("hr-sheet"), "{v}");
    let (st, v) = call(
        app,
        "POST",
        "/retrieve",
        Some(bob),
        Some(json!({ "query": "secret-sheet about", "top_k": 5, "lexical_weight": 1.0, "expansion": { "max_depth": 2 } })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(!v.to_string().contains("secret-sheet"), "{v}");
    let (st, v) = call(
        app,
        "POST",
        "/retrieve",
        Some(&w.admin),
        Some(json!({ "query": "secret-sheet about", "top_k": 5, "lexical_weight": 1.0 })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert!(
        v.to_string().contains("secret-sheet"),
        "the administrator: {v}"
    );
    let (st, v) = call(
        app,
        "POST",
        "/ask",
        Some(bob),
        Some(json!({ "query": "secret-sheet about", "top_k": 5, "lexical_weight": 1.0 })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    // The question is echoed back; the context must not hold the sheet.
    assert_eq!(
        names(&v["subgraph"]["concepts"]),
        vec!["hr-sheet", "public-sheet"]
    );
    assert!(
        v["retrieved"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["id"] != w.secret),
        "{v}"
    );
    assert!(
        !v["answer"].as_str().unwrap().contains("about secret-sheet"),
        "{v}"
    );

    // The export is the whole graph: administrators only.
    let (st, _) = call(app, "GET", "/export?format=json", Some(bob), None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, _) = call(app, "GET", "/export?format=json", Some(&w.admin), None).await;
    assert_eq!(st, StatusCode::OK);
}

#[tokio::test]
async fn writes_respect_labels_and_clearances() {
    let w = world("writes").await;
    let (app, bob) = (&w.app, w.bob.as_str());

    // No writing to a hidden sheet, no link to it, no deleting it.
    let (st, _) = call(
        app,
        "PATCH",
        &format!("/concepts/{}", w.secret),
        Some(bob),
        Some(json!({ "description": "x" })),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _) = call(
        app,
        "DELETE",
        &format!("/concepts/{}", w.secret),
        Some(bob),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, v) = call(
        app,
        "POST",
        "/concepts/delete",
        Some(bob),
        Some(json!({ "ids": [w.secret] })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["missing"], json!([w.secret]));
    let (st, _) = call(
        app,
        "POST",
        "/relations",
        Some(bob),
        Some(json!({ "id": 0, "relation_type": "related_to", "source": w.hr, "target": w.secret, "weight": 1.0 })),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _) = call(
        app,
        "PATCH",
        &format!("/relations/{}", w.rel_public_secret),
        Some(bob),
        Some(json!({ "weight": 2.0 })),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _) = call(
        app,
        "DELETE",
        &format!("/relations/{}", w.rel_public_secret),
        Some(bob),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // Labels within one's clearances only, and well-formed.
    let (st, v) = call(
        app,
        "PATCH",
        &format!("/concepts/{}", w.hr),
        Some(bob),
        Some(json!({ "labels": ["secret"] })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{v}");
    let (st, v) = call(
        app,
        "PATCH",
        &format!("/concepts/{}", w.public),
        Some(bob),
        Some(json!({ "labels": ["hr"] })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["labels"], json!(["hr"]));
    let (st, _) = call(
        app,
        "GET",
        &format!("/concepts/{}", w.public),
        Some(&w.carol),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND, "now hidden from carol");
    let (st, v) = call(
        app,
        "POST",
        "/concepts",
        Some(bob),
        Some(
            json!({ "id": 0, "concept_type": "Topic", "name": "bob-sheet", "labels": ["finance"] }),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{v}");
    let (st, v) = call(
        app,
        "POST",
        "/concepts",
        Some(&w.admin),
        Some(json!({ "id": 0, "concept_type": "Topic", "name": "bad", "labels": ["Not Valid"] })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    let (st, v) = call(
        app,
        "PATCH",
        &format!("/concepts/{}", w.hr),
        Some(&w.admin),
        Some(json!({ "labels": ["hr", "finance"] })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "the administrator labels freely: {v}");
    let (st, _) = call(app, "GET", &format!("/concepts/{}", w.hr), Some(bob), None).await;
    assert_eq!(
        st,
        StatusCode::NOT_FOUND,
        "a label beyond bob's clearances hides it"
    );
}

#[tokio::test]
async fn clearances_are_administered_and_take_effect_at_once() {
    let w = world("admin").await;
    let (app, bob) = (&w.app, w.bob.as_str());
    let uri = format!("/auth/users/{}", w.bob_id);

    let (st, _) = call(
        app,
        "PATCH",
        &uri,
        Some(&w.carol),
        Some(json!({ "clearances": ["hr"] })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "not an administrator");
    let (st, v) = call(
        app,
        "PATCH",
        &uri,
        Some(&w.admin),
        Some(json!({ "clearances": ["Bad One"] })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    let (st, _) = call(
        app,
        "PATCH",
        &uri,
        Some(&w.admin),
        Some(json!({ "role": "owner" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _) = call(
        app,
        "PATCH",
        "/auth/users/nobody",
        Some(&w.admin),
        Some(json!({ "clearances": [] })),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    let (st, v) = call(
        app,
        "PATCH",
        &uri,
        Some(&w.admin),
        Some(json!({ "clearances": ["hr", "secret"] })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (st, _) = call(
        app,
        "GET",
        &format!("/concepts/{}", w.secret),
        Some(bob),
        None,
    )
    .await;
    assert_eq!(
        st,
        StatusCode::OK,
        "visible with the new clearance, same token"
    );
    let (_, v) = call(app, "GET", "/auth/me", Some(bob), None).await;
    assert_eq!(v["user"]["clearances"], json!(["hr", "secret"]));

    // Roles: bob becomes an administrator and sees everything; an
    // administrator cannot take their own role away.
    let (st, v) = call(
        app,
        "PATCH",
        &uri,
        Some(&w.admin),
        Some(json!({ "role": "admin", "clearances": [] })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["user"]["role"], "admin");
    let (_, v) = call(app, "GET", "/concepts", Some(bob), None).await;
    assert_eq!(v["total"], 3);
    let (st, _) = call(
        app,
        "PATCH",
        &uri,
        Some(bob),
        Some(json!({ "role": "user" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, v) = call(
        app,
        "PATCH",
        &uri,
        Some(&w.admin),
        Some(json!({ "role": "user" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (_, v) = call(app, "GET", "/concepts", Some(bob), None).await;
    assert_eq!(v["total"], 1, "a plain user again, with no clearances");
}

#[tokio::test]
async fn without_a_login_everyone_sees_everything() {
    let app = app(None);
    let (st, v) = call(
        &app,
        "POST",
        "/concepts",
        None,
        Some(json!({ "id": 0, "concept_type": "Topic", "name": "secret-sheet", "labels": ["secret"] })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let id = v["id"].as_u64().unwrap();
    let (st, v) = call(&app, "GET", &format!("/concepts/{id}"), None, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["labels"], json!(["secret"]));
    let (_, v) = call(&app, "GET", "/concepts", None, None).await;
    assert_eq!(v["total"], 1);
    let (st, _) = call(&app, "GET", "/export?format=json", None, None).await;
    assert_eq!(st, StatusCode::OK);
}
