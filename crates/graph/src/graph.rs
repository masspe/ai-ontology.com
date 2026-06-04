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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::error::{GraphError, GraphResult};
use crate::id::{ActionId, ConceptId, IdAllocator, RelationId, RuleId};
use crate::model::{Action, ActionPatch, Concept, ConceptPatch, Relation, RelationPatch, Rule, RulePatch};
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

    fn bump_concepts_gen(&self) {
        self.concepts_gen.fetch_add(1, Ordering::Release);
        self.list_concepts_cache.lock().clear();
    }

    fn bump_relations_gen(&self) {
        self.relations_gen.fetch_add(1, Ordering::Release);
        self.list_relations_cache.lock().clear();
    }

    fn index_name_trigrams(&self, name: &str, id: ConceptId) {
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

    pub fn ontology(&self) -> Ontology {
        self.ontology.read().clone()
    }

    pub fn extend_ontology<F>(&self, f: F) -> GraphResult<()>
    where
        F: FnOnce(&mut Ontology) -> GraphResult<()>,
    {
        let mut g = self.ontology.write();
        f(&mut g)
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

    /// Insert a concept. Allocates an id if `concept.id == ConceptId(0)`.
    pub fn upsert_concept(&self, mut concept: Concept) -> GraphResult<ConceptId> {
        {
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
        }
        if concept.id.0 == 0 {
            concept.id = self.ids.next_concept();
        } else {
            self.ids.observe(concept.id.0);
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
        self.name_index.insert(key, concept.id);
        let id = concept.id;
        let sort_key = (concept.concept_type.clone(), concept.name.clone(), id);
        let new_name = concept.name.clone();
        if let Some(prev) = self.concepts.insert(id, concept) {
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
        Ok(id)
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

    pub fn add_relation(&self, mut rel: Relation) -> GraphResult<RelationId> {
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
        let rt = self
            .ontology
            .read()
            .relation_type(&rel.relation_type)
            .cloned()?;
        let symmetric = rt.symmetric;

        // Cardinality + functional enforcement. Counted before id assignment
        // so the rejection path costs nothing extra. The materialized inverse
        // for symmetric relations is pushed directly to the adjacency map
        // without going through add_relation, so it doesn't trip these checks.
        use crate::schema::Cardinality;
        let limits_out = matches!(rt.cardinality, Cardinality::OneToOne | Cardinality::ManyToOne)
            || rt.functional;
        let limits_in = matches!(rt.cardinality, Cardinality::OneToOne | Cardinality::OneToMany);
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
            self.ids.observe(rel.id.0);
        }
        drop(src);
        drop(tgt);

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
        self.relations_sorted.write().insert(id);

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
            self.relations_sorted.write().insert(inv_id);
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

    /// Apply a partial update to an existing relation. Only `weight` and
    /// `properties` are mutable; the adjacency index is unaffected.
    pub fn update_relation(
        &self,
        id: RelationId,
        patch: RelationPatch,
    ) -> GraphResult<Relation> {
        let mut entry = self
            .relations
            .get_mut(&id)
            .ok_or(GraphError::UnknownRelation(id))?;
        if let Some(w) = patch.weight {
            entry.weight = w;
        }
        if let Some(p) = patch.properties {
            entry.properties = p;
        }
        Ok(entry.clone())
    }

    /// Apply a partial update to an existing concept. Renaming updates the
    /// name index; clearing description / replacing properties is in-place.
    /// Returns the new concept. The concept's `concept_type` is immutable —
    /// changing types would require revalidating every incident edge.
    pub fn update_concept(&self, id: ConceptId, patch: ConceptPatch) -> GraphResult<Concept> {
        let mut entry = self
            .concepts
            .get_mut(&id)
            .ok_or(GraphError::UnknownConcept(id))?;

        if let Some(new_name) = patch.name {
            // Maintain (concept_type, lowercase name) → id index.
            let old_key = (entry.concept_type.clone(), entry.name.to_lowercase());
            let new_key = (entry.concept_type.clone(), new_name.to_lowercase());
            if old_key != new_key {
                if let Some(existing) = self.name_index.get(&new_key) {
                    if *existing != id {
                        return Err(GraphError::DuplicateConcept(
                            new_name,
                            entry.concept_type.clone(),
                        ));
                    }
                }
                self.name_index.remove(&old_key);
                self.name_index.insert(new_key, id);
            }
            let old_sort = (entry.concept_type.clone(), entry.name.clone(), id);
            let new_sort = (entry.concept_type.clone(), new_name.clone(), id);
            if old_sort != new_sort {
                let mut idx = self.concepts_sorted.write();
                idx.remove(&old_sort);
                idx.insert(new_sort);
                if let Some(mut bucket) = self.concepts_by_type.get_mut(&entry.concept_type) {
                    bucket.remove(&(entry.name.clone(), id));
                    bucket.insert((new_name.clone(), id));
                }
                self.deindex_name_trigrams(&entry.name, id);
                self.index_name_trigrams(&new_name, id);
            }
            entry.name = new_name;
        }
        if let Some(d) = patch.description {
            entry.description = d;
        }
        if let Some(props) = patch.properties {
            let onto = self.ontology.read();
            let ct = onto.concept_type(&entry.concept_type)?;
            if let Some(allowed) = &ct.properties {
                for k in props.keys() {
                    if !allowed.iter().any(|a| a == k) {
                        return Err(GraphError::InvalidProperty {
                            property: k.clone(),
                            concept_type: ct.name.clone(),
                        });
                    }
                }
            }
            for req in &ct.required_properties {
                if !props.contains_key(req) {
                    return Err(GraphError::MissingRequiredProperty {
                        property: req.clone(),
                        concept_type: ct.name.clone(),
                    });
                }
            }
            entry.properties = props;
        }
        let snapshot = entry.clone();
        drop(entry);
        self.bump_concepts_gen();
        Ok(snapshot)
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
        self.concepts_sorted
            .write()
            .remove(&(concept.concept_type.clone(), concept.name.clone(), id));
        if let Some(mut bucket) = self.concepts_by_type.get_mut(&concept.concept_type) {
            bucket.remove(&(concept.name.clone(), id));
        }
        self.deindex_name_trigrams(&concept.name, id);

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
                self.relations_sorted.write().remove(rid);
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
        self.relations_sorted.write().remove(&id);
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
    pub fn for_each_neighbor<F>(&self, node: ConceptId, direction: crate::traversal::Direction, mut f: F)
    where
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

    /// Insert (or replace) a rule. Allocates an id when `rule.id == RuleId(0)`.
    /// The named `rule_type` and every concept id referenced in `applies_to`
    /// must already exist; otherwise the call fails.
    pub fn upsert_rule(&self, mut rule: Rule) -> GraphResult<RuleId> {
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
            self.ids.observe(rule.id.0);
        }
        let id = rule.id;
        let sort_key = (rule.rule_type.clone(), rule.name.clone(), id);
        if let Some(prev) = self.rules.insert(id, rule) {
            let old = (prev.rule_type, prev.name, id);
            if old != sort_key {
                self.rules_sorted.write().remove(&old);
            }
        }
        self.rules_sorted.write().insert(sort_key);
        Ok(id)
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
        self.rules_sorted
            .write()
            .remove(&(removed.rule_type, removed.name, id));
        Ok(())
    }

    /// Apply a partial update to an existing rule. `rule_type` cannot be
    /// changed; every concept id referenced in a replacement `applies_to`
    /// must already exist.
    pub fn update_rule(&self, id: RuleId, patch: RulePatch) -> GraphResult<Rule> {
        if let Some(applies) = &patch.applies_to {
            for cid in applies {
                if !self.concepts.contains_key(cid) {
                    return Err(GraphError::UnknownConcept(*cid));
                }
            }
        }
        let mut entry = self
            .rules
            .get_mut(&id)
            .ok_or(GraphError::UnknownRelationType(format!("rule {id}")))?;
        if let Some(n) = patch.name {
            let old_sort = (entry.rule_type.clone(), entry.name.clone(), id);
            let new_sort = (entry.rule_type.clone(), n.clone(), id);
            if old_sort != new_sort {
                let mut idx = self.rules_sorted.write();
                idx.remove(&old_sort);
                idx.insert(new_sort);
            }
            entry.name = n;
        }
        if let Some(w) = patch.when {
            entry.when = w;
        }
        if let Some(t) = patch.then {
            entry.then = t;
        }
        if let Some(a) = patch.applies_to {
            entry.applies_to = a;
        }
        if let Some(s) = patch.strict {
            entry.strict = s;
        }
        if let Some(d) = patch.description {
            entry.description = d;
        }
        if let Some(p) = patch.properties {
            entry.properties = p;
        }
        Ok(entry.clone())
    }

    // ---------- actions ----------

    /// Insert (or replace) an action. The `action_type` must be declared
    /// in the ontology and both endpoints (`subject`, optional `object`)
    /// must exist as concepts.
    pub fn upsert_action(&self, mut action: Action) -> GraphResult<ActionId> {
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
            self.ids.observe(action.id.0);
        }
        let id = action.id;
        let sort_key = (action.action_type.clone(), action.name.clone(), id);
        if let Some(prev) = self.actions.insert(id, action) {
            let old = (prev.action_type, prev.name, id);
            if old != sort_key {
                self.actions_sorted.write().remove(&old);
            }
        }
        self.actions_sorted.write().insert(sort_key);
        Ok(id)
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

                let mut survivors: Vec<(String, String, ConceptId, Concept)> =
                    Vec::with_capacity(candidates.len());
                let cap = offset.saturating_add(limit);
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
                    if !track_total && survivors.len() >= cap {
                        break;
                    }
                }
                survivors.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
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
        let mut page: Vec<Concept> = Vec::with_capacity(limit);

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
                use std::collections::BinaryHeap;
                use std::cmp::Reverse;
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
                    if a.len() <= b.len() { a } else { b }
                }
                (_, _, Some(a), None) => a,
                (_, _, None, Some(b)) => b,
                (Some(s), Some(t), None, None) => {
                    let from_src = self
                        .out_edges
                        .get(&s)
                        .map(|adj| adj.len())
                        .unwrap_or(0);
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
            let mut page: Vec<Relation> = Vec::with_capacity(limit);
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
        let mut page: Vec<Relation> = Vec::with_capacity(limit);
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
        self.actions_sorted
            .write()
            .remove(&(removed.action_type, removed.name, id));
        Ok(())
    }

    /// Apply a partial update to an existing action. `action_type` cannot
    /// be changed; replacement `subject` / `object` concept ids must exist.
    pub fn update_action(
        &self,
        id: ActionId,
        patch: ActionPatch,
    ) -> GraphResult<Action> {
        if let Some(subj) = patch.subject {
            if !self.concepts.contains_key(&subj) {
                return Err(GraphError::UnknownConcept(subj));
            }
        }
        if let Some(obj_opt) = &patch.object {
            if let Some(obj) = obj_opt {
                if !self.concepts.contains_key(obj) {
                    return Err(GraphError::UnknownConcept(*obj));
                }
            }
        }
        let mut entry = self
            .actions
            .get_mut(&id)
            .ok_or(GraphError::UnknownRelationType(format!("action {id}")))?;
        if let Some(n) = patch.name {
            let old_sort = (entry.action_type.clone(), entry.name.clone(), id);
            let new_sort = (entry.action_type.clone(), n.clone(), id);
            if old_sort != new_sort {
                let mut idx = self.actions_sorted.write();
                idx.remove(&old_sort);
                idx.insert(new_sort);
            }
            entry.name = n;
        }
        if let Some(s) = patch.subject {
            entry.subject = s;
        }
        if let Some(o) = patch.object {
            entry.object = o;
        }
        if let Some(p) = patch.parameters {
            entry.parameters = p;
        }
        if let Some(e) = patch.effect {
            entry.effect = e;
        }
        if let Some(d) = patch.description {
            entry.description = d;
        }
        Ok(entry.clone())
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
        g.upsert_concept(Concept::new(Default::default(), "Person", "Alice")).unwrap();
        g.upsert_concept(Concept::new(Default::default(), "Researcher", "Bob")).unwrap();
        let (total, _) = g.list_concepts_page(Some("Person"), None, 0, 10, true, false);
        assert_eq!(total, 1);
        let (total2, page2) = g.list_concepts_page(Some("Person"), None, 0, 10, true, true);
        assert_eq!(total2, 2);
        assert_eq!(page2.len(), 2);
    }

    #[test]
    fn ontology_descendants_includes_self_and_children() {
        let mut o = Ontology::new();
        o.add_concept_type(ConceptType { name: "Animal".into(), ..Default::default() });
        o.add_concept_type(ConceptType { name: "Mammal".into(), parent: Some("Animal".into()), ..Default::default() });
        o.add_concept_type(ConceptType { name: "Dog".into(), parent: Some("Mammal".into()), ..Default::default() });
        let mut d = o.descendants("Animal");
        d.sort();
        assert_eq!(d, vec!["Animal".to_string(), "Dog".into(), "Mammal".into()]);
        assert!(o.descendants("Unknown").is_empty());
    }

    #[test]
    fn closure_walks_transitive_chain() {
        let mut o = Ontology::new();
        o.add_concept_type(ConceptType { name: "Component".into(), ..Default::default() });
        o.add_relation_type(RelationType {
            name: "partOf".into(),
            domain: "Component".into(),
            range: "Component".into(),
            transitive: true,
            ..Default::default()
        })
        .unwrap();
        let g = OntologyGraph::new(o);
        let wheel = g.upsert_concept(Concept::new(Default::default(), "Component", "Wheel")).unwrap();
        let car = g.upsert_concept(Concept::new(Default::default(), "Component", "Car")).unwrap();
        let fleet = g.upsert_concept(Concept::new(Default::default(), "Component", "Fleet")).unwrap();
        g.add_relation(Relation::new(Default::default(), "partOf", wheel, car)).unwrap();
        g.add_relation(Relation::new(Default::default(), "partOf", car, fleet)).unwrap();
        let reached = g.closure(wheel, "partOf", 3).unwrap();
        assert!(reached.contains(&car));
        assert!(reached.contains(&fleet));
    }

    #[test]
    fn inverse_of_surfaces_virtual_edges() {
        let mut o = Ontology::new();
        o.add_concept_type(ConceptType { name: "Person".into(), ..Default::default() });
        o.add_concept_type(ConceptType { name: "Paper".into(), ..Default::default() });
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
        let alice = g.upsert_concept(Concept::new(Default::default(), "Person", "Alice")).unwrap();
        let p1 = g.upsert_concept(Concept::new(Default::default(), "Paper", "P1")).unwrap();
        g.add_relation(Relation::new(Default::default(), "authored", alice, p1)).unwrap();
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
        assert!(matches!(err, Err(GraphError::MissingRequiredProperty { .. })));
        let mut props = AHashMap::new();
        props.insert("email".into(), PropertyValue::Text("a@b".into()));
        let mut c = Concept::new(Default::default(), "Person", "Alice");
        c.properties = props.clone();
        let id = g.upsert_concept(c).unwrap();
        // update_concept replacing properties without email must also fail.
        let err = g.update_concept(id, ConceptPatch { properties: Some(AHashMap::new()), ..Default::default() });
        assert!(matches!(err, Err(GraphError::MissingRequiredProperty { .. })));
    }

    #[test]
    fn one_to_many_rejects_second_inbound() {
        let mut o = Ontology::new();
        o.add_concept_type(ConceptType { name: "Person".into(), ..Default::default() });
        o.add_concept_type(ConceptType { name: "Paper".into(), ..Default::default() });
        o.add_relation_type(RelationType {
            name: "authored".into(),
            domain: "Person".into(),
            range: "Paper".into(),
            cardinality: crate::schema::Cardinality::OneToMany,
            ..Default::default()
        })
        .unwrap();
        let g = OntologyGraph::new(o);
        let alice = g.upsert_concept(Concept::new(Default::default(), "Person", "Alice")).unwrap();
        let bob = g.upsert_concept(Concept::new(Default::default(), "Person", "Bob")).unwrap();
        let p = g.upsert_concept(Concept::new(Default::default(), "Paper", "P")).unwrap();
        g.add_relation(Relation::new(Default::default(), "authored", alice, p)).unwrap();
        let err = g.add_relation(Relation::new(Default::default(), "authored", bob, p));
        assert!(matches!(err, Err(GraphError::CardinalityViolation { .. })));
    }

    #[test]
    fn functional_relation_rejects_second_outbound() {
        let mut o = Ontology::new();
        o.add_concept_type(ConceptType { name: "Person".into(), ..Default::default() });
        o.add_relation_type(RelationType {
            name: "spouse".into(),
            domain: "Person".into(),
            range: "Person".into(),
            functional: true,
            ..Default::default()
        })
        .unwrap();
        let g = OntologyGraph::new(o);
        let a = g.upsert_concept(Concept::new(Default::default(), "Person", "A")).unwrap();
        let b = g.upsert_concept(Concept::new(Default::default(), "Person", "B")).unwrap();
        let c = g.upsert_concept(Concept::new(Default::default(), "Person", "C")).unwrap();
        g.add_relation(Relation::new(Default::default(), "spouse", a, b)).unwrap();
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
        o.add_concept_type(ConceptType { name: "Robot".into(), ..Default::default() });
        let g = OntologyGraph::new(o);
        g.upsert_concept(Concept::new(Default::default(), "Person", "Alice")).unwrap();
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
}
