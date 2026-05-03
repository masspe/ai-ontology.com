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

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let mut dot = 0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
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
