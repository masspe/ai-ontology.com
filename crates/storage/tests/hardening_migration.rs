// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Legacy `graph.log` migration (STORAGE-PLAN.md §4.3): a record kind this
//! build no longer knows (`Clear`, removed by D3) stops the migration
//! before anything is created; legacy tombstones are routed by replaying
//! the log, and a tombstone of an entity that never existed is dropped.

mod hardening_common;

use hardening_common::*;
use ontology_graph::{Concept, ConceptId, Ontology, Relation, RelationId};
use ontology_storage::log::RouteHint;
use ontology_storage::migrate::{route_legacy, LEGACY_LOG};
use ontology_storage::{legacy_present, migrate_legacy, staging_dir_for, LogRecord, RecordKind};

fn person_ontology() -> Ontology {
    let mut o = Ontology::new();
    o.add_concept_type(ct("Person", None, None));
    o.add_relation_type(rt("knows", "Person", "Person", false))
        .unwrap();
    o
}

/// `[u32 BE len][JSON]` frames, exactly as `FileStore` wrote them.
fn legacy_frames(records: &[serde_json::Value]) -> Vec<u8> {
    let mut out = Vec::new();
    for r in records {
        let bytes = serde_json::to_vec(r).unwrap();
        out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        out.extend_from_slice(&bytes);
    }
    out
}

fn record_json(seq: u64, kind: &RecordKind) -> serde_json::Value {
    serde_json::json!({ "seq": seq, "kind": serde_json::to_value(kind).unwrap() })
}

/// A `Clear` record (a unit variant this build does not have) in the
/// middle of a legacy log is corruption from the reader's point of view:
/// the migration fails with `Decode`, names the offset, and leaves no
/// `store/`, no staging directory and the legacy files byte-identical.
#[tokio::test]
async fn a_legacy_log_with_an_unknown_record_kind_fails_without_side_effects() {
    let data = tempdir("legacy-clear");
    let frames = vec![
        record_json(1, &RecordKind::Ontology(person_ontology())),
        record_json(
            2,
            &RecordKind::Concept(Concept::new(ConceptId(1), "Person", "a")),
        ),
        serde_json::json!({ "seq": 3, "kind": "Clear" }),
        record_json(
            4,
            &RecordKind::Concept(Concept::new(ConceptId(2), "Person", "b")),
        ),
    ];
    let bytes = legacy_frames(&frames);
    std::fs::write(data.join(LEGACY_LOG), &bytes).unwrap();
    assert!(legacy_present(&data));
    let store_dir = data.join("store");

    let err = migrate_legacy(&data, &store_dir).await.unwrap_err();
    assert!(
        matches!(err, ontology_storage::StoreError::Decode(_)),
        "{err}"
    );
    let msg = err.to_string();
    assert!(msg.contains("not the last one"), "{msg}");
    // Offset of the third frame: 4 + len1 + 4 + len2.
    let off = frames[..2]
        .iter()
        .map(|r| 4 + serde_json::to_vec(r).unwrap().len())
        .sum::<usize>();
    assert!(msg.contains(&format!("offset {off}")), "{msg}");
    assert!(!store_dir.exists(), "no store created");
    assert!(!staging_dir_for(&store_dir).exists(), "no staging left");
    assert!(legacy_present(&data), "legacy files not renamed");
    assert_eq!(std::fs::read(data.join(LEGACY_LOG)).unwrap(), bytes);
    std::fs::remove_dir_all(&data).ok();
}

/// `route_legacy` replays the stream and attaches to every tombstone the
/// type of the entity it deletes (D3: tombstones land in their entity's
/// domain); a tombstone whose entity does not exist at that point was a
/// no-op in the legacy store and is dropped. Other records pass unchanged.
#[test]
fn route_legacy_attaches_hints_to_tombstones_and_drops_no_op_ones() {
    let onto = person_ontology();
    let (a, b) = (ConceptId(1), ConceptId(2));
    let rel = Relation::new(RelationId(7), "knows", a, b);
    let records = vec![
        LogRecord::ontology(onto),
        LogRecord::concept(Concept::new(a, "Person", "a")),
        LogRecord::concept(Concept::new(b, "Person", "b")),
        LogRecord::relation(rel),
        LogRecord::delete_relation_unrouted(RelationId(99)), // never existed
        LogRecord::delete_relation_unrouted(RelationId(7)),
        LogRecord::delete_concept_unrouted(ConceptId(42)), // never existed
        LogRecord::delete_concept_unrouted(a),
        LogRecord::delete_concept_unrouted(a), // already gone: no-op
    ];
    let out = route_legacy(records).unwrap();
    assert_eq!(out.len(), 6, "three no-op tombstones dropped");
    assert!(matches!(out[0].kind, RecordKind::Ontology(_)));
    assert!(out[0].route.is_none());
    assert!(out[1].route.is_none() && out[2].route.is_none() && out[3].route.is_none());
    assert!(matches!(
        out[4].kind,
        RecordKind::DeleteRelation(RelationId(7))
    ));
    assert_eq!(out[4].route, Some(RouteHint::RelationType("knows".into())));
    assert!(matches!(
        out[5].kind,
        RecordKind::DeleteConcept(ConceptId(1))
    ));
    assert_eq!(out[5].route, Some(RouteHint::ConceptType("Person".into())));
}
