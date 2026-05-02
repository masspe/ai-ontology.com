//! Export the graph to JSONL, then re-ingest into a fresh graph and check
//! that concepts and relations match exactly.

use ontology_graph::{
    Concept, ConceptType, Ontology, OntologyGraph, Relation, RelationType,
};
use ontology_io::{export_graph, ingest_records, JsonlSink, JsonlSource};

fn tempdir() -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "ontology-export-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn ontology() -> Ontology {
    let mut o = Ontology::new();
    o.add_concept_type(ConceptType {
        name: "Person".into(), parent: None, properties: None, description: "".into(),
    });
    o.add_concept_type(ConceptType {
        name: "Paper".into(), parent: None, properties: None, description: "".into(),
    });
    o.add_relation_type(RelationType {
        name: "authored".into(), domain: "Person".into(), range: "Paper".into(),
        cardinality: Default::default(), symmetric: false, description: "".into(),
    }).unwrap();
    o
}

#[tokio::test]
async fn export_roundtrip_preserves_symmetric_count() {
    // Regression: the export → re-ingest cycle was dropping a symmetric
    // edge to id collision in add_relation when a re-ingested forward
    // happened to land on the id of an already-materialized inverse.
    let dir = tempdir();
    let path = dir.join("g.jsonl");

    let mut o = Ontology::new();
    o.add_concept_type(ConceptType {
        name: "Topic".into(), parent: None, properties: None, description: "".into(),
    });
    o.add_relation_type(RelationType {
        name: "related_to".into(), domain: "Topic".into(), range: "Topic".into(),
        cardinality: Default::default(), symmetric: true, description: "".into(),
    }).unwrap();

    let g1 = OntologyGraph::with_arc(o);
    let a = g1.upsert_concept(Concept::new(Default::default(), "Topic", "A")).unwrap();
    let b = g1.upsert_concept(Concept::new(Default::default(), "Topic", "B")).unwrap();
    let c = g1.upsert_concept(Concept::new(Default::default(), "Topic", "C")).unwrap();
    g1.add_relation(Relation::new(Default::default(), "related_to", a, b)).unwrap();
    g1.add_relation(Relation::new(Default::default(), "related_to", a, c)).unwrap();
    let before = g1.relation_count();
    assert_eq!(before, 4, "two symmetric edges + two materialized inverses");

    let mut sink = JsonlSink::create(&path).await.unwrap();
    ontology_io::export_graph(&g1, &mut sink).await.unwrap();

    let g2 = OntologyGraph::with_arc(Ontology::new());
    let mut src = JsonlSource::open(&path).await.unwrap();
    ontology_io::ingest_records(&mut src, &g2, None).await.unwrap();
    assert_eq!(g2.relation_count(), before,
        "round-trip lost or duplicated relations");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn export_then_ingest_roundtrips() {
    let dir = tempdir();
    let path = dir.join("graph.jsonl");

    let g1 = OntologyGraph::with_arc(ontology());
    let alice = g1.upsert_concept(Concept::new(Default::default(), "Person", "Alice")).unwrap();
    let paper = g1.upsert_concept(Concept::new(Default::default(), "Paper", "P")).unwrap();
    g1.add_relation(Relation::new(Default::default(), "authored", alice, paper)).unwrap();

    let mut sink = JsonlSink::create(&path).await.unwrap();
    let stats = export_graph(&g1, &mut sink).await.unwrap();
    assert_eq!(stats.concepts, 2);
    assert_eq!(stats.relations, 1);

    let g2 = OntologyGraph::with_arc(Ontology::new());
    let mut src = JsonlSource::open(&path).await.unwrap();
    let stats = ingest_records(&mut src, &g2, None).await.unwrap();
    assert_eq!(stats.concepts, 2);
    assert_eq!(stats.relations, 1);
    assert!(g2.find_by_name("Person", "Alice").is_some());
    assert!(g2.find_by_name("Paper", "P").is_some());
    assert_eq!(g2.relation_count(), 1);

    let _ = std::fs::remove_dir_all(&dir);
}
