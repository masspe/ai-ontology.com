// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

use async_trait::async_trait;
use ontology_graph::{Concept, ConceptId, GraphError, Ontology, OntologyGraph, Relation};
use ontology_storage::{LogRecord, Store};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;

use crate::record::Record;

/// Drains the entire graph through `sink` as a stream of `Ontology` ->
/// `Concept`(s) -> `Relation`(s) records. The output round-trips through
/// `ingest_records` to rebuild the same graph.
pub async fn export_graph<S: Sink + ?Sized>(
    graph: &Arc<OntologyGraph>,
    sink: &mut S,
) -> Result<ExportStats, IngestError> {
    let mut stats = ExportStats::default();
    sink.write(&Record::Ontology(graph.ontology())).await?;

    let concepts = graph.all_concepts();
    let ontology = graph.ontology();
    let mut relation_ids: std::collections::HashSet<ontology_graph::RelationId> =
        std::collections::HashSet::new();
    // Symmetric edges materialize an inverse on insert, so the graph holds
    // both directions. Re-ingest would materialize another inverse — emit
    // only the canonical direction.
    let mut seen_symmetric: std::collections::HashSet<(
        String,
        ontology_graph::ConceptId,
        ontology_graph::ConceptId,
    )> = std::collections::HashSet::new();
    for c in &concepts {
        sink.write(&Record::Concept(c.clone())).await?;
        stats.concepts += 1;
    }
    for c in &concepts {
        for r in graph.outgoing(c.id) {
            if !relation_ids.insert(r.id) {
                continue;
            }
            let symmetric = ontology
                .relation_types
                .get(&r.relation_type)
                .map(|rt| rt.symmetric)
                .unwrap_or(false);
            if symmetric {
                let (a, b) = if r.source <= r.target {
                    (r.source, r.target)
                } else {
                    (r.target, r.source)
                };
                if !seen_symmetric.insert((r.relation_type.clone(), a, b)) {
                    continue;
                }
            }
            sink.write(&Record::Relation(r)).await?;
            stats.relations += 1;
        }
    }
    for rule in graph.all_rules() {
        sink.write(&Record::Rule(rule)).await?;
        stats.rules += 1;
    }
    for action in graph.all_actions() {
        sink.write(&Record::Action(action)).await?;
        stats.actions += 1;
    }
    sink.finish().await?;
    Ok(stats)
}

#[derive(Debug, Default, Clone)]
pub struct ExportStats {
    pub concepts: u64,
    pub relations: u64,
    pub rules: u64,
    pub actions: u64,
}

