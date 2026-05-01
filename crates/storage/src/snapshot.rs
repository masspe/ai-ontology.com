use ontology_graph::{Concept, Ontology, OntologyGraph, Relation};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::store::StoreResult;

#[derive(Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub ontology: Ontology,
    pub concepts: Vec<Concept>,
    pub relations: Vec<Relation>,
}

impl Snapshot {
    pub fn from_graph(graph: &OntologyGraph) -> Self {
        let concepts = graph.all_concepts();
        let mut relations = Vec::new();
        for c in &concepts {
            relations.extend(graph.outgoing(c.id));
        }
        Self {
            ontology: graph.ontology(),
            concepts,
            relations,
        }
    }

    pub fn restore(self, graph: &Arc<OntologyGraph>) -> StoreResult<()> {
        graph.extend_ontology(|target| { *target = self.ontology; Ok(()) })?;
        for c in self.concepts { graph.upsert_concept(c)?; }
        for r in self.relations { graph.add_relation(r)?; }
        Ok(())
    }
}
