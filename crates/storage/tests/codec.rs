// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Codec 1 (`postcard`) end to end: a store written in postcard hydrates to
//! the same graph as one written in JSON; a store switched from JSON to
//! postcard through `compact_with_codec` keeps every record and later
//! appends use the new codec; mixed stores read fine because every record
//! header carries its own codec (`STORAGE.md` §7.1, `STORAGE-PLAN.md` §6).

use ontology_graph::{
    Action, ActionId, ActionType, Concept, ConceptId, ConceptType, Ontology, OntologyGraph,
    PropertyValue, Relation, RelationId, RelationType, Rule, RuleId, RuleType,
};
use ontology_storage::{
    codec_name, parse_codec, LogRecord, RecordKind, SegmentStore, SegmentStoreConfig, Store,
    CODEC_JSON, CODEC_POSTCARD,
};
use proptest::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;

fn tempdir(tag: &str) -> PathBuf {
    // pid + counter + clock: the clock alone repeats on Windows (coarse
    // ticks) when two tests create their directory in the same instant.
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "ontology-codec-{tag}-{}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn ontology() -> Ontology {
    let mut o = Ontology::new();
    o.add_concept_type(ConceptType {
        name: "Person".into(),
        ns: Some("parties".into()),
        ..Default::default()
    });
    o.add_concept_type(ConceptType {
        name: "Invoice".into(),
        ns: Some("facturation".into()),
        ..Default::default()
    });
    o.add_relation_type(RelationType {
        name: "knows".into(),
        domain: "Person".into(),
        range: "Person".into(),
        symmetric: true,
        ..Default::default()
    })
    .unwrap();
    o.add_relation_type(RelationType {
        name: "issued_to".into(),
        domain: "Invoice".into(),
        range: "Person".into(),
        ..Default::default()
    })
    .unwrap();
    o.add_rule_type(RuleType {
        name: "must_pay".into(),
        when: "due".into(),
        then: "pay".into(),
        applies_to: vec!["Invoice".into()],
        strict: true,
        description: String::new(),
    })
    .unwrap();
    o.add_action_type(ActionType {
        name: "remind".into(),
        subject: "Person".into(),
        object: Some("Invoice".into()),
        parameters: vec!["channel".into()],
        effect: String::new(),
        description: String::new(),
    })
    .unwrap();
    o
}

fn with_codec(codec: u8) -> SegmentStoreConfig {
    SegmentStoreConfig {
        codec,
        ..Default::default()
    }
}

/// Fill a store through the graph API (R8 order) with every record kind.
async fn populate(graph: &Arc<OntologyGraph>, store: &SegmentStore) {
    store
        .append(&LogRecord::ontology(ontology()))
        .await
        .unwrap();
    let mut people = Vec::new();
    for i in 0..6 {
        let mut c = Concept::new(ConceptId(0), "Person", format!("p{i}"))
            .with_description(format!("person number {i} — é à ü"));
        c.properties
            .insert("age".into(), PropertyValue::Number(20.0 + i as f64));
        c.properties
            .insert("vip".into(), PropertyValue::Bool(i % 2 == 0));
        graph.prepare_concept(&mut c).unwrap();
        store.append(&LogRecord::concept(c.clone())).await.unwrap();
        people.push(graph.apply_prepared_concept(c).unwrap());
    }
    let mut inv = Concept::new(ConceptId(0), "Invoice", "INV-1");
    inv.properties.insert(
        "lines".into(),
        PropertyValue::List(vec![
            PropertyValue::Text("a".into()),
            PropertyValue::Number(2.5),
        ]),
    );
    graph.prepare_concept(&mut inv).unwrap();
    store
        .append(&LogRecord::concept(inv.clone()))
        .await
        .unwrap();
    let inv = graph.apply_prepared_concept(inv).unwrap();

    let mut r = Relation::new(RelationId(0), "knows", people[0], people[1]);
    r.weight = 0.5;
    graph.prepare_relation(&mut r).unwrap();
    store.append(&LogRecord::relation(r.clone())).await.unwrap();
    graph.apply_prepared_relation(r).unwrap();
    let mut r = Relation::new(RelationId(0), "issued_to", inv, people[2]);
    graph.prepare_relation(&mut r).unwrap();
    store.append(&LogRecord::relation(r.clone())).await.unwrap();
    graph.apply_prepared_relation(r).unwrap();

    let mut rule = Rule {
        id: RuleId(0),
        rule_type: "must_pay".into(),
        name: "pay INV-1".into(),
        when: "due".into(),
        then: "pay".into(),
        applies_to: vec![inv],
        strict: true,
        description: String::new(),
        properties: Default::default(),
    };
    graph.prepare_rule(&mut rule).unwrap();
    store.append(&LogRecord::rule(rule.clone())).await.unwrap();
    graph.upsert_rule(rule).unwrap();
    let mut action = Action {
        id: ActionId(0),
        action_type: "remind".into(),
        name: "remind p2".into(),
        subject: people[2],
        object: Some(inv),
        parameters: Default::default(),
        effect: String::new(),
        description: String::new(),
    };
    action
        .parameters
        .insert("channel".into(), PropertyValue::Text("email".into()));
    graph.prepare_action(&mut action).unwrap();
    store
        .append(&LogRecord::action(action.clone()))
        .await
        .unwrap();
    graph.upsert_action(action).unwrap();

    // A deletion too, so tombstones go through the codec.
    let gone = people[5];
    store
        .append(&LogRecord::delete_concept(gone, "Person".to_string()))
        .await
        .unwrap();
    graph.remove_concept(gone).unwrap();
}

