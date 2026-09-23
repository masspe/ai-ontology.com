// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

use ahash::AHasher;
use std::hash::{Hash, Hasher};

/// Pluggable embedding backend. Implementors should produce vectors of a
/// fixed dimension; `dim()` is read once at index construction time.
pub trait Embedder: Send + Sync + 'static {
    fn dim(&self) -> usize;
    fn embed(&self, text: &str) -> Vec<f32>;
}

/// Deterministic, dependency-free embedder: hashed bag-of-words with L2
/// normalization. Not as good as a real model — but fast, reproducible,
/// and good enough to demonstrate the retrieval plumbing in tests.
#[derive(Debug, Clone)]
pub struct HashEmbedder {
    dim: usize,
}

impl HashEmbedder {
    pub fn new(dim: usize) -> Self {
        assert!(dim > 0, "dim must be positive");
        Self { dim }
    }
}

impl Default for HashEmbedder {
    fn default() -> Self {
        Self::new(256)
    }
}

impl Embedder for HashEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    fn embed(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0f32; self.dim];
        for tok in tokens(text) {
            let mut h = AHasher::default();
            tok.hash(&mut h);
            let idx = (h.finish() as usize) % self.dim;
            // Sign hashing, second hasher.
            let mut h2 = AHasher::default();
            (tok, 0xC0FFEEu64).hash(&mut h2);
            let sign = if h2.finish() & 1 == 0 { 1.0 } else { -1.0 };
            v[idx] += sign;
        }
        l2_normalize(&mut v);
        v
    }
}

/// Keep the `limit` best-scored entries, best first: an O(N) selection
/// then a sort of the head, instead of sorting every score (a search over
/// 500 000 rows sorted 500 000 pairs per query).
pub(crate) fn top_k(
    mut scored: Vec<(ontology_graph::ConceptId, f32)>,
    limit: usize,
) -> Vec<(ontology_graph::ConceptId, f32)> {
    let desc = |a: &(ontology_graph::ConceptId, f32), b: &(ontology_graph::ConceptId, f32)| {
        b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
    };
    if limit == 0 {
        return Vec::new();
    }
    if limit < scored.len() {
        scored.select_nth_unstable_by(limit - 1, desc);
        scored.truncate(limit);
    }
    scored.sort_by(desc);
    scored
}

/// Dot product of two L2-normalised vectors. Eight independent
/// accumulators over chunks of eight: a single running sum cannot be
/// vectorised (float addition does not reassociate), eight lanes can, and
/// the scan of the whole corpus is this loop times N.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let n = a.len().min(b.len());
    let (a, b) = (&a[..n], &b[..n]);
    let mut acc = [0f32; 8];
    let (ca, ra) = a.as_chunks::<8>();
    let (cb, rb) = b.as_chunks::<8>();
    for (x, y) in ca.iter().zip(cb) {
        for i in 0..8 {
            acc[i] += x[i] * y[i];
        }
    }
    let mut dot: f32 = acc.iter().sum();
    for (x, y) in ra.iter().zip(rb) {
        dot += x * y;
    }
    dot
}

pub(crate) fn l2_normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

pub(crate) fn tokens(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
}
