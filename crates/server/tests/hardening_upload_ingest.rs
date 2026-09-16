// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! `POST /upload` (every `kind`, the file registry, error mapping and what
//! a refused ingest leaves behind) and the LLM review loop
//! (`/ingest/analyze`, `/ingest/apply`) driven by a scripted, offline LLM.

mod hardening_common;

use hardening_common::*;
use http::StatusCode;
use ontology_graph::{ConceptId, OntologyGraph};
use ontology_rag::{
    model::{LanguageModel, LlmRequest, LlmResponse, TokenUsage},
    LlmError,
};
use ontology_server::build_router;
use ontology_storage::{FlakyStore, RecordKind};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// /upload
// ---------------------------------------------------------------------------

const JSONL: &[u8] = br#"{"kind":"concept","id":0,"concept_type":"Topic","name":"A","description":"a","properties":{"k":"v"}}
{"kind":"concept","id":0,"concept_type":"Topic","name":"B"}
{"kind":"named_relation","relation_type":"related_to","source_type":"Topic","source_name":"A","target_type":"Topic","target_name":"B"}
"#;

/// JSONL: concepts are journaled as one batch, the relation on its own; the
/// file registry entry mirrors the ingest stats and is fully CRUD-able.
#[tokio::test]
async fn upload_jsonl_ingests_records_and_registers_the_file() {
    let (app, graph, store) = flaky_app();

    let (st, v) = upload(&app, "jsonl", None, "data.jsonl", JSONL).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(
        v["ingested"],
        json!({ "concepts": 2, "relations": 1, "ontology_updates": 0 })
    );
    let file_id = v["file_id"].as_u64().unwrap();
    assert_eq!(graph.concept_count(), 2);
    assert_eq!(graph.relation_count(), 2, "symmetric inverse materialized");
    assert_eq!(store.batch_calls(), 1, "the two concepts share one barrier");
    assert_eq!(store.append_calls(), 1, "the relation is journaled alone");
    assert_eq!(store.records_written(), 3);

    let (st, files) = get(&app, "/files").await;
    assert_eq!(st, StatusCode::OK);
    let list = files["files"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    let rec = &list[0];
    assert_eq!(rec["id"], file_id);
    assert_eq!(rec["name"], "data.jsonl");
    assert_eq!(rec["kind"], "jsonl");
    assert_eq!(rec["size"], JSONL.len());
    assert_eq!(rec["status"], "processed");
    assert_eq!(rec["concepts"], 2);
    assert_eq!(rec["relations"], 1);
    assert_eq!(rec["ontology_updates"], 0);
    assert!(rec["concept_type"].is_null());

    let (st, one) = get(&app, &format!("/files/{file_id}")).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(one["name"], "data.jsonl");
    let (st, _) = call(&app, "DELETE", &format!("/files/{file_id}"), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, _) = call(&app, "DELETE", &format!("/files/{file_id}"), None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _) = get(&app, &format!("/files/{file_id}")).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    // Deleting the registry entry does not un-ingest anything.
    assert_eq!(graph.concept_count(), 2);
}

/// Triples: each endpoint is created once even when it appears on several
/// lines; comments and blank lines are ignored.
#[tokio::test]
async fn upload_triples_creates_each_endpoint_once_and_every_relation() {
    let (app, graph, _store) = flaky_app();
    let triples =
        b"# toy graph\nTopic:A related_to Topic:B\n\nTopic:B related_to Topic:C  # trailing\n";

    let (st, v) = upload(&app, "triples", None, "g.triples", triples).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(
        v["ingested"],
        json!({ "concepts": 3, "relations": 2, "ontology_updates": 0 })
    );
    assert_eq!(graph.concept_count(), 3);
    assert_eq!(graph.relation_count(), 4);
    assert!(graph.find_by_name("Topic", "B").is_some());
}

/// CSV needs a `concept_type` field and a `name` header column; both
/// refusals are 400 and journal nothing. A valid file yields text
/// properties and a `description` column, and the registry keeps the type.
#[tokio::test]
async fn upload_csv_requires_concept_type_and_a_name_column() {
    let (app, graph, store) = flaky_app();

    let (st, v) = upload(&app, "csv", None, "a.csv", b"name,amount\nA,1\n").await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(error_text(&v).contains("csv requires concept_type"), "{v}");

    let (st, v) = upload(&app, "csv", Some("Topic"), "a.csv", b"title,amount\nA,1\n").await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(error_text(&v).contains("expected a `name` column"), "{v}");
    assert_eq!(store.records_written(), 0);
    assert_eq!(graph.concept_count(), 0);
    let (_, files) = get(&app, "/files").await;
    assert_eq!(
        files["files"],
        json!([]),
        "a refused upload is not registered"
    );

    let csv = b"name,amount,description\nA,10,first\n\"B, Inc\",20,second\n";
    let (st, v) = upload(&app, "csv", Some("Topic"), "ok.csv", csv).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["ingested"]["concepts"], 2);
    let (_, listed) = get(&app, "/concepts?type=Topic&q=inc").await;
    let b = &listed["concepts"][0];
    assert_eq!(b["name"], "B, Inc", "quoted field with embedded comma");
    assert_eq!(b["properties"]["amount"], "20", "CSV cells stay text");
    assert_eq!(b["description"], "second");
    let (_, files) = get(&app, "/files").await;
    assert_eq!(files["files"][0]["concept_type"], "Topic");
    assert_eq!(files["files"][0]["kind"], "csv");
}

