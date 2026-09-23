// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! `HybridIndex` invariants: indexing is idempotent, forgetting is
//! complete, ranking is deterministic and bounded by `top_k`, the weight
//! extremes select one signal, and retrieval degrades gracefully on empty
//! or tiny graphs.

use std::sync::Arc;

use ontology_graph::{
    Concept, ConceptId, ConceptType, Ontology, OntologyGraph, Relation, RelationId, RelationType,
    TraversalSpec,
};
use ontology_index::{
    Embedder, HashEmbedder, HybridIndex, LexicalIndex, RetrievalRequest, VectorIndex,
};

fn ontology() -> Ontology {
    let mut o = Ontology::new();
    o.add_concept_type(ConceptType {
        name: "Topic".into(),
        ..Default::default()
    });
    o.add_concept_type(ConceptType {
        name: "Subtopic".into(),
        parent: Some("Topic".into()),
        ..Default::default()
    });
    o.add_relation_type(RelationType {
        name: "related_to".into(),
        domain: "Topic".into(),
        range: "Topic".into(),
        ..Default::default()
    })
    .unwrap();
    o
}

fn topic(g: &OntologyGraph, name: &str, description: &str) -> ConceptId {
    g.upsert_concept(Concept::new(ConceptId(0), "Topic", name).with_description(description))
        .unwrap()
}

/// Three well-separated topics so lexical matches are unambiguous.
fn corpus() -> (Arc<OntologyGraph>, ConceptId, ConceptId, ConceptId) {
    let g = OntologyGraph::with_arc(ontology());
    let rag = topic(
        &g,
        "Retrieval Augmented Generation",
        "grounding language models with retrieved passages",
    );
    let knit = topic(&g, "Knitting", "looping yarn into fabric with needles");
    let bread = topic(
        &g,
        "Sourdough Baking",
        "fermenting flour and water into bread",
    );
    (g, rag, knit, bread)
}

fn req(query: &str) -> RetrievalRequest {
    RetrievalRequest {
        query: query.into(),
        ..Default::default()
    }
}

fn ranked_ids(idx: &HybridIndex, r: &RetrievalRequest) -> Vec<ConceptId> {
    idx.rank(r).iter().map(|s| s.id).collect()
}

/// `reindex_all` and repeated `index_concept` are idempotent: no duplicate
/// rows, identical scores, identical ranking.
#[test]
fn reindex_all_and_index_concept_are_idempotent() {
    let (g, rag, _, _) = corpus();
    let idx = HybridIndex::with_default_embedder(g.clone());
    idx.reindex_all();
    let once = idx.rank(&req("retrieval augmented generation"));
    idx.reindex_all();
    idx.index_concept(rag).unwrap();
    idx.index_concept(rag).unwrap();
    let thrice = idx.rank(&req("retrieval augmented generation"));
    assert_eq!(once.len(), thrice.len());
    for (a, b) in once.iter().zip(&thrice) {
        assert_eq!(a.id, b.id);
        assert_eq!(a.score, b.score, "scores must not drift with re-indexing");
        assert_eq!(a.lexical, b.lexical);
        assert_eq!(a.vector, b.vector);
    }
    assert_eq!(once.first().map(|s| s.id), Some(rag));
    // Indexing an unknown concept is an error and changes nothing.
    assert!(idx.index_concept(ConceptId(999)).is_err());
    assert_eq!(
        ranked_ids(&idx, &req("retrieval augmented generation")),
        vec![rag]
    );
}

/// After `forget`, a concept can no longer be retrieved by either signal,
/// while its neighbours still are; forgetting twice or forgetting an
/// unknown id is harmless.
#[test]
fn forget_removes_a_concept_from_both_signals() {
    let (g, rag, knit, bread) = corpus();
    let idx = HybridIndex::with_default_embedder(g.clone());
    idx.reindex_all();
    assert_eq!(ranked_ids(&idx, &req("knitting yarn needles")), vec![knit]);
    idx.forget(knit);
    idx.forget(knit);
    idx.forget(ConceptId(999));
    let after = ranked_ids(&idx, &req("knitting yarn needles"));
    assert!(
        !after.contains(&knit),
        "forgotten concept still ranked: {after:?}"
    );
    // Pure-vector and pure-lexical requests agree it is gone.
    for w in [0.0, 1.0] {
        let r = RetrievalRequest {
            lexical_weight: w,
            ..req("knitting yarn needles")
        };
        assert!(!ranked_ids(&idx, &r).contains(&knit), "weight {w}");
    }
    assert_eq!(ranked_ids(&idx, &req("sourdough bread flour")), vec![bread]);
    assert_eq!(
        ranked_ids(&idx, &req("retrieval augmented generation")),
        vec![rag]
    );
    // Forgetting everything empties the ranking entirely.
    idx.forget(rag);
    idx.forget(bread);
    assert!(idx.rank(&req("retrieval augmented generation")).is_empty());
    let (scored, sg) = idx.retrieve(&req("bread"));
    assert!(scored.is_empty() && sg.is_empty());
}

