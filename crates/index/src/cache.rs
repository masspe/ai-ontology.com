// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Vectors remembered on disk, keyed by the text they embed.
//!
//! A remote embedder costs a request per batch; the index re-embeds every
//! sheet at each start. The cache makes the second start free and an
//! import pay only for what changed. One file per embedder (its `name()`
//! is in the header: a new model is a new cache), rows of
//! `(hash of the text, vector)`; a zero vector — a request that failed —
//! is never kept, so the next reindex asks again.

use ahash::AHashMap;
use parking_lot::RwLock;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::embed::Embedder;

const MAGIC: &[u8; 8] = b"ONTEMB01";

pub struct CachedEmbedder {
    inner: Arc<dyn Embedder>,
    path: PathBuf,
    rows: RwLock<AHashMap<u64, Vec<f32>>>,
    dirty: AtomicBool,
}

fn key(text: &str) -> u64 {
    ahash::RandomState::with_seeds(0x5eed_0011, 0x5eed_0012, 0x5eed_0013, 0x5eed_0014)
        .hash_one(text)
}

impl CachedEmbedder {
    /// Opens `path` when it holds vectors of this embedder (same name and
    /// dimension); otherwise starts empty and overwrites it at `flush`.
    pub fn open(inner: Arc<dyn Embedder>, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let rows = Self::load(&path, &inner.name(), inner.dim()).unwrap_or_default();
        Self {
            inner,
            path,
            rows: RwLock::new(rows),
            dirty: AtomicBool::new(false),
        }
    }

    fn load(path: &Path, name: &str, dim: usize) -> Option<AHashMap<u64, Vec<f32>>> {
        let bytes = std::fs::read(path).ok()?;
        let mut at = 0usize;
        let take = |at: &mut usize, n: usize| -> Option<&[u8]> {
            let s = bytes.get(*at..*at + n)?;
            *at += n;
            Some(s)
        };
        if take(&mut at, 8)? != MAGIC {
            return None;
        }
        let name_len = u32::from_le_bytes(take(&mut at, 4)?.try_into().ok()?) as usize;
        if std::str::from_utf8(take(&mut at, name_len)?).ok()? != name {
            return None;
        }
        if u32::from_le_bytes(take(&mut at, 4)?.try_into().ok()?) as usize != dim {
            return None;
        }
        let count = u64::from_le_bytes(take(&mut at, 8)?.try_into().ok()?) as usize;
        let mut rows = AHashMap::with_capacity(count);
        for _ in 0..count {
            let k = u64::from_le_bytes(take(&mut at, 8)?.try_into().ok()?);
            let raw = take(&mut at, dim * 4)?;
            let v: Vec<f32> = raw
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| f32::from_le_bytes(*c))
                .collect();
            rows.insert(k, v);
        }
        Some(rows)
    }

    /// How many texts the cache knows.
    pub fn len(&self) -> usize {
        self.rows.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Writes the cache when something was added since the last write:
    /// a temporary file, then a rename, so a crash leaves the old one.
    pub fn save(&self) -> std::io::Result<()> {
        if !self.dirty.swap(false, Ordering::AcqRel) {
            return Ok(());
        }
        let name = self.inner.name();
        let dim = self.inner.dim();
        let rows = self.rows.read();
        let mut out = Vec::with_capacity(32 + rows.len() * (8 + dim * 4));
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(name.len() as u32).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&(dim as u32).to_le_bytes());
        out.extend_from_slice(&(rows.len() as u64).to_le_bytes());
        for (k, v) in rows.iter() {
            out.extend_from_slice(&k.to_le_bytes());
            for x in v {
                out.extend_from_slice(&x.to_le_bytes());
            }
        }
        drop(rows);
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, out)?;
        std::fs::rename(&tmp, &self.path)
    }

    fn remember(&self, text: &str, v: &[f32]) {
        if v.iter().any(|x| *x != 0.0) {
            self.rows.write().insert(key(text), v.to_vec());
            self.dirty.store(true, Ordering::Release);
        }
    }
}

impl Embedder for CachedEmbedder {
    fn dim(&self) -> usize {
        self.inner.dim()
    }

    fn name(&self) -> String {
        self.inner.name()
    }