/// Content comparison independent of codec: same counts, same concepts by
/// name with the same properties, same relations by (type, endpoints,
/// weight), same rules and actions.
fn assert_same_graph(a: &OntologyGraph, b: &OntologyGraph) {
    assert_eq!(a.concept_count(), b.concept_count());
    assert_eq!(a.relation_count(), b.relation_count());
    let mut ca = a.all_concepts();
    let mut cb = b.all_concepts();
    ca.sort_by_key(|c| c.id);
    cb.sort_by_key(|c| c.id);
    for (x, y) in ca.iter().zip(cb.iter()) {
        assert_eq!(x.id, y.id);
        assert_eq!(x.name, y.name);
        assert_eq!(x.concept_type, y.concept_type);
        assert_eq!(x.description, y.description);
        assert_eq!(
            serde_json::to_value(&x.properties).unwrap(),
            serde_json::to_value(&y.properties).unwrap(),
            "properties of {}",
            x.name
        );
    }
    let mut ra = a.all_relations();
    let mut rb = b.all_relations();
    ra.sort_by_key(|r| r.id);
    rb.sort_by_key(|r| r.id);
    for (x, y) in ra.iter().zip(rb.iter()) {
        assert_eq!(
            (x.id, &x.relation_type, x.source, x.target),
            (y.id, &y.relation_type, y.source, y.target)
        );
        assert_eq!(x.weight, y.weight);
    }
    assert_eq!(a.all_rules().len(), b.all_rules().len());
    assert_eq!(a.all_actions().len(), b.all_actions().len());
    assert_eq!(
        serde_json::to_value(a.ontology()).unwrap(),
        serde_json::to_value(b.ontology()).unwrap()
    );
}

/// A store written entirely in postcard hydrates to the same graph as the
/// same writes in JSON, and its manifest records codec 1.
#[tokio::test]
async fn postcard_store_hydrates_identically_to_json() {
    let mut graphs = Vec::new();
    for codec in [CODEC_JSON, CODEC_POSTCARD] {
        let dir = tempdir(codec_name(codec));
        let live = OntologyGraph::with_arc(ontology());
        {
            let store = SegmentStore::open_with(&dir, with_codec(codec))
                .await
                .unwrap();
            assert_eq!(store.codec(), codec);
            populate(&live, &store).await;
        }
        let store = SegmentStore::open(&dir).await.unwrap();
        assert_eq!(store.codec(), codec, "codec persisted in MANIFEST");
        let loaded = OntologyGraph::with_arc(Ontology::new());
        store.load_into(&loaded).await.unwrap();
        assert_same_graph(&live, &loaded);
        graphs.push(loaded);
    }
    assert_same_graph(&graphs[0], &graphs[1]);
}