/// A CSV refused on its third row leaves nothing behind: the ingester
/// stages rows in one batch, so a mid-file error before the barrier drops
/// them all (graph, store and registry untouched).
#[tokio::test]
async fn upload_csv_rejected_mid_file_leaves_nothing_behind() {
    let (app, graph, store) = flaky_app();
    let before = fingerprint(&graph);

    let (st, v) = upload(
        &app,
        "csv",
        Some("Topic"),
        "bad.csv",
        b"name,amount\nA,1\nB,2\nC,3,extra\n",
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(
        error_text(&v).contains("csv: row has 3 columns, expected 2"),
        "{v}"
    );
    assert_eq!(fingerprint(&graph), before);
    assert_eq!(store.records_written(), 0);
    assert_eq!(store.batch_calls(), 0);
    let (_, files) = get(&app, "/files").await;
    assert_eq!(files["files"], json!([]));
}

/// XLSX: the finance fixture imports one concept per row, numeric cells
/// typed as numbers; the `concept_type` field is mandatory.
#[tokio::test]
async fn upload_xlsx_ingests_the_finance_invoices_fixture() {
    let (app, graph, _store) = flaky_app();
    let xlsx = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/finance/invoices.xlsx"
    ))
    .unwrap();

    let (st, v) = upload(&app, "xlsx", None, "invoices.xlsx", &xlsx).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(error_text(&v).contains("xlsx requires concept_type"), "{v}");
    assert_eq!(graph.concept_count(), 0);

    let (st, v) = upload(&app, "xlsx", Some("Topic"), "invoices.xlsx", &xlsx).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let n = v["ingested"]["concepts"].as_u64().unwrap();
    assert!(n >= 1, "{v}");
    assert_eq!(graph.concept_count() as u64, n);
    assert_eq!(v["ingested"]["relations"], 0);
    let (_, listed) = get(&app, "/concepts?type=Topic&limit=1").await;
    let first = &listed["concepts"][0];
    assert!(
        first["properties"]["amount_eur"].is_number(),
        "numeric cell typed as number: {first}"
    );
    let (_, files) = get(&app, "/files").await;
    assert_eq!(files["files"][0]["concepts"], n);
    assert_eq!(files["files"][0]["concept_type"], "Topic");
}