#[derive(Debug, Error)]
pub enum IngestError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("graph: {0}")]
    Graph(#[from] ontology_graph::GraphError),
    #[error("store: {0}")]
    Store(#[from] ontology_storage::StoreError),
    #[error("source error: {0}")]
    Source(String),
    #[error("unknown concept name `{name}` of type `{concept_type}`")]
    UnknownNamed { concept_type: String, name: String },
}

/// A pull-based async record source. Returns `Ok(None)` to signal end of stream.
#[async_trait]
pub trait Source: Send + Sync {
    async fn next(&mut self) -> Result<Option<Record>, IngestError>;
}

/// A push-based async record sink.
#[async_trait]
pub trait Sink: Send + Sync {
    async fn write(&mut self, record: &Record) -> Result<(), IngestError>;
    async fn finish(&mut self) -> Result<(), IngestError> {
        Ok(())
    }
}

/// Maximum number of consecutive concepts journaled under a single
/// durability barrier (`Store::append_batch`). A 10 000-row CSV costs ~40
/// syncs instead of 10 000 (`STORAGE.md` §7.2).
pub const INGEST_BATCH_SIZE: usize = 256;

/// Drains `source` into the graph, optionally journaling every applied
/// record to `store`. Maintains a name -> id mapping so `NamedRelation`
/// records can be resolved without imposing an ordering requirement on
/// the source — concepts can come before or after the relations that
/// reference them, as long as both arrive within the same call.
///
/// # Write-ahead ordering (`STORAGE.md` R8)
///
/// Every instance record is validated (`prepare_*`), **then** journaled,
/// **then** applied to the in-memory graph. If the store rejects a write the
/// graph is left exactly as it was, so memory never runs ahead of disk.
///
/// # Batching
///
/// Runs of consecutive `Concept` records are journaled in batches of
/// [`INGEST_BATCH_SIZE`] under one durability barrier. A concept is only
/// admitted to a batch if it is valid against the graph *and* against the
/// concepts already waiting in the batch (duplicate or disjoint names), so
/// replaying a durable prefix of a batch can never fail. Any other record
/// kind flushes the batch first, because it may depend on those concepts.
///
/// # Schema records
///
/// Type declarations (`ConceptTypeDecl`, …) are applied to the in-memory
/// ontology as they arrive but journaled as **one** `Ontology` record, just
/// before the first instance that could depend on them (and at the end).
/// This replaces the previous one-snapshot-per-declaration behaviour that
/// made the ontology ~95 % of a typical log. The trade-off is explicit: a
/// store failure at that flush leaves schema *types* (never instances) in
/// memory that are not on disk; the ingest is aborted with the error.
pub async fn ingest_records<S: Source + ?Sized>(
    source: &mut S,
    graph: &Arc<OntologyGraph>,
    store: Option<&dyn Store>,
) -> Result<IngestStats, IngestError> {
    let mut ing = Ingester::new(graph, store);
    let outcome = ing.run(source).await;
    if outcome.is_err() {
        // Whatever failed, the live ontology must not keep type declarations
        // the store never saw (R8 for the schema): a later record of such a
        // type would be accepted by the graph, journaled, and refused on
        // replay because its type is not on disk.
        ing.rollback_schema();
    }
    outcome.map(|()| ing.stats)
}

/// Streaming state of one `ingest_records` call: the concept batch waiting
/// for its durability barrier, and whether schema changes await journaling.
struct Ingester<'a> {
    graph: &'a Arc<OntologyGraph>,
    store: Option<&'a dyn Store>,
    stats: IngestStats,
    /// Concepts already validated and id-allocated (`prepare_concept`), not
    /// yet journaled nor applied.
    pending: Vec<Concept>,
    /// `(concept_type, lowercase name)` → id for every pending concept, so
    /// intra-batch duplicates are caught before anything hits the disk.
    pending_names: HashMap<(String, String), ConceptId>,
    /// The in-memory ontology has type declarations not yet journaled.
    schema_dirty: bool,
    /// The ontology as it was before the first un-journaled declaration;
    /// restored if the ingest fails before the schema is flushed.
    schema_baseline: Option<Ontology>,
}

impl<'a> Ingester<'a> {
    fn new(graph: &'a Arc<OntologyGraph>, store: Option<&'a dyn Store>) -> Self {
        Self {
            graph,
            store,
            stats: IngestStats::default(),
            pending: Vec::new(),
            pending_names: HashMap::new(),
            schema_dirty: false,
            schema_baseline: None,
        }
    }

    /// Drain the source, then the deferred records, flushing at the end.
    async fn run<S: Source + ?Sized>(&mut self, source: &mut S) -> Result<(), IngestError> {
        // Buffer NamedRelations (and other dependent records) whose
        // prerequisites haven't shown up yet.
        let mut deferred: Vec<Record> = Vec::new();
        while let Some(rec) = source.next().await? {
            if !self.apply(&rec).await? {
                deferred.push(rec);
            }
        }
        self.flush().await?;
        // Retry deferred records once both endpoints should now exist.
        for rec in deferred {
            if !self.apply(&rec).await? {
                if let Record::NamedRelation {
                    source_type,
                    source_name,
                    ..
                } = &rec
                {
                    return Err(IngestError::UnknownNamed {
                        concept_type: source_type.clone(),
                        name: source_name.clone(),
                    });
                }
            }
        }
        self.flush().await
    }

    /// Remember the schema before the first un-journaled declaration.
    fn begin_schema_change(&mut self) {
        if self.schema_baseline.is_none() {
            self.schema_baseline = Some(self.graph.ontology());
        }
    }

    /// Put the live ontology back to what it was before the declarations
    /// that were never journaled. No instance of those types can exist:
    /// instances are only applied after the schema is flushed.
    fn rollback_schema(&mut self) {
        if !self.schema_dirty {
            return;
        }
        if let Some(baseline) = self.schema_baseline.take() {
            // Restoring a previously valid schema cannot be refused: no
            // instance of the withdrawn types exists yet.
            let _ = self.graph.extend_ontology(|o| {
                *o = baseline;
                Ok(())
            });
        }
        self.schema_dirty = false;
    }

    /// Journal the ontology if type declarations are waiting. Must precede
    /// any instance record that may reference the new types.
    async fn flush_schema(&mut self) -> Result<(), IngestError> {
        if !self.schema_dirty {
            return Ok(());
        }
        if let Some(s) = self.store {
            s.append(&LogRecord::ontology(self.graph.ontology()))
                .await?;
        }
        self.schema_dirty = false;
        self.schema_baseline = None;
        Ok(())
    }

