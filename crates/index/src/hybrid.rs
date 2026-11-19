// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

use ahash::AHashMap;
use ontology_graph::{ConceptId, OntologyGraph, Subgraph, TraversalSpec};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::embed::{tokens, Embedder, HashEmbedder};
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
    /// The reader's visibility: a hidden concept is never a candidate.
    /// Set by the server from the caller's clearances, never by a client;
    /// `retrieve` hands it to the expansion when that has none.
    #[serde(skip)]
    pub visible: Option<ontology_graph::Visibility>,
}

fn default_top_k() -> usize {
    8
}
fn default_lex_w() -> f32 {
    0.5
}

/// Minimum fused score, expressed as a fraction of the top hit's score, for a
/// concept to be considered relevant enough to return. Anything below this is
/// treated as noise from the vector index and dropped before truncation. See
/// the filtering step in [`HybridIndex::rank`].
const RELEVANCE_RATIO: f32 = 0.4;

/// Added to the fused score (≤ 1) of a sheet the query names exactly,
/// scaled by how identifying the naming word is (`db660`: most of it;
/// `gestion`, a word in thousands of sheets: little). Twice the top
/// fused score, so a screen code names its screen over the programs and
/// pages that merely mention it, even when its help text is long.
const NAME_BOOST: f32 = 2.0;

/// The query term that names the sheet, if one does. A sheet is named by
/// its whole name or its last dotted segment (`sdba` names `appdb.sdba`);
/// a one-character segment (`w` of `gl/w-gl100.w`) never counts. A detail
/// sheet is named only by its full name or the name without its first
/// segment (`appdb.sdba.nogf`, `sdba.nogf`): its bare last segment is a
/// short code that collides with ordinary words (`comment`, `km`).
fn naming_term(query_terms: &[String], query_lc: &str, name: &str, detail: bool) -> Option<String> {
    let name = name.to_lowercase();
    let last = name.rsplit('.').next().unwrap_or(&name).to_string();
    if detail {
        let tail = name.split_once('.').map_or(name.as_str(), |(_, t)| t);
        let asked = if tail.contains('.') {
            query_lc.contains(tail)
        } else {
            query_terms.iter().any(|t| t == tail)
        };
        return asked.then_some(last);
    }
    query_terms
        .iter()
        .find(|t| **t == name || (last.len() >= 2 && **t == last))
        .cloned()
}

/// `true` when `ty` or one of its ancestors is declared `detail`.
fn is_detail(onto: &ontology_graph::Ontology, ty: &str) -> bool {
    let mut cursor = onto.concept_types.get(ty);
    let mut hops = 0;
    while let Some(ct) = cursor {
        if ct.detail {
            return true;
        }
        hops += 1;
        if hops > 64 {
            return false;
        }
        cursor = ct.parent.as_deref().and_then(|p| onto.concept_types.get(p));
    }
    false
}

impl Default for RetrievalRequest {
    fn default() -> Self {
        Self {
            query: String::new(),
            top_k: default_top_k(),
            lexical_weight: default_lex_w(),
            concept_types: Vec::new(),
            expansion: TraversalSpec::default(),
            visible: None,
        }
    }
}

/// Where `reindex_all` spends its time (milliseconds).
#[derive(Debug, Clone)]
pub struct ReindexReport {
    pub concepts: usize,
    pub texts_ms: f64,
    pub lexical_ms: f64,
    pub vector_ms: f64,
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

    /// Let the embedder persist its cache (after an import, before exit).
    pub fn flush(&self) {
        self.vector.flush();
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
        self.reindex_all_timed();
    }

