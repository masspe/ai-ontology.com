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
    world_on(tag, Arc::new(MemoryStore::new())).await
}

async fn world_on(tag: &str, store: Arc<dyn Store>) -> World {
    let dir = tempdir(tag);
    let graph = OntologyGraph::with_arc(ontology());
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
    for _ in 0..3000 {
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
    assert_eq!(
        v["password_hint"], "••••••••",
        "that one is set, nothing of it"
    );
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
        v["password_hint"], "••••••••",
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

/// A CSV from the administrator's computer is kept beside the sources
/// and answered as the path to give one; the drivers are listed.
#[tokio::test]
async fn a_csv_is_uploaded_for_a_source_and_the_odbc_drivers_are_listed() {
    let w = world("upload").await;
    let boundary = "XBOUNDARY";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"my clients.csv\"\r\nContent-Type: text/csv\r\n\r\ncode,name\r\nC1,Acme\r\n\r\n--{boundary}--\r\n"
    );
    let up = |body: String| {
        Request::builder()
            .method("POST")
            .uri("/sources/files")
            .header("authorization", format!("Bearer {}", w.admin))
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap()
    };
    let resp = w.app.clone().oneshot(up(body.clone())).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v: Value =
        serde_json::from_slice(&to_bytes(resp.into_body(), 1 << 20).await.unwrap()).unwrap();
    let path = PathBuf::from(v["path"].as_str().unwrap());
    assert_eq!(path, w.dir.join("source-files").join("my_clients.csv"));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "code,name\r\nC1,Acme\r\n"
    );

    // Not a text table: refused.
    let resp = w
        .app
        .clone()
        .oneshot(up(body.replace("my clients.csv", "tool.exe")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let resp = w
        .app
        .clone()
        .oneshot(up(format!("--{boundary}--\r\n")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let (st, v) = call(&w.app, "GET", "/sources/odbc", Some(&w.admin), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(v["drivers"].is_array() && v["dsns"].is_array(), "{v}");
    if !cfg!(feature = "odbc") {
        assert_eq!(v["available"], false);
    }
}

/// A table past one page is read by keyset pages on its key (each page
/// one statement), a key read twice is one sheet, and the table's
/// indexes are listed, unique first.
#[tokio::test]
async fn a_big_table_is_read_by_pages_and_its_indexes_listed() {
    let w = world("pages").await;
    let n = ontology_server::sources::PAGE + 500;
    let mut rows: Vec<Vec<Value>> = (1..=n)
        .map(|i| vec![json!(i), json!(format!("Item {i}")), json!(i % 7)])
        .collect();
    // The last key again, with another value: merged, not a failure.
    rows.push(vec![json!(n), json!(format!("Item {n}")), json!(99)]);
    let big = Arc::new(
        MemoryConnector::new()
            .table("items", &["id", "label", "amount"], rows)
            .index("items", "by_amount", &["amount"], false)
            .index("items", "pk", &["id"], true),
    );
    w.state.sources.register_test_connector("big", big);
    let (st, v) = call(
        &w.app,
        "POST",
        "/sources",
        Some(&w.admin),
        Some(json!({ "name": "big", "kind": "memory" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let sid = v["id"].as_u64().unwrap();
    let (st, v) = call(
        &w.app,
        "GET",
        &format!("/sources/{sid}/indexes?table=items"),
        Some(&w.admin),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(
        v["indexes"],
        json!([
            { "name": "pk", "columns": ["id"], "unique": true },
            { "name": "by_amount", "columns": ["amount"], "unique": false }
        ]),
        "unique first"
    );
    let mid = make_mapping(
        &w,
        sid,
        json!({ "name": "items", "from": "items", "concept_type": "Item", "key_column": "id", "name_column": "label", "property_columns": ["amount"], "relations": [] }),
    )
    .await;
    let (st, v) = call(
        &w.app,
        "POST",
        &format!("/sources/{sid}/mappings/{mid}/run"),
        Some(&w.admin),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::ACCEPTED, "{v}");
    let j = wait_job(&w, v["job"].as_u64().unwrap()).await;
    assert_eq!(j["status"], "done", "{j}");
    assert_eq!(j["result"]["rows"], n + 1, "{j}");
    assert_eq!(j["result"]["sheets_new"], n);
    assert_eq!(j["result"]["duplicates"], 1);
    assert_eq!(j["result"]["stale_marked"], 0);
    let last = w
        .state
        .graph
        .find_by_name("Item", &format!("Item {n}"))
        .unwrap();
    assert_eq!(
        w.state.graph.get_concept(last).unwrap().properties["amount"],
        ontology_graph::PropertyValue::Number(99.0),
        "the last row's value"
    );
    assert!(
        j["log"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l.as_str() == Some("10000 rows so far")),
        "{j}"
    );
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

/// A store that asks for the running sync to be cancelled once `after`
/// records are written: the request lands in the middle of a batch.
struct CancelMidSync {
    inner: MemoryStore,
    written: std::sync::atomic::AtomicUsize,
    after: usize,
    jobs: std::sync::OnceLock<Arc<ontology_server::jobs::Jobs>>,
}

#[async_trait::async_trait]
impl Store for CancelMidSync {
    async fn append(
        &self,
        record: &ontology_storage::LogRecord,
    ) -> ontology_storage::StoreResult<()> {
        self.inner.append(record).await?;
        let n = self
            .written
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        if n == self.after {
            let jobs = self.jobs.get().expect("the jobs of the world");
            assert!(jobs.cancel(jobs.running("sync", "erp/orders").expect("a sync runs")));
        }
        Ok(())
    }

    async fn load_into(&self, graph: &Arc<OntologyGraph>) -> ontology_storage::StoreResult<()> {
        self.inner.load_into(graph).await
    }
}

/// A sync stopped between two batches: the batch in flight is finished —
/// every sheet with its link, in the store as in memory — nothing is
/// marked stale, the watermark stays where it was, a cancel is not a
/// failure of the source; and the next sync finishes the work.
#[tokio::test]
async fn a_sync_cancelled_mid_run_keeps_whole_batches_and_the_next_one_finishes() {
    let store = Arc::new(CancelMidSync {
        inner: MemoryStore::new(),
        written: 0.into(),
        after: 100,
        jobs: Default::default(),
    });
    let w = world_on("cancel", store.clone()).await;
    assert!(store.jobs.set(w.state.jobs.clone()).is_ok());
    // 4 500 orders of 50 customers: three batches.
    let rows = (1..=4500)
        .map(|i| {
            vec![
                json!(format!("O-{i}")),
                json!(format!("Order {i}")),
                json!(i),
                json!(format!("C-{}", i % 50)),
                json!(i),
            ]
        })
        .collect();
    w.state.sources.register_test_connector(
        "erp",
        Arc::new(MemoryConnector::new().table(
            "orders",
            &["id", "label", "amount", "customer", "updated"],
            rows,
        )),
    );
    let sid = make_source(&w).await;
    let mid = make_mapping(&w, sid, json!({ "watermark_column": "updated" })).await;
    let run = format!("/sources/{sid}/mappings/{mid}/run");

    let (st, v) = call(&w.app, "POST", &run, Some(&w.admin), None).await;
    assert_eq!(st, StatusCode::ACCEPTED, "{v}");
    let j = wait_job(&w, v["job"].as_u64().unwrap()).await;
    assert_eq!(j["status"], "cancelled", "{j}");

    let graph = &w.state.graph;
    let orders = graph.concepts_of_type("Order", false);
    assert_eq!(
        orders.len(),
        2000,
        "the batch in flight is finished, the next one not started"
    );
    for o in &orders {
        let links = graph.outgoing(o.id);
        assert_eq!(links.len(), 1, "{}", o.name);
        assert_eq!(
            graph.get_concept(links[0].target).unwrap().concept_type,
            "Customer"
        );
        assert_eq!(
            o.properties["stale"],
            ontology_graph::PropertyValue::Bool(false),
            "{}",
            o.name
        );
    }
    assert_eq!(graph.concepts_of_type("Customer", false).len(), 50);
    assert_eq!(graph.relation_count(), 2000);
    // What a restart would replay is what is in memory.
    let replayed = OntologyGraph::with_arc(ontology());
    w.state.store.load_into(&replayed).await.unwrap();
    assert_eq!(
        (replayed.concept_count(), replayed.relation_count()),
        (graph.concept_count(), graph.relation_count())
    );
    // The sheets written are found.
    assert!(w.state.index.index_concept(orders[0].id).is_ok());
    let (_, v) = call(
        &w.app,
        "GET",
        &format!("/sources/{sid}"),
        Some(&w.admin),
        None,
    )
    .await;
    assert_eq!(v["mappings"][0]["last_status"], "cancelled", "{v}");
    assert_eq!(v["mappings"][0]["last_watermark"], Value::Null, "{v}");
    let health = w.state.sources.health(sid);
    assert_eq!((health.syncs, health.sync_failures), (1, 0));

    // The next run: the rest is written, what was there is recognised.
    let (_, v) = call(&w.app, "POST", &run, Some(&w.admin), None).await;
    let j = wait_job(&w, v["job"].as_u64().unwrap()).await;
    assert_eq!(j["status"], "done", "{j}");
    assert_eq!(j["result"]["rows"], 4500);
    assert_eq!(j["result"]["sheets_new"], 2500, "{j}");
    assert_eq!(j["result"]["watermark"], 4500);
    assert_eq!(graph.concepts_of_type("Order", false).len(), 4500);
    assert_eq!(graph.relation_count(), 4500);
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

#[tokio::test]
async fn a_source_is_counted_on_metrics_and_its_circuit_opens_over_http() {
    let w = world("health").await;
    let sid = make_source(&w).await;
    let mid = make_mapping(&w, sid, json!({})).await;
    let (st, v) = call(
        &w.app,
        "POST",
        &format!("/sources/{sid}/mappings/{mid}/run"),
        Some(&w.admin),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::ACCEPTED, "{v}");
    let job = wait_job(&w, v["job"].as_u64().unwrap()).await;
    assert_eq!(job["status"], "done", "{job}");

    let metrics = |app: Router, token: String| async move {
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        String::from_utf8(to_bytes(resp.into_body(), 1 << 20).await.unwrap().to_vec()).unwrap()
    };
    let text = metrics(w.app.clone(), w.admin.clone()).await;
    for line in [
        "# TYPE ontology_source_calls_total counter",
        "ontology_source_calls_total{source=\"erp\"} 1",
        "ontology_source_rows_total{source=\"erp\"} 3",
        "ontology_source_syncs_total{source=\"erp\"} 1",
        "ontology_source_sync_failures_total{source=\"erp\"} 0",
        "ontology_source_failures_total{source=\"erp\"} 0",
        "ontology_source_lookups_total{source=\"erp\"} 0",
        "# TYPE ontology_source_breaker_open gauge",
        "ontology_source_breaker_open{source=\"erp\"} 0",
    ] {
        assert!(text.contains(line), "missing `{line}` in:\n{text}");
    }

    // The engine goes away: five listings fail, the sixth is not sent.
    w.state
        .sources
        .register_test_connector("erp", Arc::new(MemoryConnector::down("connection refused")));
    for _ in 0..5 {
        let (st, v) = call(
            &w.app,
            "GET",
            &format!("/sources/{sid}/tables"),
            Some(&w.admin),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
        assert!(v.to_string().contains("connection refused"), "{v}");
    }
    let (st, v) = call(
        &w.app,
        "GET",
        &format!("/sources/{sid}/tables"),
        Some(&w.admin),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(v.to_string().contains("circuit open"), "{v}");
    // A sync started now fails at once, and is counted.
    let (_, v) = call(
        &w.app,
        "POST",
        &format!("/sources/{sid}/mappings/{mid}/run"),
        Some(&w.admin),
        None,
    )
    .await;
    let job = wait_job(&w, v["job"].as_u64().unwrap()).await;
    assert_eq!(job["status"], "failed", "{job}");
    assert!(job.to_string().contains("circuit open"), "{job}");
    let text = metrics(w.app.clone(), w.admin.clone()).await;
    assert!(
        text.contains("ontology_source_breaker_open{source=\"erp\"} 1"),
        "{text}"
    );
    assert!(
        text.contains("ontology_source_sync_failures_total{source=\"erp\"} 1"),
        "{text}"
    );

    // The engine is back: "Test connection" goes through and closes it.
    w.state
        .sources
        .register_test_connector("erp", w.orders.clone());
    let (st, v) = call(
        &w.app,
        "POST",
        &format!("/sources/{sid}/test"),
        Some(&w.admin),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["ok"], true, "{v}");
    let (st, v) = call(
        &w.app,
        "GET",
        &format!("/sources/{sid}/tables"),
        Some(&w.admin),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(w.state.sources.health(sid).failures_in_a_row, 0);
}

/// The whole path on a real engine, when `ONTOLOGY_TEST_POSTGRES` names
/// one seeded with `crates/io/tests/fixtures/live/postgres.sql` (see
/// `crates/io/tests/live_engines.rs`): a source, its tables, a mapping, a
/// sync into sheets, a second sync by watermark, a question answered
/// with live rows.
#[cfg(feature = "postgres")]
#[tokio::test]
async fn a_real_engine_is_synced_into_sheets_and_asked_live() {
    let Ok(spec) = std::env::var("ONTOLOGY_TEST_POSTGRES") else {
        assert!(
            std::env::var("ONTOLOGY_TEST_ENGINES").as_deref() != Ok("required"),
            "ONTOLOGY_TEST_POSTGRES is not set"
        );
        return;
    };
    let e: std::collections::HashMap<&str, &str> = spec
        .split_whitespace()
        .filter_map(|p| p.split_once('='))
        .collect();
    let w = world("live").await;
    let body = json!({
        "name": "live", "kind": "postgres", "host": e["host"],
        "port": e["port"].parse::<u16>().unwrap(), "database": e["database"],
        "user": e["user"], "password": e["password"],
        "encrypt": true, "trust_cert": true, "timeout_secs": 30
    });
    let (st, v) = call(&w.app, "POST", "/sources", Some(&w.admin), Some(body)).await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let sid = v["id"].as_u64().unwrap();
    // The test engine is local: refused until the source allows it.
    let local = e["host"]
        .parse::<std::net::IpAddr>()
        .is_ok_and(|ip| ip.is_loopback());
    if local {
        let (st, v) = call(
            &w.app,
            "POST",
            &format!("/sources/{sid}/test"),
            Some(&w.admin),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{v}");
        assert!(v.to_string().contains("allow_private_hosts"), "{v}");
    }
    let (st, v) = call(
        &w.app,
        "PATCH",
        &format!("/sources/{sid}"),
        Some(&w.admin),
        Some(json!({ "allow_private_hosts": true })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (st, v) = call(
        &w.app,
        "POST",
        &format!("/sources/{sid}/test"),
        Some(&w.admin),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["ok"], true, "{v}");
    let (_, v) = call(
        &w.app,
        "GET",
        &format!("/sources/{sid}/tables"),
        Some(&w.admin),
        None,
    )
    .await;
    assert!(v.to_string().contains("ontology_live_item"), "{v}");

    let (st, v) = call(
        &w.app,
        "POST",
        &format!("/sources/{sid}/mappings"),
        Some(&w.admin),
        Some(json!({
            "name": "items", "from": "public.ontology_live_item", "concept_type": "Order",
            "key_column": "id", "name_column": "name", "property_columns": ["price", "ok"],
            "watermark_column": "id"
        })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let mid = v["id"].as_u64().unwrap();
    let run = || async {
        let (st, v) = call(
            &w.app,
            "POST",
            &format!("/sources/{sid}/mappings/{mid}/run"),
            Some(&w.admin),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::ACCEPTED, "{v}");
        let mut j = wait_job(&w, v["job"].as_u64().unwrap()).await;
        // A real engine may take longer than the in-memory one.
        for _ in 0..50 {
            if j["status"] != "running" {
                break;
            }
            j = wait_job(&w, v["job"].as_u64().unwrap()).await;
        }
        j
    };
    let j = run().await;
    assert_eq!(j["status"], "done", "{j}");
    assert_eq!(j["result"]["rows"], 3, "{j}");
    assert_eq!(j["result"]["sheets_new"], 3, "{j}");
    assert_eq!(j["result"]["watermark"], 3, "{j}");
    let valve = w.state.graph.find_by_name("Order", "Valve").unwrap();
    let c = w.state.graph.get_concept(valve).unwrap();
    assert_eq!(
        c.properties["ok"],
        ontology_graph::PropertyValue::Bool(true)
    );
    assert!(w
        .state
        .graph
        .find_by_name("Order", "O'Brien & fils — été")
        .is_some());
    // Nothing past the watermark: nothing read.
    let j = run().await;
    assert_eq!(j["status"], "done", "{j}");
    assert_eq!(j["result"]["rows"], 0, "{j}");

    // A question fills the blank of the administrator's statement.
    let (st, v) = call(
        &w.app,
        "POST",
        &format!("/sources/{sid}/queries"),
        Some(&w.admin),
        Some(json!({
            "name": "item", "trigger": r"(?i)\bitem\s+(?P<name>[\w']+)",
            "sql": "SELECT id, name, price FROM public.ontology_live_item WHERE name = {name}",
            "max_rows": 5
        })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let (st, v) = call(
        &w.app,
        "POST",
        "/ask",
        Some(&w.admin),
        Some(json!({ "query": "What is the price of item Valve?", "top_k": 3 })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let answer = v["answer"].as_str().unwrap();
    assert!(
        answer.contains("---DATA---") && answer.contains("9.99") && answer.contains("query `item`"),
        "{answer}"
    );
    // A quote in the question is not a value: the query does not run.
    let (_, v) = call(
        &w.app,
        "POST",
        "/ask",
        Some(&w.admin),
        Some(json!({ "query": "item x' OR '1'='1", "top_k": 3 })),
    )
    .await;
    assert!(!v["answer"].as_str().unwrap().contains("---DATA---"), "{v}");
    // The engine was counted.
    let h = w.state.sources.health(sid);
    assert!(h.calls >= 5 && h.rows >= 4 && h.lookups == 1, "{h:?}");
    let _ = std::fs::remove_dir_all(&w.dir);
}