/// Switching codec is a compaction: nothing lost, every later append in the
/// new codec, and the store reopens with the new codec.
#[tokio::test]
async fn compact_with_codec_switches_json_to_postcard_without_loss() {
    let dir = tempdir("switch");
    let live = OntologyGraph::with_arc(ontology());
    let store = SegmentStore::open_with(&dir, with_codec(CODEC_JSON))
        .await
        .unwrap();
    populate(&live, &store).await;
    let json_bytes: u64 = walk_bytes(&dir);

    let report = store
        .compact_with_codec(&live, CODEC_POSTCARD)
        .await
        .unwrap();
    assert_eq!(store.codec(), CODEC_POSTCARD);
    assert!(report.records_after <= report.records_before);
    assert!(
        report.bytes_after < json_bytes,
        "postcard is denser: {report:?} vs {json_bytes}"
    );

    // Later appends go through the new codec and still hydrate.
    let mut c = Concept::new(ConceptId(0), "Person", "late");
    live.prepare_concept(&mut c).unwrap();
    store.append(&LogRecord::concept(c.clone())).await.unwrap();
    live.apply_prepared_concept(c).unwrap();
    drop(store);

    let store = SegmentStore::open(&dir).await.unwrap();
    assert_eq!(store.codec(), CODEC_POSTCARD);
    let loaded = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&loaded).await.unwrap();
    assert_same_graph(&live, &loaded);

    // Every record on disk is now codec 1, including the late append.
    let (n, _) = store.scan_records(|_| {}).unwrap();
    assert!(n >= 10);
    for entry in std::fs::read_dir(dir.join("store").join("graph").join("parties"))
        .or_else(|_| std::fs::read_dir(dir.join("graph").join("parties")))
        .unwrap()
        .flatten()
    {
        let p = entry.path();
        if p.extension().is_some_and(|e| e == "data") {
            let bytes = std::fs::read(&p).unwrap();
            assert_eq!(
                bytes[6],
                CODEC_POSTCARD,
                "segment header codec of {}",
                p.display()
            );
        }
    }
}

/// Switching back to JSON works too (the codec byte is per record, the
/// manifest only says what to write next).
#[tokio::test]
async fn codec_round_trip_postcard_then_json() {
    let dir = tempdir("roundtrip");
    let live = OntologyGraph::with_arc(ontology());
    let store = SegmentStore::open_with(&dir, with_codec(CODEC_POSTCARD))
        .await
        .unwrap();
    populate(&live, &store).await;
    store.compact_with_codec(&live, CODEC_JSON).await.unwrap();
    assert_eq!(store.codec(), CODEC_JSON);
    let loaded = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&loaded).await.unwrap();
    assert_same_graph(&live, &loaded);
}

/// An unknown codec is refused before anything is touched.
#[tokio::test]
async fn unknown_codec_is_refused() {
    let dir = tempdir("unknown");
    let live = OntologyGraph::with_arc(ontology());
    let store = SegmentStore::open(&dir).await.unwrap();
    populate(&live, &store).await;
    let err = store.compact_with_codec(&live, 42).await.unwrap_err();
    assert!(err.to_string().contains("codec 42"), "{err}");
    assert_eq!(store.codec(), CODEC_JSON);
    let loaded = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&loaded).await.unwrap();
    assert_same_graph(&live, &loaded);
    assert_eq!(parse_codec("42"), None);
}

/// `scan_records` yields every record in global seq order, whatever the
/// codec, and counts payload bytes.
#[tokio::test]
async fn scan_records_is_seq_ordered_across_domains_and_codecs() {
    let dir = tempdir("scan");
    let live = OntologyGraph::with_arc(ontology());
    let store = SegmentStore::open_with(&dir, with_codec(CODEC_JSON))
        .await
        .unwrap();
    populate(&live, &store).await;
    // Mixed store: JSON sealed content, postcard for what follows.
    store
        .compact_with_codec(&live, CODEC_POSTCARD)
        .await
        .unwrap();
    let mut c = Concept::new(ConceptId(0), "Invoice", "INV-2");
    live.prepare_concept(&mut c).unwrap();
    store.append(&LogRecord::concept(c.clone())).await.unwrap();
    live.apply_prepared_concept(c).unwrap();

    let mut seqs = Vec::new();
    let mut kinds = Vec::new();
    let (n, bytes) = store
        .scan_records(|r| {
            seqs.push(r.seq);
            kinds.push(std::mem::discriminant(&r.kind));
        })
        .unwrap();
    assert_eq!(n as usize, seqs.len());
    assert!(bytes > 0);
    assert!(
        seqs.windows(2).all(|w| w[0] < w[1]),
        "strictly increasing: {seqs:?}"
    );
    assert!(
        kinds.contains(&std::mem::discriminant(&RecordKind::Ontology(
            Ontology::new()
        )))
    );
}

