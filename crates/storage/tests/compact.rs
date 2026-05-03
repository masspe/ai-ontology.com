// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Verifies WAL compaction: snapshot + truncate, with watermark gating so
//! replay doesn't double-apply records that were captured by the snapshot.

use ontology_graph::{Concept, ConceptType, Ontology, OntologyGraph, Relation, RelationType};
use ontology_storage::{FileStore, LogRecord, Store};
use std::sync::Arc;

fn tempdir() -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "ontology-compact-{}-{}",
        std::process::id(),
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
        parent: None,
        properties: None,
        description: "".into(),
    });
    o.add_concept_type(ConceptType {
        name: "Paper".into(),
        parent: None,
        properties: None,
        description: "".into(),
    });
    o.add_concept_type(ConceptType {
        name: "Topic".into(),
        parent: None,
        properties: None,
        description: "".into(),
    });
    o.add_relation_type(RelationType {
        name: "authored".into(),
        domain: "Person".into(),
        range: "Paper".into(),
        cardinality: Default::default(),
        symmetric: false,
        description: "".into(),
    })
    .unwrap();
    o.add_relation_type(RelationType {
        name: "related_to".into(),
        domain: "Topic".into(),
        range: "Topic".into(),
        cardinality: Default::default(),
        symmetric: true,
        description: "".into(),
    })
    .unwrap();
    o
}

#[tokio::test]
async fn compact_preserves_symmetric_relation_count() {
    // Symmetric edges auto-materialize an inverse on insert; verify a
    // compact + reload doesn't double-materialize.
    let dir = tempdir();
    let store: Arc<dyn Store> = Arc::new(FileStore::open(&dir).await.unwrap());

    let g = OntologyGraph::with_arc(ontology());
    store
        .append(&LogRecord::ontology(g.ontology()))
        .await
        .unwrap();

    let t1 = g
        .upsert_concept(Concept::new(Default::default(), "Topic", "RAG"))
        .unwrap();
    let t2 = g
        .upsert_concept(Concept::new(Default::default(), "Topic", "ANN"))
        .unwrap();
    let _ = g
        .add_relation(Relation::new(Default::default(), "related_to", t1, t2))
        .unwrap();
    store
        .append(&LogRecord::concept(g.get_concept(t1).unwrap()))
        .await
        .unwrap();
    store
        .append(&LogRecord::concept(g.get_concept(t2).unwrap()))
        .await
        .unwrap();
    let pre_count = g.relation_count();
    assert_eq!(pre_count, 2, "symmetric edge should yield 2 relations");

    store.compact(&g).await.unwrap();

    let g2 = OntologyGraph::with_arc(Ontology::new());
    let store2: Arc<dyn Store> = Arc::new(FileStore::open(&dir).await.unwrap());
    store2.load_into(&g2).await.unwrap();
    assert_eq!(
        g2.relation_count(),
        pre_count,
        "relation_count drifted after compact + reload"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn compact_truncates_log_and_replays_clean() {
    let dir = tempdir();
    let store: Arc<dyn Store> = Arc::new(FileStore::open(&dir).await.unwrap());

    let g = OntologyGraph::with_arc(ontology());
    store
        .append(&LogRecord::ontology(g.ontology()))
        .await
        .unwrap();

    let alice = g
        .upsert_concept(Concept::new(Default::default(), "Person", "Alice"))
        .unwrap();
    let paper = g
        .upsert_concept(Concept::new(Default::default(), "Paper", "P"))
        .unwrap();
    let alice_c = g.get_concept(alice).unwrap();
    let paper_c = g.get_concept(paper).unwrap();
    store.append(&LogRecord::concept(alice_c)).await.unwrap();
    store.append(&LogRecord::concept(paper_c)).await.unwrap();
    let rid = g
        .add_relation(Relation::new(Default::default(), "authored", alice, paper))
        .unwrap();
    store
        .append(&LogRecord::relation(g.get_relation(rid).unwrap()))
        .await
        .unwrap();

    // Compact: snapshot the current state and truncate the WAL.
    store.compact(&g).await.unwrap();

    // The WAL file should now be empty.
    let log_path = dir.join("graph.log");
    assert_eq!(
        std::fs::metadata(&log_path).unwrap().len(),
        0,
        "WAL not truncated after compact"
    );

    // Append another record post-compaction.
    let bob = g
        .upsert_concept(Concept::new(Default::default(), "Person", "Bob"))
        .unwrap();
    store
        .append(&LogRecord::concept(g.get_concept(bob).unwrap()))
        .await
        .unwrap();

    // Reload from disk: snapshot restores Alice/Paper/relation, WAL replay
    // adds Bob, and the watermark prevents the snapshot's own records from
    // being re-applied (which would otherwise duplicate the relation).
    let g2 = OntologyGraph::with_arc(Ontology::new());
    let store2: Arc<dyn Store> = Arc::new(FileStore::open(&dir).await.unwrap());
    store2.load_into(&g2).await.unwrap();

    assert_eq!(g2.concept_count(), 3, "expected Alice + Paper + Bob");
    assert_eq!(
        g2.relation_count(),
        1,
        "relation should not be double-applied"
    );
    assert!(g2.find_by_name("Person", "Alice").is_some());
    assert!(g2.find_by_name("Person", "Bob").is_some());
    assert!(g2.find_by_name("Paper", "P").is_some());

    let _ = std::fs::remove_dir_all(&dir);
}
