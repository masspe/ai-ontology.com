// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

use ahash::AHashMap;
use ontology_graph::ConceptId;
use parking_lot::RwLock;
use std::sync::Arc;

use crate::embed::{cosine, top_k, Embedder};

/// Flat vector index with brute-force cosine search.
///
/// Suitable for tens of thousands of concepts on a single node. For larger
/// corpora, swap this out for an ANN backend by re-implementing the trait
/// surface used by `HybridIndex`.
pub struct VectorIndex {
    embedder: Arc<dyn Embedder>,
    rows: RwLock<Rows>,
}

/// Dense rows for the scan, plus the position of each id so an insert or a
/// removal is O(1): the linear `find` this replaced made `reindex_all`
/// O(N²) — seven minutes at 500 000 concepts (STORAGE-PLAN.md §8 R).
#[derive(Default)]
struct Rows {
    rows: Vec<(ConceptId, Vec<f32>)>,
    pos: AHashMap<ConceptId, usize>,
}

impl VectorIndex {
    pub fn new(embedder: Arc<dyn Embedder>) -> Self {
        Self {
            embedder,
            rows: RwLock::new(Rows::default()),
        }
    }

    pub fn insert(&self, id: ConceptId, text: &str) {
        let v = self.embedder.embed(text);
        let mut r = self.rows.write();
        match r.pos.get(&id) {
            Some(&i) => r.rows[i].1 = v,
            None => {
                let at = r.rows.len();
                r.pos.insert(id, at);
                r.rows.push((id, v));
            }
        }
    }

    pub fn remove(&self, id: ConceptId) {
        let mut r = self.rows.write();
        let Some(i) = r.pos.remove(&id) else { return };
        // Swap-remove keeps the vector dense; the moved row gets its new
        // position.
        r.rows.swap_remove(i);
        if let Some(moved) = r.rows.get(i).map(|(id, _)| *id) {
            r.pos.insert(moved, i);
        }
    }

    /// Brute-force cosine over every row: O(N · dim) per query.
    // ponytail: at 10⁷ concepts this is seconds per query; an HNSW built at
    // segment seal (STORAGE-PLAN.md §8 R, item 2) is the upgrade when the
    // measured p95 (STORAGE.md §7.8) leaves the interactive budget.
    pub fn search(&self, query: &str, limit: usize) -> Vec<(ConceptId, f32)> {
        let q = self.embedder.embed(query);
        let r = self.rows.read();
        let scored: Vec<(ConceptId, f32)> =
            r.rows.iter().map(|(id, v)| (*id, cosine(&q, v))).collect();
        top_k(scored, limit)
    }

    pub fn len(&self) -> usize {
        self.rows.read().rows.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