/// Retrieval on an empty graph, or on an index that was never filled,
/// returns empty results rather than failing.
#[test]
fn retrieve_on_an_empty_graph_or_unfilled_index_is_empty() {
    let g = OntologyGraph::with_arc(ontology());
    let idx = HybridIndex::with_default_embedder(g.clone());
    idx.reindex_all();
    let (scored, sg) = idx.retrieve(&req("anything at all"));
    assert!(scored.is_empty());
    assert!(sg.is_empty() && sg.relations.is_empty() && sg.seeds.is_empty());
    let (scored, _) = idx.retrieve(&req(""));
    assert!(scored.is_empty());
    // Concepts in the graph but never indexed are not retrievable.
    topic(&g, "Knitting", "yarn");
    assert!(idx.rank(&req("knitting yarn")).is_empty());
    // An empty query over a filled index yields nothing lexical; the
    // vector side of an empty query is a zero vector, so nothing scores.
    idx.reindex_all();
    assert!(
        idx.rank(&req("")).is_empty(),
        "empty query must not return the whole corpus"
    );
}

/// `lexical_weight` at the extremes (and beyond, clamped) selects exactly
/// one signal: the fused score equals that component for every hit.
#[test]
fn weight_extremes_select_a_single_signal_deterministically() {
    let (g, _, _, _) = corpus();
    let idx = HybridIndex::with_default_embedder(g.clone());
    idx.reindex_all();
    let q = "retrieval augmented generation with retrieved passages";
    for (w, label) in [(1.0f32, "lexical"), (5.0, "lexical (clamped)")] {
        let out = idx.rank(&RetrievalRequest {
            lexical_weight: w,
            top_k: 10,
            ..req(q)
        });
        assert!(!out.is_empty());
        for s in &out {
            assert_eq!(
                s.score, s.lexical,
                "{label}: score must equal the lexical component"
            );
        }
    }
    for (w, label) in [(0.0f32, "vector"), (-3.0, "vector (clamped)")] {
        let out = idx.rank(&RetrievalRequest {
            lexical_weight: w,
            top_k: 10,
            ..req(q)
        });
        assert!(!out.is_empty());
        for s in &out {
            assert_eq!(
                s.score, s.vector,
                "{label}: score must equal the vector component"
            );
        }
    }
    // The blend is the documented convex combination and the best hit is
    // normalised to 1 on each side.
    let out = idx.rank(&RetrievalRequest {
        lexical_weight: 0.25,
        top_k: 10,
        ..req(q)
    });
    for s in &out {
        let expected = 0.25 * s.lexical + 0.75 * s.vector;
        assert!((s.score - expected).abs() < 1e-6, "{s:?}");
        assert!(s.lexical <= 1.0 + 1e-6 && s.vector <= 1.0 + 1e-6);
    }
    assert!(out.iter().any(|s| (s.lexical - 1.0).abs() < 1e-6));
    // Two identical calls give byte-identical rankings.
    let a = idx.rank(&RetrievalRequest {
        lexical_weight: 0.5,
        ..req(q)
    });
    let b = idx.rank(&RetrievalRequest {
        lexical_weight: 0.5,
        ..req(q)
    });
    assert_eq!(
        a.iter()
            .map(|s| (s.id, s.score.to_bits()))
            .collect::<Vec<_>>(),
        b.iter()
            .map(|s| (s.id, s.score.to_bits()))
            .collect::<Vec<_>>()
    );
    // Ranking is sorted by descending score.
    for w in a.windows(2) {
        assert!(w[0].score >= w[1].score);
    }
}

