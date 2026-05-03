use ahash::AHashMap;
use ontology_graph::{ConceptId, OntologyGraph, Subgraph, TraversalSpec};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::embed::{Embedder, HashEmbedder};
use crate::lexical::LexicalIndex;
use crate::vector::VectorIndex;

/// Concept ranked by hybrid score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredConcept {
    pub id: ConceptId,
    pub score: f32,
    /// Decomposed scores for explainability/observability.
    pub lexical: f32,
    pub vector: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalRequest {
    pub query: String,
    /// How many concepts to consider before subgraph expansion.
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    /// Weight on the lexical score (0..=1). Vector weight is `1 - lexical_weight`.
    #[serde(default = "default_lex_w")]
    pub lexical_weight: f32,
    /// Optional whitelist of concept type names. When set, ranking only
    /// considers concepts whose type matches one of these (post-filter on
    /// the lexical and vector pools, before fusion). Subtype inheritance
    /// is honored — passing `["Person"]` matches any subtype of `Person`
    /// declared in the ontology.
    #[serde(default)]
    pub concept_types: Vec<String>,
    /// Subgraph expansion spec applied to top-k seeds.
    #[serde(default)]
    pub expansion: TraversalSpec,
}

fn default_top_k() -> usize {
    8
}
fn default_lex_w() -> f32 {
    0.5
}

impl Default for RetrievalRequest {
    fn default() -> Self {
        Self {
            query: String::new(),
            top_k: default_top_k(),
            lexical_weight: default_lex_w(),
            concept_types: Vec::new(),
            expansion: TraversalSpec::default(),
        }
    }
}

/// Combines lexical, vector and graph indexes into a single retrieval
/// surface. All inserts route to both underlying indexes; queries fuse
/// scores using `lexical_weight`.
pub struct HybridIndex {
    graph: Arc<OntologyGraph>,
    lexical: LexicalIndex,
    vector: VectorIndex,
}

impl HybridIndex {
    pub fn new(graph: Arc<OntologyGraph>, embedder: Arc<dyn Embedder>) -> Self {
        Self {
            graph,
            lexical: LexicalIndex::new(),
            vector: VectorIndex::new(embedder),
        }
    }

    /// Convenience constructor with the built-in [`HashEmbedder`].
    pub fn with_default_embedder(graph: Arc<OntologyGraph>) -> Self {
        Self::new(graph, Arc::new(HashEmbedder::default()))
    }

    pub fn graph(&self) -> &Arc<OntologyGraph> {
        &self.graph
    }

    pub fn index_concept(&self, id: ConceptId) -> ontology_graph::GraphResult<()> {
        let c = self.graph.get_concept(id)?;
        let text = c.indexable_text();
        self.lexical.insert(id, &text);
        self.vector.insert(id, &text);
        Ok(())
    }

    /// Drop a concept from both indexes. Called after the graph removes it.
    pub fn forget(&self, id: ConceptId) {
        self.lexical.remove(id);
        self.vector.remove(id);
    }

    /// Reindex every concept currently in the graph.
    pub fn reindex_all(&self) {
        for c in self.graph.all_concepts() {
            let text = c.indexable_text();
            self.lexical.insert(c.id, &text);
            self.vector.insert(c.id, &text);
        }
    }

