// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

use ontology_graph::{ConceptType, Ontology, OntologyGraph};
use ontology_io::{ingest_records, CsvSource};
use std::sync::Arc;

#[tokio::test]
async fn csv_ingests_concepts_with_properties() {
    let dir = std::env::temp_dir().join(format!(
        "ontology-csv-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("people.csv");

    tokio::fs::write(
        &path,
        "name,affiliation,description\n\
         Alice,Acme Labs,\"Quoted, with comma\"\n\
         Bob,Globex Corp,\n",
    )
    .await
    .unwrap();

    let mut ont = Ontology::new();
    ont.add_concept_type(ConceptType {
        name: "Person".into(),
        parent: None,
        properties: None,
        description: "human".into(),
    });
    let graph = OntologyGraph::with_arc(ont);

    let mut src = CsvSource::open(&path, "Person").await.unwrap();
    let stats = ingest_records(&mut src, &graph, None).await.unwrap();
    assert_eq!(stats.concepts, 2);

    let alice = graph.find_by_name("Person", "Alice").unwrap();
    let alice = graph.get_concept(alice).unwrap();
    assert_eq!(alice.description, "Quoted, with comma");
    assert!(alice.properties.contains_key("affiliation"));

    let _ = std::fs::remove_dir_all(&dir);
    drop(Arc::clone(&graph)); // touch Arc to silence unused-import lints
}