/// `kind=text`: one concept per document named after the file stem (or the
/// `name` field), body as description; a long body is fragmented into
/// `TopicFragment` concepts linked by `fragment_of_topic`, one relation per
/// fragment.
#[tokio::test]
async fn upload_text_makes_one_concept_per_document_and_fragments_long_bodies() {
    let (app, graph, _store) = flaky_app();

    let (st, v) = upload(&app, "text", None, "memo.txt", b"Hello").await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(error_text(&v).contains("text requires concept_type"), "{v}");

    let body = "Hello world.\nSecond line.";
    let (st, v) = upload(&app, "text", Some("Topic"), "memo.txt", body.as_bytes()).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["ingested"]["concepts"], 1);
    assert_eq!(v["ingested"]["relations"], 0);
    let id = graph
        .find_by_name("Topic", "memo")
        .expect("named after the stem");
    assert_eq!(graph.get_concept(id).unwrap().description, body);

    let (st, v) = post_multipart(
        &app,
        "/upload",
        &[
            ("kind", "text"),
            ("concept_type", "Topic"),
            ("name", "Custom"),
        ],
        Some(("ignored.txt", b"Body")),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(graph.find_by_name("Topic", "Custom").is_some());
    assert!(graph.find_by_name("Topic", "ignored").is_none());

    // 6 paragraphs of 1 000 chars: above the 4 000-char default chunk size.
    let paragraph = "word ".repeat(200);
    let long = std::iter::repeat_n(paragraph.trim_end(), 6)
        .collect::<Vec<_>>()
        .join("\n\n");
    let (st, v) = upload(&app, "text", Some("Topic"), "long.txt", long.as_bytes()).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let doc = graph
        .get_concept(graph.find_by_name("Topic", "long").unwrap())
        .unwrap();
    let fragments = match doc.properties.get("fragments") {
        Some(ontology_graph::PropertyValue::Number(n)) => *n as u64,
        other => panic!("fragments property missing or not a number: {other:?}"),
    };
    assert!(fragments >= 2, "{doc:?}");
    assert_eq!(v["ingested"]["concepts"], 1 + fragments);
    assert_eq!(v["ingested"]["relations"], fragments);
    assert!(v["ingested"]["ontology_updates"].as_u64().unwrap() >= 1);
    assert!(graph.with_ontology(|o| {
        o.concept_types.contains_key("TopicFragment")
            && o.relation_types.contains_key("fragment_of_topic")
    }));
    assert!(
        doc.description.chars().count() < long.chars().count(),
        "excerpt only"
    );
}

