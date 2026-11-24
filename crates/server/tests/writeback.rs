// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Write-back over HTTP, against a connector held in memory: off until a
//! source opts in; a statement is checked when it is saved; a run only
//! files an approval; another administrator account — never the one who
//! asked, a key or the service token — approves it, and only then does
//! the frozen statement run, once, within its row-count guard; each step
//! is in the audit log.

mod hardening_common;

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::Router;
use hardening_common::*;
use http::{Request, StatusCode};
use ontology_graph::{ActionType, ConceptType, Ontology, OntologyGraph};
use ontology_io::MemoryConnector;
use ontology_server::audit::AuditLog;
use ontology_server::auth::{UserAuth, UserStore};
use ontology_server::sources::SourceStore;
use ontology_server::{build_router_with_config, JwtAuth, RouterConfig};
use ontology_storage::MemoryStore;
use serde_json::{json, Value};
use std::path::PathBuf;
use tower::ServiceExt;

const SECRET: &[u8] = b"test-secret-for-the-built-in-auth";
const SERVICE: &str = "service-token-of-the-tests";
const CLOSE: &str = "update orders set state = 'done' where code = 'O-7'";
const CLOSE_ALL: &str = "update orders set state = 'done' where customer = 'C-1'";

fn tempdir(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "ontology-writeback-{tag}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn ontology() -> Ontology {
    let mut o = Ontology::new();
    for name in ["Customer", "Order"] {
        o.add_concept_type(ConceptType {
            name: name.into(),
            ..Default::default()
        });
    }
    for name in ["close", "close_all"] {
        o.add_action_type(ActionType {
            name: name.into(),
            subject: "Order".into(),
            object: Some("Customer".into()),
            parameters: vec!["state".into()],
            effect: String::new(),
            description: String::new(),
        })
        .unwrap();
    }
    o
}

