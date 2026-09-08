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

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct RuleId(pub u64);

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct ActionId(pub u64);

/// Largest `ConceptId` the storage format can represent: relation endpoints
/// are packed as `(source << 32) | target` in the on-disk index
/// (`STORAGE.md` §4.3, decision D6). Checked, never assumed.
pub const MAX_CONCEPT_ID: u64 = u32::MAX as u64;

impl ConceptId {
    /// `true` when the id fits the storage format's 32-bit endpoint packing.
    pub fn fits_storage(self) -> bool {
        self.0 <= MAX_CONCEPT_ID
    }
}

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
impl fmt::Debug for RuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Ru#{}", self.0)
    }
}
impl fmt::Display for RuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Ru#{}", self.0)
    }
}
impl fmt::Debug for ActionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "A#{}", self.0)
    }
}
impl fmt::Display for ActionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "A#{}", self.0)
    }
}

/// Snapshot of the four allocators' next values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdWatermarks {
    pub concepts: u64,
    pub relations: u64,
    pub rules: u64,
    pub actions: u64,
}

/// Monotonically-increasing id allocators, one **per family**. Thread-safe
/// and lock-free.
///
/// The four id types are distinct (`ConceptId`, `RelationId`, `RuleId`,
/// `ActionId`), so nothing requires uniqueness *across* families. Separate
/// counters keep concept ids dense — which is what makes the storage
/// zone maps selective (`STORAGE.md` H15) — and quadruple the headroom under
/// the format's 2³² concept limit (D6). Stores written before this split
/// simply carry gaps; `observe_*` on replay keeps every counter above what
/// is on disk.
#[derive(Debug, Default)]
pub struct IdAllocator {
    concepts: AtomicU64,
    relations: AtomicU64,
    rules: AtomicU64,
    actions: AtomicU64,
}

impl IdAllocator {
    /// Every family starts at `start` (1 in practice; 0 means "unset").
    pub fn new(start: u64) -> Self {
        Self {
            concepts: AtomicU64::new(start),
            relations: AtomicU64::new(start),
            rules: AtomicU64::new(start),
            actions: AtomicU64::new(start),
        }
    }
    pub fn next_concept(&self) -> ConceptId {
        ConceptId(self.concepts.fetch_add(1, Ordering::Relaxed))
    }
    pub fn next_relation(&self) -> RelationId {
        RelationId(self.relations.fetch_add(1, Ordering::Relaxed))
    }
    pub fn next_rule(&self) -> RuleId {
        RuleId(self.rules.fetch_add(1, Ordering::Relaxed))
    }
    pub fn next_action(&self) -> ActionId {
        ActionId(self.actions.fetch_add(1, Ordering::Relaxed))
    }
    /// Next value of each family — the id the next allocation would return.
    pub fn watermarks(&self) -> IdWatermarks {
        IdWatermarks {
            concepts: self.concepts.load(Ordering::Relaxed),
            relations: self.relations.load(Ordering::Relaxed),
            rules: self.rules.load(Ordering::Relaxed),
            actions: self.actions.load(Ordering::Relaxed),
        }
    }
    /// Reset every family so the next allocated id is `start`.
    pub fn reset(&self, start: u64) {
        for c in [&self.concepts, &self.relations, &self.rules, &self.actions] {
            c.store(start, Ordering::Release);
        }
    }
    pub fn observe_concept(&self, id: ConceptId) {
        Self::observe(&self.concepts, id.0);
    }
    pub fn observe_relation(&self, id: RelationId) {
        Self::observe(&self.relations, id.0);
    }
    pub fn observe_rule(&self, id: RuleId) {
        Self::observe(&self.rules, id.0);
    }
    pub fn observe_action(&self, id: ActionId) {
        Self::observe(&self.actions, id.0);
    }
    /// Bump `counter` so future allocations don't collide with an id
    /// restored from disk.
    fn observe(counter: &AtomicU64, value: u64) {
        let mut current = counter.load(Ordering::Relaxed);
        while value >= current {
            match counter.compare_exchange_weak(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_allocate_independently() {
        let ids = IdAllocator::new(1);
        assert_eq!(ids.next_concept(), ConceptId(1));
        assert_eq!(ids.next_relation(), RelationId(1));
        assert_eq!(ids.next_concept(), ConceptId(2));
        assert_eq!(ids.next_rule(), RuleId(1));
        assert_eq!(ids.next_action(), ActionId(1));
        assert_eq!(
            ids.watermarks(),
            IdWatermarks {
                concepts: 3,
                relations: 2,
                rules: 2,
                actions: 2
            }
        );
    }

    #[test]
    fn observe_only_raises_its_own_family() {
        let ids = IdAllocator::new(1);
        ids.observe_relation(RelationId(500));
        assert_eq!(ids.next_relation(), RelationId(501));
        assert_eq!(ids.next_concept(), ConceptId(1), "concepts untouched");
        ids.observe_concept(ConceptId(3));
        ids.observe_concept(ConceptId(2)); // lower than current: no-op
        assert_eq!(ids.next_concept(), ConceptId(4));
    }

    #[test]
    fn reset_restarts_every_family() {
        let ids = IdAllocator::new(1);
        ids.next_concept();
        ids.next_action();
        ids.reset(1);
        assert_eq!(ids.next_concept(), ConceptId(1));
        assert_eq!(ids.next_action(), ActionId(1));
    }

    #[test]
    fn concept_id_storage_limit() {
        assert!(ConceptId(MAX_CONCEPT_ID).fits_storage());
        assert!(!ConceptId(MAX_CONCEPT_ID + 1).fits_storage());
    }
}
