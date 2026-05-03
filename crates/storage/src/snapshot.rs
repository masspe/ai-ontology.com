use ontology_graph::{Concept, Ontology, OntologyGraph, Relation};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::store::StoreResult;

#[derive(Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub ontology: Ontology,
    pub concepts: Vec<Concept>,
    pub relations: Vec<Relation>,
    /// WAL sequence number at the time the snapshot was taken. WAL records
    /// with `seq <= high_water_seq` are skipped on replay so a compacted
    /// store doesn't double-apply.
    #[serde(default)]
    pub high_water_seq: u64,
}

impl Snapshot {
    pub fn from_graph(graph: &OntologyGraph) -> Self {
        Self::from_graph_with_seq(graph, 0)
    }

    pub fn from_graph_with_seq(graph: &OntologyGraph, high_water_seq: u64) -> Self {
        let concepts = graph.all_concepts();
        let ontology = graph.ontology();
        let mut relations = Vec::new();
        // Dedupe symmetric inverse edges: `add_relation` materializes an
        // inverse for every symmetric edge, so the graph holds two relations
        // for one logical link. On restore we re-run `add_relation`, which
        // would materialize *another* inverse — keep only the canonical
        // direction so the round-trip preserves relation_count().
        let mut seen_symmetric: ahash::AHashSet<(
            String,
            ontology_graph::ConceptId,
            ontology_graph::ConceptId,
        )> = ahash::AHashSet::new();
        for c in &concepts {
            for r in graph.outgoing(c.id) {
                let symmetric = ontology
                    .relation_types
                    .get(&r.relation_type)
                    .map(|rt| rt.symmetric)
                    .unwrap_or(false);
                if symmetric {
                    let (a, b) = if r.source <= r.target {
                        (r.source, r.target)
                    } else {
                        (r.target, r.source)
                    };
                    if !seen_symmetric.insert((r.relation_type.clone(), a, b)) {
                        continue;
                    }
                }
                relations.push(r);
            }
        }
        Self {
            ontology,
            concepts,
            relations,
            high_water_seq,
        }
    }

    pub fn restore(self, graph: &Arc<OntologyGraph>) -> StoreResult<()> {
        graph.extend_ontology(|target| {
            *target = self.ontology;
            Ok(())
        })?;
        for c in self.concepts {
            graph.upsert_concept(c)?;
        }
        for r in self.relations {
            graph.add_relation(r)?;
        }
        Ok(())
    }
}