    /// Settle the concept batch if there is one (schema first, as always).
    /// A no-op when nothing is pending, so a run of type declarations does
    /// not journal the ontology once per declaration.
    async fn settle_pending(&mut self) -> Result<(), IngestError> {
        if self.pending.is_empty() {
            return Ok(());
        }
        self.flush().await
    }

    /// Durability barrier: journal pending schema and concepts, then apply
    /// the concepts to the graph. On store failure nothing is applied and
    /// the batch is dropped (its ids stay burnt — harmless).
    async fn flush(&mut self) -> Result<(), IngestError> {
        self.flush_schema().await?;
        if self.pending.is_empty() {
            return Ok(());
        }
        let batch = std::mem::take(&mut self.pending);
        self.pending_names.clear();
        if let Some(s) = self.store {
            let records: Vec<LogRecord> = batch.iter().cloned().map(LogRecord::concept).collect();
            s.append_batch(&records).await?;
        }
        for c in batch {
            // Cannot fail semantically: prepare_concept + the pending checks
            // already rejected everything apply would.
            self.graph.apply_prepared_concept(c)?;
            self.stats.concepts += 1;
        }
        Ok(())
    }

    /// Journal one non-concept instance record, after the barrier.
    async fn journal(&mut self, record: LogRecord) -> Result<(), IngestError> {
        self.flush().await?;
        if let Some(s) = self.store {
            s.append(&record).await?;
        }
        Ok(())
    }