/// An unknown `kind`, a missing `kind` or a missing `file` are 400 before
/// anything is read into the graph or the registry.
#[tokio::test]
async fn upload_unknown_kind_or_missing_parts_is_400() {
    let (app, graph, store) = flaky_app();
    let before = fingerprint(&graph);

    let (st, v) = upload(&app, "pdf", None, "x.pdf", b"%PDF-1.4").await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(error_text(&v).contains("unknown kind: pdf"), "{v}");

    let (st, v) = post_multipart(&app, "/upload", &[], Some(("x.jsonl", JSONL))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(error_text(&v).contains("missing `kind`"), "{v}");

    let (st, v) = post_multipart(&app, "/upload", &[("kind", "jsonl")], None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(error_text(&v).contains("missing `file`"), "{v}");

    assert_eq!(fingerprint(&graph), before);
    assert_eq!(store.append_calls() + store.batch_calls(), 0);
    let (_, files) = get(&app, "/files").await;
    assert_eq!(files["files"], json!([]));
}

/// R8 through the ingester: a store failure is a 500, the graph and the
/// registry are untouched, and the same upload succeeds once the store is
/// back.
#[tokio::test]
async fn upload_with_a_failing_store_is_500_and_leaves_graph_and_registry_untouched() {
    let (app, graph, store) = flaky_app();
    let before = fingerprint(&graph);
    store.set_failing(true);

    let (st, v) = upload(&app, "jsonl", None, "data.jsonl", JSONL).await;
    assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR, "{v}");
    assert!(error_text(&v).starts_with("store:"), "{v}");
    assert_eq!(fingerprint(&graph), before);
    let (_, files) = get(&app, "/files").await;
    assert_eq!(files["files"], json!([]));

    store.set_failing(false);
    let (st, v) = upload(&app, "jsonl", None, "data.jsonl", JSONL).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(graph.concept_count(), 2);
}

/// `kind=ontology` goes through the same guards as `PUT /ontology`: a
/// valid schema is journaled once, a guarded one is 400, malformed JSON is
/// 400.
#[tokio::test]
async fn upload_ontology_kind_replaces_the_schema_after_the_guards() {
    let (app, graph, store) = flaky_app();
    let mut o = ontology();
    o.add_concept_type(ontology_graph::ConceptType {
        name: "Extra".into(),
        ..Default::default()
    });
    let bytes = serde_json::to_vec(&o).unwrap();

    let (st, v) = upload(&app, "ontology", None, "schema.json", &bytes).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(
        v["ingested"],
        json!({ "concepts": 0, "relations": 0, "ontology_updates": 1 })
    );
    assert_eq!(store.records_written(), 1);
    assert!(matches!(store.records()[0].kind, RecordKind::Ontology(_)));
    assert!(graph.with_ontology(|o| o.concept_types.contains_key("Extra")));

    create_topic(&app, "A").await;
    let mut dropped = ontology();
    dropped.concept_types.remove("Topic");
    let written = store.records_written();
    let (st, v) = upload(
        &app,
        "ontology",
        None,
        "schema.json",
        &serde_json::to_vec(&dropped).unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(error_text(&v).contains("still has 1 instance(s)"), "{v}");

    let (st, v) = upload(&app, "ontology", None, "schema.json", b"{ not json").await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(error_text(&v).contains("ontology:"), "{v}");
    assert_eq!(store.records_written(), written);
    assert!(graph.with_ontology(|o| o.concept_types.contains_key("Extra")));
}

// ---------------------------------------------------------------------------
// /ingest/analyze + /ingest/apply
// ---------------------------------------------------------------------------

/// Offline LLM: returns a scripted JSON and counts its calls.
struct ScriptedLlm {
    calls: AtomicUsize,
    reply: Mutex<String>,
}

impl ScriptedLlm {
    fn new(reply: &str) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            reply: Mutex::new(reply.to_string()),
        })
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl LanguageModel for ScriptedLlm {
    async fn generate(&self, _req: &LlmRequest) -> Result<LlmResponse, LlmError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(LlmResponse {
            content: self.reply.lock().unwrap().clone(),
            model: "scripted".into(),
            stop_reason: Some("end_turn".into()),
            usage: TokenUsage::default(),
        })
    }
}

const ONE_CONCEPT: &str = r#"{"concept_types":[],"relation_types":[],
  "concepts":[{"client_ref":"c0","concept_type":"Topic","name":"Acme","description":"x","confidence":0.9}],
  "relations":[],"rules":[],"actions":[]}"#;

fn review_app(llm: Arc<ScriptedLlm>) -> (axum::Router, Arc<OntologyGraph>, Arc<FlakyStore>) {
    let store = Arc::new(FlakyStore::new());
    let graph = OntologyGraph::with_arc(ontology());
    let app = build_router(state_with_llm(store.clone(), graph.clone(), llm));
    (app, graph, store)
}

fn apply_body(concepts: Value, decisions: Value, strict: bool) -> Value {
    json!({
        "proposal": { "concepts": concepts },
        "decisions": decisions,
        "strict": strict
    })
}

