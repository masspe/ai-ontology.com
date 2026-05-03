// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct ConceptId(pub u64);

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct RelationId(pub u64);

impl fmt::Debug for ConceptId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "C#{}", self.0)
    }
}
impl fmt::Display for ConceptId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "C#{}", self.0)
    }
}
impl fmt::Debug for RelationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "R#{}", self.0)
    }
}
impl fmt::Display for RelationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "R#{}", self.0)
    }
}

/// Monotonically-increasing id allocator. Thread-safe and lock-free.
#[derive(Debug, Default)]
pub struct IdAllocator {
    next: AtomicU64,
}

impl IdAllocator {
    pub fn new(start: u64) -> Self {
        Self {
            next: AtomicU64::new(start),
        }
    }
    pub fn next_concept(&self) -> ConceptId {
        ConceptId(self.next.fetch_add(1, Ordering::Relaxed))
    }
    pub fn next_relation(&self) -> RelationId {
        RelationId(self.next.fetch_add(1, Ordering::Relaxed))
    }
    pub fn high_water(&self) -> u64 {
        self.next.load(Ordering::Relaxed)
    }
    pub fn observe(&self, value: u64) {
        // Bump the watermark so future allocations don't collide with
        // ids restored from disk.
        let mut current = self.next.load(Ordering::Relaxed);
        while value >= current {
            match self.next.compare_exchange_weak(
                current,
                value + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(observed) => current = observed,
            }
        }
    }
}