fn walk_bytes(dir: &std::path::Path) -> u64 {
    let mut total = 0;
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            total += walk_bytes(&p);
        } else if p.extension().is_some_and(|x| x == "data") {
            total += p.metadata().unwrap().len();
        }
    }
    total
}

/// The repository's finance schema has a type without `ns` (`Subsidiary`
/// inherits its parent's): a postcard store must journal it, reopen and
/// hydrate it — schema records stay JSON whatever the store codec.
#[tokio::test]
async fn postcard_store_accepts_a_schema_type_without_ns() {
    let dir = tempdir("finance-postcard");
    let finance: Ontology =
        serde_json::from_str(include_str!("../../../examples/finance/ontology.json")).unwrap();
    assert!(
        finance.concept_types["Subsidiary"].ns.is_none(),
        "fixture drifted"
    );
    let live = OntologyGraph::with_arc(finance.clone());
    {
        let store = SegmentStore::open_with(&dir, with_codec(CODEC_POSTCARD))
            .await
            .unwrap();
        store
            .append(&LogRecord::ontology(finance.clone()))
            .await
            .unwrap();
        let mut c = Concept::new(ConceptId(0), "Subsidiary", "Acme Labs SA");
        live.prepare_concept(&mut c).unwrap();
        store.append(&LogRecord::concept(c.clone())).await.unwrap();
        live.apply_prepared_concept(c).unwrap();
        let (n, _) = store.scan_records(|_| {}).unwrap();
        assert_eq!(n, 2);
        let meta_data = std::fs::read_dir(dir.join("meta"))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "data"))
            .unwrap();
        assert_eq!(
            std::fs::read(&meta_data).unwrap()[6],
            CODEC_JSON,
            "meta segment header stays JSON"
        );
        // A switch of codec (both ways) keeps working with such a schema.
        store.compact_with_codec(&live, CODEC_JSON).await.unwrap();
        store
            .compact_with_codec(&live, CODEC_POSTCARD)
            .await
            .unwrap();
    }
    let store = SegmentStore::open(&dir).await.unwrap();
    let loaded = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&loaded).await.unwrap();
    assert_same_graph(&live, &loaded);
    assert!(loaded.with_ontology(|o| o.concept_types["Subsidiary"].ns.is_none()));
}

/// A store that writes a non-JSON codec declares `format_version` 2, which a
/// JSON-only build refuses at open; this build refuses an unknown codec or
/// format version at open, before touching any segment (R17), and a JSON
/// config on reopen never overrides the manifest's codec.
#[tokio::test]
async fn format_version_follows_the_codec_and_unknown_codecs_are_refused_at_open() {
    let dir = tempdir("fv");
    let live = OntologyGraph::with_arc(ontology());
    {
        let store = SegmentStore::open(&dir).await.unwrap();
        populate(&live, &store).await;
        assert_eq!(store.manifest().format_version, 1);
        store
            .compact_with_codec(&live, CODEC_POSTCARD)
            .await
            .unwrap();
        assert_eq!(store.manifest().format_version, 2);
        store.compact_with_codec(&live, CODEC_JSON).await.unwrap();
        assert_eq!(store.manifest().format_version, 1);
        store
            .compact_with_codec(&live, CODEC_POSTCARD)
            .await
            .unwrap();
    }
    let manifest_path = dir.join("MANIFEST.json");
    let original = std::fs::read_to_string(&manifest_path).unwrap();
    let before = walk_bytes(&dir);

    // Unknown codec in the manifest (a store from a newer build).
    let tampered = original.replace("\"codec\": 1", "\"codec\": 7");
    assert_ne!(tampered, original);
    std::fs::write(&manifest_path, &tampered).unwrap();
    let err = SegmentStore::open(&dir).await.unwrap_err();
    assert!(err.to_string().contains("codec 7"), "{err}");
    assert_eq!(walk_bytes(&dir), before, "nothing truncated or created");

    // Unknown format version.
    let tampered = original.replace("\"format_version\": 2", "\"format_version\": 9");
    assert_ne!(tampered, original);
    std::fs::write(&manifest_path, &tampered).unwrap();
    let err = SegmentStore::open(&dir).await.unwrap_err();
    assert!(err.to_string().contains("format version 9"), "{err}");
    assert_eq!(walk_bytes(&dir), before);

    // Restored: opens, and a JSON config on a postcard store keeps postcard.
    std::fs::write(&manifest_path, &original).unwrap();
    let store = SegmentStore::open_with(&dir, with_codec(CODEC_JSON))
        .await
        .unwrap();
    assert_eq!(
        store.codec(),
        CODEC_POSTCARD,
        "the manifest's codec wins on reopen"
    );
    let mut c = Concept::new(ConceptId(0), "Person", "after");
    live.prepare_concept(&mut c).unwrap();
    store.append(&LogRecord::concept(c.clone())).await.unwrap();
    live.apply_prepared_concept(c).unwrap();
    drop(store);
    let store = SegmentStore::open(&dir).await.unwrap();
    let loaded = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&loaded).await.unwrap();
    assert_same_graph(&live, &loaded);
}