/// `top_k = 0` yields nothing; `top_k` above the corpus size never pads
/// the result beyond the relevant concepts; results never exceed `top_k`.
#[test]
fn top_k_zero_and_oversized_are_handled() {
    let (g, rag, _, _) = corpus();
    let idx = HybridIndex::with_default_embedder(g.clone());
    idx.reindex_all();
    let q = "retrieval augmented generation";
    let (scored, sg) = idx.retrieve(&RetrievalRequest { top_k: 0, ..req(q) });
    assert!(scored.is_empty());
    assert!(sg.is_empty());
    let big = idx.rank(&RetrievalRequest {
        top_k: 1_000,
        ..req(q)
    });
    assert!(big.len() <= 3);
    assert_eq!(big.first().map(|s| s.id), Some(rag));
    // A query touching every concept is still capped at top_k.
    let broad = "retrieval knitting sourdough";
    assert_eq!(
        idx.rank(&RetrievalRequest {
            top_k: 3,
            lexical_weight: 1.0,
            ..req(broad)
        })
        .len(),
        3
    );
    assert_eq!(
        idx.rank(&RetrievalRequest {
            top_k: 2,
            lexical_weight: 1.0,
            ..req(broad)
        })
        .len(),
        2
    );
    assert_eq!(
        idx.rank(&RetrievalRequest {
            top_k: 1,
            lexical_weight: 1.0,
            ..req(broad)
        })
        .len(),
        1
    );
}

/// The expansion spec is honoured end to end: depth 0 returns only the
/// seeds, depth 1 pulls in neighbours and the connecting relations.
#[test]
fn retrieve_expands_seeds_according_to_the_spec() {
    let (g, rag, knit, bread) = corpus();
    g.add_relation(Relation::new(RelationId(0), "related_to", rag, knit))
        .unwrap();
    g.add_relation(Relation::new(RelationId(0), "related_to", knit, bread))
        .unwrap();
    let idx = HybridIndex::with_default_embedder(g.clone());
    idx.reindex_all();
    let q = "retrieval augmented generation";
    let (scored, sg0) = idx.retrieve(&RetrievalRequest {
        expansion: TraversalSpec {
            max_depth: 0,
            ..Default::default()
        },
        ..req(q)
    });
    assert_eq!(scored.iter().map(|s| s.id).collect::<Vec<_>>(), vec![rag]);
    assert_eq!(sg0.seeds, vec![rag]);
    assert_eq!(sg0.concepts.len(), 1);
    assert!(sg0.relations.is_empty());
    let (_, sg1) = idx.retrieve(&RetrievalRequest {
        expansion: TraversalSpec {
            max_depth: 1,
            ..Default::default()
        },
        ..req(q)
    });
    assert_eq!(sg1.concepts.len(), 2);
    assert_eq!(sg1.relations.len(), 1);
    assert_eq!(sg1.depth_of[&knit], 1);
    let (_, sg2) = idx.retrieve(&RetrievalRequest {
        expansion: TraversalSpec {
            max_depth: 2,
            ..Default::default()
        },
        ..req(q)
    });
    assert_eq!(sg2.concepts.len(), 3);
    assert_eq!(sg2.depth_of[&bread], 2);
    // The expansion follows the graph as it is *now*, not as indexed.
    g.remove_concept(knit).unwrap();
    idx.forget(knit);
    let (_, sg) = idx.retrieve(&RetrievalRequest {
        expansion: TraversalSpec {
            max_depth: 2,
            ..Default::default()
        },
        ..req(q)
    });
    assert_eq!(sg.concepts.len(), 1);
}

/// The concept-type filter honours subtypes and an unknown type yields
/// nothing rather than everything.
#[test]
fn concept_type_filter_honours_subtypes_and_unknown_types() {
    let g = OntologyGraph::with_arc(ontology());
    let sub = g
        .upsert_concept(Concept::new(ConceptId(0), "Subtopic", "Vector Search"))
        .unwrap();
    let idx = HybridIndex::with_default_embedder(g.clone());
    idx.reindex_all();
    let with_parent = RetrievalRequest {
        concept_types: vec!["Topic".into()],
        ..req("vector search")
    };
    assert_eq!(ranked_ids(&idx, &with_parent), vec![sub]);
    let with_child = RetrievalRequest {
        concept_types: vec!["Subtopic".into()],
        ..req("vector search")
    };
    assert_eq!(ranked_ids(&idx, &with_child), vec![sub]);
    let unknown = RetrievalRequest {
        concept_types: vec!["Nope".into()],
        ..req("vector search")
    };
    assert!(ranked_ids(&idx, &unknown).is_empty());
}