    pub fn rank(&self, req: &RetrievalRequest) -> Vec<ScoredConcept> {
        let lex_w = req.lexical_weight.clamp(0.0, 1.0);
        let vec_w = 1.0 - lex_w;
        // Pull a wider pool when type-filtering so we have candidates left
        // after the filter trims rejects.
        let pool_mult = if req.concept_types.is_empty() { 4 } else { 16 };
        let pool = req.top_k.max(1) * pool_mult;

        let lex = self.lexical.search(&req.query, pool);
        let vec = self.vector.search(&req.query, pool);

        let lex_max = lex.iter().map(|(_, s)| *s).fold(0f32, f32::max).max(1e-6);
        let vec_max = vec.iter().map(|(_, s)| *s).fold(0f32, f32::max).max(1e-6);

        // Build the type whitelist closure once, honoring subtype inheritance.
        let onto = self.graph.ontology();
        let allow_id = |id: ConceptId| -> bool {
            if req.concept_types.is_empty() {
                return true;
            }
            match self.graph.get_concept(id) {
                Ok(c) => req
                    .concept_types
                    .iter()
                    .any(|wanted| onto.is_subtype(&c.concept_type, wanted)),
                Err(_) => false,
            }
        };

        let mut combined: AHashMap<ConceptId, ScoredConcept> = AHashMap::new();
        for (id, s) in &lex {
            if !allow_id(*id) {
                continue;
            }
            let n = s / lex_max;
            combined
                .entry(*id)
                .or_insert(ScoredConcept {
                    id: *id,
                    score: 0.0,
                    lexical: 0.0,
                    vector: 0.0,
                })
                .lexical = n;
        }
        for (id, s) in &vec {
            if !allow_id(*id) {
                continue;
            }
            let n = s / vec_max;
            combined
                .entry(*id)
                .or_insert(ScoredConcept {
                    id: *id,
                    score: 0.0,
                    lexical: 0.0,
                    vector: 0.0,
                })
                .vector = n;
        }
        let mut out: Vec<ScoredConcept> = combined
            .into_values()
            .map(|mut sc| {
                sc.score = lex_w * sc.lexical + vec_w * sc.vector;
                sc
            })
            .collect();
        out.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out.truncate(req.top_k);
        out
    }

    /// Full retrieval pipeline: rank → take top-k seeds → expand subgraph.
    pub fn retrieve(&self, req: &RetrievalRequest) -> (Vec<ScoredConcept>, Subgraph) {
        let scored = self.rank(req);
        let seeds: Vec<ConceptId> = scored.iter().map(|s| s.id).collect();
        let subgraph = self.graph.expand(&seeds, &req.expansion);
        (scored, subgraph)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ontology_graph::{Concept, ConceptType, Ontology, RelationType};

    fn ontology() -> Ontology {
        let mut o = Ontology::new();
        o.add_concept_type(ConceptType {
            name: "Topic".into(),
            parent: None,
            properties: None,
            description: "topic".into(),
        });
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

    #[test]
    fn hybrid_ranking_finds_relevant_concept() {
        let g = OntologyGraph::with_arc(ontology());
        let rag = g
            .upsert_concept(
                Concept::new(
                    Default::default(),
                    "Topic",
                    "Retrieval Augmented Generation",
                )
                .with_description("LLMs grounded with retrieved documents"),
            )
            .unwrap();
        let _other = g
            .upsert_concept(Concept::new(Default::default(), "Topic", "Knitting"))
            .unwrap();

        let idx = HybridIndex::with_default_embedder(g.clone());
        idx.reindex_all();

        let (ranked, _sg) = idx.retrieve(&RetrievalRequest {
            query: "retrieval augmented generation".into(),
            ..Default::default()
        });
        assert_eq!(ranked.first().map(|s| s.id), Some(rag));
    }

    #[test]
    fn concept_type_filter_excludes_other_types() {
        // Two concept types share lexical overlap; the filter must keep
        // only the requested type in the ranking output.
        let mut o = ontology();
        o.add_concept_type(ontology_graph::ConceptType {
            name: "Person".into(),
            parent: None,
            properties: None,
            description: "human".into(),
        });
        let g = OntologyGraph::with_arc(o);
        let topic = g
            .upsert_concept(Concept::new(Default::default(), "Topic", "Vector Search"))
            .unwrap();
        let person = g
            .upsert_concept(Concept::new(
                Default::default(),
                "Person",
                "Vector Search Researcher",
            ))
            .unwrap();

        let idx = HybridIndex::with_default_embedder(g.clone());
        idx.reindex_all();

        // Without filter: both rank.
        let (ranked, _) = idx.retrieve(&RetrievalRequest {
            query: "vector search".into(),
            top_k: 5,
            ..Default::default()
        });
        let ids: Vec<_> = ranked.iter().map(|s| s.id).collect();
        assert!(ids.contains(&topic) && ids.contains(&person));

        // With filter: only Topic.
        let (ranked, _) = idx.retrieve(&RetrievalRequest {
            query: "vector search".into(),
            top_k: 5,
            concept_types: vec!["Topic".into()],
            ..Default::default()
        });
        let ids: Vec<_> = ranked.iter().map(|s| s.id).collect();
        assert!(ids.contains(&topic), "Topic should rank");
        assert!(!ids.contains(&person), "Person should be filtered out");
    }
}
