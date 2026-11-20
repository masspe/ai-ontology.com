// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Data sources over HTTP, against tables held in memory: a source and
//! its mappings are administered, browsed and previewed; a sync becomes
//! sheets and links through a job, runs again by watermark, marks what
//! the source dropped as stale; secrets are sealed on disk and never
//! served; everything is for administrators.

mod hardening_common;

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::Router;
use hardening_common::*;
use http::{Request, StatusCode};
use ontology_graph::{ConceptType, Ontology, OntologyGraph, RelationType};
use ontology_io::MemoryConnector;
use ontology_server::auth::{UserAuth, UserStore};
use ontology_server::sources::SourceStore;
use ontology_server::{build_router_with_config, AppState, JwtAuth, RouterConfig};
use ontology_storage::{MemoryStore, Store};
use serde_json::{json, Value};
use std::path::PathBuf;
use tower::ServiceExt;

const SECRET: &[u8] = b"test-secret-for-the-built-in-auth";

fn tempdir(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "ontology-sources-{tag}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn ontology() -> Ontology {
    let mut o = Ontology::new();
    o.add_concept_type(ConceptType {
        name: "Customer".into(),
        ..Default::default()
    });
    o.add_concept_type(ConceptType {
        name: "Order".into(),
        ..Default::default()
    });
    o.add_relation_type(RelationType {
        name: "placed_by".into(),
        domain: "Order".into(),
        range: "Customer".into(),
        ..Default::default()
    })
    .unwrap();
    o
}

struct World {
    app: Router,
    state: AppState,
    admin: String,
    dir: PathBuf,
    orders: Arc<MemoryConnector>,
}