    /// Reject a concept that collides with one already waiting in the batch
    /// — the graph cannot see those yet, so `prepare_concept` alone would
    /// let the second one through and the *apply* would fail after the
    /// records are durable.
    fn check_pending_conflicts(&self, c: &Concept) -> Result<(), IngestError> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let lname = c.name.to_lowercase();
        if let Some(existing) = self
            .pending_names
            .get(&(c.concept_type.clone(), lname.clone()))
        {
            if c.id.0 == 0 || *existing != c.id {
                return Err(
                    GraphError::DuplicateConcept(c.name.clone(), c.concept_type.clone()).into(),
                );
            }
        }
        let disjoint: Vec<String> = self.graph.with_ontology(|o| {
            o.concept_types
                .get(&c.concept_type)
                .map(|ct| ct.disjoint_with.clone())
                .unwrap_or_default()
        });
        for other in disjoint {
            if self
                .pending_names
                .contains_key(&(other.clone(), lname.clone()))
            {
                return Err(GraphError::DisjointTypeViolation {
                    type_a: c.concept_type.clone(),
                    type_b: other,
                }
                .into());
            }
        }
        Ok(())
    }

    async fn stage_concept(&mut self, mut c: Concept) -> Result<(), IngestError> {
        // An explicit id that is already pending (same entity twice in one
        // batch, e.g. a rename mid-export) is an upsert over an unapplied
        // record: settle the batch first rather than reason about it.
        if c.id.0 != 0 && self.pending.iter().any(|p| p.id == c.id) {
            self.flush().await?;
        }
        self.check_pending_conflicts(&c)?;
        self.graph.prepare_concept(&mut c)?;
        self.pending_names
            .insert((c.concept_type.clone(), c.name.to_lowercase()), c.id);
        self.pending.push(c);
        if self.pending.len() >= INGEST_BATCH_SIZE {
            self.flush().await?;
        }
        Ok(())
    }

    async fn add_relation(&mut self, mut rel: Relation) -> Result<(), IngestError> {
        // Endpoints must be applied, so settle the batch before validating.
        self.flush().await?;
        self.graph.prepare_relation(&mut rel)?;
        self.journal(LogRecord::relation(rel.clone())).await?;
        self.graph.apply_prepared_relation(rel)?;
        self.stats.relations += 1;
        Ok(())
    }

    /// Returns `Ok(false)` when the record must be retried later.
    async fn apply(&mut self, rec: &Record) -> Result<bool, IngestError> {
        match rec {
            Record::Ontology(o) => {
                // Full schema replacement: durable first, then live.
                self.flush().await?;
                if let Some(s) = self.store {
                    s.append(&LogRecord::ontology(o.clone())).await?;
                }
                self.graph.extend_ontology(|target| {
                    *target = o.clone();
                    Ok(())
                })?;
                self.schema_dirty = false;
                self.schema_baseline = None;
                self.stats.ontology_updates += 1;
                Ok(true)
            }
            Record::Concept(c) => {
                self.stage_concept(c.clone()).await?;
                Ok(true)
            }
            Record::Relation(r) => {
                self.add_relation(r.clone()).await?;
                Ok(true)
            }
            Record::NamedRelation {
                relation_type,
                source_type,
                source_name,
                target_type,
                target_name,
                weight,
            } => {
                let mut src = self.graph.find_by_name(source_type, source_name);
                let mut tgt = self.graph.find_by_name(target_type, target_name);
                if (src.is_none() || tgt.is_none()) && !self.pending.is_empty() {
                    // The endpoints may be sitting in the batch.
                    self.flush().await?;
                    src = self.graph.find_by_name(source_type, source_name);
                    tgt = self.graph.find_by_name(target_type, target_name);
                }
                match (src, tgt) {
                    (Some(s), Some(t)) => {
                        let mut rel =
                            Relation::new(Default::default(), relation_type.clone(), s, t);
                        rel.weight = *weight;
                        self.add_relation(rel).await?;
                        Ok(true)
                    }
                    _ => Ok(false),
                }
            }
            Record::ConceptTypeDecl(ct) => {
                // Pending concepts were validated against the current schema;
                // settle them before it changes. The schema itself is only
                // journaled once, before the first instance that needs it.
                self.settle_pending().await?;
                self.begin_schema_change();
                self.graph.extend_ontology(|o| {
                    o.add_concept_type(ct.clone());
                    Ok(())
                })?;
                self.schema_dirty = true;
                self.stats.ontology_updates += 1;
                Ok(true)
            }
            Record::RelationTypeDecl(rt) => {
                // Deferred if domain/range aren't known yet — let the retry pass
                // see the concept types first.
                let known = self.graph.with_ontology(|o| {
                    o.concept_types.contains_key(&rt.domain)
                        && o.concept_types.contains_key(&rt.range)
                });
                if !known {
                    return Ok(false);
                }
                self.settle_pending().await?;
                self.begin_schema_change();
                self.graph
                    .extend_ontology(|o| o.add_relation_type(rt.clone()))?;
                self.schema_dirty = true;
                self.stats.ontology_updates += 1;
                Ok(true)
            }
            Record::RuleTypeDecl(rule) => {
                let known = self.graph.with_ontology(|o| {
                    rule.applies_to
                        .iter()
                        .all(|t| o.concept_types.contains_key(t))
                });
                if !known {
                    return Ok(false);
                }
                self.settle_pending().await?;
                self.begin_schema_change();
                self.graph
                    .extend_ontology(|o| o.add_rule_type(rule.clone()))?;
                self.schema_dirty = true;
                self.stats.ontology_updates += 1;
                Ok(true)
            }
            Record::ActionTypeDecl(action) => {
                let known = self.graph.with_ontology(|o| {
                    o.concept_types.contains_key(&action.subject)
                        && action
                            .object
                            .as_ref()
                            .map(|t| o.concept_types.contains_key(t))
                            .unwrap_or(true)
                });
                if !known {
                    return Ok(false);
                }
                self.settle_pending().await?;
                self.begin_schema_change();
                self.graph
                    .extend_ontology(|o| o.add_action_type(action.clone()))?;
                self.schema_dirty = true;
                self.stats.ontology_updates += 1;
                Ok(true)
            }
            Record::Rule(rule) => {
                // Need the rule_type and all referenced concepts to exist —
                // including any still in the batch.
                self.flush().await?;
                let ready = self
                    .graph
                    .with_ontology(|o| o.rule_type(&rule.rule_type).is_some())
                    && rule
                        .applies_to
                        .iter()
                        .all(|cid| self.graph.get_concept(*cid).is_ok());
                if !ready {
                    return Ok(false);
                }
                let mut r = rule.clone();
                self.graph.prepare_rule(&mut r)?;
                self.journal(LogRecord::rule(r.clone())).await?;
                self.graph.apply_prepared_rule(r)?;
                self.stats.rules += 1;
                Ok(true)
            }
            Record::Action(action) => {
                self.flush().await?;
                let ready = self
                    .graph
                    .with_ontology(|o| o.action_type(&action.action_type).is_some())
                    && self.graph.get_concept(action.subject).is_ok()
                    && action
                        .object
                        .map(|cid| self.graph.get_concept(cid).is_ok())
                        .unwrap_or(true);
                if !ready {
                    return Ok(false);
                }
                let mut a = action.clone();
                self.graph.prepare_action(&mut a)?;
                self.journal(LogRecord::action(a.clone())).await?;
                self.graph.apply_prepared_action(a)?;
                self.stats.actions += 1;
                Ok(true)
            }
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct IngestStats {
    pub concepts: u64,
    pub relations: u64,
    pub ontology_updates: u64,
    pub rules: u64,
    pub actions: u64,
}