/// Every record header names the codec of its own payload: after a codec
/// switch, graph records say postcard and meta records say JSON, whatever
/// the segment they sit in.
#[tokio::test]
async fn record_headers_carry_their_own_codec() {
    let dir = tempdir("headers");
    let live = OntologyGraph::with_arc(ontology());
    let store = SegmentStore::open(&dir).await.unwrap();
    populate(&live, &store).await;
    store
        .compact_with_codec(&live, CODEC_POSTCARD)
        .await
        .unwrap();
    let mut c = Concept::new(ConceptId(0), "Person", "mixed");
    live.prepare_concept(&mut c).unwrap();
    store.append(&LogRecord::concept(c.clone())).await.unwrap();
    live.apply_prepared_concept(c).unwrap();
    drop(store);

    let mut graph_codecs = std::collections::BTreeSet::new();
    let mut meta_codecs = std::collections::BTreeSet::new();
    for (sub, set) in [("graph", &mut graph_codecs), ("meta", &mut meta_codecs)] {
        for entry in walkdir(&dir.join(sub)) {
            if entry.extension().is_some_and(|e| e == "data") {
                set.extend(record_codecs(&std::fs::read(&entry).unwrap()));
            }
        }
    }
    assert_eq!(graph_codecs, [CODEC_POSTCARD].into_iter().collect());
    assert_eq!(meta_codecs, [CODEC_JSON].into_iter().collect());
}

/// Codec byte of every record header in a `.data` file (format §4.2:
/// 32-byte file header, then records of a 32-byte header + payload padded
/// to 8 bytes; the header carries `payload_len` and the codec byte).
fn record_codecs(bytes: &[u8]) -> Vec<u8> {
    use ontology_storage::segment::{decode_record, FILE_HEADER_LEN};
    let mut out = Vec::new();
    let mut at = FILE_HEADER_LEN;
    while at < bytes.len() {
        let Ok(v) = decode_record(bytes, at, false) else {
            break;
        };
        out.push(v.header.codec);
        at = v.next;
    }
    out
}