    /// `reindex_all` with its three costs apart, for the bench:
    /// reading every concept's text (payloads come
    /// from disk in P1), the lexical postings, the vectors. Tokenising and
    /// embedding are pure, so they run on every core with scoped threads;
    /// only the inserts take the locks.
    pub fn reindex_all_timed(&self) -> ReindexReport {
        let t = std::time::Instant::now();
        let texts: Vec<(ConceptId, String)> = self
            .graph
            .all_concepts()
            .into_iter()
            .map(|c| (c.id, c.indexable_text()))
            .collect();
        let texts_ms = t.elapsed().as_secs_f64() * 1e3;

        // Batches bound the transient memory: at 2×10⁶ the vectors alone are
        // 2 GB, so they must not all exist before the first insert.
        const BATCH: usize = 65_536;
        let t = std::time::Instant::now();
        for batch in texts.chunks(BATCH) {
            let docs = parallel_map(batch, |(_, text)| LexicalIndex::tokenize(text));
            for ((id, _), doc) in batch.iter().zip(docs) {
                self.lexical.insert_tokenized(*id, doc);
            }
        }
        let lexical_ms = t.elapsed().as_secs_f64() * 1e3;

        let t = std::time::Instant::now();
        self.vector.reserve(texts.len());
        // Groups of texts go to the embedder together — a remote one sends
        // one request per group, a local one maps them — on every core.
        const GROUP: usize = 64;
        for batch in texts.chunks(BATCH) {
            let groups: Vec<&[(ConceptId, String)]> = batch.chunks(GROUP).collect();
            let vectors = parallel_map_min(&groups, 1, |group| {
                let refs: Vec<&str> = group.iter().map(|(_, t)| t.as_str()).collect();
                self.vector.embed_batch(&refs)
            });
            for (group, vs) in groups.iter().zip(vectors) {
                for ((id, _), v) in group.iter().zip(vs) {
                    self.vector.insert_vector(*id, &v);
                }
            }
        }
        self.vector.flush();
        let vector_ms = t.elapsed().as_secs_f64() * 1e3;
        ReindexReport {
            concepts: texts.len(),
            texts_ms,
            lexical_ms,
            vector_ms,
        }
    }

    pub fn rank(&self, req: &RetrievalRequest) -> Vec<ScoredConcept> {
        let lex_w = req.lexical_weight.clamp(0.0, 1.0);
        let vec_w = 1.0 - lex_w;
        // The pool is a cut after full scoring, so a wide one is free; it
        // must hold the sheet a question names when its text scored low
        // (`sdba` among six other words).
        let pool = req.top_k.max(1) * 16;
        let onto = self.graph.ontology();
        let query_terms: Vec<String> = tokens(&req.query).collect();
        let query_lc = req.query.to_lowercase();

        // The term naming this sheet, if the query has one.
        let named = |id: ConceptId| -> Option<String> {
            let (ty, name) = self.graph.concept_header(id)?;
            naming_term(&query_terms, &query_lc, &name, is_detail(&onto, &ty))
        };
        // Who may be a seed: the requested types (subtypes included), and
        // among them the detail sheets only when the query names them.
        let keep = |id: ConceptId| -> bool {
            if req.visible.as_ref().is_some_and(|v| !v.allows(id)) {
                return false;
            }
            let Some((ty, _)) = self.graph.concept_header(id) else {
                return false;
            };
            if !req.concept_types.is_empty() {
                return req.concept_types.iter().any(|w| onto.is_subtype(&ty, w));
            }
            !is_detail(&onto, &ty) || named(id).is_some()
        };

        let lex = self.lexical.search_where(&req.query, pool, keep);
        let vec = self.vector.search_where(&req.query, pool, &keep);

        let lex_max = lex.iter().map(|(_, s)| *s).fold(0f32, f32::max).max(1e-6);
        let vec_max = vec.iter().map(|(_, s)| *s).fold(0f32, f32::max).max(1e-6);

        let mut combined: AHashMap<ConceptId, ScoredConcept> = AHashMap::new();
        let blank = |id| ScoredConcept {
            id,
            score: 0.0,
            lexical: 0.0,
            vector: 0.0,
        };
        for (id, s) in &lex {
            combined.entry(*id).or_insert_with(|| blank(*id)).lexical = s / lex_max;
        }
        for (id, s) in &vec {
            combined.entry(*id).or_insert_with(|| blank(*id)).vector = s / vec_max;
        }
        let mut out: Vec<ScoredConcept> = combined
            .into_values()
            .map(|mut sc| {
                sc.score = lex_w * sc.lexical + vec_w * sc.vector;
                sc
            })
            .collect();

        // Drop the weakly-scored long tail. The vector index assigns a small
        // nonzero score to *every* concept (the hash embedder never returns a
        // hard zero), so a plain `truncate(top_k)` pads the result with
        // concepts that don't actually match whenever fewer than `top_k`
        // genuinely relevant ones exist — surfacing irrelevant concepts in the
        // grounded answer. Keep only concepts scoring within
        // `RELEVANCE_RATIO` of the best hit; this trims the noise floor while
        // preserving every concept that is competitive with the top match.
        // No positive evidence is no hit: an empty query embeds to the zero
        // vector and matches no term, which must not surface the whole
        // corpus at score 0 (and a negative cosine is noise, not a match).
        let floor = out.iter().map(|s| s.score).fold(0f32, f32::max) * RELEVANCE_RATIO;
        // A sheet the query names exactly (`db660`, `sdba`, `msdb.sdba`)
        // is the answer whatever its text scored: above everything, and
        // above the floor a long help text would have put it under.
        out.retain(|s| s.score > 0.0 && (s.score >= floor || named(s.id).is_some()));
        for s in &mut out {
            if let Some(term) = named(s.id) {
                s.score += NAME_BOOST * self.lexical.rarity(&term);
            }
        }
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
        let subgraph = match (&req.expansion.visible, &req.visible) {
            (None, Some(v)) => {
                let spec = TraversalSpec {
                    visible: Some(v.clone()),
                    ..req.expansion.clone()
                };
                self.graph.expand(&seeds, &spec)
            }
            _ => self.graph.expand(&seeds, &req.expansion),
        };
        (scored, subgraph)
    }
}

