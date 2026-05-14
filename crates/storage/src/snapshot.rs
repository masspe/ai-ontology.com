// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

use ontology_graph::{Action, Concept, Ontology, OntologyGraph, Relation, Rule};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::store::StoreResult;

#[derive(Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub ontology: Ontology,
    pub concepts: Vec<Concept>,
    pub relations: Vec<Relation>,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub actions: Vec<Action>,
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
            rules: graph.all_rules(),
            actions: graph.all_actions(),
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
        for r in self.rules {
            graph.upsert_rule(r)?;
        }
        for a in self.actions {
            graph.upsert_action(a)?;
        }
        Ok(())
    }
}
