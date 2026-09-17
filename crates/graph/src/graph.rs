// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

use ahash::AHashMap;
use dashmap::DashMap;
use parking_lot::{Mutex, RwLock};
use smallvec::SmallVec;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crate::error::{GraphError, GraphResult};
use crate::id::{ActionId, ConceptId, IdAllocator, RelationId, RuleId};
use crate::model::{
    Action, ActionPatch, Concept, ConceptPatch, Relation, RelationPatch, Rule, RulePatch,
};
use crate::schema::Ontology;

type AdjList = SmallVec<[RelationId; 4]>;

/// Yield every 3-character window of `s` in lowercase, deduplicated.
/// Walks `chars()` so the windows are codepoint-aligned (works for accents,
/// CJK, etc., not just ASCII). Returns an empty vec if `s` has fewer than 3
/// characters — callers fall back to a linear scan in that regime.
fn trigrams(s: &str) -> Vec<[char; 3]> {
    let chars: Vec<char> = s.chars().flat_map(char::to_lowercase).collect();
    if chars.len() < 3 {
        return Vec::new();
    }
    let mut out: Vec<[char; 3]> = Vec::with_capacity(chars.len() - 2);
    for w in chars.windows(3) {
        out.push([w[0], w[1], w[2]]);
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Ordered key for the concept sort index: `(concept_type, name, id)`.
/// Iteration yields the same total order used by `list_concepts_page`,
/// so filtering + pagination can stream without materializing the world.
type ConceptKey = (String, String, ConceptId);
type RuleKey = (String, String, RuleId);
type ActionKey = (String, String, ActionId);

fn ms(d: std::time::Duration) -> f64 {
    (d.as_secs_f64() * 1e5).round() / 100.0
}

/// What [`OntologyGraph::end_bulk`] rebuilt, and how long each derived
/// index took — the per-index profile `STORAGE-PLAN.md` §6 asked for
/// before optimising further.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BulkLoadReport {
    pub concepts: usize,
    pub relations: usize,
    pub rules: usize,
    pub actions: usize,
    /// Distinct name trigrams indexed.
    pub trigrams: usize,
    pub concepts_sorted_ms: f64,
    pub concepts_by_type_ms: f64,
    pub trigrams_ms: f64,
    pub relations_sorted_ms: f64,
    pub total_ms: f64,
}

/// Guard of a bulk load (see [`OntologyGraph::begin_bulk`]). Dropping it
/// rebuilds the derived indexes; [`finish`](Self::finish) does the same and
/// hands back the report.
#[must_use = "dropping the guard ends the bulk load immediately"]
pub struct BulkLoad<'a> {
    graph: &'a OntologyGraph,
    /// Only the guard that switched the mode on switches it off.
    owner: bool,
    finished: bool,
}

impl BulkLoad<'_> {
    /// Rebuild the derived indexes now and return what was rebuilt.
    pub fn finish(mut self) -> BulkLoadReport {
        self.finished = true;
        if self.owner {
            self.graph.end_bulk()
        } else {
            BulkLoadReport::default()
        }
    }
}

impl Drop for BulkLoad<'_> {
    fn drop(&mut self) {
        if !self.finished && self.owner {
            self.graph.end_bulk();
        }
    }
}

/// Cache entry for [`OntologyGraph::list_concepts_page`].
/// Stored under the same `concepts_gen` snapshot that was current when the
/// page was built; a write bumps the generation and silently invalidates
/// every entry without us having to walk the map.
#[derive(Clone, Debug)]
struct ListConceptsCacheEntry {
    gen: u64,
    total: usize,
    page: Vec<Concept>,
}

type ListConceptsCacheKey = (
    Option<String>, // concept_type
    Option<String>, // name needle (lowercased)
    usize,          // offset
    usize,          // limit
    bool,           // track_total
    bool,           // include_subtypes
);

const LIST_CONCEPTS_CACHE_CAP: usize = 256;

#[derive(Clone, Debug)]
struct ListRelationsCacheEntry {
    gen: u64,
    total: usize,
    page: Vec<Relation>,
}

type ListRelationsCacheKey = (
    Option<ConceptId>, // source
    Option<ConceptId>, // target
    Option<String>,    // relation_type
    usize,             // offset
    usize,             // limit
    bool,              // track_total
);

const LIST_RELATIONS_CACHE_CAP: usize = 256;

/// Upper bound on the page buffer pre-allocated from a caller-supplied
/// `limit`. The HTTP layer caps `limit` at 1000, but the graph API must not
/// abort the process (`Vec::with_capacity` overflow / OOM) when handed a
/// huge one (`STORAGE.md` R17: fail explicitly, never get killed).
const LIST_PREALLOC_CAP: usize = 1024;

/// In-memory ontology graph. Built for high read concurrency: lookups go
/// through `DashMap`s (sharded, lock-free reads) while edge index updates
/// take a single short write lock. Ordered listings are served from
/// `BTreeSet` side-indexes that mirror the primary maps.
#[derive(Debug)]
pub struct OntologyGraph {
    ontology: RwLock<Ontology>,
    concepts: DashMap<ConceptId, Concept>,
    rules: DashMap<RuleId, Rule>,
    actions: DashMap<ActionId, Action>,
    relations: DashMap<RelationId, Relation>,
    /// (concept_type, lowercased name) -> id, for natural-language lookup.
    name_index: DashMap<(String, String), ConceptId>,
    out_edges: DashMap<ConceptId, AdjList>,
    in_edges: DashMap<ConceptId, AdjList>,
    /// Typed adjacency, à la Neo4j relationship-type chains: per-node
    /// per-`relation_type` buckets so `list_relations?source=X&type=T`
    /// scans only the edges of that exact type, not every edge of `X`.
    out_edges_typed: DashMap<ConceptId, AHashMap<String, AdjList>>,
    in_edges_typed: DashMap<ConceptId, AHashMap<String, AdjList>>,
    /// Ordered side-indexes. Reads iterate these under a short read lock so
    /// list endpoints can paginate without cloning every entity.
    concepts_sorted: RwLock<BTreeSet<ConceptKey>>,
    /// Label index, à la Neo4j: per `concept_type` ordered set of
    /// `(name, id)`. Filtering by type short-circuits the global scan.
    concepts_by_type: DashMap<String, BTreeSet<(String, ConceptId)>>,
    /// Trigram inverted index over lowercased concept names (à la
    /// pg_trgm / Elasticsearch ngram). Maps a 3-char window to the set of
    /// concept ids whose name contains it. Substring queries of ≥3 chars
    /// intersect candidate sets here instead of scanning every concept.
    name_trigrams: RwLock<AHashMap<[char; 3], BTreeSet<ConceptId>>>,
    /// Monotonic generation counter bumped on every concept-mutating call.
    /// Read-side caches snapshot it and treat any mismatch as an invalidation,
    /// so we never have to walk the cache to evict entries on write.
    concepts_gen: AtomicU64,
    /// Memoized pages produced by `list_concepts_page`. Bounded; on overflow
    /// we clear the whole map (cheaper than LRU bookkeeping and acceptable
    /// since pages get rebuilt cheaply from the indexes anyway).
    list_concepts_cache: Mutex<AHashMap<ListConceptsCacheKey, ListConceptsCacheEntry>>,
    /// Symmetric generation+cache pair for `list_relations_page`. Bumped by
    /// any relation-mutating call (and by `remove_concept`, which cascades).
    relations_gen: AtomicU64,
    list_relations_cache: Mutex<AHashMap<ListRelationsCacheKey, ListRelationsCacheEntry>>,
    relations_sorted: RwLock<BTreeSet<RelationId>>,
    rules_sorted: RwLock<BTreeSet<RuleKey>>,
    actions_sorted: RwLock<BTreeSet<ActionKey>>,
    ids: IdAllocator,
    /// Bulk-load mode (`begin_bulk`): the derived indexes above — sorted
    /// sets, per-type buckets, trigrams, list caches, generations — are not
    /// maintained per mutation; `end_bulk` rebuilds them once from the
    /// primary maps. Primary maps, the name index and adjacency stay live,
    /// so validation and cascades keep working during the load.
    bulk: AtomicBool,
}

impl OntologyGraph {
    pub fn new(ontology: Ontology) -> Self {
        Self {
            ontology: RwLock::new(ontology),
            concepts: DashMap::new(),
            rules: DashMap::new(),
            actions: DashMap::new(),
            relations: DashMap::new(),
            name_index: DashMap::new(),
            out_edges: DashMap::new(),
            in_edges: DashMap::new(),
            out_edges_typed: DashMap::new(),
            in_edges_typed: DashMap::new(),
            concepts_sorted: RwLock::new(BTreeSet::new()),
            concepts_by_type: DashMap::new(),
            name_trigrams: RwLock::new(AHashMap::new()),
            concepts_gen: AtomicU64::new(0),
            list_concepts_cache: Mutex::new(AHashMap::new()),
            relations_gen: AtomicU64::new(0),
            list_relations_cache: Mutex::new(AHashMap::new()),
            relations_sorted: RwLock::new(BTreeSet::new()),
            rules_sorted: RwLock::new(BTreeSet::new()),
            actions_sorted: RwLock::new(BTreeSet::new()),
            ids: IdAllocator::new(1),
            bulk: AtomicBool::new(false),
        }
    }

    pub fn with_arc(ontology: Ontology) -> Arc<Self> {
        Arc::new(Self::new(ontology))
    }

    /// Called by every concept-mutating operation. Bumps the generation
    /// counter and clears the list cache so the next reader rebuilds from
    /// fresh indexes.
    /// Current monotonic generation of the concept set. Bumped on every
    /// concept-mutating call. Used by HTTP handlers to derive an ETag for
    /// conditional GET.
    pub fn concepts_generation(&self) -> u64 {
        self.concepts_gen.load(Ordering::Acquire)
    }

    /// Current monotonic generation of the relation set.
    pub fn relations_generation(&self) -> u64 {
        self.relations_gen.load(Ordering::Acquire)
    }

    /// `true` when derived indexes are maintained per mutation (normal
    /// mode); `false` inside a bulk load, where `end_bulk` rebuilds them.
    #[inline]
    fn derived(&self) -> bool {
        !self.bulk.load(Ordering::Relaxed)
    }

    fn bump_concepts_gen(&self) {
        if !self.derived() {
            return;
        }
        self.concepts_gen.fetch_add(1, Ordering::Release);
        self.list_concepts_cache.lock().clear();
    }

    fn bump_relations_gen(&self) {
        if !self.derived() {
            return;
        }
        self.relations_gen.fetch_add(1, Ordering::Release);
        self.list_relations_cache.lock().clear();
    }

    fn index_name_trigrams(&self, name: &str, id: ConceptId) {
        if !self.derived() {
            return;
        }
        let grams = trigrams(name);
        if grams.is_empty() {
            return;
        }
        let mut idx = self.name_trigrams.write();
        for g in grams {
            idx.entry(g).or_default().insert(id);
        }
    }

    fn deindex_name_trigrams(&self, name: &str, id: ConceptId) {
        if !self.derived() {
            return;
        }
        let grams = trigrams(name);
        if grams.is_empty() {
            return;
        }
        let mut idx = self.name_trigrams.write();
        for g in grams {
            if let Some(bucket) = idx.get_mut(&g) {
                bucket.remove(&id);
                if bucket.is_empty() {
                    idx.remove(&g);
                }
            }
        }
    }

