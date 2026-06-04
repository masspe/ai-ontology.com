// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

use ahash::{AHashMap, AHashSet};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

use crate::error::GraphResult;
use crate::graph::OntologyGraph;
use crate::id::{ConceptId, RelationId};
use crate::model::{Concept, Relation};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Outgoing,
    Incoming,
    #[default]
    Both,
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
    #[serde(default = "default_true")]
    pub subsume_concept_types: bool,
}

fn default_max_nodes() -> usize {
    64
}

fn default_true() -> bool {
    true
}

impl Default for TraversalSpec {
    fn default() -> Self {
        Self {
            max_depth: 2,
            relation_types: vec![],
            concept_types: vec![],
            direction: Direction::Both,
            max_nodes: 64,
            subsume_concept_types: true,
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
    pub fn is_empty(&self) -> bool {
        self.concepts.is_empty()
    }
}

/// One hop along a path: the relation, and the concept on the far side of it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathStep {
    pub relation: Relation,
    pub concept: Concept,
}

/// Result of [`OntologyGraph::shortest_path`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Path {
    pub source: ConceptId,
    pub target: ConceptId,
    /// `start` is the source concept. `steps[i]` is the i-th hop; the final
    /// element's `concept` equals `target`.
    pub start: Concept,
    pub steps: Vec<PathStep>,
}

impl Path {
    pub fn len(&self) -> usize {
        self.steps.len()
    }
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

impl OntologyGraph {
    /// Undirected shortest path between two concepts, bounded by `max_depth`
    /// to keep the BFS cost predictable on very dense graphs. Returns `None`
    /// if the target isn't reachable within the bound.
    pub fn shortest_path(
        &self,
        source: ConceptId,
        target: ConceptId,
        max_depth: u32,
    ) -> GraphResult<Option<Path>> {
        let start = self.get_concept(source)?;
        let _ = self.get_concept(target)?;
        if source == target {
            return Ok(Some(Path {
                source,
                target,
                start,
                steps: Vec::new(),
            }));
        }

        let mut visited: AHashSet<ConceptId> = AHashSet::new();
        // parent[node] = (predecessor, relation traversed to reach node)
        let mut parent: AHashMap<ConceptId, (ConceptId, RelationId)> = AHashMap::new();
        let mut queue: VecDeque<(ConceptId, u32)> = VecDeque::new();
        visited.insert(source);
        queue.push_back((source, 0));

        let mut found = false;
        let mut next_queue: Vec<(ConceptId, u32)> = Vec::new();
        'bfs: while let Some((node, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }
            let mut hit_target = false;
            // for_each_neighbor walks adjacency lists without cloning the
            // Relation struct; we only need (neighbor, relation_id) during
            // BFS, then look up the full relations at reconstruction time.
            self.for_each_neighbor(node, Direction::Both, |neighbor, rid| {
                if !visited.insert(neighbor) {
                    return true;
                }
                parent.insert(neighbor, (node, rid));
                if neighbor == target {
                    hit_target = true;
                    return false;
                }
                next_queue.push((neighbor, depth + 1));
                true
            });
            if hit_target {
                found = true;
                break 'bfs;
            }
            for entry in next_queue.drain(..) {
                queue.push_back(entry);
            }
        }
        if !found {
            return Ok(None);
        }

        // Reconstruct.
        let mut steps_rev: Vec<PathStep> = Vec::new();
        let mut cur = target;
        while cur != source {
            let (prev, rid) = parent[&cur];
            let rel = self.get_relation(rid)?;
            let concept = self.get_concept(cur)?;
            steps_rev.push(PathStep {
                relation: rel,
                concept,
            });
            cur = prev;
        }
        steps_rev.reverse();
        Ok(Some(Path {
            source,
            target,
            start,
            steps: steps_rev,
        }))
    }

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
        // Expand the concept_type filter through the subtype lattice once.
        let concept_filter: Option<AHashSet<String>> = if spec.concept_types.is_empty() {
            None
        } else if spec.subsume_concept_types {
            let onto = self.ontology();
            let mut s: AHashSet<String> = AHashSet::new();
            for t in &spec.concept_types {
                for d in onto.descendants(t) {
                    s.insert(d);
                }
            }
            Some(s)
        } else {
            Some(spec.concept_types.iter().cloned().collect())
        };
        let want_concept = |name: &str| -> bool {
            concept_filter.as_ref().map_or(true, |f| f.contains(name))
        };

        while let Some((node, depth)) = queue.pop_front() {
            if depth >= spec.max_depth {
                continue;
            }
            if concepts.len() >= spec.max_nodes {
                break;
            }

            // When a relation_type whitelist is supplied, jump straight to
            // the typed adjacency buckets and skip cloning edges of types
            // we'd discard anyway.
            let use_typed = !spec.relation_types.is_empty();
            let mut edges: Vec<Relation> = Vec::new();
            match (spec.direction, use_typed) {
                (Direction::Outgoing, true) => {
                    edges.extend(self.outgoing_typed(node, &spec.relation_types))
                }
                (Direction::Incoming, true) => {
                    edges.extend(self.incoming_typed(node, &spec.relation_types))
                }
                (Direction::Both, true) => {
                    edges.extend(self.outgoing_typed(node, &spec.relation_types));
                    edges.extend(self.incoming_typed(node, &spec.relation_types));
                }
                (Direction::Outgoing, false) => edges.extend(self.outgoing(node)),
                (Direction::Incoming, false) => edges.extend(self.incoming(node)),
                (Direction::Both, false) => {
                    edges.extend(self.outgoing(node));
                    edges.extend(self.incoming(node));
                }
            }

            for rel in edges {
                if !want_rel(&rel.relation_type) {
                    continue;
                }
                let neighbor = if rel.source == node {
                    rel.target
                } else {
                    rel.source
                };
                let nc = match self.get_concept(neighbor) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                if !want_concept(&nc.concept_type) {
                    continue;
                }

                if visited.insert(neighbor) {
                    depth_of.insert(neighbor, depth + 1);
                    concepts.push(nc);
                    queue.push_back((neighbor, depth + 1));
                    if concepts.len() >= spec.max_nodes {
                        break;
                    }
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

    /// BFS along edges of `rel_type` (out- and in-edges both treated as
    /// "follow this relation") up to `max_depth`. Used by the RAG to ramp
    /// up transitive `partOf` / `locatedIn` chains. Returns reached
    /// concept ids in BFS order, excluding the seed.
    pub fn closure(
        &self,
        seed: ConceptId,
        rel_type: &str,
        max_depth: u32,
    ) -> GraphResult<Vec<ConceptId>> {
        let _ = self.get_concept(seed)?;
        let mut visited: AHashSet<ConceptId> = AHashSet::new();
        visited.insert(seed);
        let mut queue: VecDeque<(ConceptId, u32)> = VecDeque::new();
        queue.push_back((seed, 0));
        let mut out: Vec<ConceptId> = Vec::new();
        let rt = rel_type.to_string();
        while let Some((node, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }
            for r in self.outgoing_typed(node, std::slice::from_ref(&rt)) {
                let nxt = r.target;
                if visited.insert(nxt) {
                    out.push(nxt);
                    queue.push_back((nxt, depth + 1));
                }
            }
            for r in self.incoming_typed(node, std::slice::from_ref(&rt)) {
                let nxt = r.source;
                if visited.insert(nxt) {
                    out.push(nxt);
                    queue.push_back((nxt, depth + 1));
                }
            }
        }
        Ok(out)
    }
}
