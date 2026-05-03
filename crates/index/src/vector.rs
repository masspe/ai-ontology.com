// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

use ontology_graph::ConceptId;
use parking_lot::RwLock;
use std::sync::Arc;

use crate::embed::{cosine, Embedder};

/// Flat vector index with brute-force cosine search.
///
/// Suitable for tens of thousands of concepts on a single node. For larger
/// corpora, swap this out for an ANN backend by re-implementing the trait
/// surface used by `HybridIndex`.
pub struct VectorIndex {
    embedder: Arc<dyn Embedder>,
    rows: RwLock<Vec<(ConceptId, Vec<f32>)>>,
}

impl VectorIndex {
    pub fn new(embedder: Arc<dyn Embedder>) -> Self {
        Self {
            embedder,
            rows: RwLock::new(Vec::new()),
        }
    }

    pub fn insert(&self, id: ConceptId, text: &str) {
        let v = self.embedder.embed(text);
        let mut rows = self.rows.write();
        if let Some(slot) = rows.iter_mut().find(|(rid, _)| *rid == id) {
            slot.1 = v;
        } else {
            rows.push((id, v));
        }
    }

    pub fn remove(&self, id: ConceptId) {
        self.rows.write().retain(|(rid, _)| *rid != id);
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<(ConceptId, f32)> {
        let q = self.embedder.embed(query);
        let rows = self.rows.read();
        let mut scored: Vec<(ConceptId, f32)> =
            rows.iter().map(|(id, v)| (*id, cosine(&q, v))).collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        scored
    }

    pub fn len(&self) -> usize {
        self.rows.read().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