    /// Enter bulk-load mode (`STORAGE-PLAN.md` phase 4, item 4;
    /// `PERFORMANCE.md` §7.8). Every public mutation keeps working — primary
    /// maps, the name index and adjacency are maintained, so validation,
    /// upserts, deletes and cascades behave exactly as in normal mode — but
    /// the derived indexes (sorted sets, per-type buckets, name trigrams,
    /// list caches, generations) are left alone until the returned guard is
    /// finished or dropped, when [`end_bulk`](Self::end_bulk) rebuilds them
    /// in one pass and bumps each generation once (R1, R2).
    ///
    /// Intended for hydration, before the graph is served: **readers that
    /// go through the derived indexes (list pages, `?q=`, type buckets)
    /// see stale results while the guard is alive.** Nested calls are
    /// harmless — only the outermost guard rebuilds.
    pub fn begin_bulk(&self) -> BulkLoad<'_> {
        let owner = !self.bulk.swap(true, Ordering::AcqRel);
        BulkLoad {
            graph: self,
            owner,
            finished: false,
        }
    }

    /// `true` while a bulk load is in progress.
    pub fn is_bulk(&self) -> bool {
        !self.derived()
    }

    /// Leave bulk-load mode: rebuild every derived index from the primary
    /// maps and bump both generations once. A no-op when not in bulk mode.
    /// Prefer the guard returned by [`begin_bulk`](Self::begin_bulk).
    pub fn end_bulk(&self) -> BulkLoadReport {
        if !self.bulk.swap(false, Ordering::AcqRel) {
            return BulkLoadReport::default();
        }
        let started = Instant::now();
        let mut report = BulkLoadReport::default();

        // Concepts: one pass collects the three derived views.
        let t = Instant::now();
        let mut keys: Vec<ConceptKey> = Vec::with_capacity(self.concepts.len());
        let mut by_type: AHashMap<String, Vec<(String, ConceptId)>> = AHashMap::new();
        let mut grams: AHashMap<[char; 3], Vec<ConceptId>> = AHashMap::new();
        for c in self.concepts.iter() {
            let id = *c.key();
            keys.push((c.concept_type.clone(), c.name.clone(), id));
            by_type
                .entry(c.concept_type.clone())
                .or_default()
                .push((c.name.clone(), id));
            for g in trigrams(&c.name) {
                grams.entry(g).or_default().push(id);
            }
        }
        report.concepts = keys.len();
        keys.sort_unstable();
        *self.concepts_sorted.write() = keys.into_iter().collect();
        report.concepts_sorted_ms = ms(t.elapsed());

        let t = Instant::now();
        self.concepts_by_type.clear();
        for (ty, mut names) in by_type {
            names.sort_unstable();
            self.concepts_by_type
                .insert(ty, names.into_iter().collect());
        }
        report.concepts_by_type_ms = ms(t.elapsed());

        let t = Instant::now();
        let mut idx: AHashMap<[char; 3], BTreeSet<ConceptId>> =
            AHashMap::with_capacity(grams.len());
        for (g, mut ids) in grams {
            ids.sort_unstable();
            ids.dedup();
            idx.insert(g, ids.into_iter().collect());
        }
        report.trigrams = idx.len();
        *self.name_trigrams.write() = idx;
        report.trigrams_ms = ms(t.elapsed());

        // Relations, rules, actions: sorted views from the primary maps.
        let t = Instant::now();
        let mut rel_ids: Vec<RelationId> = self.relations.iter().map(|r| *r.key()).collect();
        report.relations = rel_ids.len();
        rel_ids.sort_unstable();
        *self.relations_sorted.write() = rel_ids.into_iter().collect();
        let mut rule_keys: Vec<RuleKey> = self
            .rules
            .iter()
            .map(|r| (r.rule_type.clone(), r.name.clone(), *r.key()))
            .collect();
        report.rules = rule_keys.len();
        rule_keys.sort_unstable();
        *self.rules_sorted.write() = rule_keys.into_iter().collect();
        let mut action_keys: Vec<ActionKey> = self
            .actions
            .iter()
            .map(|a| (a.action_type.clone(), a.name.clone(), *a.key()))
            .collect();
        report.actions = action_keys.len();
        action_keys.sort_unstable();
        *self.actions_sorted.write() = action_keys.into_iter().collect();
        report.relations_sorted_ms = ms(t.elapsed());

        // One generation bump per family (R2): every cached page is stale.
        self.bump_concepts_gen();
        self.bump_relations_gen();
        report.total_ms = ms(started.elapsed());
        report
    }

    pub fn ontology(&self) -> Ontology {
        self.ontology.read().clone()
    }

    /// Run `f` against the live ontology under a short read lock, without
    /// cloning it. Prefer this over [`ontology`](Self::ontology) on hot paths
    /// that only need to look something up. `f` must not call back into
    /// methods that take the ontology write lock.
    pub fn with_ontology<R>(&self, f: impl FnOnce(&Ontology) -> R) -> R {
        let g = self.ontology.read();
        f(&g)
    }

    /// Mutate the ontology **atomically**: `f` runs on a copy, the result is
    /// validated (domain rules, no domain change under existing instances),
    /// and only then replaces the live schema. If `f` or a validation fails
    /// the live ontology is untouched — R1's single write door for the
    /// schema.
    pub fn extend_ontology<F>(&self, f: F) -> GraphResult<()>
    where
        F: FnOnce(&mut Ontology) -> GraphResult<()>,
    {
        let mut g = self.ontology.write();
        let mut candidate = g.clone();
        f(&mut candidate)?;
        self.validate_candidate(&g, &candidate)?;
        *g = candidate;
        Ok(())
    }

    /// Would `candidate` be accepted as the new schema? Same checks as
    /// [`extend_ontology`](Self::extend_ontology), nothing applied. Callers
    /// that journal the schema before applying it (R8) must call this first,
    /// or a refused schema ends up on disk and breaks the next replay.
    pub fn check_ontology(&self, candidate: &Ontology) -> GraphResult<()> {
        let g = self.ontology.read();
        self.validate_candidate(&g, candidate)
    }

    /// Schema transition rules (STORAGE-PLAN.md §5.1, hardened after the
    /// phase 3 review): domain identifiers valid, a child in its parent's
    /// domain, and nothing that has instances may move, change shape or
    /// disappear — the records on disk would otherwise be routed, validated
    /// or replayed against a schema that no longer describes them.
    fn validate_candidate(&self, old: &Ontology, candidate: &Ontology) -> GraphResult<()> {
        candidate.validate_hierarchy()?;
        candidate.validate_namespaces()?;
        // Concept types: no domain move, no removal, while instances exist.
        for name in old.concept_types.keys() {
            let instances = self
                .concepts_by_type
                .get(name)
                .map(|b| b.len())
                .unwrap_or(0);
            if instances == 0 {
                continue;
            }
            if !candidate.concept_types.contains_key(name) {
                return Err(GraphError::TypeInUse {
                    kind: "concept",
                    name: name.clone(),
                    instances,
                });
            }
            let (from, to) = (old.ns_of_type(name), candidate.ns_of_type(name));
            if from != to {
                return Err(GraphError::NamespaceChangeWithInstances {
                    concept_type: name.clone(),
                    from: from.to_string(),
                    to: to.to_string(),
                    instances,
                });
            }
        }
        // Relation types: no removal, no domain/range change, while
        // relations of that type exist (counted lazily — schema edits are
        // rare admin operations).
        for (name, rt) in old.relation_types.iter() {
            let changed = match candidate.relation_types.get(name) {
                None => true,
                Some(new) => new.domain != rt.domain || new.range != rt.range,
            };
            if !changed {
                continue;
            }
            let instances = self
                .relations
                .iter()
                .filter(|r| r.relation_type == *name)
                .count();
            if instances > 0 {
                return Err(if candidate.relation_types.contains_key(name) {
                    GraphError::RelationTypeChangeWithInstances {
                        relation_type: name.clone(),
                        instances,
                    }
                } else {
                    GraphError::TypeInUse {
                        kind: "relation",
                        name: name.clone(),
                        instances,
                    }
                });
            }
        }
        // Rule and action types: no removal while instances exist.
        for name in old.rule_types.keys() {
            if candidate.rule_types.contains_key(name) {
                continue;
            }
            let instances = self.rules.iter().filter(|r| r.rule_type == *name).count();
            if instances > 0 {
                return Err(GraphError::TypeInUse {
                    kind: "rule",
                    name: name.clone(),
                    instances,
                });
            }
        }
        for name in old.action_types.keys() {
            if candidate.action_types.contains_key(name) {
                continue;
            }
            let instances = self
                .actions
                .iter()
                .filter(|a| a.action_type == *name)
                .count();
            if instances > 0 {
                return Err(GraphError::TypeInUse {
                    kind: "action",
                    name: name.clone(),
                    instances,
                });
            }
        }
        Ok(())
    }

    /// Storage domain of a concept type (see `Ontology::ns_of_type`).
    pub fn ns_of_type(&self, concept_type: &str) -> String {
        self.ontology.read().ns_of_type(concept_type).to_string()
    }

    /// Remove every concept, relation, rule and action from the graph,
    /// leaving the ontology schema untouched. The id allocator is reset so
    /// subsequent inserts start from id 1 again.
    pub fn clear_instances(&self) {
        self.concepts.clear();
        self.rules.clear();
        self.actions.clear();
        self.relations.clear();
        self.name_index.clear();
        self.out_edges.clear();
        self.in_edges.clear();
        self.out_edges_typed.clear();
        self.in_edges_typed.clear();
        self.concepts_sorted.write().clear();
        self.concepts_by_type.clear();
        self.name_trigrams.write().clear();
        self.bump_concepts_gen();
        self.bump_relations_gen();
        self.relations_sorted.write().clear();
        self.rules_sorted.write().clear();
        self.actions_sorted.write().clear();
        self.ids.reset(1);
    }

    // ---------- concepts ----------

    /// Validate a concept's *contents* against the schema: known type,
    /// allowed and required properties, disjoint-type name clashes. Pure —
    /// touches no index and allocates nothing.
    fn validate_concept_schema(&self, concept: &Concept) -> GraphResult<()> {
        let onto = self.ontology.read();
        let ct = onto.concept_type(&concept.concept_type)?;
        if let Some(allowed) = &ct.properties {
            for k in concept.properties.keys() {
                if !allowed.iter().any(|a| a == k) {
                    return Err(GraphError::InvalidProperty {
                        property: k.clone(),
                        concept_type: ct.name.clone(),
                    });
                }
            }
        }
        for req in &ct.required_properties {
            if !concept.properties.contains_key(req) {
                return Err(GraphError::MissingRequiredProperty {
                    property: req.clone(),
                    concept_type: ct.name.clone(),
                });
            }
        }
        // Disjoint-with: same lowercase name already used under a sibling
        // type → reject.
        let lname = concept.name.to_lowercase();
        for other in &ct.disjoint_with {
            if self
                .name_index
                .get(&(other.clone(), lname.clone()))
                .is_some()
            {
                return Err(GraphError::DisjointTypeViolation {
                    type_a: ct.name.clone(),
                    type_b: other.clone(),
                });
            }
        }
        Ok(())
    }

    /// Validate `concept` against the schema and the live graph, and
    /// allocate its id when `concept.id == ConceptId(0)` — **without
    /// inserting anything**.
    ///
    /// This is the first half of the write-ahead sequence mandated by
    /// `STORAGE.md` R8: `prepare_concept` → durable append →
    /// [`apply_prepared_concept`](Self::apply_prepared_concept). Everything
    /// that can reject the concept is checked here, so that once the record
    /// is on disk the apply step cannot fail for a semantic reason.
    ///
    /// The allocated id is consumed even if the caller never applies the
    /// concept (e.g. the append fails). Ids are only required to be unique,
    /// so a gap is harmless.
    pub fn prepare_concept(&self, concept: &mut Concept) -> GraphResult<()> {
        self.validate_concept_schema(concept)?;
        let key = (concept.concept_type.clone(), concept.name.to_lowercase());
        if let Some(existing) = self.name_index.get(&key) {
            // A fresh concept (id 0) can never legitimately share a name; an
            // explicit id may only if it *is* the existing concept (upsert).
            if concept.id.0 == 0 || *existing != concept.id {
                return Err(GraphError::DuplicateConcept(
                    concept.name.clone(),
                    concept.concept_type.clone(),
                ));
            }
        }
        if concept.id.0 == 0 {
            concept.id = self.ids.next_concept();
        } else {
            // An explicit id is an upsert; the type is immutable (H5).
            if let Some(prev) = self.concepts.get(&concept.id) {
                if prev.concept_type != concept.concept_type {
                    return Err(GraphError::ImmutableConceptType(
                        prev.concept_type.clone(),
                        concept.concept_type.clone(),
                    ));
                }
            }
            // Refuse *before* observing: an out-of-range explicit id must
            // not raise the watermark, or every later fresh allocation
            // would be out of range too.
            if !concept.id.fits_storage() {
                return Err(GraphError::ConceptIdOutOfRange(concept.id));
            }
            self.ids.observe_concept(concept.id);
        }
        // Invariant of the storage format (STORAGE.md D6), checked where the
        // id is born rather than assumed downstream.
        if !concept.id.fits_storage() {
            return Err(GraphError::ConceptIdOutOfRange(concept.id));
        }
        Ok(())
    }

    /// Insert a concept previously validated by
    /// [`prepare_concept`](Self::prepare_concept). Second half of the R8
    /// sequence; call it only after the matching log record is durable.
    ///
    /// The duplicate-name check is re-run defensively (it is O(1)) so a
    /// caller that skips `prepare_concept` still cannot corrupt the name
    /// index. `concept.id` must be non-zero.
    pub fn apply_prepared_concept(&self, concept: Concept) -> GraphResult<ConceptId> {
        if concept.id.0 == 0 {
            return Err(GraphError::NotPrepared("concept"));
        }
        let key = (concept.concept_type.clone(), concept.name.to_lowercase());
        if let Some(existing) = self.name_index.get(&key) {
            if *existing != concept.id {
                return Err(GraphError::DuplicateConcept(
                    concept.name.clone(),
                    concept.concept_type.clone(),
                ));
            }
        }
        if let Some(prev) = self.concepts.get(&concept.id) {
            if prev.concept_type != concept.concept_type {
                return Err(GraphError::ImmutableConceptType(
                    prev.concept_type.clone(),
                    concept.concept_type.clone(),
                ));
            }
            // Upsert with a rename: the old (type, name) binding must go,
            // or the old name keeps resolving to this id.
            let prev_key = (prev.concept_type.clone(), prev.name.to_lowercase());
            if prev_key != key {
                self.name_index.remove(&prev_key);
            }
        }
        self.ids.observe_concept(concept.id);
        self.name_index.insert(key, concept.id);
        let id = concept.id;
        let sort_key = (concept.concept_type.clone(), concept.name.clone(), id);
        let new_name = concept.name.clone();
        let prev = self.concepts.insert(id, concept);
        if self.derived() {
            if let Some(prev) = prev {
                let old = (prev.concept_type.clone(), prev.name.clone(), id);
                if old != sort_key {
                    self.concepts_sorted.write().remove(&old);
                    if let Some(mut bucket) = self.concepts_by_type.get_mut(&prev.concept_type) {
                        bucket.remove(&(prev.name.clone(), id));
                    }
                    self.deindex_name_trigrams(&prev.name, id);
                }
            }
            self.concepts_sorted.write().insert(sort_key.clone());
            self.concepts_by_type
                .entry(sort_key.0)
                .or_default()
                .insert((sort_key.1, id));
            self.index_name_trigrams(&new_name, id);
            self.bump_concepts_gen();
        }
        Ok(id)
    }

    /// Insert a concept. Allocates an id if `concept.id == ConceptId(0)`.
    ///
    /// Equivalent to [`prepare_concept`](Self::prepare_concept) immediately
    /// followed by [`apply_prepared_concept`](Self::apply_prepared_concept).
    /// Use the two-step form when a durable append must sit in between.
    pub fn upsert_concept(&self, mut concept: Concept) -> GraphResult<ConceptId> {
        self.prepare_concept(&mut concept)?;
        self.apply_prepared_concept(concept)
    }

    pub fn get_concept(&self, id: ConceptId) -> GraphResult<Concept> {
        self.concepts
            .get(&id)
            .map(|c| c.clone())
            .ok_or(GraphError::UnknownConcept(id))
    }

    pub fn find_by_name(&self, concept_type: &str, name: &str) -> Option<ConceptId> {
        self.name_index
            .get(&(concept_type.to_string(), name.to_lowercase()))
            .map(|v| *v)
    }

    pub fn concept_count(&self) -> usize {
        self.concepts.len()
    }
    pub fn relation_count(&self) -> usize {
        self.relations.len()
    }

    pub fn all_concepts(&self) -> Vec<Concept> {
        self.concepts.iter().map(|e| e.value().clone()).collect()
    }

    // ---------- relations ----------

    /// Validate `rel` against the schema and the live graph (endpoints exist,
    /// domain/range, cardinality, functional) and allocate its id when
    /// `rel.id == RelationId(0)` — **without inserting anything**.
    ///
    /// First half of the R8 write-ahead sequence; see
    /// [`prepare_concept`](Self::prepare_concept). A caller-supplied id that
    /// collides with an existing relation is reassigned (this happens when
    /// re-ingesting an export whose explicit ids collide with materialized
    /// symmetric inverses).
    pub fn prepare_relation(&self, rel: &mut Relation) -> GraphResult<()> {
        let src = self
            .concepts
            .get(&rel.source)
            .ok_or(GraphError::UnknownConcept(rel.source))?;
        let tgt = self
            .concepts
            .get(&rel.target)
            .ok_or(GraphError::UnknownConcept(rel.target))?;
        {
            let onto = self.ontology.read();
            onto.validate_edge(&rel.relation_type, &src.concept_type, &tgt.concept_type)?;
        }
        drop(src);
        drop(tgt);
        // Endpoints are packed as (source << 32) | target on disk (D6).
        for end in [rel.source, rel.target] {
            if !end.fits_storage() {
                return Err(GraphError::ConceptIdOutOfRange(end));
            }
        }
        let rt = self
            .ontology
            .read()
            .relation_type(&rel.relation_type)
            .cloned()?;

        // Cardinality + functional enforcement. Counted before id assignment
        // so the rejection path costs nothing extra. The materialized inverse
        // for symmetric relations is pushed directly to the adjacency map
        // without going through add_relation, so it doesn't trip these checks.
        use crate::schema::Cardinality;
        let limits_out = matches!(
            rt.cardinality,
            Cardinality::OneToOne | Cardinality::ManyToOne
        ) || rt.functional;
        let limits_in = matches!(
            rt.cardinality,
            Cardinality::OneToOne | Cardinality::OneToMany
        );
        if limits_out {
            if let Some(by_type) = self.out_edges_typed.get(&rel.source) {
                if let Some(adj) = by_type.get(&rel.relation_type) {
                    if !adj.is_empty() {
                        return Err(GraphError::CardinalityViolation {
                            relation: rel.relation_type.clone(),
                            concept: rel.source,
                        });
                    }
                }
            }
        }
        if limits_in {
            if let Some(by_type) = self.in_edges_typed.get(&rel.target) {
                if let Some(adj) = by_type.get(&rel.relation_type) {
                    if !adj.is_empty() {
                        return Err(GraphError::CardinalityViolation {
                            relation: rel.relation_type.clone(),
                            concept: rel.target,
                        });
                    }
                }
            }
        }

        if rel.id.0 == 0 {
            rel.id = self.ids.next_relation();
        } else if self.relations.contains_key(&rel.id) {
            // Caller-supplied id collides with an existing relation. This
            // happens during snapshot restore / export re-ingest when an
            // explicit id collides with a previously-allocated materialized
            // inverse. Reassign rather than silently overwrite.
            rel.id = self.ids.next_relation();
        } else {
            self.ids.observe_relation(rel.id);
        }
        Ok(())
    }

    /// Insert a relation previously validated by
    /// [`prepare_relation`](Self::prepare_relation). Second half of the R8
    /// sequence; call it only after the matching log record is durable.
    /// Symmetric relation types materialize their inverse edge here.
    /// `rel.id` must be non-zero and must not already be present.
    pub fn apply_prepared_relation(&self, rel: Relation) -> GraphResult<RelationId> {
        if rel.id.0 == 0 {
            return Err(GraphError::NotPrepared("relation"));
        }
        debug_assert!(
            !self.relations.contains_key(&rel.id),
            "apply_prepared_relation: id {} already present — prepare_relation reassigns \
             colliding ids, so a caller bypassed it",
            rel.id
        );
        let symmetric = self
            .ontology
            .read()
            .relation_type(&rel.relation_type)?
            .symmetric;
        self.ids.observe_relation(rel.id);

        let id = rel.id;
        let (s, t) = (rel.source, rel.target);
        self.out_edges.entry(s).or_default().push(id);
        self.in_edges.entry(t).or_default().push(id);
        let rt_name = rel.relation_type.clone();
        self.out_edges_typed
            .entry(s)
            .or_default()
            .entry(rt_name.clone())
            .or_default()
            .push(id);
        self.in_edges_typed
            .entry(t)
            .or_default()
            .entry(rt_name)
            .or_default()
            .push(id);
        self.relations.insert(id, rel);
        if self.derived() {
            self.relations_sorted.write().insert(id);
        }

        if symmetric && s != t {
            // Materialize the inverse so traversals are direction-agnostic.
            let inverse = Relation {
                id: self.ids.next_relation(),
                relation_type: self.relations.get(&id).unwrap().relation_type.clone(),
                source: t,
                target: s,
                weight: self.relations.get(&id).unwrap().weight,
                properties: AHashMap::new(),
            };
            let inv_id = inverse.id;
            self.out_edges.entry(t).or_default().push(inv_id);
            self.in_edges.entry(s).or_default().push(inv_id);
            let inv_type = inverse.relation_type.clone();
            self.out_edges_typed
                .entry(t)
                .or_default()
                .entry(inv_type.clone())
                .or_default()
                .push(inv_id);
            self.in_edges_typed
                .entry(s)
                .or_default()
                .entry(inv_type)
                .or_default()
                .push(inv_id);
            self.relations.insert(inv_id, inverse);
            if self.derived() {
                self.relations_sorted.write().insert(inv_id);
            }
        }
        self.bump_relations_gen();
        Ok(id)
    }

    /// Insert a relation. Allocates an id if `rel.id == RelationId(0)`.
    ///
    /// Equivalent to [`prepare_relation`](Self::prepare_relation) immediately
    /// followed by [`apply_prepared_relation`](Self::apply_prepared_relation).
    pub fn add_relation(&self, mut rel: Relation) -> GraphResult<RelationId> {
        self.prepare_relation(&mut rel)?;
        self.apply_prepared_relation(rel)
    }

    /// Insert a relation **exactly as given**: its id is kept, no symmetric
    /// inverse is materialized, no cardinality check runs. This is the
    /// replay path of `RelationExact` records, which compaction writes for
    /// every live relation (both directions of a symmetric pair, each with
    /// its live id) so that ids on disk and in memory stay equal
    /// (`STORAGE.md` §5). Endpoints must exist and the id must be free.
    pub fn insert_relation_exact(&self, rel: Relation) -> GraphResult<RelationId> {
        if rel.id.0 == 0 {
            return Err(GraphError::NotPrepared("relation"));
        }
        if !self.concepts.contains_key(&rel.source) {
            return Err(GraphError::UnknownConcept(rel.source));
        }
        if !self.concepts.contains_key(&rel.target) {
            return Err(GraphError::UnknownConcept(rel.target));
        }
        if self.relations.contains_key(&rel.id) {
            return Err(GraphError::RelationExists(rel.id));
        }
        self.ids.observe_relation(rel.id);
        let id = rel.id;
        let (s, t) = (rel.source, rel.target);
        let rt_name = rel.relation_type.clone();
        self.out_edges.entry(s).or_default().push(id);
        self.in_edges.entry(t).or_default().push(id);
        self.out_edges_typed
            .entry(s)
            .or_default()
            .entry(rt_name.clone())
            .or_default()
            .push(id);
        self.in_edges_typed
            .entry(t)
            .or_default()
            .entry(rt_name)
            .or_default()
            .push(id);
        self.relations.insert(id, rel);
        if self.derived() {
            self.relations_sorted.write().insert(id);
        }
        self.bump_relations_gen();
        Ok(id)
    }

    pub fn get_relation(&self, id: RelationId) -> GraphResult<Relation> {
        self.relations
            .get(&id)
            .map(|r| r.clone())
            .ok_or(GraphError::UnknownRelation(id))
    }

    pub fn all_relations(&self) -> Vec<Relation> {
        self.relations.iter().map(|e| e.value().clone()).collect()
    }

    /// Compute the relation that `patch` would produce, without applying it.
    /// Pure: the graph is unchanged. Pairs with
    /// [`apply_relation_update`](Self::apply_relation_update) for the R8
    /// write-ahead sequence.
    pub fn preview_relation_update(
        &self,
        id: RelationId,
        patch: &RelationPatch,
    ) -> GraphResult<Relation> {
        let mut rel = self.get_relation(id)?;
        if let Some(w) = patch.weight {
            rel.weight = w;
        }
        if let Some(p) = &patch.properties {
            rel.properties = p.clone();
        }
        Ok(rel)
    }

    /// Replace the mutable fields (`weight`, `properties`) of the relation
    /// `updated.id` with those of `updated`. `source`, `target` and
    /// `relation_type` are immutable (H6) and are ignored.
    pub fn apply_relation_update(&self, updated: Relation) -> GraphResult<Relation> {
        let mut entry = self
            .relations
            .get_mut(&updated.id)
            .ok_or(GraphError::UnknownRelation(updated.id))?;
        entry.weight = updated.weight;
        entry.properties = updated.properties;
        let snapshot = entry.clone();
        drop(entry);
        // R2: the relation set changed — the `list_relations_page` cache
        // and the HTTP ETag both key on this generation.
        self.bump_relations_gen();
        Ok(snapshot)
    }

    /// Apply a partial update to an existing relation. Only `weight` and
    /// `properties` are mutable; the adjacency index is unaffected.
    pub fn update_relation(&self, id: RelationId, patch: RelationPatch) -> GraphResult<Relation> {
        let updated = self.preview_relation_update(id, &patch)?;
        self.apply_relation_update(updated)
    }

    /// Compute the concept that `patch` would produce, fully validated
    /// (schema, duplicate name, disjoint types), without applying it. Pure.
    pub fn preview_concept_update(
        &self,
        id: ConceptId,
        patch: &ConceptPatch,
    ) -> GraphResult<Concept> {
        let mut c = self.get_concept(id)?;
        if let Some(new_name) = &patch.name {
            let new_key = (c.concept_type.clone(), new_name.to_lowercase());
            if let Some(existing) = self.name_index.get(&new_key) {
                if *existing != id {
                    return Err(GraphError::DuplicateConcept(
                        new_name.clone(),
                        c.concept_type.clone(),
                    ));
                }
            }
            c.name = new_name.clone();
        }
        if let Some(d) = &patch.description {
            c.description = d.clone();
        }
        if let Some(props) = &patch.properties {
            c.properties = props.clone();
        }
        self.validate_concept_schema(&c)?;
        Ok(c)
    }

    /// Replace the concept `updated.id` with `updated`, maintaining the
    /// name, sorted, per-type and trigram indexes. The `concept_type` must
    /// be unchanged (H5). Second half of the R8 sequence for updates.
    pub fn apply_concept_update(&self, updated: Concept) -> GraphResult<Concept> {
        let id = updated.id;
        let mut entry = self
            .concepts
            .get_mut(&id)
            .ok_or(GraphError::UnknownConcept(id))?;
        if entry.concept_type != updated.concept_type {
            return Err(GraphError::ImmutableConceptType(
                entry.concept_type.clone(),
                updated.concept_type,
            ));
        }
        if entry.name != updated.name {
            // Maintain (concept_type, lowercase name) → id index.
            let old_key = (entry.concept_type.clone(), entry.name.to_lowercase());
            let new_key = (entry.concept_type.clone(), updated.name.to_lowercase());
            if old_key != new_key {
                if let Some(existing) = self.name_index.get(&new_key) {
                    if *existing != id {
                        return Err(GraphError::DuplicateConcept(
                            updated.name,
                            entry.concept_type.clone(),
                        ));
                    }
                }
                self.name_index.remove(&old_key);
                self.name_index.insert(new_key, id);
            }
            if self.derived() {
                let old_sort = (entry.concept_type.clone(), entry.name.clone(), id);
                let new_sort = (entry.concept_type.clone(), updated.name.clone(), id);
                {
                    let mut idx = self.concepts_sorted.write();
                    idx.remove(&old_sort);
                    idx.insert(new_sort);
                }
                if let Some(mut bucket) = self.concepts_by_type.get_mut(&entry.concept_type) {
                    bucket.remove(&(entry.name.clone(), id));
                    bucket.insert((updated.name.clone(), id));
                }
                self.deindex_name_trigrams(&entry.name, id);
                self.index_name_trigrams(&updated.name, id);
            }
        }
        entry.name = updated.name;
        entry.description = updated.description;
        entry.properties = updated.properties;
        let snapshot = entry.clone();
        drop(entry);
        self.bump_concepts_gen();
        Ok(snapshot)
    }

    /// Apply a partial update to an existing concept. Renaming updates the
    /// name index; clearing description / replacing properties is in-place.
    /// Returns the new concept. The concept's `concept_type` is immutable —
    /// changing types would require revalidating every incident edge.
    ///
    /// Equivalent to [`preview_concept_update`](Self::preview_concept_update)
    /// followed by [`apply_concept_update`](Self::apply_concept_update).
    pub fn update_concept(&self, id: ConceptId, patch: ConceptPatch) -> GraphResult<Concept> {
        let updated = self.preview_concept_update(id, &patch)?;
        self.apply_concept_update(updated)
    }

    /// Ids of every relation incident to `id` (outgoing and incoming,
    /// sorted, deduplicated), or an error when the concept does not exist.
    ///
    /// Read-only companion of [`remove_concept`](Self::remove_concept): it
    /// returns exactly the cascade that `remove_concept` would perform, so a
    /// caller can journal the deletion **before** mutating anything (R8).
    pub fn incident_relation_ids(&self, id: ConceptId) -> GraphResult<Vec<RelationId>> {
        if !self.concepts.contains_key(&id) {
            return Err(GraphError::UnknownConcept(id));
        }
        let mut ids: Vec<RelationId> = Vec::new();
        if let Some(adj) = self.out_edges.get(&id) {
            ids.extend(adj.iter().copied());
        }
        if let Some(adj) = self.in_edges.get(&id) {
            ids.extend(adj.iter().copied());
        }
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    /// Remove a concept and every relation incident to it. Returns the
    /// list of relation ids that were removed alongside the concept so
    /// callers (e.g. WAL) can journal the cascade.
    pub fn remove_concept(&self, id: ConceptId) -> GraphResult<Vec<RelationId>> {
        let concept = self
            .concepts
            .remove(&id)
            .ok_or(GraphError::UnknownConcept(id))?
            .1;
        let key = (concept.concept_type.clone(), concept.name.to_lowercase());
        self.name_index.remove(&key);
        if self.derived() {
            self.concepts_sorted.write().remove(&(
                concept.concept_type.clone(),
                concept.name.clone(),
                id,
            ));
            if let Some(mut bucket) = self.concepts_by_type.get_mut(&concept.concept_type) {
                bucket.remove(&(concept.name.clone(), id));
            }
            self.deindex_name_trigrams(&concept.name, id);
        }

        let mut removed: Vec<RelationId> = Vec::new();
        if let Some((_, adj)) = self.out_edges.remove(&id) {
            for rid in adj {
                removed.push(rid);
            }
        }
        if let Some((_, adj)) = self.in_edges.remove(&id) {
            for rid in adj {
                removed.push(rid);
            }
        }
        self.out_edges_typed.remove(&id);
        self.in_edges_typed.remove(&id);
        removed.sort();
        removed.dedup();

        for rid in &removed {
            if let Some((_, rel)) = self.relations.remove(rid) {
                if self.derived() {
                    self.relations_sorted.write().remove(rid);
                }
                // Scrub the surviving endpoint's adjacency list.
                let other = if rel.source == id {
                    rel.target
                } else {
                    rel.source
                };
                if let Some(mut adj) = self.out_edges.get_mut(&other) {
                    adj.retain(|x| x != rid);
                }
                if let Some(mut adj) = self.in_edges.get_mut(&other) {
                    adj.retain(|x| x != rid);
                }
                if let Some(mut by_type) = self.out_edges_typed.get_mut(&other) {
                    if let Some(adj) = by_type.get_mut(&rel.relation_type) {
                        adj.retain(|x| x != rid);
                    }
                }
                if let Some(mut by_type) = self.in_edges_typed.get_mut(&other) {
                    if let Some(adj) = by_type.get_mut(&rel.relation_type) {
                        adj.retain(|x| x != rid);
                    }
                }
            }
        }
        self.bump_concepts_gen();
        self.bump_relations_gen();
        Ok(removed)
    }

    /// Remove a single relation by id. No-op if already gone.
    pub fn remove_relation(&self, id: RelationId) -> GraphResult<()> {
        let rel = self
            .relations
            .remove(&id)
            .ok_or(GraphError::UnknownRelation(id))?
            .1;
        if self.derived() {
            self.relations_sorted.write().remove(&id);
        }
        if let Some(mut adj) = self.out_edges.get_mut(&rel.source) {
            adj.retain(|x| *x != id);
        }
        if let Some(mut adj) = self.in_edges.get_mut(&rel.target) {
            adj.retain(|x| *x != id);
        }
        if let Some(mut by_type) = self.out_edges_typed.get_mut(&rel.source) {
            if let Some(adj) = by_type.get_mut(&rel.relation_type) {
                adj.retain(|x| *x != id);
            }
        }
        if let Some(mut by_type) = self.in_edges_typed.get_mut(&rel.target) {
            if let Some(adj) = by_type.get_mut(&rel.relation_type) {
                adj.retain(|x| *x != id);
            }
        }
        self.bump_relations_gen();
        Ok(())
    }

    pub fn outgoing(&self, id: ConceptId) -> Vec<Relation> {
        self.out_edges
            .get(&id)
            .map(|adj| {
                adj.iter()
                    .filter_map(|rid| self.relations.get(rid).map(|r| r.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn incoming(&self, id: ConceptId) -> Vec<Relation> {
        self.in_edges
            .get(&id)
            .map(|adj| {
                adj.iter()
                    .filter_map(|rid| self.relations.get(rid).map(|r| r.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Same as [`outgoing`] but restricted to edges whose `relation_type`
    /// is in `types`. Resolved through the typed adjacency index, so the
    /// scan is O(sum of typed-bucket sizes) rather than the full degree.
    pub fn outgoing_typed(&self, id: ConceptId, types: &[String]) -> Vec<Relation> {
        let onto = self.ontology.read();
        let mut out: Vec<Relation> = Vec::new();
        for t in types {
            if let Some(by_type) = self.out_edges_typed.get(&id) {
                if let Some(adj) = by_type.get(t) {
                    for rid in adj.iter() {
                        if let Some(r) = self.relations.get(rid) {
                            out.push(r.clone());
                        }
                    }
                }
            }
            // Virtual edges: if `t` declares an inverse, surface the real
            // type's in-edges as out-edges of `t` with endpoints swapped.
            if let Ok(rt) = onto.relation_type(t) {
                if let Some(real) = &rt.inverse_of {
                    if let Some(by_type) = self.in_edges_typed.get(&id) {
                        if let Some(adj) = by_type.get(real) {
                            for rid in adj.iter() {
                                if let Some(r) = self.relations.get(rid) {
                                    let mut v = r.clone();
                                    v.relation_type = t.clone();
                                    std::mem::swap(&mut v.source, &mut v.target);
                                    out.push(v);
                                }
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// Same as [`incoming`] but restricted to edges whose `relation_type`
    /// is in `types`.
    pub fn incoming_typed(&self, id: ConceptId, types: &[String]) -> Vec<Relation> {
        let onto = self.ontology.read();
        let mut out: Vec<Relation> = Vec::new();
        for t in types {
            if let Some(by_type) = self.in_edges_typed.get(&id) {
                if let Some(adj) = by_type.get(t) {
                    for rid in adj.iter() {
                        if let Some(r) = self.relations.get(rid) {
                            out.push(r.clone());
                        }
                    }
                }
            }
            if let Ok(rt) = onto.relation_type(t) {
                if let Some(real) = &rt.inverse_of {
                    if let Some(by_type) = self.out_edges_typed.get(&id) {
                        if let Some(adj) = by_type.get(real) {
                            for rid in adj.iter() {
                                if let Some(r) = self.relations.get(rid) {
                                    let mut v = r.clone();
                                    v.relation_type = t.clone();
                                    std::mem::swap(&mut v.source, &mut v.target);
                                    out.push(v);
                                }
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// Visit each `(neighbor, RelationId)` reachable from `node` without
    /// cloning the underlying `Relation`. Used by traversal hot paths that
    /// only need ids during the walk and look up entities at the end.
    /// `f` returns `false` to stop early.
    pub fn for_each_neighbor<F>(
        &self,
        node: ConceptId,
        direction: crate::traversal::Direction,
        mut f: F,
    ) where
        F: FnMut(ConceptId, RelationId) -> bool,
    {
        use crate::traversal::Direction;
        let mut keep_going = true;
        let visit = |adj_ref: &AdjList, in_dir: bool, f: &mut F, keep_going: &mut bool| {
            for rid in adj_ref.iter() {
                let Some(r) = self.relations.get(rid) else {
                    continue;
                };
                let other = if in_dir { r.source } else { r.target };
                if !f(other, *rid) {
                    *keep_going = false;
                    return;
                }
            }
        };
        if matches!(direction, Direction::Outgoing | Direction::Both) {
            if let Some(adj) = self.out_edges.get(&node) {
                visit(&adj, false, &mut f, &mut keep_going);
            }
        }
        if keep_going && matches!(direction, Direction::Incoming | Direction::Both) {
            if let Some(adj) = self.in_edges.get(&node) {
                visit(&adj, true, &mut f, &mut keep_going);
            }
        }
    }

    // ---------- rules ----------

    /// Validate a rule and allocate its id when `rule.id == RuleId(0)`,
    /// without inserting. The named `rule_type` and every concept id in
    /// `applies_to` must exist. First half of the R8 sequence.
    pub fn prepare_rule(&self, rule: &mut Rule) -> GraphResult<()> {
        {
            let onto = self.ontology.read();
            if onto.rule_type(&rule.rule_type).is_none() {
                return Err(GraphError::UnknownRelationType(rule.rule_type.clone()));
            }
        }
        for cid in &rule.applies_to {
            if !self.concepts.contains_key(cid) {
                return Err(GraphError::UnknownConcept(*cid));
            }
        }
        if rule.id.0 == 0 {
            rule.id = self.ids.next_rule();
        } else {
            self.ids.observe_rule(rule.id);
        }
        Ok(())
    }

    /// Insert (or replace) a rule previously validated by
    /// [`prepare_rule`](Self::prepare_rule). `rule.id` must be non-zero.
    pub fn apply_prepared_rule(&self, rule: Rule) -> GraphResult<RuleId> {
        if rule.id.0 == 0 {
            return Err(GraphError::NotPrepared("rule"));
        }
        self.ids.observe_rule(rule.id);
        let id = rule.id;
        let sort_key = (rule.rule_type.clone(), rule.name.clone(), id);
        let prev = self.rules.insert(id, rule);
        if self.derived() {
            if let Some(prev) = prev {
                let old = (prev.rule_type, prev.name, id);
                if old != sort_key {
                    self.rules_sorted.write().remove(&old);
                }
            }
            self.rules_sorted.write().insert(sort_key);
        }
        Ok(id)
    }

    /// Insert (or replace) a rule. Allocates an id when `rule.id == RuleId(0)`.
    /// The named `rule_type` and every concept id referenced in `applies_to`
    /// must already exist; otherwise the call fails.
    pub fn upsert_rule(&self, mut rule: Rule) -> GraphResult<RuleId> {
        self.prepare_rule(&mut rule)?;
        self.apply_prepared_rule(rule)
    }

    pub fn get_rule(&self, id: RuleId) -> GraphResult<Rule> {
        self.rules
            .get(&id)
            .map(|r| r.clone())
            .ok_or(GraphError::UnknownRelationType(format!("rule {id}")))
    }

    pub fn all_rules(&self) -> Vec<Rule> {
        self.rules.iter().map(|e| e.value().clone()).collect()
    }

    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    pub fn remove_rule(&self, id: RuleId) -> GraphResult<()> {
        let removed = self
            .rules
            .remove(&id)
            .ok_or(GraphError::UnknownRelationType(format!("rule {id}")))?
            .1;
        if self.derived() {
            self.rules_sorted
                .write()
                .remove(&(removed.rule_type, removed.name, id));
        }
        Ok(())
    }

    /// Compute the rule that `patch` would produce, validated, without
    /// applying it. Pure.
    pub fn preview_rule_update(&self, id: RuleId, patch: &RulePatch) -> GraphResult<Rule> {
        if let Some(applies) = &patch.applies_to {
            for cid in applies {
                if !self.concepts.contains_key(cid) {
                    return Err(GraphError::UnknownConcept(*cid));
                }
            }
        }
        let mut rule = self.get_rule(id)?;
        if let Some(n) = &patch.name {
            rule.name = n.clone();
        }
        if let Some(w) = &patch.when {
            rule.when = w.clone();
        }
        if let Some(t) = &patch.then {
            rule.then = t.clone();
        }
        if let Some(a) = &patch.applies_to {
            rule.applies_to = a.clone();
        }
        if let Some(s) = patch.strict {
            rule.strict = s;
        }
        if let Some(d) = &patch.description {
            rule.description = d.clone();
        }
        if let Some(p) = &patch.properties {
            rule.properties = p.clone();
        }
        Ok(rule)
    }

    /// Replace the rule `updated.id` with `updated`, maintaining the sorted
    /// index. `rule_type` is immutable and taken from the stored rule.
    pub fn apply_rule_update(&self, updated: Rule) -> GraphResult<Rule> {
        let id = updated.id;
        let mut entry = self
            .rules
            .get_mut(&id)
            .ok_or(GraphError::UnknownRelationType(format!("rule {id}")))?;
        if entry.name != updated.name && self.derived() {
            let old_sort = (entry.rule_type.clone(), entry.name.clone(), id);
            let new_sort = (entry.rule_type.clone(), updated.name.clone(), id);
            let mut idx = self.rules_sorted.write();
            idx.remove(&old_sort);
            idx.insert(new_sort);
        }
        entry.name = updated.name;
        entry.when = updated.when;
        entry.then = updated.then;
        entry.applies_to = updated.applies_to;
        entry.strict = updated.strict;
        entry.description = updated.description;
        entry.properties = updated.properties;
        Ok(entry.clone())
    }

    /// Apply a partial update to an existing rule. `rule_type` cannot be
    /// changed; every concept id referenced in a replacement `applies_to`
    /// must already exist.
    pub fn update_rule(&self, id: RuleId, patch: RulePatch) -> GraphResult<Rule> {
        let updated = self.preview_rule_update(id, &patch)?;
        self.apply_rule_update(updated)
    }

    // ---------- actions ----------

    /// Validate an action and allocate its id when `action.id ==
    /// ActionId(0)`, without inserting. The `action_type` must be declared
    /// and both endpoints (`subject`, optional `object`) must exist. First
    /// half of the R8 sequence.
    pub fn prepare_action(&self, action: &mut Action) -> GraphResult<()> {
        {
            let onto = self.ontology.read();
            if onto.action_type(&action.action_type).is_none() {
                return Err(GraphError::UnknownRelationType(action.action_type.clone()));
            }
        }
        if !self.concepts.contains_key(&action.subject) {
            return Err(GraphError::UnknownConcept(action.subject));
        }
        if let Some(obj) = action.object {
            if !self.concepts.contains_key(&obj) {
                return Err(GraphError::UnknownConcept(obj));
            }
        }
        if action.id.0 == 0 {
            action.id = self.ids.next_action();
        } else {
            self.ids.observe_action(action.id);
        }
        Ok(())
    }

    /// Insert (or replace) an action previously validated by
    /// [`prepare_action`](Self::prepare_action). `action.id` must be non-zero.
    pub fn apply_prepared_action(&self, action: Action) -> GraphResult<ActionId> {
        if action.id.0 == 0 {
            return Err(GraphError::NotPrepared("action"));
        }
        self.ids.observe_action(action.id);
        let id = action.id;
        let sort_key = (action.action_type.clone(), action.name.clone(), id);
        let prev = self.actions.insert(id, action);
        if self.derived() {
            if let Some(prev) = prev {
                let old = (prev.action_type, prev.name, id);
                if old != sort_key {
                    self.actions_sorted.write().remove(&old);
                }
            }
            self.actions_sorted.write().insert(sort_key);
        }
        Ok(id)
    }

    /// Insert (or replace) an action. The `action_type` must be declared
    /// in the ontology and both endpoints (`subject`, optional `object`)
    /// must exist as concepts.
    pub fn upsert_action(&self, mut action: Action) -> GraphResult<ActionId> {
        self.prepare_action(&mut action)?;
        self.apply_prepared_action(action)
    }

    pub fn get_action(&self, id: ActionId) -> GraphResult<Action> {
        self.actions
            .get(&id)
            .map(|a| a.clone())
            .ok_or(GraphError::UnknownRelationType(format!("action {id}")))
    }

    pub fn all_actions(&self) -> Vec<Action> {
        self.actions.iter().map(|e| e.value().clone()).collect()
    }

    // ---------- ordered listings ----------

    /// Paginated, ordered listing of concepts. Iterates the
    /// `(concept_type, name)` sort index and only clones entities that
    /// survive the filter and fall inside `[offset, offset+limit)`.
    /// Returns `(total_matching, page)`.
    pub fn list_concepts_page(
        &self,
        concept_type: Option<&str>,
        name_substring_lowercase: Option<&str>,
        offset: usize,
        limit: usize,
        track_total: bool,
        include_subtypes: bool,
    ) -> (usize, Vec<Concept>) {
        let gen = self.concepts_gen.load(Ordering::Acquire);
        let cache_key: ListConceptsCacheKey = (
            concept_type.map(|s| s.to_string()),
            name_substring_lowercase.map(|s| s.to_string()),
            offset,
            limit,
            track_total,
            include_subtypes,
        );
        if let Some(entry) = self.list_concepts_cache.lock().get(&cache_key) {
            if entry.gen == gen {
                return (entry.total, entry.page.clone());
            }
        }

        let (total, page) = self.list_concepts_page_uncached(
            concept_type,
            name_substring_lowercase,
            offset,
            limit,
            track_total,
            include_subtypes,
        );

        let mut cache = self.list_concepts_cache.lock();
        if cache.len() >= LIST_CONCEPTS_CACHE_CAP {
            cache.clear();
        }
        cache.insert(
            cache_key,
            ListConceptsCacheEntry {
                gen,
                total,
                page: page.clone(),
            },
        );
        (total, page)
    }

    fn list_concepts_page_uncached(
        &self,
        concept_type: Option<&str>,
        name_substring_lowercase: Option<&str>,
        offset: usize,
        limit: usize,
        track_total: bool,
        include_subtypes: bool,
    ) -> (usize, Vec<Concept>) {
        // Resolve concept_type filter into the set of accepted type names
        // once, honouring subtype subsumption when requested.
        let type_filter: Option<std::collections::HashSet<String>> = concept_type.map(|t| {
            if include_subtypes {
                self.ontology.read().descendants(t).into_iter().collect()
            } else {
                std::iter::once(t.to_string()).collect()
            }
        });

        if let Some(needle) = name_substring_lowercase {
            let qgrams = trigrams(needle);
            if !qgrams.is_empty() {
                let idx = self.name_trigrams.read();
                let mut sets: Vec<&BTreeSet<ConceptId>> = Vec::with_capacity(qgrams.len());
                for g in &qgrams {
                    match idx.get(g) {
                        Some(s) => sets.push(s),
                        None => return (0, Vec::new()),
                    }
                }
                sets.sort_by_key(|s| s.len());
                let (head, rest) = sets.split_first().unwrap();
                let mut candidates: Vec<ConceptId> = head
                    .iter()
                    .copied()
                    .filter(|id| rest.iter().all(|s| s.contains(id)))
                    .collect();
                drop(idx);

                // Candidates arrive in id order, not in output order, so
                // every survivor must be collected before sorting — an early
                // exit at `offset + limit` would page over the wrong subset
                // when `track_total` is off. The intersection is already the
                // selective step; `total` is simply exact here.
                let mut survivors: Vec<(String, String, ConceptId, Concept)> =
                    Vec::with_capacity(candidates.len());
                for id in candidates.drain(..) {
                    let Some(c) = self.concepts.get(&id) else {
                        continue;
                    };
                    if let Some(filter) = &type_filter {
                        if !filter.contains(&c.concept_type) {
                            continue;
                        }
                    }
                    if !c.name.to_lowercase().contains(needle) {
                        continue;
                    }
                    survivors.push((c.concept_type.clone(), c.name.clone(), id, c.clone()));
                }
                // R6: same order as the needle-less listing with the same
                // filters — `(name, id)` across the buckets of a type filter
                // (the k-way merge below), `(concept_type, name, id)` for the
                // global scan.
                if type_filter.is_some() {
                    survivors.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.2.cmp(&b.2)));
                } else {
                    survivors.sort_by(|a, b| {
                        a.0.cmp(&b.0)
                            .then_with(|| a.1.cmp(&b.1))
                            .then_with(|| a.2.cmp(&b.2))
                    });
                }
                let total = survivors.len();
                let page: Vec<Concept> = survivors
                    .into_iter()
                    .skip(offset)
                    .take(limit)
                    .map(|(_, _, _, c)| c)
                    .collect();
                return (total, page);
            }
        }

        let mut total = 0usize;
        let mut page: Vec<Concept> = Vec::with_capacity(limit.min(LIST_PREALLOC_CAP));

        let mut consume = |name: &str, id: &ConceptId| -> bool {
            if let Some(n) = name_substring_lowercase {
                if !name.to_lowercase().contains(n) {
                    return true;
                }
            }
            let rank = total;
            total += 1;
            if rank >= offset && page.len() < limit {
                if let Some(c) = self.concepts.get(id) {
                    page.push(c.clone());
                }
            }
            !(page.len() >= limit && !track_total)
        };

        if let Some(t) = concept_type {
            if include_subtypes {
                // K-way merge across descendant buckets to keep stable
                // (name, id) order without globally re-sorting.
                let descs = self.ontology.read().descendants(t);
                use std::cmp::Reverse;
                use std::collections::BinaryHeap;
                let buckets: Vec<_> = descs
                    .iter()
                    .filter_map(|d| self.concepts_by_type.get(d).map(|b| b.clone()))
                    .collect();
                let mut iters: Vec<_> = buckets.iter().map(|b| b.iter().peekable()).collect();
                let mut heap: BinaryHeap<Reverse<(String, ConceptId, usize)>> = BinaryHeap::new();
                for (i, it) in iters.iter_mut().enumerate() {
                    if let Some((n, id)) = it.peek() {
                        heap.push(Reverse(((*n).clone(), *id, i)));
                    }
                }
                while let Some(Reverse((name, id, i))) = heap.pop() {
                    iters[i].next();
                    if let Some((n2, id2)) = iters[i].peek() {
                        heap.push(Reverse(((*n2).clone(), *id2, i)));
                    }
                    if !consume(&name, &id) {
                        break;
                    }
                }
            } else if let Some(bucket) = self.concepts_by_type.get(t) {
                for (name, id) in bucket.iter() {
                    if !consume(name, id) {
                        break;
                    }
                }
            }
        } else {
            let idx = self.concepts_sorted.read();
            for (_ct, name, id) in idx.iter() {
                if !consume(name, id) {
                    break;
                }
            }
        }
        (total, page)
    }

    /// Public helper for traversal/RAG: every concept of `t`, optionally
    /// including subtype instances.
    pub fn concepts_of_type(&self, t: &str, include_subtypes: bool) -> Vec<Concept> {
        let types: Vec<String> = if include_subtypes {
            self.ontology.read().descendants(t)
        } else {
            vec![t.to_string()]
        };
        let mut out: Vec<Concept> = Vec::new();
        for ty in types {
            if let Some(bucket) = self.concepts_by_type.get(&ty) {
                for (_, id) in bucket.iter() {
                    if let Some(c) = self.concepts.get(id) {
                        out.push(c.clone());
                    }
                }
            }
        }
        out
    }

    /// Paginated, ordered listing of relations (sorted by id ascending).
    /// `source`/`target`/`relation_type` are optional filters applied
    /// in-stream.
    pub fn list_relations_page(
        &self,
        source: Option<ConceptId>,
        target: Option<ConceptId>,
        relation_type: Option<&str>,
        offset: usize,
        limit: usize,
        track_total: bool,
    ) -> (usize, Vec<Relation>) {
        // Query cache — same gen-snapshot trick as `list_concepts_page`.
        let gen = self.relations_gen.load(Ordering::Acquire);
        let cache_key: ListRelationsCacheKey = (
            source,
            target,
            relation_type.map(|s| s.to_string()),
            offset,
            limit,
            track_total,
        );
        if let Some(entry) = self.list_relations_cache.lock().get(&cache_key) {
            if entry.gen == gen {
                return (entry.total, entry.page.clone());
            }
        }

        let (total, page) = self.list_relations_page_uncached(
            source,
            target,
            relation_type,
            offset,
            limit,
            track_total,
        );

        let mut cache = self.list_relations_cache.lock();
        if cache.len() >= LIST_RELATIONS_CACHE_CAP {
            cache.clear();
        }
        cache.insert(
            cache_key,
            ListRelationsCacheEntry {
                gen,
                total,
                page: page.clone(),
            },
        );
        (total, page)
    }

    fn list_relations_page_uncached(
        &self,
        source: Option<ConceptId>,
        target: Option<ConceptId>,
        relation_type: Option<&str>,
        offset: usize,
        limit: usize,
        track_total: bool,
    ) -> (usize, Vec<Relation>) {
        // Adjacency fast path. When `source` and/or `target` is set, pull
        // candidate relation ids from the adjacency index rather than scanning
        // the full `relations_sorted` set. Picking the smaller of the two
        // when both are present plays the role of a tiny query planner.
        if source.is_some() || target.is_some() {
            // Typed adjacency fast path: when a relation_type filter is
            // also set, jump straight to the per-(node, type) bucket.
            let typed_src = source.and_then(|s| {
                relation_type.and_then(|rt| {
                    self.out_edges_typed
                        .get(&s)
                        .and_then(|m| m.get(rt).map(|v| v.iter().copied().collect::<Vec<_>>()))
                })
            });
            let typed_tgt = target.and_then(|t| {
                relation_type.and_then(|rt| {
                    self.in_edges_typed
                        .get(&t)
                        .and_then(|m| m.get(rt).map(|v| v.iter().copied().collect::<Vec<_>>()))
                })
            });

            let mut candidates: Vec<RelationId> = match (source, target, typed_src, typed_tgt) {
                (_, _, Some(a), Some(b)) => {
                    if a.len() <= b.len() {
                        a
                    } else {
                        b
                    }
                }
                (_, _, Some(a), None) => a,
                (_, _, None, Some(b)) => b,
                (Some(s), Some(t), None, None) => {
                    let from_src = self.out_edges.get(&s).map(|adj| adj.len()).unwrap_or(0);
                    let from_tgt = self.in_edges.get(&t).map(|adj| adj.len()).unwrap_or(0);
                    if from_src <= from_tgt {
                        self.out_edges
                            .get(&s)
                            .map(|adj| adj.iter().copied().collect::<Vec<_>>())
                            .unwrap_or_default()
                    } else {
                        self.in_edges
                            .get(&t)
                            .map(|adj| adj.iter().copied().collect::<Vec<_>>())
                            .unwrap_or_default()
                    }
                }
                (Some(s), None, None, None) => self
                    .out_edges
                    .get(&s)
                    .map(|adj| adj.iter().copied().collect::<Vec<_>>())
                    .unwrap_or_default(),
                (None, Some(t), None, None) => self
                    .in_edges
                    .get(&t)
                    .map(|adj| adj.iter().copied().collect::<Vec<_>>())
                    .unwrap_or_default(),
                (None, None, _, _) => unreachable!(),
            };
            candidates.sort_unstable();
            let mut total = 0usize;
            let mut page: Vec<Relation> = Vec::with_capacity(limit.min(LIST_PREALLOC_CAP));
            for rid in &candidates {
                let Some(rel) = self.relations.get(rid) else {
                    continue;
                };
                if let Some(s) = source {
                    if rel.source != s {
                        continue;
                    }
                }
                if let Some(t) = target {
                    if rel.target != t {
                        continue;
                    }
                }
                if let Some(rt) = relation_type {
                    if rel.relation_type != rt {
                        continue;
                    }
                }
                let rank = total;
                total += 1;
                if rank >= offset && page.len() < limit {
                    page.push(rel.clone());
                }
                if page.len() >= limit && !track_total {
                    break;
                }
            }
            return (total, page);
        }

        let idx = self.relations_sorted.read();
        let mut total = 0usize;
        let mut page: Vec<Relation> = Vec::with_capacity(limit.min(LIST_PREALLOC_CAP));
        for rid in idx.iter() {
            let Some(rel) = self.relations.get(rid) else {
                continue;
            };
            if let Some(s) = source {
                if rel.source != s {
                    continue;
                }
            }
            if let Some(t) = target {
                if rel.target != t {
                    continue;
                }
            }
            if let Some(rt) = relation_type {
                if rel.relation_type != rt {
                    continue;
                }
            }
            let rank = total;
            total += 1;
            if rank >= offset && page.len() < limit {
                page.push(rel.clone());
            }
            if page.len() >= limit && !track_total {
                break;
            }
        }
        (total, page)
    }

    /// Paginated, ordered listing of rules.
    pub fn list_rules_page(&self, offset: usize, limit: usize) -> (usize, Vec<Rule>) {
        let idx = self.rules_sorted.read();
        let total = idx.len();
        let page: Vec<Rule> = idx
            .iter()
            .skip(offset)
            .take(limit)
            .filter_map(|(_, _, id)| self.rules.get(id).map(|r| r.clone()))
            .collect();
        (total, page)
    }

    /// Paginated, ordered listing of actions.
    pub fn list_actions_page(&self, offset: usize, limit: usize) -> (usize, Vec<Action>) {
        let idx = self.actions_sorted.read();
        let total = idx.len();
        let page: Vec<Action> = idx
            .iter()
            .skip(offset)
            .take(limit)
            .filter_map(|(_, _, id)| self.actions.get(id).map(|a| a.clone()))
            .collect();
        (total, page)
    }

    pub fn action_count(&self) -> usize {
        self.actions.len()
    }

    pub fn remove_action(&self, id: ActionId) -> GraphResult<()> {
        let removed = self
            .actions
            .remove(&id)
            .ok_or(GraphError::UnknownRelationType(format!("action {id}")))?
            .1;
        if self.derived() {
            self.actions_sorted
                .write()
                .remove(&(removed.action_type, removed.name, id));
        }
        Ok(())
    }

    /// Compute the action that `patch` would produce, validated, without
    /// applying it. Pure.
    pub fn preview_action_update(&self, id: ActionId, patch: &ActionPatch) -> GraphResult<Action> {
        if let Some(subj) = patch.subject {
            if !self.concepts.contains_key(&subj) {
                return Err(GraphError::UnknownConcept(subj));
            }
        }
        if let Some(Some(obj)) = &patch.object {
            if !self.concepts.contains_key(obj) {
                return Err(GraphError::UnknownConcept(*obj));
            }
        }
        let mut action = self.get_action(id)?;
        if let Some(n) = &patch.name {
            action.name = n.clone();
        }
        if let Some(s) = patch.subject {
            action.subject = s;
        }
        if let Some(o) = patch.object {
            action.object = o;
        }
        if let Some(p) = &patch.parameters {
            action.parameters = p.clone();
        }
        if let Some(e) = &patch.effect {
            action.effect = e.clone();
        }
        if let Some(d) = &patch.description {
            action.description = d.clone();
        }
        Ok(action)
    }

    /// Replace the action `updated.id` with `updated`, maintaining the
    /// sorted index. `action_type` is immutable and taken from the stored
    /// action.
    pub fn apply_action_update(&self, updated: Action) -> GraphResult<Action> {
        let id = updated.id;
        let mut entry = self
            .actions
            .get_mut(&id)
            .ok_or(GraphError::UnknownRelationType(format!("action {id}")))?;
        if entry.name != updated.name && self.derived() {
            let old_sort = (entry.action_type.clone(), entry.name.clone(), id);
            let new_sort = (entry.action_type.clone(), updated.name.clone(), id);
            let mut idx = self.actions_sorted.write();
            idx.remove(&old_sort);
            idx.insert(new_sort);
        }
        entry.name = updated.name;
        entry.subject = updated.subject;
        entry.object = updated.object;
        entry.parameters = updated.parameters;
        entry.effect = updated.effect;
        entry.description = updated.description;
        Ok(entry.clone())
    }

    /// Apply a partial update to an existing action. `action_type` cannot
    /// be changed; replacement `subject` / `object` concept ids must exist.
    pub fn update_action(&self, id: ActionId, patch: ActionPatch) -> GraphResult<Action> {
        let updated = self.preview_action_update(id, &patch)?;
        self.apply_action_update(updated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Concept;
    use crate::schema::{ConceptType, RelationType};

    fn toy_ontology() -> Ontology {
        let mut o = Ontology::new();
        o.add_concept_type(ConceptType {
            name: "Person".into(),
            description: "a human".into(),
            ..Default::default()
        });
        o.add_concept_type(ConceptType {
            name: "Paper".into(),
            description: "research paper".into(),
            ..Default::default()
        });
        o.add_relation_type(RelationType {
            name: "authored".into(),
            domain: "Person".into(),
            range: "Paper".into(),
            description: "authorship".into(),
            ..Default::default()
        })
        .unwrap();
        o
    }

    #[test]
    fn insert_and_traverse() {
        let g = OntologyGraph::new(toy_ontology());
        let alice = g
            .upsert_concept(Concept::new(Default::default(), "Person", "Alice"))
            .unwrap();
        let paper = g
            .upsert_concept(Concept::new(Default::default(), "Paper", "On RAG"))
            .unwrap();
        g.add_relation(Relation::new(Default::default(), "authored", alice, paper))
            .unwrap();
        assert_eq!(g.outgoing(alice).len(), 1);
        assert_eq!(g.incoming(paper).len(), 1);
    }

    #[test]
    fn remove_concept_cascades_relations() {
        let g = OntologyGraph::new(toy_ontology());
        let alice = g
            .upsert_concept(Concept::new(Default::default(), "Person", "Alice"))
            .unwrap();
        let p1 = g
            .upsert_concept(Concept::new(Default::default(), "Paper", "P1"))
            .unwrap();
        let p2 = g
            .upsert_concept(Concept::new(Default::default(), "Paper", "P2"))
            .unwrap();
        g.add_relation(Relation::new(Default::default(), "authored", alice, p1))
            .unwrap();
        g.add_relation(Relation::new(Default::default(), "authored", alice, p2))
            .unwrap();
        assert_eq!(g.relation_count(), 2);

        let removed = g.remove_concept(alice).unwrap();
        assert_eq!(removed.len(), 2);
        assert_eq!(g.relation_count(), 0);
        assert_eq!(g.outgoing(p1).len(), 0);
        assert_eq!(g.incoming(p1).len(), 0);
    }

    #[test]
    fn update_concept_renames_and_persists_index() {
        use crate::model::{ConceptPatch, PropertyValue};
        use ahash::AHashMap;
        let g = OntologyGraph::new(toy_ontology());
        let alice = g
            .upsert_concept(
                Concept::new(Default::default(), "Person", "Alice")
                    .with_description("the original"),
            )
            .unwrap();

        // Rename + new description + properties.
        let mut props = AHashMap::new();
        props.insert("nickname".into(), PropertyValue::Text("Ali".into()));
        let patched = g
            .update_concept(
                alice,
                ConceptPatch {
                    name: Some("Alicia".into()),
                    description: Some("renamed".into()),
                    properties: Some(props),
                },
            )
            .unwrap();
        assert_eq!(patched.name, "Alicia");
        assert_eq!(patched.description, "renamed");
        assert_eq!(
            patched.properties.get("nickname").and_then(|v| v.as_text()),
            Some("Ali")
        );

        // Old name binding cleared, new one in place.
        assert!(g.find_by_name("Person", "Alice").is_none());
        assert_eq!(g.find_by_name("Person", "Alicia"), Some(alice));

        // Renaming onto an occupied name fails.
        let bob = g
            .upsert_concept(Concept::new(Default::default(), "Person", "Bob"))
            .unwrap();
        let err = g.update_concept(
            bob,
            ConceptPatch {
                name: Some("Alicia".into()),
                ..Default::default()
            },
        );
        assert!(err.is_err());
    }

    #[test]
    fn shortest_path_finds_two_hop_link() {
        let g = OntologyGraph::new(toy_ontology());
        let alice = g
            .upsert_concept(Concept::new(Default::default(), "Person", "Alice"))
            .unwrap();
        let p = g
            .upsert_concept(Concept::new(Default::default(), "Paper", "P"))
            .unwrap();
        let bob = g
            .upsert_concept(Concept::new(Default::default(), "Person", "Bob"))
            .unwrap();
        g.add_relation(Relation::new(Default::default(), "authored", alice, p))
            .unwrap();
        g.add_relation(Relation::new(Default::default(), "authored", bob, p))
            .unwrap();

        let path = g
            .shortest_path(alice, bob, 4)
            .unwrap()
            .expect("path exists");
        assert_eq!(path.len(), 2);
        assert_eq!(path.start.name, "Alice");
        assert_eq!(path.steps.last().unwrap().concept.name, "Bob");

        // Same node returns an empty path, not None.
        let same = g.shortest_path(alice, alice, 4).unwrap().expect("self");
        assert!(same.is_empty());

        // Bound the depth — disconnect should report None.
        let lone = g
            .upsert_concept(Concept::new(Default::default(), "Person", "Eve"))
            .unwrap();
        assert!(g.shortest_path(alice, lone, 4).unwrap().is_none());
    }

    #[test]
    fn list_concepts_with_subsumption() {
        let mut o = toy_ontology();
        o.add_concept_type(ConceptType {
            name: "Researcher".into(),
            parent: Some("Person".into()),
            ..Default::default()
        });
        let g = OntologyGraph::new(o);
        g.upsert_concept(Concept::new(Default::default(), "Person", "Alice"))
            .unwrap();
        g.upsert_concept(Concept::new(Default::default(), "Researcher", "Bob"))
            .unwrap();
        let (total, _) = g.list_concepts_page(Some("Person"), None, 0, 10, true, false);
        assert_eq!(total, 1);
        let (total2, page2) = g.list_concepts_page(Some("Person"), None, 0, 10, true, true);
        assert_eq!(total2, 2);
        assert_eq!(page2.len(), 2);
    }

    #[test]
    fn ontology_descendants_includes_self_and_children() {
        let mut o = Ontology::new();
        o.add_concept_type(ConceptType {
            name: "Animal".into(),
            ..Default::default()
        });
        o.add_concept_type(ConceptType {
            name: "Mammal".into(),
            parent: Some("Animal".into()),
            ..Default::default()
        });
        o.add_concept_type(ConceptType {
            name: "Dog".into(),
            parent: Some("Mammal".into()),
            ..Default::default()
        });
        let mut d = o.descendants("Animal");
        d.sort();
        assert_eq!(d, vec!["Animal".to_string(), "Dog".into(), "Mammal".into()]);
        assert!(o.descendants("Unknown").is_empty());
    }

    #[test]
    fn closure_walks_transitive_chain() {
        let mut o = Ontology::new();
        o.add_concept_type(ConceptType {
            name: "Component".into(),
            ..Default::default()
        });
        o.add_relation_type(RelationType {
            name: "partOf".into(),
            domain: "Component".into(),
            range: "Component".into(),
            transitive: true,
            ..Default::default()
        })
        .unwrap();
        let g = OntologyGraph::new(o);
        let wheel = g
            .upsert_concept(Concept::new(Default::default(), "Component", "Wheel"))
            .unwrap();
        let car = g
            .upsert_concept(Concept::new(Default::default(), "Component", "Car"))
            .unwrap();
        let fleet = g
            .upsert_concept(Concept::new(Default::default(), "Component", "Fleet"))
            .unwrap();
        g.add_relation(Relation::new(Default::default(), "partOf", wheel, car))
            .unwrap();
        g.add_relation(Relation::new(Default::default(), "partOf", car, fleet))
            .unwrap();
        let reached = g.closure(wheel, "partOf", 3).unwrap();
        assert!(reached.contains(&car));
        assert!(reached.contains(&fleet));
    }

    #[test]
    fn inverse_of_surfaces_virtual_edges() {
        let mut o = Ontology::new();
        o.add_concept_type(ConceptType {
            name: "Person".into(),
            ..Default::default()
        });
        o.add_concept_type(ConceptType {
            name: "Paper".into(),
            ..Default::default()
        });
        o.add_relation_type(RelationType {
            name: "authored".into(),
            domain: "Person".into(),
            range: "Paper".into(),
            ..Default::default()
        })
        .unwrap();
        o.add_relation_type(RelationType {
            name: "authoredBy".into(),
            domain: "Paper".into(),
            range: "Person".into(),
            inverse_of: Some("authored".into()),
            ..Default::default()
        })
        .unwrap();
        o.validate_inverses().unwrap();
        let g = OntologyGraph::new(o);
        let alice = g
            .upsert_concept(Concept::new(Default::default(), "Person", "Alice"))
            .unwrap();
        let p1 = g
            .upsert_concept(Concept::new(Default::default(), "Paper", "P1"))
            .unwrap();
        g.add_relation(Relation::new(Default::default(), "authored", alice, p1))
            .unwrap();
        let virt = g.outgoing_typed(p1, &["authoredBy".to_string()]);
        assert_eq!(virt.len(), 1);
        assert_eq!(virt[0].source, p1);
        assert_eq!(virt[0].target, alice);
        assert_eq!(virt[0].relation_type, "authoredBy");
    }

    #[test]
    fn required_properties_enforced() {
        use crate::model::{ConceptPatch, PropertyValue};
        use ahash::AHashMap;
        let mut o = Ontology::new();
        o.add_concept_type(ConceptType {
            name: "Person".into(),
            required_properties: vec!["email".into()],
            ..Default::default()
        });
        let g = OntologyGraph::new(o);
        let err = g.upsert_concept(Concept::new(Default::default(), "Person", "Alice"));
        assert!(matches!(
            err,
            Err(GraphError::MissingRequiredProperty { .. })
        ));
        let mut props = AHashMap::new();
        props.insert("email".into(), PropertyValue::Text("a@b".into()));
        let mut c = Concept::new(Default::default(), "Person", "Alice");
        c.properties = props.clone();
        let id = g.upsert_concept(c).unwrap();
        // update_concept replacing properties without email must also fail.
        let err = g.update_concept(
            id,
            ConceptPatch {
                properties: Some(AHashMap::new()),
                ..Default::default()
            },
        );
        assert!(matches!(
            err,
            Err(GraphError::MissingRequiredProperty { .. })
        ));
    }

    #[test]
    fn one_to_many_rejects_second_inbound() {
        let mut o = Ontology::new();
        o.add_concept_type(ConceptType {
            name: "Person".into(),
            ..Default::default()
        });
        o.add_concept_type(ConceptType {
            name: "Paper".into(),
            ..Default::default()
        });
        o.add_relation_type(RelationType {
            name: "authored".into(),
            domain: "Person".into(),
            range: "Paper".into(),
            cardinality: crate::schema::Cardinality::OneToMany,
            ..Default::default()
        })
        .unwrap();
        let g = OntologyGraph::new(o);
        let alice = g
            .upsert_concept(Concept::new(Default::default(), "Person", "Alice"))
            .unwrap();
        let bob = g
            .upsert_concept(Concept::new(Default::default(), "Person", "Bob"))
            .unwrap();
        let p = g
            .upsert_concept(Concept::new(Default::default(), "Paper", "P"))
            .unwrap();
        g.add_relation(Relation::new(Default::default(), "authored", alice, p))
            .unwrap();
        let err = g.add_relation(Relation::new(Default::default(), "authored", bob, p));
        assert!(matches!(err, Err(GraphError::CardinalityViolation { .. })));
    }

    #[test]
    fn functional_relation_rejects_second_outbound() {
        let mut o = Ontology::new();
        o.add_concept_type(ConceptType {
            name: "Person".into(),
            ..Default::default()
        });
        o.add_relation_type(RelationType {
            name: "spouse".into(),
            domain: "Person".into(),
            range: "Person".into(),
            functional: true,
            ..Default::default()
        })
        .unwrap();
        let g = OntologyGraph::new(o);
        let a = g
            .upsert_concept(Concept::new(Default::default(), "Person", "A"))
            .unwrap();
        let b = g
            .upsert_concept(Concept::new(Default::default(), "Person", "B"))
            .unwrap();
        let c = g
            .upsert_concept(Concept::new(Default::default(), "Person", "C"))
            .unwrap();
        g.add_relation(Relation::new(Default::default(), "spouse", a, b))
            .unwrap();
        let err = g.add_relation(Relation::new(Default::default(), "spouse", a, c));
        assert!(matches!(err, Err(GraphError::CardinalityViolation { .. })));
    }

    #[test]
    fn disjoint_types_rejected() {
        let mut o = Ontology::new();
        o.add_concept_type(ConceptType {
            name: "Person".into(),
            disjoint_with: vec!["Robot".into()],
            ..Default::default()
        });
        o.add_concept_type(ConceptType {
            name: "Robot".into(),
            ..Default::default()
        });
        let g = OntologyGraph::new(o);
        g.upsert_concept(Concept::new(Default::default(), "Person", "Alice"))
            .unwrap();
        let err = g.upsert_concept(Concept::new(Default::default(), "Robot", "Alice"));
        assert!(matches!(err, Err(GraphError::DisjointTypeViolation { .. })));
    }

    #[test]
    fn schema_violation_rejected() {
        let g = OntologyGraph::new(toy_ontology());
        let a = g
            .upsert_concept(Concept::new(Default::default(), "Paper", "P1"))
            .unwrap();
        let b = g
            .upsert_concept(Concept::new(Default::default(), "Paper", "P2"))
            .unwrap();
        let res = g.add_relation(Relation::new(Default::default(), "authored", a, b));
        assert!(res.is_err());
    }

    // ---- domains (ns) ----

    #[test]
    fn extend_ontology_is_atomic_and_guards_domain_moves() {
        let g = OntologyGraph::new(toy_ontology());
        // A failing closure leaves the schema untouched.
        let before = g.ontology().concept_types.len();
        let err = g.extend_ontology(|o| {
            o.add_concept_type(ConceptType {
                name: "Ghost".into(),
                ..Default::default()
            });
            Err(GraphError::Serde("boom".into()))
        });
        assert!(err.is_err());
        assert_eq!(g.ontology().concept_types.len(), before);
        // An invalid domain is rejected as a whole.
        let err = g.extend_ontology(|o| {
            o.add_concept_type(ConceptType {
                name: "Bad".into(),
                ns: Some("Not Valid".into()),
                ..Default::default()
            });
            Ok(())
        });
        assert!(matches!(err, Err(GraphError::InvalidNamespace { .. })));
        assert!(!g.ontology().concept_types.contains_key("Bad"));
        // Moving Person to another domain is fine while it has no instances…
        g.extend_ontology(|o| {
            let mut ct = o.concept_type("Person").unwrap().clone();
            ct.ns = Some("people".into());
            o.add_concept_type(ct);
            Ok(())
        })
        .unwrap();
        assert_eq!(g.ns_of_type("Person"), "people");
        g.upsert_concept(Concept::new(Default::default(), "Person", "Alice"))
            .unwrap();
        // …and refused once instances exist.
        let err = g.extend_ontology(|o| {
            let mut ct = o.concept_type("Person").unwrap().clone();
            ct.ns = Some("staff".into());
            o.add_concept_type(ct);
            Ok(())
        });
        assert!(
            matches!(
                err,
                Err(GraphError::NamespaceChangeWithInstances { instances: 1, .. })
            ),
            "{err:?}"
        );
        assert_eq!(g.ns_of_type("Person"), "people");
        // Unrelated types keep resolving to the default domain.
        assert_eq!(g.ns_of_type("Paper"), "default");
    }

    #[test]
    fn schema_changes_that_orphan_instances_are_refused() {
        let mut o = toy_ontology();
        o.add_relation_type(RelationType {
            name: "knows".into(),
            domain: "Person".into(),
            range: "Person".into(),
            symmetric: true,
            ..Default::default()
        })
        .unwrap();
        o.add_rule_type(crate::schema::RuleType {
            name: "must_review".into(),
            when: String::new(),
            then: String::new(),
            applies_to: vec!["Paper".into()],
            strict: false,
            description: String::new(),
        })
        .unwrap();
        let g = OntologyGraph::new(o);
        let a = g
            .upsert_concept(Concept::new(Default::default(), "Person", "A"))
            .unwrap();
        let b = g
            .upsert_concept(Concept::new(Default::default(), "Person", "B"))
            .unwrap();
        let p = g
            .upsert_concept(Concept::new(Default::default(), "Paper", "P"))
            .unwrap();
        g.add_relation(Relation::new(Default::default(), "authored", a, p))
            .unwrap();
        let mut rule = crate::model::Rule::new(Default::default(), "must_review", "r");
        rule.applies_to = vec![p];
        g.upsert_rule(rule).unwrap();

        // Dropping a concept type with instances.
        let err = g.extend_ontology(|o| {
            o.concept_types.remove("Person");
            Ok(())
        });
        assert!(
            matches!(
                err,
                Err(GraphError::TypeInUse {
                    kind: "concept",
                    ..
                })
            ),
            "{err:?}"
        );
        // Re-pointing a relation type with relations.
        let err = g.extend_ontology(|o| {
            let mut rt = o.relation_type("authored").unwrap().clone();
            rt.domain = "Paper".into();
            o.relation_types.insert(rt.name.clone(), rt);
            Ok(())
        });
        assert!(
            matches!(
                err,
                Err(GraphError::RelationTypeChangeWithInstances { instances: 1, .. })
            ),
            "{err:?}"
        );
        // Dropping a relation type with relations, a rule type with rules.
        let err = g.extend_ontology(|o| {
            o.relation_types.remove("authored");
            Ok(())
        });
        assert!(
            matches!(
                err,
                Err(GraphError::TypeInUse {
                    kind: "relation",
                    ..
                })
            ),
            "{err:?}"
        );
        let err = g.extend_ontology(|o| {
            o.rule_types.remove("must_review");
            Ok(())
        });
        assert!(
            matches!(err, Err(GraphError::TypeInUse { kind: "rule", .. })),
            "{err:?}"
        );
        // The same edits are fine once nothing depends on them: `knows` has
        // no relations, so it can be re-pointed or dropped.
        g.extend_ontology(|o| {
            o.relation_types.remove("knows");
            Ok(())
        })
        .unwrap();
        // check_ontology is the pure form of the same judgement.
        let mut bad = g.ontology();
        bad.concept_types.remove("Person");
        assert!(g.check_ontology(&bad).is_err());
        assert!(g.check_ontology(&g.ontology()).is_ok());
        assert_eq!(g.concept_count(), 3);
        let _ = b;
    }

    #[test]
    fn insert_relation_exact_keeps_ids_and_materializes_nothing() {
        let mut o = toy_ontology();
        o.add_relation_type(RelationType {
            name: "knows".into(),
            domain: "Person".into(),
            range: "Person".into(),
            symmetric: true,
            ..Default::default()
        })
        .unwrap();
        let g = OntologyGraph::new(o);
        let a = g
            .upsert_concept(Concept::new(Default::default(), "Person", "A"))
            .unwrap();
        let b = g
            .upsert_concept(Concept::new(Default::default(), "Person", "B"))
            .unwrap();
        // Live: a symmetric add materializes the inverse → two relations.
        let id = g
            .add_relation(Relation::new(Default::default(), "knows", a, b))
            .unwrap();
        let live: Vec<Relation> = g.all_relations();
        assert_eq!(live.len(), 2);
        // Exact replay of both, into a fresh graph, keeps both ids.
        let h = OntologyGraph::new(g.ontology());
        h.upsert_concept(Concept::new(a, "Person", "A")).unwrap();
        h.upsert_concept(Concept::new(b, "Person", "B")).unwrap();
        for r in &live {
            h.insert_relation_exact(r.clone()).unwrap();
        }
        assert_eq!(h.relation_count(), 2);
        assert_eq!(h.get_relation(id).unwrap().source, a);
        for r in &live {
            assert_eq!(h.get_relation(r.id).unwrap().target, r.target);
        }
        // A duplicate id, an unprepared id, an unknown endpoint are refused.
        assert!(matches!(
            h.insert_relation_exact(live[0].clone()),
            Err(GraphError::RelationExists(_))
        ));
        assert!(matches!(
            h.insert_relation_exact(Relation::new(Default::default(), "knows", a, b)),
            Err(GraphError::NotPrepared(_))
        ));
        assert!(matches!(
            h.insert_relation_exact(Relation::new(RelationId(99), "knows", a, ConceptId(77))),
            Err(GraphError::UnknownConcept(_))
        ));
        // Allocation continues past the observed ids.
        let next = h
            .add_relation(Relation::new(Default::default(), "knows", b, a))
            .unwrap();
        assert!(next.0 > live.iter().map(|r| r.id.0).max().unwrap());
    }

    // ---- write-ahead (R8) API: prepare / apply, preview / apply ----

    #[test]
    fn prepare_concept_allocates_id_without_inserting() {
        let g = OntologyGraph::new(toy_ontology());
        let gen_before = g.concepts_generation();
        let mut c = Concept::new(Default::default(), "Person", "Alice");
        g.prepare_concept(&mut c).unwrap();
        assert_ne!(c.id.0, 0, "prepare must allocate an id");
        assert_eq!(g.concept_count(), 0, "prepare must not insert");
        assert!(g.find_by_name("Person", "Alice").is_none());
        assert_eq!(
            g.concepts_generation(),
            gen_before,
            "prepare must not bump gen"
        );

        let id = g.apply_prepared_concept(c).unwrap();
        assert_eq!(g.concept_count(), 1);
        assert_eq!(g.find_by_name("Person", "Alice"), Some(id));
        assert!(g.concepts_generation() > gen_before);
    }

    #[test]
    fn prepare_concept_rejects_everything_apply_would() {
        let g = OntologyGraph::new(toy_ontology());
        g.upsert_concept(Concept::new(Default::default(), "Person", "Alice"))
            .unwrap();
        // Duplicate name for a fresh concept.
        let mut dup = Concept::new(Default::default(), "Person", "alice");
        assert!(matches!(
            g.prepare_concept(&mut dup),
            Err(GraphError::DuplicateConcept(_, _))
        ));
        assert_eq!(dup.id.0, 0, "a rejected prepare must not allocate");
        // Unknown type.
        let mut bad = Concept::new(Default::default(), "Alien", "Zed");
        assert!(matches!(
            g.prepare_concept(&mut bad),
            Err(GraphError::UnknownConceptType(_))
        ));
        assert_eq!(g.concept_count(), 1);
    }

    #[test]
    fn prepare_concept_with_explicit_id_is_an_upsert() {
        let g = OntologyGraph::new(toy_ontology());
        let id = g
            .upsert_concept(Concept::new(Default::default(), "Person", "Alice"))
            .unwrap();
        let mut same = Concept::new(id, "Person", "Alice").with_description("v2");
        g.prepare_concept(&mut same).unwrap();
        assert_eq!(same.id, id);
        g.apply_prepared_concept(same).unwrap();
        assert_eq!(g.concept_count(), 1);
        assert_eq!(g.get_concept(id).unwrap().description, "v2");
    }

    #[test]
    fn explicit_id_upsert_rename_drops_the_old_name_and_refuses_a_type_change() {
        let g = OntologyGraph::new(toy_ontology());
        let id = g
            .upsert_concept(Concept::new(Default::default(), "Person", "Alice"))
            .unwrap();
        // Rename through an explicit-id upsert (export re-ingest path).
        g.upsert_concept(Concept::new(id, "Person", "Alicia"))
            .unwrap();
        assert_eq!(g.find_by_name("Person", "Alicia"), Some(id));
        assert!(
            g.find_by_name("Person", "Alice").is_none(),
            "stale name binding must be removed"
        );
        // The old name is free again.
        let other = g
            .upsert_concept(Concept::new(Default::default(), "Person", "Alice"))
            .unwrap();
        assert_ne!(other, id);
        // Changing the type through an explicit id is refused at prepare.
        let mut retyped = Concept::new(id, "Paper", "Alicia");
        assert!(matches!(
            g.prepare_concept(&mut retyped),
            Err(GraphError::ImmutableConceptType(_, _))
        ));
        assert!(matches!(
            g.apply_prepared_concept(Concept::new(id, "Paper", "Alicia")),
            Err(GraphError::ImmutableConceptType(_, _))
        ));
        assert_eq!(g.get_concept(id).unwrap().concept_type, "Person");
    }

    #[test]
    fn apply_prepared_rejects_unprepared_entities() {
        let g = OntologyGraph::new(toy_ontology());
        let c = Concept::new(Default::default(), "Person", "Alice");
        assert!(matches!(
            g.apply_prepared_concept(c),
            Err(GraphError::NotPrepared("concept"))
        ));
        let a = g
            .upsert_concept(Concept::new(Default::default(), "Person", "A"))
            .unwrap();
        let p = g
            .upsert_concept(Concept::new(Default::default(), "Paper", "P"))
            .unwrap();
        let r = Relation::new(Default::default(), "authored", a, p);
        assert!(matches!(
            g.apply_prepared_relation(r),
            Err(GraphError::NotPrepared("relation"))
        ));
        assert_eq!(g.relation_count(), 0);
    }

    #[test]
    fn prepare_relation_validates_and_allocates_without_inserting() {
        let g = OntologyGraph::new(toy_ontology());
        let a = g
            .upsert_concept(Concept::new(Default::default(), "Person", "A"))
            .unwrap();
        let p = g
            .upsert_concept(Concept::new(Default::default(), "Paper", "P"))
            .unwrap();
        // Wrong direction → schema violation, nothing allocated.
        let mut wrong = Relation::new(Default::default(), "authored", p, a);
        assert!(matches!(
            g.prepare_relation(&mut wrong),
            Err(GraphError::SchemaViolation { .. })
        ));
        assert_eq!(wrong.id.0, 0);
        // Unknown endpoint.
        let mut dangling = Relation::new(Default::default(), "authored", a, ConceptId(999));
        assert!(matches!(
            g.prepare_relation(&mut dangling),
            Err(GraphError::UnknownConcept(_))
        ));
        // Happy path.
        let mut ok = Relation::new(Default::default(), "authored", a, p);
        let gen_before = g.relations_generation();
        g.prepare_relation(&mut ok).unwrap();
        assert_ne!(ok.id.0, 0);
        assert_eq!(g.relation_count(), 0);
        assert_eq!(g.relations_generation(), gen_before);
        let id = g.apply_prepared_relation(ok).unwrap();
        assert_eq!(g.relation_count(), 1);
        assert_eq!(g.get_relation(id).unwrap().source, a);
    }

    #[test]
    fn prepare_relation_materializes_symmetric_inverse_on_apply() {
        let mut o = toy_ontology();
        o.add_relation_type(RelationType {
            name: "knows".into(),
            domain: "Person".into(),
            range: "Person".into(),
            symmetric: true,
            ..Default::default()
        })
        .unwrap();
        let g = OntologyGraph::new(o);
        let a = g
            .upsert_concept(Concept::new(Default::default(), "Person", "A"))
            .unwrap();
        let b = g
            .upsert_concept(Concept::new(Default::default(), "Person", "B"))
            .unwrap();
        let mut r = Relation::new(Default::default(), "knows", a, b);
        g.prepare_relation(&mut r).unwrap();
        g.apply_prepared_relation(r).unwrap();
        assert_eq!(g.relation_count(), 2, "inverse materialized on apply");
        assert_eq!(g.outgoing(b).len(), 1);
    }

    #[test]
    fn preview_concept_update_is_pure_and_validated() {
        let g = OntologyGraph::new(toy_ontology());
        let a = g
            .upsert_concept(Concept::new(Default::default(), "Person", "Alice"))
            .unwrap();
        g.upsert_concept(Concept::new(Default::default(), "Person", "Bob"))
            .unwrap();
        let gen_before = g.concepts_generation();
        // Renaming onto an existing name is rejected at preview time.
        let clash = ConceptPatch {
            name: Some("bob".into()),
            ..Default::default()
        };
        assert!(matches!(
            g.preview_concept_update(a, &clash),
            Err(GraphError::DuplicateConcept(_, _))
        ));
        // A valid preview changes nothing.
        let patch = ConceptPatch {
            name: Some("Alicia".into()),
            description: Some("renamed".into()),
            ..Default::default()
        };
        let previewed = g.preview_concept_update(a, &patch).unwrap();
        assert_eq!(previewed.name, "Alicia");
        assert_eq!(g.get_concept(a).unwrap().name, "Alice");
        assert_eq!(g.concepts_generation(), gen_before);
        // Applying the previewed concept updates every index.
        g.apply_concept_update(previewed).unwrap();
        assert_eq!(g.find_by_name("Person", "alicia"), Some(a));
        assert!(g.find_by_name("Person", "alice").is_none());
        let (_, page) = g.list_concepts_page(Some("Person"), Some("lici"), 0, 10, true, true);
        assert_eq!(page.len(), 1, "trigram index follows the rename");
        assert!(g.concepts_generation() > gen_before);
    }

    #[test]
    fn apply_concept_update_refuses_type_change() {
        let g = OntologyGraph::new(toy_ontology());
        let a = g
            .upsert_concept(Concept::new(Default::default(), "Person", "Alice"))
            .unwrap();
        let mut retyped = g.get_concept(a).unwrap();
        retyped.concept_type = "Paper".into();
        assert!(matches!(
            g.apply_concept_update(retyped),
            Err(GraphError::ImmutableConceptType(_, _))
        ));
        assert_eq!(g.get_concept(a).unwrap().concept_type, "Person");
    }

    #[test]
    fn incident_relation_ids_matches_remove_concept_cascade() {
        let g = OntologyGraph::new(toy_ontology());
        let alice = g
            .upsert_concept(Concept::new(Default::default(), "Person", "Alice"))
            .unwrap();
        let p1 = g
            .upsert_concept(Concept::new(Default::default(), "Paper", "P1"))
            .unwrap();
        let p2 = g
            .upsert_concept(Concept::new(Default::default(), "Paper", "P2"))
            .unwrap();
        g.add_relation(Relation::new(Default::default(), "authored", alice, p1))
            .unwrap();
        g.add_relation(Relation::new(Default::default(), "authored", alice, p2))
            .unwrap();
        assert!(matches!(
            g.incident_relation_ids(ConceptId(4242)),
            Err(GraphError::UnknownConcept(_))
        ));
        let planned = g.incident_relation_ids(alice).unwrap();
        assert_eq!(planned.len(), 2);
        assert_eq!(g.relation_count(), 2, "incident_relation_ids is read-only");
        let removed = g.remove_concept(alice).unwrap();
        assert_eq!(planned, removed);
        assert_eq!(g.relation_count(), 0);
    }

    #[test]
    fn preview_relation_update_then_apply() {
        let g = OntologyGraph::new(toy_ontology());
        let a = g
            .upsert_concept(Concept::new(Default::default(), "Person", "A"))
            .unwrap();
        let p = g
            .upsert_concept(Concept::new(Default::default(), "Paper", "P"))
            .unwrap();
        let rid = g
            .add_relation(Relation::new(Default::default(), "authored", a, p))
            .unwrap();
        let patch = RelationPatch {
            weight: Some(0.25),
            properties: None,
        };
        let previewed = g.preview_relation_update(rid, &patch).unwrap();
        assert_eq!(previewed.weight, 0.25);
        assert_eq!(g.get_relation(rid).unwrap().weight, 1.0, "preview is pure");
        g.apply_relation_update(previewed).unwrap();
        assert_eq!(g.get_relation(rid).unwrap().weight, 0.25);
        assert!(matches!(
            g.preview_relation_update(RelationId(999), &patch),
            Err(GraphError::UnknownRelation(_))
        ));
    }
}