/// `items.iter().map(f)` on every available core, order preserved. Scoped
/// threads: no pool, no dependency; below one chunk per core it is a plain
/// map.
fn parallel_map<T: Sync, U: Send>(items: &[T], f: impl Fn(&T) -> U + Sync) -> Vec<U> {
    parallel_map_min(items, 4_096, f)
}

/// `parallel_map` with the smallest share of items worth a thread: 1 when
/// each item is a request to a remote embedder.
fn parallel_map_min<T: Sync, U: Send>(
    items: &[T],
    chunk_min: usize,
    f: impl Fn(&T) -> U + Sync,
) -> Vec<U> {
    let cores = std::thread::available_parallelism()
        .map(|c| c.get())
        .unwrap_or(1);
    let threads = (items.len() / chunk_min.max(1)).clamp(1, cores);
    if threads == 1 {
        return items.iter().map(&f).collect();
    }
    let chunk = items.len().div_ceil(threads);
    std::thread::scope(|s| {
        let handles: Vec<_> = items
            .chunks(chunk)
            .map(|part| {
                let f = &f;
                s.spawn(move || part.iter().map(f).collect::<Vec<U>>())
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("map thread panicked"))
            .collect()
    })
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
            ..Default::default()
        });
        o.add_relation_type(RelationType {
            name: "related_to".into(),
            domain: "Topic".into(),
            range: "Topic".into(),
            cardinality: Default::default(),
            symmetric: true,
            description: "".into(),
            ..Default::default()
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
    fn ranking_drops_irrelevant_concepts() {
        // A query that strongly matches one concept must not pad the result
        // with concepts that have no real overlap, even though `top_k` leaves
        // room and the vector index scores everything nonzero.
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
        let knitting = g
            .upsert_concept(
                Concept::new(Default::default(), "Topic", "Knitting")
                    .with_description("looping yarn into fabric with needles"),
            )
            .unwrap();
        let baking = g
            .upsert_concept(
                Concept::new(Default::default(), "Topic", "Sourdough Baking")
                    .with_description("fermenting flour and water into bread"),
            )
            .unwrap();

        let idx = HybridIndex::with_default_embedder(g.clone());
        idx.reindex_all();

        let ranked = idx.rank(&RetrievalRequest {
            query: "retrieval augmented generation".into(),
            top_k: 8,
            ..Default::default()
        });
        let ids: Vec<_> = ranked.iter().map(|s| s.id).collect();
        assert!(ids.contains(&rag), "the matching concept must be returned");
        assert!(
            !ids.contains(&knitting) && !ids.contains(&baking),
            "irrelevant concepts must be filtered out, got {ids:?}"
        );
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
            ..Default::default()
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