struct World {
    app: Router,
    /// The first account: an administrator.
    admin: String,
    /// Another administrator account.
    second: String,
    /// An account that is not an administrator.
    user: String,
    /// An unscoped API key.
    key: String,
    dir: PathBuf,
    erp: Arc<MemoryConnector>,
    source: u64,
    order: u64,
    customer: u64,
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

async fn account(app: &Router, admin: &str, email: &str, role: &str) -> String {
    let (st, v) = call(
        app,
        "POST",
        "/auth/users",
        Some(admin),
        Some(json!({ "email": email, "name": email, "password": "Passw0rd!x" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    if role == "admin" {
        let uri = format!("/auth/users/{}", v["user"]["id"].as_str().unwrap());
        let (st, v) = call(
            app,
            "PATCH",
            &uri,
            Some(admin),
            Some(json!({ "role": "admin" })),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{v}");
    }
    let (st, v) = call(
        app,
        "POST",
        "/auth/login",
        None,
        Some(json!({ "email": email, "password": "Passw0rd!x" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    v["token"].as_str().unwrap().to_string()
}

/// Two administrators, a plain account, a key; a source named `erp`
/// (closed to writes) over a connector that knows two writes; an order
/// and its customer.
async fn world(tag: &str) -> World {
    let dir = tempdir(tag);
    let graph = OntologyGraph::with_arc(ontology());
    let mut state = state_with(Arc::new(MemoryStore::new()), graph);
    state.sources = Arc::new(SourceStore::open(dir.join("sources.json")).unwrap());
    let erp = Arc::new(MemoryConnector::new().write(CLOSE, 1).write(CLOSE_ALL, 5));
    state.sources.register_test_connector("erp", erp.clone());
    let app = build_router_with_config(
        state,
        RouterConfig {
            bearer_token: Some(SERVICE.into()),
            jwt: Some(JwtAuth::from_secret(SECRET.to_vec())),
            rate_limit: None,
            users: Some(UserAuth {
                store: Arc::new(UserStore::open(dir.join("users.json")).unwrap()),
                allow_signup: true,
            }),
            web_dir: None,
            audit: Some(Arc::new(AuditLog::open(dir.join("audit.jsonl")).unwrap())),
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
    let second = account(&app, &admin, "second@example.com", "admin").await;
    let user = account(&app, &admin, "user@example.com", "user").await;
    let (st, v) = call(
        &app,
        "POST",
        "/auth/keys",
        Some(&admin),
        Some(json!({ "name": "bot" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let key = v["key"].as_str().unwrap().to_string();

    let (st, v) = call(
        &app,
        "POST",
        "/sources",
        Some(&admin),
        Some(json!({ "name": "erp", "kind": "memory" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    assert_eq!(v["allow_writes"], false, "off until asked: {v}");
    assert_eq!(v["statements"], json!([]));
    let source = v["id"].as_u64().unwrap();

    let mut ids = Vec::new();
    for (kind, name, code) in [("Order", "Order 7", "O-7"), ("Customer", "Acme", "C-1")] {
        let (st, v) = call(
            &app,
            "POST",
            "/concepts",
            Some(&admin),
            Some(json!({ "id": 0, "concept_type": kind, "name": name, "properties": { "code": code } })),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{v}");
        ids.push(v["id"].as_u64().unwrap());
    }
    World {
        app,
        admin,
        second,
        user,
        key,
        dir,
        erp,
        source,
        order: ids[0],
        customer: ids[1],
    }
}

impl World {
    async fn admin(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        call(&self.app, method, uri, Some(&self.admin), body).await
    }

    async fn second(&self, method: &str, uri: &str) -> (StatusCode, Value) {
        call(&self.app, method, uri, Some(&self.second), None).await
    }

    async fn open(&self, allow: bool) {
        let (st, v) = self
            .admin(
                "PATCH",
                &format!("/sources/{}", self.source),
                Some(json!({ "allow_writes": allow })),
            )
            .await;
        assert_eq!(st, StatusCode::OK, "{v}");
        assert_eq!(v["allow_writes"], allow);
    }

    async fn statement(&self, body: Value) -> (StatusCode, Value) {
        self.admin(
            "POST",
            &format!("/sources/{}/statements", self.source),
            Some(body),
        )
        .await
    }

    /// An action of `action_type` on the order, for the customer.
    async fn action(&self, action_type: &str, name: &str, state: Value) -> u64 {
        let (st, v) = self
            .admin(
                "POST",
                "/actions",
                Some(json!({
                    "id": 0, "action_type": action_type, "name": name, "subject": self.order,
                    "object": self.customer, "parameters": { "state": state }
                })),
            )
            .await;
        assert_eq!(st, StatusCode::OK, "{v}");
        v["id"].as_u64().unwrap()
    }

    async fn audit(&self) -> Vec<Value> {
        let (st, v) = self.admin("GET", "/audit?limit=200", None).await;
        assert_eq!(st, StatusCode::OK, "{v}");
        v["entries"].as_array().unwrap().clone()
    }
}

fn entry<'a>(entries: &'a [Value], path: &str, part: &str) -> &'a Value {
    entries
        .iter()
        .find(|e| e["path"] == path && e["detail"].as_str().unwrap_or("").contains(part))
        .unwrap_or_else(|| panic!("no entry {path} with {part} in {entries:?}"))
}

#[tokio::test]
async fn write_back_is_off_until_a_source_opts_in_and_a_statement_is_checked() {
    let w = world("config").await;
    let close = json!({
        "name": "close an order", "action_type": "close",
        "sql": " UPDATE orders SET state = {state} WHERE code = {subject.code} ; "
    });
    let with = |k: &str, v: Value| {
        let mut b = close.clone();
        b[k] = v;
        b
    };

    for (body, why) in [
        (with("name", json!(" ")), "a name of 1 to 80 characters"),
        (with("name", json!("x".repeat(81))), "a name of 1 to 80"),
        (with("action_type", json!("ship")), "no action type named"),
        (with("sql", json!("select * from orders")), "one UPDATE"),
        (with("sql", json!("")), "one UPDATE"),
        (
            with("sql", json!("delete from a; delete from b")),
            "without `;`",
        ),
        (
            with(
                "sql",
                json!(format!("delete from a where x = '{}'", "y".repeat(4000))),
            ),
            "at most 4000",
        ),
        (
            with("sql", json!("delete from a where x = {state")),
            "never closed",
        ),
        (
            with("sql", json!("delete from a where x = state}")),
            "closes nothing",
        ),
        (with("max_rows", json!(0)), "between 1 and 10000"),
        (with("max_rows", json!(10_001)), "between 1 and 10000"),
    ] {
        let (st, v) = w.statement(body).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{why}: {v}");
        assert!(v["error"].as_str().unwrap().contains(why), "{why}: {v}");
    }
    let uri = format!("/sources/{}/statements", w.source);
    for token in [&w.user, &w.key, SERVICE] {
        let (st, _) = call(&w.app, "POST", &uri, Some(token), Some(close.clone())).await;
        // A key and the service token administer sources, as before; a plain account does not.
        let expected = if token == w.user {
            StatusCode::FORBIDDEN
        } else {
            StatusCode::CREATED
        };
        assert_eq!(st, expected);
    }
    let (st, _) = w
        .admin("POST", "/sources/99/statements", Some(close.clone()))
        .await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    let (st, v) = w.statement(close.clone()).await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    assert_eq!(
        v,
        json!({
            "id": 3, "name": "close an order", "description": "", "action_type": "close",
            "sql": "UPDATE orders SET state = {state} WHERE code = {subject.code}",
            "max_rows": 1, "enabled": true
        })
    );
    let one = format!("/sources/{}/statements/3", w.source);

    // A change that does not hold is not kept, in part or in whole.
    let (st, v) = w
        .admin(
            "PATCH",
            &one,
            Some(json!({ "description": "kept?", "sql": "drop table orders" })),
        )
        .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    let (_, v) = w
        .admin("GET", &format!("/sources/{}", w.source), None)
        .await;
    assert_eq!(v["statements"][2]["description"], "");
    assert!(v["statements"][2]["sql"]
        .as_str()
        .unwrap()
        .starts_with("UPDATE"));

    let (st, v) = w
        .admin(
            "PATCH",
            &one,
            Some(json!({ "description": "by its code", "max_rows": 2, "enabled": false })),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(
        (&v["description"], &v["max_rows"], &v["enabled"]),
        (&json!("by its code"), &json!(2), &json!(false))
    );
    let missing = format!("/sources/{}/statements/99", w.source);
    for (method, uri, expected) in [
        ("PATCH", missing.as_str(), StatusCode::NOT_FOUND),
        ("DELETE", missing.as_str(), StatusCode::NOT_FOUND),
        ("PATCH", "/sources/99/statements/1", StatusCode::NOT_FOUND),
        ("DELETE", "/sources/99/statements/1", StatusCode::NOT_FOUND),
        ("DELETE", one.as_str(), StatusCode::NO_CONTENT),
        ("DELETE", one.as_str(), StatusCode::NOT_FOUND),
    ] {
        let (st, v) = w.admin(method, uri, Some(json!({}))).await;
        assert_eq!(st, expected, "{method} {uri}: {v}");
    }
    for token in [&w.user] {
        for method in ["PATCH", "DELETE"] {
            let (st, _) = call(&w.app, method, &one, Some(token), Some(json!({}))).await;
            assert_eq!(st, StatusCode::FORBIDDEN, "{method}");
        }
    }

    // Two statements for `close` are left, both enabled, and the source
    // is still closed: nothing is bound.
    let action = w.action("close", "close order 7", json!("done")).await;
    let run = format!("/actions/{action}/run");
    let (st, v) = w.admin("POST", &run, None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"]
        .as_str()
        .unwrap()
        .contains("no source writes back actions of type close"));

    w.open(true).await;
    let (st, v) = w.admin("POST", &run, None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("disable all but one"));
    w.open(false).await;
    // Saving something else leaves no note about writes.
    let (st, _) = w
        .admin(
            "PATCH",
            &format!("/sources/{}", w.source),
            Some(json!({ "timeout_secs": 30 })),
        )
        .await;
    assert_eq!(st, StatusCode::OK);

    let entries = w.audit().await;
    let source = format!("/sources/{}", w.source);
    assert_eq!(
        entry(&entries, &source, "write-back allowed")["who"],
        "admin@example.com"
    );
    entry(&entries, &source, "write-back closed");
    assert_eq!(
        entries
            .iter()
            .filter(|e| e["path"] == source.as_str()
                && e["detail"].as_str().unwrap_or("").contains("write-back"))
            .count(),
        2
    );
    assert!(w.erp.executed().is_empty());
}

#[tokio::test]
async fn an_action_runs_only_once_another_administrator_approves_it() {
    let w = world("approve").await;
    w.open(true).await;
    let (st, v) = w
        .statement(json!({
            "name": "close an order", "action_type": "close",
            "sql": "update orders set state = {state} where code = {subject.code}"
        }))
        .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let action = w.action("close", "close order 7", json!("done")).await;
    let run = format!("/actions/{action}/run");

    // Asking is for a signed-in administrator account.
    for token in [&w.user, &w.key, SERVICE] {
        let (st, v) = call(&w.app, "POST", &run, Some(token), None).await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{v}");
    }
    let (st, _) = w.admin("POST", "/actions/999/run", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    let (st, v) = w.admin("POST", &run, None).await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    assert_eq!(v["status"], "pending");
    assert_eq!(v["sql"], CLOSE, "the blanks are filled when asked");
    assert_eq!(v["requested_by"], "admin@example.com");
    assert_eq!(v["max_rows"], 1);
    let id = v["id"].as_u64().unwrap();
    assert!(w.erp.executed().is_empty(), "asking runs nothing");

    let (st, v) = w.admin("POST", &run, None).await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert!(v["error"]
        .as_str()
        .unwrap()
        .contains("already awaits an approval"));

    let (st, v) = w.admin("GET", "/approvals", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["approvals"].as_array().unwrap().len(), 1);
    assert_eq!(v["approvals"][0]["action_name"], "close order 7");
    let (st, _) = call(&w.app, "GET", "/approvals", Some(&w.user), None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // Not the one who asked, not a key, not the service token, not a plain account.
    let approve = format!("/approvals/{id}/approve");
    let reject = format!("/approvals/{id}/reject");
    for token in [&w.admin, &w.user, &w.key, SERVICE] {
        for uri in [&approve, &reject] {
            if token == w.admin && uri == &reject {
                continue;
            }
            let (st, v) = call(&w.app, "POST", uri, Some(token), None).await;
            assert_eq!(st, StatusCode::FORBIDDEN, "{uri}: {v}");
        }
    }
    assert!(w.erp.executed().is_empty());
    let (st, _) = w.second("POST", "/approvals/99/approve").await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    let (st, v) = w.second("POST", &approve).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["status"], "done");
    assert_eq!(v["rows_affected"], 1);
    assert_eq!(v["decided_by"], "second@example.com");
    assert!(v["decided_at"].is_string());
    assert_eq!(w.erp.executed(), vec![CLOSE.to_string()]);

    // Decided is decided.
    for uri in [&approve, &reject] {
        let (st, v) = w.second("POST", uri).await;
        assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
        assert!(v["error"].as_str().unwrap().contains("is already done"));
    }
    assert_eq!(w.erp.executed().len(), 1, "once");

    let entries = w.audit().await;
    let asked = entry(&entries, &run, "write-back asked");
    assert_eq!(asked["who"], "admin@example.com");
    assert!(asked["detail"].as_str().unwrap().contains(CLOSE));
    let approved = entry(&entries, &approve, "write-back approved");
    assert_eq!(approved["who"], "second@example.com");
    let detail = approved["detail"].as_str().unwrap();
    assert!(
        detail.contains("asked by admin@example.com")
            && detail.contains(CLOSE)
            && detail.contains("1 row(s) changed"),
        "{detail}"
    );

    // On disk: a restart still knows it ran.
    let again = SourceStore::open(w.dir.join("sources.json")).unwrap();
    let v = serde_json::to_value(again.list()).unwrap();
    assert_eq!(v[0]["allow_writes"], true);
    let file = std::fs::read_to_string(w.dir.join("sources.json")).unwrap();
    assert!(file.contains("\"status\": \"done\""), "{file}");
}

#[tokio::test]
async fn a_write_past_its_guard_fails_a_rejection_runs_nothing_and_a_change_voids_what_was_asked() {
    let w = world("guard").await;
    w.open(true).await;
    let (st, v) = w
        .statement(json!({
            "name": "close a customer's orders", "action_type": "close_all", "max_rows": 2,
            "sql": "update orders set state = {state} where customer = {object.code}"
        }))
        .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let statement = format!("/sources/{}/statements/1", w.source);
    let action = w.action("close_all", "close for Acme", json!("done")).await;
    let run = format!("/actions/{action}/run");
    let ask = || async {
        let (st, v) = w.admin("POST", &run, None).await;
        assert_eq!(st, StatusCode::CREATED, "{v}");
        assert_eq!(v["sql"], CLOSE_ALL);
        v["id"].as_u64().unwrap()
    };

    // The engine would change 5 rows, the statement allows 2: rolled back.
    let id = ask().await;
    let (st, v) = w.second("POST", &format!("/approvals/{id}/approve")).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["status"], "failed");
    assert!(
        v["error"]
            .as_str()
            .unwrap()
            .contains("would change 5 rows, at most 2"),
        "{v}"
    );
    assert_eq!(v["rows_affected"], Value::Null);
    assert!(w.erp.executed().is_empty());

    // The one who asked may take it back.
    let id = ask().await;
    let reject = format!("/approvals/{id}/reject");
    let (st, v) = w.admin("POST", &reject, None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["status"], "rejected");
    assert_eq!(v["decided_by"], "admin@example.com");
    let (st, _) = w.second("POST", "/approvals/99/reject").await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // The statement changes, or the source closes, after the request:
    // what was asked no longer runs.
    let id = ask().await;
    let approve = format!("/approvals/{id}/approve");
    let (st, _) = w
        .admin("PATCH", &statement, Some(json!({ "max_rows": 10 })))
        .await;
    assert_eq!(st, StatusCode::OK);
    let (st, v) = w.second("POST", &approve).await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert!(v["error"].as_str().unwrap().contains("changed since"));
    // Bound to another action type: no longer the statement of this one.
    let (st, _) = w
        .admin(
            "PATCH",
            &statement,
            Some(json!({ "max_rows": 2, "action_type": "close" })),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    let (st, v) = w.second("POST", &approve).await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    let (st, _) = w
        .admin(
            "PATCH",
            &statement,
            Some(json!({ "action_type": "close_all" })),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    w.open(false).await;
    let (st, v) = w.second("POST", &approve).await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    let (_, v) = w.admin("GET", "/approvals", None).await;
    assert_eq!(v["approvals"][0]["status"], "pending", "still to reject");
    assert_eq!(v["approvals"][1]["status"], "rejected", "newest first");
    assert_eq!(v["approvals"][2]["status"], "failed");
    let (st, _) = w.second("POST", &format!("/approvals/{id}/reject")).await;
    assert_eq!(st, StatusCode::OK);
    w.open(true).await;

    // A value that is no plain literal is never written.
    let odd = w
        .action("close_all", "odd state", json!("done' or 1=1 --"))
        .await;
    let (st, v) = w.admin("POST", &format!("/actions/{odd}/run"), None).await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert!(v["error"].as_str().unwrap().contains("cannot be written"));

    // A source whose engine cannot be reached: failed, and said so.
    let (st, v) = w
        .admin(
            "POST",
            "/sources",
            Some(json!({ "name": "elsewhere", "kind": "memory", "allow_writes": true })),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    assert_eq!(v["allow_writes"], true);
    let elsewhere = v["id"].as_u64().unwrap();
    let (st, v) = w
        .admin(
            "POST",
            &format!("/sources/{elsewhere}/statements"),
            Some(json!({ "name": "close", "action_type": "close", "sql": "delete from orders where code = {subject.code}" })),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let lost = w.action("close", "close elsewhere", json!("done")).await;
    let (st, v) = w.admin("POST", &format!("/actions/{lost}/run"), None).await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    assert_eq!(v["source_name"], "elsewhere");
    let approve = format!("/approvals/{}/approve", v["id"]);
    let (st, v) = w.second("POST", &approve).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["status"], "failed");
    assert!(
        v["error"]
            .as_str()
            .unwrap()
            .contains("no memory connector registered"),
        "{v}"
    );

    assert!(w.erp.executed().is_empty(), "nothing was ever written");
    let entries = w.audit().await;
    entry(&entries, &reject, "write-back rejected");
    entry(
        &entries,
        "/sources",
        "elsewhere (memory), write-back allowed",
    );
    let failed = entry(&entries, &approve, "write-back approved");
    assert!(failed["detail"].as_str().unwrap().contains("failed:"));
}

#[tokio::test]
async fn without_accounts_nothing_is_written_back() {
    let graph = OntologyGraph::with_arc(ontology());
    let state = state_with(Arc::new(MemoryStore::new()), graph);
    let app = build_router_with_config(
        state,
        RouterConfig {
            bearer_token: None,
            jwt: None,
            rate_limit: None,
            users: None,
            web_dir: None,
            audit: None,
        },
    );
    for uri in [
        "/actions/1/run",
        "/approvals/1/approve",
        "/approvals/1/reject",
    ] {
        let (st, v) = call(&app, "POST", uri, None, None).await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{uri}: {v}");
    }
    // The list is the administrators' — everyone, without accounts — and empty.
    let (st, v) = call(&app, "GET", "/approvals", None, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v, json!({ "approvals": [] }));
}