/// A `.csv` is analyzed as text (one LLM call); an empty or blank file is a
/// 422 that never reaches the LLM; a form without `file` is a 400.
#[tokio::test]
async fn analyze_passes_a_csv_through_and_refuses_an_empty_file_before_calling_the_llm() {
    let llm = ScriptedLlm::new(ONE_CONCEPT);
    let (app, graph, store) = review_app(llm.clone());

    let (st, v) = post_multipart(
        &app,
        "/ingest/analyze",
        &[],
        Some(("x.csv", b"name,amount\nAcme,1\n")),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["source"]["name"], "x.csv");
    assert_eq!(v["concepts"].as_array().unwrap().len(), 1);
    assert_eq!(llm.calls(), 1);

    for (label, bytes) in [("empty", &b""[..]), ("blank", &b"  \n\t\r\n"[..])] {
        let (st, v) = post_multipart(&app, "/ingest/analyze", &[], Some(("e.txt", bytes))).await;
        assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{label}: {v}");
        assert!(error_text(&v).contains("empty"), "{label}: {v}");
    }
    assert_eq!(llm.calls(), 1, "no LLM call for an empty upload");

    let (st, v) = post_multipart(&app, "/ingest/analyze", &[("provider", "default")], None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(error_text(&v).contains("missing `file`"), "{v}");

    // Analyze writes nothing.
    assert_eq!(graph.concept_count(), 0);
    assert_eq!(store.records_written(), 0);
}

/// `strict: true` stops at the first failing item: it is reported, the
/// items after it are not even listed, and nothing was written. Without
/// `strict` the same proposal reports the failure and lands the rest.
#[tokio::test]
async fn apply_strict_stops_at_the_first_failure_and_reports_it() {
    let (app, graph, store) = review_app(ScriptedLlm::new(ONE_CONCEPT));
    let concepts = json!([
        { "client_ref": "c0", "concept_type": "Nope", "name": "X" },
        { "client_ref": "c1", "concept_type": "Topic", "name": "B" }
    ]);
    let decisions = json!([
        { "client_ref": "c0", "action": "create_new" },
        { "client_ref": "c1", "action": "create_new" }
    ]);

    let (st, report) = call(
        &app,
        "POST",
        "/ingest/apply",
        Some(apply_body(concepts.clone(), decisions.clone(), true)),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{report}");
    assert_eq!(report["failed"], 1);
    assert_eq!(report["created"], 0);
    let outcomes = report["concepts"].as_array().unwrap();
    assert_eq!(outcomes.len(), 1, "c1 never processed: {report}");
    assert_eq!(outcomes[0][0], "c0");
    assert_eq!(outcomes[0][1]["status"], "failed");
    assert!(
        outcomes[0][1]["error"].as_str().unwrap().contains("Nope"),
        "{report}"
    );
    assert_eq!(graph.concept_count(), 0);
    assert_eq!(store.records_written(), 0);

    let (st, report) = call(
        &app,
        "POST",
        "/ingest/apply",
        Some(apply_body(concepts, decisions, false)),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{report}");
    assert_eq!(report["failed"], 1);
    assert_eq!(report["created"], 1);
    assert_eq!(report["concepts"].as_array().unwrap().len(), 2);
    assert_eq!(report["concepts"][1][1]["status"], "created");
    assert!(graph.find_by_name("Topic", "B").is_some());
    assert_eq!(store.records_written(), 1);
}

/// `merge` on an existing `(type, name)` patches it: properties are typed
/// with `from_loose_text` and replace the map wholesale, an empty proposal
/// description keeps the current one, and the write is one `UpdateConcept`
/// record. `merge` on a name that does not exist behaves as `create_new`.
#[tokio::test]
async fn apply_merge_patches_an_existing_concept_with_typed_properties() {
    let (app, graph, store) = review_app(ScriptedLlm::new(ONE_CONCEPT));
    let (st, v) = call(
        &app,
        "POST",
        "/concepts",
        Some(json!({ "id": 0, "concept_type": "Topic", "name": "Acme",
                     "description": "old", "properties": { "keep": "x" } })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let id = v["id"].as_u64().unwrap();
    let records = store.records().len();

    let concepts = json!([{
        "client_ref": "c0", "concept_type": "Topic", "name": "Acme", "description": "",
        "properties": [["amount", "18000"], ["active", "true"], ["code", "007"]]
    }]);
    let decisions = json!([{ "client_ref": "c0", "action": "merge" }]);
    let (st, report) = call(
        &app,
        "POST",
        "/ingest/apply",
        Some(apply_body(concepts, decisions, true)),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{report}");
    assert_eq!(report["merged"], 1);
    assert_eq!(report["created"], 0);
    assert_eq!(report["failed"], 0);
    assert_eq!(
        report["concepts"][0][1],
        json!({ "status": "merged", "id": id.to_string() })
    );

    let c = graph.get_concept(ConceptId(id)).unwrap();
    assert_eq!(
        c.description, "old",
        "empty proposal description keeps the current one"
    );
    let props = serde_json::to_value(&c.properties).unwrap();
    assert_eq!(props["amount"], json!(18000.0));
    assert_eq!(props["active"], json!(true));
    assert_eq!(props["code"], "007", "leading zero keeps it text");
    assert!(
        props.get("keep").is_none(),
        "properties replaced wholesale: {props}"
    );
    let tail = &store.records()[records..];
    assert_eq!(tail.len(), 1);
    assert!(matches!(tail[0].kind, RecordKind::UpdateConcept(_)));
    assert_eq!(graph.concept_count(), 1);

    let (st, report) = call(
        &app,
        "POST",
        "/ingest/apply",
        Some(apply_body(
            json!([{ "client_ref": "n0", "concept_type": "Topic", "name": "Newco" }]),
            json!([{ "client_ref": "n0", "action": "merge" }]),
            true,
        )),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{report}");
    assert_eq!(
        report["created"], 1,
        "merge with no match creates: {report}"
    );
    assert_eq!(report["merged"], 0);
    assert!(graph.find_by_name("Topic", "Newco").is_some());
}

/// Items without a decision follow `default_action`, which defaults to
/// `skip`: nothing the user did not review is written.
#[tokio::test]
async fn apply_default_action_skip_leaves_unreviewed_items_alone() {
    let (app, graph, _store) = review_app(ScriptedLlm::new(ONE_CONCEPT));
    let concepts = json!([
        { "client_ref": "c0", "concept_type": "Topic", "name": "A" },
        { "client_ref": "c1", "concept_type": "Topic", "name": "B" }
    ]);
    let decisions = json!([{ "client_ref": "c0", "action": "create_new" }]);

    let (st, report) = call(
        &app,
        "POST",
        "/ingest/apply",
        Some(apply_body(concepts.clone(), decisions.clone(), false)),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{report}");
    assert_eq!(report["created"], 1);
    assert_eq!(report["skipped"], 1);
    assert_eq!(
        report["concepts"][1],
        json!(["c1", { "status": "skipped" }])
    );
    assert!(graph.find_by_name("Topic", "B").is_none());

    let mut body = apply_body(json!([concepts[1].clone()]), json!([]), false);
    body["default_action"] = json!("create_new");
    let (st, report) = call(&app, "POST", "/ingest/apply", Some(body)).await;
    assert_eq!(st, StatusCode::OK, "{report}");
    assert_eq!(report["created"], 1);
    assert!(graph.find_by_name("Topic", "B").is_some());
}

/// Pins the reporting contract: a store failure on an *instance* is a
/// per-item `failed` outcome inside a 200 report (the graph stays clean,
/// R8), not an error status — only a failed schema snapshot is a 500
/// (covered in `write_ahead.rs`).
#[tokio::test]
async fn apply_reports_an_instance_store_failure_per_item_and_keeps_the_graph_clean() {
    let (app, graph, store) = review_app(ScriptedLlm::new(ONE_CONCEPT));
    let before = fingerprint(&graph);
    store.set_failing(true);

    let (st, report) = call(
        &app,
        "POST",
        "/ingest/apply",
        Some(apply_body(
            json!([{ "client_ref": "c0", "concept_type": "Topic", "name": "Z" }]),
            json!([{ "client_ref": "c0", "action": "create_new" }]),
            true,
        )),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{report}");
    assert_eq!(report["failed"], 1);
    assert_eq!(report["created"], 0);
    assert_eq!(report["concepts"][0][1]["status"], "failed");
    assert!(
        report["concepts"][0][1]["error"]
            .as_str()
            .unwrap()
            .contains("FlakyStore"),
        "{report}"
    );
    assert_eq!(fingerprint(&graph), before);
    assert_eq!(store.records_written(), 0);
}
