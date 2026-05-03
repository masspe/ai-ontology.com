// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! End-to-end test: file-backed store + triples ingest + retrieval + RAG echo.
use ontology_graph::{Ontology, OntologyGraph};
use ontology_index::{HybridIndex, RetrievalRequest};
use ontology_io::{ingest_records, TripleSource};
use ontology_rag::{EchoModel, RagPipeline};
use ontology_storage::{FileStore, Store};
use std::sync::Arc;

#[tokio::test]
async fn ingest_persist_retrieve_answer() {
    let tmp = tempdir();
    let store: Arc<dyn Store> = Arc::new(FileStore::open(&tmp).await.unwrap());

    // Bootstrap the ontology (we own it directly here for the test).
    let mut ont = Ontology::new();
    ont.add_concept_type(ontology_graph::ConceptType {
        name: "Person".into(),
        parent: None,
        properties: None,
        description: "human".into(),
    });
    ont.add_concept_type(ontology_graph::ConceptType {
        name: "Paper".into(),
        parent: None,
        properties: None,
        description: "paper".into(),
    });
    ont.add_concept_type(ontology_graph::ConceptType {
        name: "Topic".into(),
        parent: None,
        properties: None,
        description: "topic".into(),
    });
    ont.add_relation_type(ontology_graph::RelationType {
        name: "authored".into(),
        domain: "Person".into(),
        range: "Paper".into(),
        cardinality: Default::default(),
        symmetric: false,
        description: "".into(),
    })
    .unwrap();
    ont.add_relation_type(ontology_graph::RelationType {
        name: "covers".into(),
        domain: "Paper".into(),
        range: "Topic".into(),
        cardinality: Default::default(),
        symmetric: false,
        description: "".into(),
    })
    .unwrap();

    let graph = OntologyGraph::with_arc(ont.clone());
    store
        .append(&ontology_storage::LogRecord::ontology(ont))
        .await
        .unwrap();

    let triple_path = tmp.join("seed.triples");
    tokio::fs::write(
        &triple_path,
        "Person:Alice authored Paper:RAG\nPaper:RAG covers Topic:RetrievalAugmentedGeneration\n",
    )
    .await
    .unwrap();

    let mut src = TripleSource::open(&triple_path).await.unwrap();
    let stats = ingest_records(&mut src, &graph, Some(store.as_ref()))
        .await
        .unwrap();
    assert_eq!(stats.concepts, 3);
    assert_eq!(stats.relations, 2);

    // Reload from disk into a fresh graph.
    let graph2 = OntologyGraph::with_arc(Ontology::new());
    let store2: Arc<dyn Store> = Arc::new(FileStore::open(&tmp).await.unwrap());
    store2.load_into(&graph2).await.unwrap();
    assert_eq!(graph2.concept_count(), 3);
    assert_eq!(graph2.relation_count(), 2);

    let idx = Arc::new(HybridIndex::with_default_embedder(graph2.clone()));
    idx.reindex_all();

    let pipe = RagPipeline::new(idx, Arc::new(EchoModel));
    let ans = pipe
        .answer_with(RetrievalRequest {
            query: "retrieval augmented generation".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(!ans.retrieved.is_empty());
    assert!(!ans.subgraph.concepts.is_empty());

    let _ = std::fs::remove_dir_all(&tmp);
}

fn tempdir() -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "ontology-e2e-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}