/// A custom embedder is used as given and its dimension is respected.
#[test]
fn custom_embedder_is_used_as_given() {
    struct Constant;
    impl Embedder for Constant {
        fn dim(&self) -> usize {
            4
        }
        fn embed(&self, _text: &str) -> Vec<f32> {
            vec![0.5, 0.5, 0.5, 0.5]
        }
    }
    let (g, rag, _, _) = corpus();
    let idx = HybridIndex::new(g.clone(), Arc::new(Constant));
    idx.reindex_all();
    // Every vector is identical, so the vector signal cannot discriminate:
    // the lexical signal decides and RAG wins on its own query.
    let out = idx.rank(&RetrievalRequest {
        lexical_weight: 0.5,
        ..req("retrieval augmented generation")
    });
    assert_eq!(out.first().map(|s| s.id), Some(rag));
    assert!(
        out.iter().all(|s| (s.vector - 1.0).abs() < 1e-6),
        "all vectors tie at the max"
    );
    assert_eq!(HashEmbedder::new(16).dim(), 16);
    assert_eq!(HashEmbedder::new(16).embed("x y").len(), 16);
}

/// The standalone indexes: re-inserting an id replaces its row instead of
/// duplicating it, `remove` is complete, and search honours `limit`.
#[test]
fn lexical_and_vector_indexes_replace_rows_and_honour_limit() {
    let lex = LexicalIndex::new();
    lex.insert(ConceptId(1), "alpha beta");
    lex.insert(ConceptId(1), "gamma");
    assert!(lex.search("alpha", 10).is_empty(), "old postings replaced");
    assert_eq!(lex.search("gamma", 10).len(), 1);
    lex.insert(ConceptId(2), "gamma gamma");
    assert_eq!(lex.search("gamma", 1).len(), 1);
    assert_eq!(lex.search("gamma", 10).len(), 2);
    assert_eq!(lex.search("gamma", 0).len(), 0);
    lex.remove(ConceptId(1));
    lex.remove(ConceptId(1));
    assert_eq!(
        lex.search("gamma", 10)
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>(),
        vec![ConceptId(2)]
    );
    lex.remove(ConceptId(2));
    assert!(lex.search("gamma", 10).is_empty());

    let vec = VectorIndex::new(Arc::new(HashEmbedder::new(32)));
    assert!(vec.is_empty());
    vec.insert(ConceptId(1), "alpha");
    vec.insert(ConceptId(1), "beta");
    assert_eq!(vec.len(), 1, "re-insert replaces");
    vec.insert(ConceptId(2), "gamma");
    assert_eq!(vec.search("beta", 1).len(), 1);
    assert_eq!(vec.search("beta", 1)[0].0, ConceptId(1));
    assert_eq!(vec.search("beta", 0).len(), 0);
    vec.remove(ConceptId(1));
    vec.remove(ConceptId(1));
    assert_eq!(vec.len(), 1);
    assert_eq!(vec.search("anything", 10).len(), 1);
}

/// The vector index removes by swap: the row moved into the hole keeps a
/// correct position, so a later re-insert of it replaces instead of
/// duplicating and a later removal finds it.
#[test]
fn vector_swap_remove_keeps_positions_consistent() {
    let vec = VectorIndex::new(Arc::new(HashEmbedder::new(32)));
    vec.insert(ConceptId(1), "alpha");
    vec.insert(ConceptId(2), "beta");
    vec.insert(ConceptId(3), "gamma");
    vec.remove(ConceptId(1)); // gamma moves to slot 0
    assert_eq!(vec.len(), 2);
    vec.insert(ConceptId(3), "gamma delta");
    assert_eq!(vec.len(), 2, "re-insert of the moved row replaces it");
    // Which ids are present is the invariant; their order depends on the
    // hash embedder's collisions, which differ between platforms.
    let ids = |v: Vec<(ConceptId, f32)>| {
        let mut ids: Vec<u64> = v.into_iter().map(|(id, _)| id.0).collect();
        ids.sort();
        ids
    };
    assert_eq!(ids(vec.search("delta", 10)), vec![2, 3]);
    vec.remove(ConceptId(3));
    assert_eq!(vec.len(), 1);
    assert_eq!(ids(vec.search("beta", 10)), vec![2]);
    vec.remove(ConceptId(2));
    assert!(vec.is_empty());

    // Lexical: an update only rewrites the document's own terms; a term
    // whose last document leaves disappears from the vocabulary.
    let lex = LexicalIndex::new();
    lex.insert(ConceptId(1), "alpha beta");
    lex.insert(ConceptId(2), "beta gamma");
    lex.insert(ConceptId(1), "delta");
    assert_eq!(lex.search("alpha", 10).len(), 0);
    assert_eq!(lex.search("beta", 10).len(), 1);
    assert_eq!(lex.search("delta", 10)[0].0, ConceptId(1));
    lex.remove(ConceptId(2));
    assert!(lex.search("gamma", 10).is_empty());
    assert!(lex.search("beta", 10).is_empty());
}
