use ahash::AHashMap;
use ontology_graph::ConceptId;
use parking_lot::RwLock;

use crate::embed::tokens;

/// Inverted index with tf-idf scoring. Thread-safe; updates take a write lock,
/// queries take a read lock.
#[derive(Debug, Default)]
pub struct LexicalIndex {
    inner: RwLock<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    /// term -> postings (concept_id, term frequency)
    postings: AHashMap<String, Vec<(ConceptId, u32)>>,
    /// concept_id -> total tokens (for length normalization)
    doc_len: AHashMap<ConceptId, u32>,
    n_docs: u32,
}

impl LexicalIndex {
    pub fn new() -> Self { Self::default() }

    pub fn insert(&self, id: ConceptId, text: &str) {
        let mut tf: AHashMap<String, u32> = AHashMap::new();
        let mut total = 0u32;
        for t in tokens(text) {
            *tf.entry(t).or_insert(0) += 1;
            total += 1;
        }
        let mut g = self.inner.write();
        if g.doc_len.insert(id, total).is_none() {
            g.n_docs += 1;
        } else {
            // Replace existing postings for `id`.
            for postings in g.postings.values_mut() {
                postings.retain(|(cid, _)| *cid != id);
            }
        }
        for (term, count) in tf {
            g.postings.entry(term).or_default().push((id, count));
        }
    }

    pub fn remove(&self, id: ConceptId) {
        let mut g = self.inner.write();
        if g.doc_len.remove(&id).is_some() {
            g.n_docs = g.n_docs.saturating_sub(1);
        }
        for postings in g.postings.values_mut() {
            postings.retain(|(cid, _)| *cid != id);
        }
    }

    /// Returns concept ids ranked by tf-idf score.
    pub fn search(&self, query: &str, limit: usize) -> Vec<(ConceptId, f32)> {
        let g = self.inner.read();
        if g.n_docs == 0 { return Vec::new(); }
        let mut scores: AHashMap<ConceptId, f32> = AHashMap::new();
        let n = g.n_docs as f32;
        for term in tokens(query) {
            let postings = match g.postings.get(&term) { Some(p) => p, None => continue };
            let df = postings.len() as f32;
            let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
            for (id, tf) in postings {
                let dl = *g.doc_len.get(id).unwrap_or(&1) as f32;
                let weight = (*tf as f32 / dl).sqrt() * idf;
                *scores.entry(*id).or_insert(0.0) += weight;
            }
        }
        let mut out: Vec<(ConceptId, f32)> = scores.into_iter().collect();
        out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        out.truncate(limit);
        out
    }
}
