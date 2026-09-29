// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A.. See LICENSE and LICENSE-COMMERCIAL.md.

use ahash::AHashMap;
use ontology_graph::ConceptId;
use parking_lot::RwLock;
use std::sync::Arc;

use crate::embed::{cosine, top_k, Embedder};

/// Flat vector index with brute-force cosine search, contiguous rows,
/// scanned in parallel.
///
/// One `Vec<f32>` of `n × dim` instead of a `Vec` per row: the scan reads
/// memory sequentially, and the layout is the one a memory-mapped file
/// would have (STORAGE-PLAN.md §8 R, tranche 2b). The scan is split across
/// the available cores with scoped threads — no dependency, no pool — so a
/// query costs O(N · dim / cores).
pub struct VectorIndex {
    embedder: Arc<dyn Embedder>,
    rows: RwLock<Rows>,
}

/// Dense rows for the scan, plus the position of each id so an insert or a
/// removal is O(1): the linear `find` this replaced made `reindex_all`
/// O(N²) — seven minutes at 500 000 concepts (STORAGE-PLAN.md §8 R).
#[derive(Default)]
struct Rows {
    dim: usize,
    ids: Vec<ConceptId>,
    /// Row `i` is `data[i * dim..(i + 1) * dim]`.
    data: Vec<f32>,
    pos: AHashMap<ConceptId, usize>,
}

/// Rows per thread below which a second thread costs more than it saves.
const ROWS_PER_THREAD_MIN: usize = 16_384;

impl Rows {
    fn row(&self, i: usize) -> &[f32] {
        &self.data[i * self.dim..(i + 1) * self.dim]
    }

    fn upsert(&mut self, id: ConceptId, v: &[f32]) {
        // A wrong-length row would shift every row after it: refuse before
        // touching anything (a custom `Embedder` may disagree with its `dim`).
        assert_eq!(
            v.len(),
            self.dim,
            "embedder returned {} dims, the index has {}",
            v.len(),
            self.dim
        );
        match self.pos.get(&id) {
            Some(&i) => {
                let d = self.dim;
                self.data[i * d..(i + 1) * d].copy_from_slice(v);
            }
            None => {
                self.pos.insert(id, self.ids.len());
                self.ids.push(id);
                self.data.extend_from_slice(v);
            }
        }
    }

    fn remove(&mut self, id: ConceptId) {
        let Some(i) = self.pos.remove(&id) else {
            return;
        };
        // Swap-remove keeps the rows dense; the moved row gets its new
        // position.
        let last = self.ids.len() - 1;
        let d = self.dim;
        if i != last {
            let (head, tail) = self.data.split_at_mut(last * d);
            head[i * d..(i + 1) * d].copy_from_slice(&tail[..d]);
            let moved = self.ids[last];
            self.ids[i] = moved;
            self.pos.insert(moved, i);
        }
        self.ids.pop();
        self.data.truncate(last * d);
    }

    /// Best `limit` rows of `[from, to)` by cosine with `q`.
    fn scan(&self, q: &[f32], from: usize, to: usize, limit: usize) -> Vec<(ConceptId, f32)> {
        let scored: Vec<(ConceptId, f32)> = (from..to)
            .map(|i| (self.ids[i], cosine(q, self.row(i))))
            .collect();
        top_k(scored, limit)
    }
}

impl VectorIndex {
    pub fn new(embedder: Arc<dyn Embedder>) -> Self {
        let dim = embedder.dim();
        Self {
            embedder,
            rows: RwLock::new(Rows {
                dim,
                ..Default::default()
            }),
        }
    }

    pub fn insert(&self, id: ConceptId, text: &str) {
        let v = self.embedder.embed(text);
        self.insert_vector(id, &v);
    }

    /// Make room for `n` more rows before a bulk insert, so the data does
    /// not double its way up (a 2 GB block briefly needing 3 GB).
    pub fn reserve(&self, n: usize) {
        let mut r = self.rows.write();
        let d = r.dim;
        r.ids.reserve(n);
        r.data.reserve(n * d);
    }

    /// Insert an already-computed vector (embedding is a pure function, so
    /// `reindex_all` computes the vectors in parallel and inserts them here).
    pub fn insert_vector(&self, id: ConceptId, v: &[f32]) {
        self.rows.write().upsert(id, v);
    }

    /// Embed `text` without inserting: the parallel half of an insert.
    pub fn embed(&self, text: &str) -> Vec<f32> {
        self.embedder.embed(text)
    }

    pub fn remove(&self, id: ConceptId) {
        self.rows.write().remove(id);
    }

    /// Brute-force cosine over every row, split across the cores:
    /// O(N · dim / cores) per query. Each thread keeps its own top `limit`,
    /// the heads are merged, so the result is the one a single scan gives.
    // ponytail: at 10⁷ concepts a query reads ~10 GB of rows and is bound by
    // memory bandwidth (unmeasured, hundreds of ms); each query also spawns
    // up to `cores` threads, so concurrent queries oversubscribe the CPU. An
    // ANN index (STORAGE-PLAN.md §8 R, tranche 2b) is the upgrade when the
    // measured p95 (STORAGE.md §7.8) leaves the interactive budget.
    pub fn search(&self, query: &str, limit: usize) -> Vec<(ConceptId, f32)> {
        let q = self.embedder.embed(query);
        let r = self.rows.read();
        let n = r.ids.len();
        if n == 0 || limit == 0 {
            return Vec::new();
        }
        let cores = std::thread::available_parallelism()
            .map(|c| c.get())
            .unwrap_or(1);
        let threads = (n / ROWS_PER_THREAD_MIN).clamp(1, cores);
        if threads == 1 {
            return r.scan(&q, 0, n, limit);
        }
        let chunk = n.div_ceil(threads);
        let heads: Vec<Vec<(ConceptId, f32)>> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..threads)
                .map(|t| {
                    let (r, q) = (&*r, &q);
                    let from = t * chunk;
                    let to = ((t + 1) * chunk).min(n);
                    s.spawn(move || r.scan(q, from, to, limit))
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("scan thread panicked"))
                .collect()
        });
        top_k(heads.into_iter().flatten().collect(), limit)
    }

    pub fn len(&self) -> usize {
        self.rows.read().ids.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
