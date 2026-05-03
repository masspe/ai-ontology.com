use ontology_graph::{Concept, ConceptType, Ontology, OntologyGraph, Relation, RelationType};
use ontology_storage::{FileStore, LogRecord, Store};
use std::sync::Arc;

fn tempdir() -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "ontology-wal-del-{}-{}",
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
    o.add_relation_type(RelationType {
        name: "authored".into(),
        domain: "Person".into(),
        range: "Paper".into(),
        cardinality: Default::default(),
        symmetric: false,
        description: "".into(),
    })
    .unwrap();
    o
}

#[tokio::test]
async fn deletes_replay_through_wal() {
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

    let rel_id = g
        .add_relation(Relation::new(Default::default(), "authored", alice, paper))
        .unwrap();
    store
        .append(&LogRecord::relation(g.get_relation(rel_id).unwrap()))
        .await
        .unwrap();

    // Delete Alice and journal the cascade.
    let removed = g.remove_concept(alice).unwrap();
    store
        .append(&LogRecord::delete_concept(alice))
        .await
        .unwrap();
    for r in &removed {
        store.append(&LogRecord::delete_relation(*r)).await.unwrap();
    }

    // Reload from disk. Final state should have only the Paper.
    let g2 = OntologyGraph::with_arc(Ontology::new());
    let store2: Arc<dyn Store> = Arc::new(FileStore::open(&dir).await.unwrap());
    store2.load_into(&g2).await.unwrap();
    assert_eq!(g2.concept_count(), 1);
    assert_eq!(g2.relation_count(), 0);
    assert!(g2.find_by_name("Person", "Alice").is_none());
    assert!(g2.find_by_name("Paper", "P").is_some());

    let _ = std::fs::remove_dir_all(&dir);
}
