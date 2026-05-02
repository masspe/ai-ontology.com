use ahash::{AHashMap, AHashSet};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

use crate::graph::OntologyGraph;
use crate::id::{ConceptId, RelationId};
use crate::model::{Concept, Relation};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Outgoing,
    Incoming,
    Both,
}

impl Default for Direction {
    fn default() -> Self { Direction::Both }
}

/// Spec for an n-hop subgraph expansion around a set of seed concepts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraversalSpec {
    /// Maximum BFS depth from the seeds.
    pub max_depth: u32,
    /// Optional whitelist of relation type names to follow. Empty = all.
    #[serde(default)]
    pub relation_types: Vec<String>,
    /// Optional concept-type filter applied to expanded nodes.
    #[serde(default)]
    pub concept_types: Vec<String>,
    #[serde(default)]
    pub direction: Direction,
    /// Cap on the number of concepts in the resulting subgraph.
    #[serde(default = "default_max_nodes")]
    pub max_nodes: usize,
}

fn default_max_nodes() -> usize { 64 }

impl Default for TraversalSpec {
    fn default() -> Self {
        Self {
            max_depth: 2,
            relation_types: vec![],
            concept_types: vec![],
            direction: Direction::Both,
            max_nodes: 64,
        }
    }
}

/// Materialised result of a traversal — small enough to ship to a prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subgraph {
    pub seeds: Vec<ConceptId>,
    pub concepts: Vec<Concept>,
    pub relations: Vec<Relation>,
    pub depth_of: AHashMap<ConceptId, u32>,
}

impl Subgraph {
    pub fn is_empty(&self) -> bool { self.concepts.is_empty() }
}

impl OntologyGraph {
    /// Breadth-first expansion bounded by `spec.max_depth` and `spec.max_nodes`.
    pub fn expand(&self, seeds: &[ConceptId], spec: &TraversalSpec) -> Subgraph {
        let mut visited: AHashSet<ConceptId> = AHashSet::new();
        let mut emitted_edges: AHashSet<RelationId> = AHashSet::new();
        let mut depth_of: AHashMap<ConceptId, u32> = AHashMap::new();
        let mut concepts: Vec<Concept> = Vec::new();
        let mut relations: Vec<Relation> = Vec::new();
        let mut queue: VecDeque<(ConceptId, u32)> = VecDeque::new();

        for &s in seeds {
            if visited.insert(s) {
                if let Ok(c) = self.get_concept(s) {
                    concepts.push(c);
                    depth_of.insert(s, 0);
                    queue.push_back((s, 0));
                }
            }
        }

        let want_rel = |name: &str| -> bool {
            spec.relation_types.is_empty() || spec.relation_types.iter().any(|r| r == name)
        };
        let want_concept = |name: &str| -> bool {
            spec.concept_types.is_empty() || spec.concept_types.iter().any(|c| c == name)
        };

        while let Some((node, depth)) = queue.pop_front() {
            if depth >= spec.max_depth { continue; }
            if concepts.len() >= spec.max_nodes { break; }

            let mut edges: Vec<Relation> = Vec::new();
            match spec.direction {
                Direction::Outgoing => edges.extend(self.outgoing(node)),
                Direction::Incoming => edges.extend(self.incoming(node)),
                Direction::Both => {
                    edges.extend(self.outgoing(node));
                    edges.extend(self.incoming(node));
                }
            }

            for rel in edges {
                if !want_rel(&rel.relation_type) { continue; }
                let neighbor = if rel.source == node { rel.target } else { rel.source };
                let nc = match self.get_concept(neighbor) { Ok(c) => c, Err(_) => continue };
                if !want_concept(&nc.concept_type) { continue; }

                if visited.insert(neighbor) {
                    depth_of.insert(neighbor, depth + 1);
                    concepts.push(nc);
                    queue.push_back((neighbor, depth + 1));
                    if concepts.len() >= spec.max_nodes { break; }
                }
                if emitted_edges.insert(rel.id) {
                    relations.push(rel);
                }
            }
        }

        Subgraph {
            seeds: seeds.to_vec(),
            concepts,
            relations,
            depth_of,
        }
    }
}