async fn world(tag: &str) -> World {
    let dir = tempdir(tag);
    let graph = OntologyGraph::with_arc(ontology());
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let mut state = state_with(store, graph);
    state.sources = Arc::new(SourceStore::open(dir.join("sources.json")).unwrap());
    let orders = Arc::new(MemoryConnector::new().table(
        "orders",
        &["id", "label", "amount", "customer", "updated"],
        vec![
            vec![
                json!("O-1"),
                json!("Pumps"),
                json!(120.5),
                json!("C-7"),
                json!(1),
            ],
            vec![
                json!("O-2"),
                json!("Valves"),
                json!(80),
                json!("C-9"),
                json!(2),
            ],
            vec![json!(""), json!("no key"), json!(1), json!("C-7"), json!(3)],
        ],
    ));
    state.sources.register_test_connector("erp", orders.clone());
    let app = build_router_with_config(
        state.clone(),
        RouterConfig {
            bearer_token: None,
            jwt: Some(JwtAuth::from_secret(SECRET.to_vec())),
            rate_limit: None,
            users: Some(UserAuth {
                store: Arc::new(UserStore::open(dir.join("users.json")).unwrap()),
                allow_signup: true,
            }),
            web_dir: None,
            audit: None,
        },
    );
    let (st, v) = call(
        &app,
        "POST",
        "/auth/signup",
        None,
        Some(json!({ "email": "admin@example.com", "name": "Admin", "password": "Passw0rd!x" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let admin = v["token"].as_str().unwrap().to_string();
    World {
        app,
        state,
        admin,
        dir,
        orders,
    }
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

async fn wait_job(w: &World, id: u64) -> Value {
    for _ in 0..200 {
        let j = w.state.jobs.get(id).unwrap();
        if j.status != ontology_server::jobs::JobStatus::Running {
            return serde_json::to_value(j).unwrap();
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("job {id} still running");
}

async fn make_source(w: &World) -> u64 {
    let (st, v) = call(
        &w.app,
        "POST",
        "/sources",
        Some(&w.admin),
        Some(json!({ "name": "erp", "kind": "memory", "host": "db.example", "port": 1433, "user": "ro", "password": "s3cret-pass" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    assert!(v.get("password").is_none(), "never served: {v}");
    assert_eq!(v["password_hint"], "s3c...pass");
    v["id"].as_u64().unwrap()
}

async fn make_mapping(w: &World, sid: u64, extra: Value) -> u64 {
    let mut body = json!({
        "name": "orders", "from": "orders", "concept_type": "Order", "key_column": "id",
        "name_column": "label", "property_columns": ["amount"],
        "relations": [{ "column": "customer", "relation_type": "placed_by", "target_type": "Customer" }]
    });
    if let (Some(a), Some(b)) = (body.as_object_mut(), extra.as_object()) {
        for (k, v) in b {
            a.insert(k.clone(), v.clone());
        }
    }
    let (st, v) = call(
        &w.app,
        "POST",
        &format!("/sources/{sid}/mappings"),
        Some(&w.admin),
        Some(body),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    v["id"].as_u64().unwrap()
}

#[tokio::test]
async fn a_source_is_administered_tested_browsed_and_its_secret_sealed() {
    let w = world("crud").await;
    let sid = make_source(&w).await;

    let (_, v) = call(&w.app, "GET", "/sources", Some(&w.admin), None).await;
    assert_eq!(v["sources"].as_array().unwrap().len(), 1);
    let (st, v) = call(
        &w.app,
        "PATCH",
        &format!("/sources/{sid}"),
        Some(&w.admin),
        Some(json!({ "name": "erp", "timeout_secs": 5, "password": "" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["timeout_secs"], 5);
    assert_eq!(
        v["password_hint"], "s3c...pass",
        "an empty password keeps the stored one"
    );
    let (st, v) = call(
        &w.app,
        "PATCH",
        &format!("/sources/{sid}"),
        Some(&w.admin),
        Some(json!({ "kind": "oracle" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");

    let (st, v) = call(
        &w.app,
        "POST",
        &format!("/sources/{sid}/test"),
        Some(&w.admin),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["ok"], true);
    let (_, v) = call(
        &w.app,
        "GET",
        &format!("/sources/{sid}/tables"),
        Some(&w.admin),
        None,
    )
    .await;
    assert_eq!(v["tables"][0]["name"], "orders");
    let (_, v) = call(
        &w.app,
        "GET",
        &format!("/sources/{sid}/columns?table=orders"),
        Some(&w.admin),
        None,
    )
    .await;
    assert_eq!(v["columns"].as_array().unwrap().len(), 5);
    let (st, _) = call(
        &w.app,
        "GET",
        &format!("/sources/{sid}/columns?table=nope"),
        Some(&w.admin),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    // On disk: sealed, the password never in clear.
    let raw = std::fs::read_to_string(w.dir.join("sources.json")).unwrap();
    assert!(
        !raw.contains("s3cret-pass") && raw.contains("enc:v1:"),
        "{raw}"
    );
    let reopened = SourceStore::open(w.dir.join("sources.json")).unwrap();
    assert_eq!(reopened.get(sid).unwrap().password, "s3cret-pass");

    // A source that cannot be reached says so without failing the call.
    w.state
        .sources
        .register_test_connector("dead", Arc::new(MemoryConnector::down("refused")));
    let (_, v) = call(
        &w.app,
        "POST",
        "/sources",
        Some(&w.admin),
        Some(json!({ "name": "dead", "kind": "memory" })),
    )
    .await;
    let dead = v["id"].as_u64().unwrap();
    let (st, v) = call(
        &w.app,
        "POST",
        &format!("/sources/{dead}/test"),
        Some(&w.admin),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["ok"], false);
    assert!(v["error"].as_str().unwrap().contains("refused"));

    let (st, _) = call(
        &w.app,
        "DELETE",
        &format!("/sources/{dead}"),
        Some(&w.admin),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, _) = call(
        &w.app,
        "DELETE",
        &format!("/sources/{dead}"),
        Some(&w.admin),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _) = call(&w.app, "GET", "/sources/999", Some(&w.admin), None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    // Unknown kinds and missing locations are refused at connection time.
    let (_, v) = call(
        &w.app,
        "POST",
        "/sources",
        Some(&w.admin),
        Some(json!({ "name": "file", "kind": "csv" })),
    )
    .await;
    let csv = v["id"].as_u64().unwrap();
    let (st, v) = call(
        &w.app,
        "GET",
        &format!("/sources/{csv}/tables"),
        Some(&w.admin),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
}

#[tokio::test]
async fn a_sync_makes_sheets_and_links_then_follows_the_watermark_and_marks_stale_rows() {
    let w = world("sync").await;
    let sid = make_source(&w).await;
    let mid = make_mapping(&w, sid, json!({})).await;

    // Preview: nothing written, the sheets as they would be.
    let (st, v) = call(
        &w.app,
        "POST",
        &format!("/sources/{sid}/mappings/{mid}/preview"),
        Some(&w.admin),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["sheets"].as_array().unwrap().len(), 2);
    assert_eq!(v["links"], 2);
    assert_eq!(v["skipped"], 1);
    assert_eq!(v["sheets"][0]["name"], "Pumps");
    assert_eq!(w.state.graph.concept_count(), 0);

    // Run: a job, then the sheets and links are there.
    let (st, v) = call(
        &w.app,
        "POST",
        &format!("/sources/{sid}/mappings/{mid}/run"),
        Some(&w.admin),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::ACCEPTED, "{v}");
    let job = v["job"].as_u64().unwrap();
    let j = wait_job(&w, job).await;
    assert_eq!(j["status"], "done", "{j}");
    assert_eq!(j["result"]["rows"], 3);
    assert_eq!(j["result"]["skipped"], 1);
    assert_eq!(
        j["result"]["sheets_new"], 4,
        "two orders, two customer stubs"
    );
    assert_eq!(j["result"]["links"], 2);
    assert_eq!(
        j["result"]["watermark"],
        Value::Null,
        "no watermark column yet"
    );
    let pumps = w.state.graph.find_by_name("Order", "Pumps").unwrap();
    let c = w.state.graph.get_concept(pumps).unwrap();
    assert_eq!(
        c.properties["amount"],
        ontology_graph::PropertyValue::Number(120.5)
    );
    assert_eq!(
        c.properties["source"],
        ontology_graph::PropertyValue::Text("erp/orders".into())
    );
    assert!(w.state.graph.find_by_name("Customer", "C-7").is_some());
    let rels = w.state.graph.outgoing(pumps);
    assert_eq!(rels.len(), 1);
    assert_eq!(rels[0].relation_type, "placed_by");
    // The mapping remembers the run.
    let (_, v) = call(
        &w.app,
        "GET",
        &format!("/sources/{sid}"),
        Some(&w.admin),
        None,
    )
    .await;
    assert_eq!(v["mappings"][0]["last_status"], "ok");
    assert!(v["mappings"][0]["last_run_at"].is_string());
    // The sheets are searchable.
    let (_, v) = call(
        &w.app,
        "POST",
        "/retrieve",
        Some(&w.admin),
        Some(json!({ "query": "Valves", "top_k": 3, "lexical_weight": 1.0 })),
    )
    .await;
    assert!(v.to_string().contains("Valves"));

    // Second run, nothing changed: unchanged, no stale.
    let (_, v) = call(
        &w.app,
        "POST",
        &format!("/sources/{sid}/mappings/{mid}/run"),
        Some(&w.admin),
        None,
    )
    .await;
    let j = wait_job(&w, v["job"].as_u64().unwrap()).await;
    assert_eq!(j["result"]["sheets_new"], 0, "{j}");
    assert_eq!(j["result"]["stale_marked"], 0);

    // The source drops an order and changes one: full sync marks the gone
    // one stale, updates the other.
    w.orders.remove("orders", &json!("O-2"));
    w.orders.append(
        "orders",
        vec![vec![
            json!("O-1"),
            json!("Pumps"),
            json!(130),
            json!("C-7"),
            json!(4),
        ]],
    );
    let (_, v) = call(
        &w.app,
        "POST",
        &format!("/sources/{sid}/mappings/{mid}/run"),
        Some(&w.admin),
        None,
    )
    .await;
    let j = wait_job(&w, v["job"].as_u64().unwrap()).await;
    assert_eq!(j["result"]["stale_marked"], 1, "{j}");
    let valves = w.state.graph.find_by_name("Order", "Valves").unwrap();
    assert_eq!(
        w.state.graph.get_concept(valves).unwrap().properties["stale"],
        ontology_graph::PropertyValue::Bool(true)
    );
    assert_eq!(
        w.state.graph.get_concept(pumps).unwrap().properties["amount"],
        ontology_graph::PropertyValue::Number(130.0)
    );

    // Incremental: a watermark column; only newer rows come back.
    let (st, v) = call(
        &w.app,
        "PATCH",
        &format!("/sources/{sid}/mappings/{mid}"),
        Some(&w.admin),
        Some(json!({ "watermark_column": "updated" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (_, v) = call(
        &w.app,
        "POST",
        &format!("/sources/{sid}/mappings/{mid}/run"),
        Some(&w.admin),
        None,
    )
    .await;
    let j = wait_job(&w, v["job"].as_u64().unwrap()).await;
    assert_eq!(j["result"]["watermark"], 4, "{j}");
    w.orders.append(
        "orders",
        vec![vec![
            json!("O-3"),
            json!("Gaskets"),
            json!(5),
            json!("C-7"),
            json!(9),
        ]],
    );
    let (_, v) = call(
        &w.app,
        "POST",
        &format!("/sources/{sid}/mappings/{mid}/run"),
        Some(&w.admin),
        None,
    )
    .await;
    let j = wait_job(&w, v["job"].as_u64().unwrap()).await;
    assert_eq!(
        j["result"]["rows"], 1,
        "only the row past the watermark: {j}"
    );
    assert_eq!(j["result"]["watermark"], 9);
    assert_eq!(
        j["result"]["stale_marked"], 0,
        "an incremental run marks nothing stale"
    );
    assert!(w.state.graph.find_by_name("Order", "Gaskets").is_some());
    let (_, v) = call(
        &w.app,
        "GET",
        &format!("/sources/{sid}"),
        Some(&w.admin),
        None,
    )
    .await;
    assert_eq!(v["mappings"][0]["last_watermark"], 9);

    // A mapping that names a missing column fails the job, not the server.
    let bad = make_mapping(&w, sid, json!({ "name": "bad", "key_column": "ghost" })).await;
    let (_, v) = call(
        &w.app,
        "POST",
        &format!("/sources/{sid}/mappings/{bad}/run"),
        Some(&w.admin),
        None,
    )
    .await;
    let j = wait_job(&w, v["job"].as_u64().unwrap()).await;
    assert_eq!(j["status"], "failed");
    assert!(j["message"].as_str().unwrap().contains("ghost"), "{j}");
    let (_, v) = call(
        &w.app,
        "GET",
        &format!("/sources/{sid}"),
        Some(&w.admin),
        None,
    )
    .await;
    assert!(v["mappings"][1]["last_status"]
        .as_str()
        .unwrap()
        .starts_with("failed"));
    let (st, _) = call(
        &w.app,
        "DELETE",
        &format!("/sources/{sid}/mappings/{bad}"),
        Some(&w.admin),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, _) = call(
        &w.app,
        "DELETE",
        &format!("/sources/{sid}/mappings/{bad}"),
        Some(&w.admin),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn sources_are_for_administrators_and_a_schedule_is_due_once_its_interval_passed() {
    let w = world("admin").await;
    let sid = make_source(&w).await;
    let (_, v) = call(
        &w.app,
        "POST",
        "/auth/users",
        Some(&w.admin),
        Some(json!({ "email": "u@example.com", "name": "U", "password": "Passw0rd!x" })),
    )
    .await;
    assert!(v["user"]["id"].is_string());
    let (_, v) = call(
        &w.app,
        "POST",
        "/auth/login",
        None,
        Some(json!({ "email": "u@example.com", "password": "Passw0rd!x" })),
    )
    .await;
    let user = v["token"].as_str().unwrap().to_string();
    for (m, p) in [
        ("GET", "/sources".to_string()),
        ("POST", format!("/sources/{sid}/test")),
        ("GET", format!("/sources/{sid}/tables")),
    ] {
        let (st, _) = call(&w.app, m, &p, Some(&user), None).await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{m} {p}");
    }
    let (st, _) = call(&w.app, "GET", "/sources", None, None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);

    let mid = make_mapping(&w, sid, json!({ "schedule_minutes": 30 })).await;
    assert_eq!(
        w.state.sources.due(),
        vec![(sid, mid)],
        "never run: due now"
    );
    let (_, v) = call(
        &w.app,
        "POST",
        &format!("/sources/{sid}/mappings/{mid}/run"),
        Some(&w.admin),
        None,
    )
    .await;
    wait_job(&w, v["job"].as_u64().unwrap()).await;
    assert!(w.state.sources.due().is_empty(), "just ran: not due");
    // One sync per mapping at a time.
    let (st, v) = call(
        &w.app,
        "PATCH",
        &format!("/sources/{sid}/mappings/{mid}"),
        Some(&w.admin),
        Some(json!({ "schedule_minutes": 0, "reset_watermark": true })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["schedule_minutes"], 0);
    assert!(w.state.sources.due().is_empty(), "by hand only");
}

/// A named query: a question that matches its trigger runs the SQL with
/// the groups bound as literals and the rows join the prompt; an
/// administrator tries it; a question that matches nothing is unchanged.
#[tokio::test]
async fn a_named_query_answers_a_question_from_the_live_source() {
    let w = world("lookup").await;
    let sid = make_source(&w).await;
    // The memory source knows the bound statement.
    let tickets = Arc::new(MemoryConnector::new().statement(
        "SELECT id, title, status FROM tickets WHERE id = '1234'",
        &["id", "title", "status"],
        vec![vec![json!(1234), json!("Printer offline"), json!("open")]],
    ));
    w.state.sources.register_test_connector("erp", tickets);
    let (st, v) = call(
        &w.app,
        "POST",
        &format!("/sources/{sid}/queries"),
        Some(&w.admin),
        Some(json!({ "name": "ticket", "trigger": r"(?i)ticket\s*(?:n°|#)?\s*(?P<id>\d{1,9})", "sql": "SELECT id, title, status FROM tickets WHERE id = {id}", "max_rows": 5 })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let qid = v["id"].as_u64().unwrap();
    let (st, v) = call(
        &w.app,
        "POST",
        &format!("/sources/{sid}/queries"),
        Some(&w.admin),
        Some(json!({ "name": "bad", "trigger": "(", "sql": "SELECT 1" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    let (st, v) = call(
        &w.app,
        "POST",
        &format!("/sources/{sid}/queries"),
        Some(&w.admin),
        Some(json!({ "name": "bad", "trigger": "x", "sql": "DELETE FROM t" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");

    // Try: the bound SQL and the rows.
    let (st, v) = call(
        &w.app,
        "POST",
        &format!("/sources/{sid}/queries/{qid}/try"),
        Some(&w.admin),
        Some(json!({ "question": "J'ai le ticket n° 1234, comment résoudre ?" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["matched"], true);
    assert_eq!(
        v["sql"],
        "SELECT id, title, status FROM tickets WHERE id = '1234'"
    );
    assert_eq!(v["rows"][0][1], "Printer offline");
    let (_, v) = call(
        &w.app,
        "POST",
        &format!("/sources/{sid}/queries/{qid}/try"),
        Some(&w.admin),
        Some(json!({ "question": "rien" })),
    )
    .await;
    assert_eq!(v["matched"], false);

    // The question: the rows are in the prompt (the echo model shows it).
    let (st, v) = call(
        &w.app,
        "POST",
        "/ask",
        Some(&w.admin),
        Some(json!({ "query": "J'ai le ticket n° 1234, comment résoudre ?", "top_k": 3 })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let answer = v["answer"].as_str().unwrap();
    assert!(
        answer.contains("---DATA---")
            && answer.contains("Printer offline")
            && answer.contains("query `ticket`"),
        "{answer}"
    );
    let (_, v) = call(
        &w.app,
        "POST",
        "/ask",
        Some(&w.admin),
        Some(json!({ "query": "une question sans ticket", "top_k": 3 })),
    )
    .await;
    assert!(!v["answer"].as_str().unwrap().contains("---DATA---"));

    // Administrators only when asked; disabled queries never run.
    let (_, v) = call(
        &w.app,
        "POST",
        "/auth/users",
        Some(&w.admin),
        Some(json!({ "email": "u@example.com", "name": "U", "password": "Passw0rd!x" })),
    )
    .await;
    assert!(v["user"]["id"].is_string());
    let (_, v) = call(
        &w.app,
        "POST",
        "/auth/login",
        None,
        Some(json!({ "email": "u@example.com", "password": "Passw0rd!x" })),
    )
    .await;
    let user = v["token"].as_str().unwrap().to_string();
    let (_, v) = call(
        &w.app,
        "POST",
        "/ask",
        Some(&user),
        Some(json!({ "query": "ticket 1234", "top_k": 3 })),
    )
    .await;
    assert!(
        v["answer"].as_str().unwrap().contains("Printer offline"),
        "open to every reader by default"
    );
    let (st, _) = call(
        &w.app,
        "PATCH",
        &format!("/sources/{sid}/queries/{qid}"),
        Some(&w.admin),
        Some(json!({ "admin_only": true })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (_, v) = call(
        &w.app,
        "POST",
        "/ask",
        Some(&user),
        Some(json!({ "query": "ticket 1234", "top_k": 3 })),
    )
    .await;
    assert!(
        !v["answer"].as_str().unwrap().contains("Printer offline"),
        "administrators only"
    );
    let (st, _) = call(
        &w.app,
        "PATCH",
        &format!("/sources/{sid}/queries/{qid}"),
        Some(&w.admin),
        Some(json!({ "admin_only": false, "enabled": false })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (_, v) = call(
        &w.app,
        "POST",
        "/ask",
        Some(&w.admin),
        Some(json!({ "query": "ticket 1234", "top_k": 3 })),
    )
    .await;
    assert!(
        !v["answer"].as_str().unwrap().contains("Printer offline"),
        "disabled"
    );
    let (st, _) = call(
        &w.app,
        "DELETE",
        &format!("/sources/{sid}/queries/{qid}"),
        Some(&w.admin),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, _) = call(
        &w.app,
        "DELETE",
        &format!("/sources/{sid}/queries/{qid}"),
        Some(&w.admin),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}