    fn embed(&self, text: &str) -> Vec<f32> {
        if let Some(v) = self.rows.read().get(&key(text)) {
            return v.clone();
        }
        let v = self.inner.embed(text);
        self.remember(text, &v);
        v
    }

    fn embed_batch(&self, texts: &[&str]) -> Vec<Vec<f32>> {
        let mut out: Vec<Option<Vec<f32>>> = {
            let rows = self.rows.read();
            texts.iter().map(|t| rows.get(&key(t)).cloned()).collect()
        };
        let missing: Vec<usize> = (0..texts.len()).filter(|i| out[*i].is_none()).collect();
        if !missing.is_empty() {
            let asked: Vec<&str> = missing.iter().map(|&i| texts[i]).collect();
            let fresh = self.inner.embed_batch(&asked);
            for (i, v) in missing.into_iter().zip(fresh) {
                self.remember(texts[i], &v);
                out[i] = Some(v);
            }
        }
        out.into_iter().map(Option::unwrap_or_default).collect()
    }

    fn flush(&self) {
        if let Err(e) = self.save() {
            tracing::warn!(error = %e, path = %self.path.display(), "embedding cache not saved");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::HashEmbedder;
    use std::sync::atomic::AtomicUsize;

    /// Counts what it is asked, so a test sees what the cache spared.
    struct Counting(HashEmbedder, AtomicUsize);
    impl Embedder for Counting {
        fn dim(&self) -> usize {
            self.0.dim()
        }
        fn name(&self) -> String {
            "counting".into()
        }
        fn embed(&self, text: &str) -> Vec<f32> {
            self.1.fetch_add(1, Ordering::Relaxed);
            self.0.embed(text)
        }
    }

    fn counting() -> Arc<Counting> {
        Arc::new(Counting(HashEmbedder::new(8), AtomicUsize::new(0)))
    }

    #[test]
    fn a_text_is_embedded_once_and_survives_a_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("emb").join("counting.bin");
        let inner = counting();
        let cache = CachedEmbedder::open(inner.clone(), &path);
        let a = cache.embed("alpha");
        let again = cache.embed("alpha");
        assert_eq!(a, again);
        assert_eq!(inner.1.load(Ordering::Relaxed), 1);
        let batch = cache.embed_batch(&["alpha", "beta", "beta"]);
        assert_eq!(batch[0], a);
        assert_eq!(batch[1], batch[2]);
        assert_eq!(
            inner.1.load(Ordering::Relaxed),
            3,
            "two misses in the batch"
        );
        assert_eq!(cache.len(), 2);
        cache.flush();
        cache.save().unwrap();
        assert!(path.exists());

        let inner2 = counting();
        let reopened = CachedEmbedder::open(inner2.clone(), &path);
        assert_eq!(reopened.len(), 2);
        assert!(!reopened.is_empty());
        assert_eq!(reopened.embed("beta"), batch[1]);
        assert_eq!(inner2.1.load(Ordering::Relaxed), 0, "served from disk");
    }

    #[test]
    fn another_embedder_or_dimension_starts_a_fresh_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.bin");
        let cache = CachedEmbedder::open(counting(), &path);
        cache.embed("x");
        cache.save().unwrap();
        let other = CachedEmbedder::open(Arc::new(HashEmbedder::new(8)), &path);
        assert!(other.is_empty(), "a different name is a different cache");
        let other_dim = CachedEmbedder::open(
            Arc::new(Counting(HashEmbedder::new(4), AtomicUsize::new(0))),
            &path,
        );
        assert!(other_dim.is_empty());
        // Garbage on disk is ignored, not an error.
        std::fs::write(&path, b"nonsense").unwrap();
        assert!(CachedEmbedder::open(counting(), &path).is_empty());
    }

    #[test]
    fn a_zero_vector_is_not_kept_and_an_unchanged_cache_is_not_rewritten() {
        struct Zero;
        impl Embedder for Zero {
            fn dim(&self) -> usize {
                3
            }
            fn embed(&self, _: &str) -> Vec<f32> {
                vec![0.0; 3]
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("z.bin");
        let cache = CachedEmbedder::open(Arc::new(Zero), &path);
        assert_eq!(cache.embed("x"), vec![0.0; 3]);
        assert!(cache.is_empty());
        cache.save().unwrap();
        assert!(!path.exists(), "nothing to write");
    }
}
