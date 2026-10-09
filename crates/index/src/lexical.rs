// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

use ahash::AHashMap;
use ontology_graph::ConceptId;
use parking_lot::RwLock;

use crate::embed::{tokens, top_k};

/// Term frequencies of one document: the parallel half of an insert
/// (tokenising is pure), see [`LexicalIndex::tokenize`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DocTerms {
    /// `(term, count)`, one entry per distinct term.
    pub tf: Vec<(String, u32)>,
    /// Tokens in the document, repeats included.
    pub total: u32,
}

#[derive(Debug, Default)]
pub struct LexicalIndex {
    inner: RwLock<Inner>,
}

/// Terms are interned to a `u32` once: a posting is two integers and a
/// document's term list is a `Vec<u32>`. The `String`-per-posting layout
/// this replaced allocated ~100 strings per document, and was half of
/// `reindex_all` at 2×10⁵.
#[derive(Debug, Default)]
struct Inner {
    /// term -> term id
    terms: AHashMap<String, u32>,
    /// term id -> postings (concept_id, term frequency)
    postings: Vec<Vec<(ConceptId, u32)>>,
    /// concept_id -> total tokens (for length normalization)
    doc_len: AHashMap<ConceptId, u32>,
    /// concept_id -> its term ids, so an update or a removal touches only
    /// its own postings instead of every posting list of the vocabulary.
    doc_terms: AHashMap<ConceptId, Vec<u32>>,
    n_docs: u32,
}

impl Inner {
    fn unpost(&mut self, id: ConceptId) {
        for term in self.doc_terms.remove(&id).unwrap_or_default() {
            let p = &mut self.postings[term as usize];
            p.retain(|(cid, _)| *cid != id);
            if p.is_empty() {
                // ponytail: the interned vocabulary never shrinks (the term
                // keeps its id); only the capacity is returned. Rebuild the
                // index if a store churns its whole vocabulary.
                *p = Vec::new();
            }
        }
    }

    fn intern(&mut self, term: String) -> u32 {
        if let Some(&t) = self.terms.get(&term) {
            return t;
        }
        let t = self.postings.len() as u32;
        self.postings.push(Vec::new());
        self.terms.insert(term, t);
        t
    }

    fn insert(&mut self, id: ConceptId, doc: DocTerms) {
        if self.doc_len.insert(id, doc.total).is_none() {
            self.n_docs += 1;
        } else {
            self.unpost(id);
        }
        let mut terms = Vec::with_capacity(doc.tf.len());
        for (term, count) in doc.tf {
            let t = self.intern(term);
            self.postings[t as usize].push((id, count));
            terms.push(t);
        }
        self.doc_terms.insert(id, terms);
    }
}

impl LexicalIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Term frequencies of `text`, without touching the index.
    pub(crate) fn tokenize(text: &str) -> DocTerms {
        let mut tf: AHashMap<String, u32> = AHashMap::new();
        let mut total = 0u32;
        for t in tokens(text) {
            *tf.entry(t).or_insert(0) += 1;
            total += 1;
        }
        DocTerms {
            tf: tf.into_iter().collect(),
            total,
        }
    }

    pub fn insert(&self, id: ConceptId, text: &str) {
        self.insert_tokenized(id, Self::tokenize(text));
    }

    /// Insert a document tokenised elsewhere (`reindex_all` tokenises in
    /// parallel, then inserts under the lock).
    pub(crate) fn insert_tokenized(&self, id: ConceptId, doc: DocTerms) {
        self.inner.write().insert(id, doc);
    }

    pub fn remove(&self, id: ConceptId) {
        let mut g = self.inner.write();
        if g.doc_len.remove(&id).is_some() {
            g.n_docs = g.n_docs.saturating_sub(1);
        }
        g.unpost(id);
    }

    /// Returns concept ids ranked by tf-idf score.
    pub fn search(&self, query: &str, limit: usize) -> Vec<(ConceptId, f32)> {
        let g = self.inner.read();
        if g.n_docs == 0 {
            return Vec::new();
        }
        let mut scores: AHashMap<ConceptId, f32> = AHashMap::new();
        let n = g.n_docs as f32;
        for term in tokens(query) {
            let postings = match g.terms.get(&term) {
                Some(&t) => &g.postings[t as usize],
                None => continue,
            };
            if postings.is_empty() {
                continue;
            }
            // A term in more than half of a large corpus weighs ~0 (idf ≈
            // ln 1) and would still cost a walk of most of the corpus: skip
            // it, like a stop word learnt from the data. Small posting lists
            // are walked whatever their share — the walk is microseconds and
            // a two-document corpus must still find its shared word.
            if postings.len() > 1_000 && postings.len() * 2 > g.n_docs as usize {
                continue;
            }
            let df = postings.len() as f32;
            let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
            for (id, tf) in postings {
                let dl = *g.doc_len.get(id).unwrap_or(&1) as f32;
                let weight = (*tf as f32 / dl).sqrt() * idf;
                *scores.entry(*id).or_insert(0.0) += weight;
            }
        }
        top_k(scores.into_iter().collect(), limit)
    }
}