fn walkdir(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walkdir(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}

// ---------------------------------------------------------------- property tests

fn arb_value() -> impl Strategy<Value = PropertyValue> {
    let leaf = prop_oneof![
        any::<f64>()
            .prop_filter("finite", |f| f.is_finite())
            .prop_map(PropertyValue::Number),
        any::<bool>().prop_map(PropertyValue::Bool),
        "[a-zA-Z0-9 éà]{0,12}".prop_map(PropertyValue::Text),
    ];
    leaf.prop_recursive(2, 6, 3, |inner| {
        prop::collection::vec(inner, 0..3).prop_map(PropertyValue::List)
    })
}

fn arb_props() -> impl Strategy<Value = ahash::AHashMap<String, PropertyValue>> {
    prop::collection::hash_map("[a-z_]{1,8}", arb_value(), 0..4)
        .prop_map(|m| m.into_iter().collect())
}

fn arb_concept() -> impl Strategy<Value = Concept> {
    (
        1u64..1_000_000,
        "[A-Z][a-z]{1,6}",
        "[^\\x00]{0,20}",
        "[^\\x00]{0,40}",
        arb_props(),
    )
        .prop_map(|(id, ty, name, desc, props)| {
            let mut c = Concept::new(ConceptId(id), ty, name).with_description(desc);
            c.properties = props;
            c
        })
}

fn arb_relation() -> impl Strategy<Value = Relation> {
    (
        1u64..1_000_000,
        "[a-z_]{1,8}",
        1u64..1_000_000,
        1u64..1_000_000,
        any::<f32>().prop_filter("finite", |f| f.is_finite()),
        arb_props(),
    )
        .prop_map(|(id, ty, s, t, w, props)| {
            let mut r = Relation::new(RelationId(id), ty, ConceptId(s), ConceptId(t));
            r.weight = w;
            r.properties = props;
            r
        })
}

fn arb_record() -> impl Strategy<Value = RecordKind> {
    prop_oneof![
        arb_concept().prop_map(RecordKind::Concept),
        arb_concept().prop_map(RecordKind::UpdateConcept),
        arb_relation().prop_map(RecordKind::Relation),
        arb_relation().prop_map(RecordKind::RelationExact),
        arb_relation().prop_map(RecordKind::UpdateRelation),
        (1u64..1_000_000).prop_map(|i| RecordKind::DeleteConcept(ConceptId(i))),
        (1u64..1_000_000).prop_map(|i| RecordKind::DeleteRelation(RelationId(i))),
        (1u64..1_000_000).prop_map(|i| RecordKind::DeleteRule(RuleId(i))),
        (1u64..1_000_000).prop_map(|i| RecordKind::DeleteAction(ActionId(i))),
        Just(RecordKind::Ontology(ontology())),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// For every record, decode(encode(x)) under the codec the store would
    /// use for it (postcard for graph records, JSON for schema records) is
    /// JSON-equivalent to x (the JSON view is the reference representation).
    #[test]
    fn store_codec_round_trip_equals_json_view(kind in arb_record()) {
        let c = ontology_storage::codec::codec_for(&kind, CODEC_POSTCARD);
        let bytes = ontology_storage::codec::encode(c, &kind).unwrap();
        let back = ontology_storage::codec::decode(c, &bytes).unwrap();
        prop_assert_eq!(
            serde_json::to_value(&back).unwrap(),
            serde_json::to_value(&kind).unwrap()
        );
        // And the JSON codec is its own round trip too.
        let j = ontology_storage::codec::encode(CODEC_JSON, &kind).unwrap();
        let jb = ontology_storage::codec::decode(CODEC_JSON, &j).unwrap();
        prop_assert_eq!(serde_json::to_value(&jb).unwrap(), serde_json::to_value(&kind).unwrap());
        // Meta-stream records (schema, rules, actions) never go through
        // postcard; graph records do.
        prop_assert_eq!(c == CODEC_JSON, ontology_storage::segment::Kind::of(&kind).is_meta());
    }

    /// Postcard bytes of the same record are identical (sorted properties).
    #[test]
    fn postcard_encoding_is_deterministic(kind in arb_record()) {
        let c = ontology_storage::codec::codec_for(&kind, CODEC_POSTCARD);
        let a = ontology_storage::codec::encode(c, &kind).unwrap();
        let b = ontology_storage::codec::encode(c, &kind).unwrap();
        prop_assert_eq!(a, b);
    }

    /// Truncated or garbage postcard bytes never panic.
    #[test]
    fn corrupt_postcard_bytes_fail_cleanly(kind in arb_record(), cut in 0usize..64, junk in prop::collection::vec(any::<u8>(), 0..32)) {
        let c = ontology_storage::codec::codec_for(&kind, CODEC_POSTCARD);
        let bytes = ontology_storage::codec::encode(c, &kind).unwrap();
        let cut = cut.min(bytes.len());
        let _ = ontology_storage::codec::decode(CODEC_POSTCARD, &bytes[..cut]);
        let _ = ontology_storage::codec::decode(CODEC_POSTCARD, &junk);
    }
}
