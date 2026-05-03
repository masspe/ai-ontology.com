// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

use async_trait::async_trait;
use ontology_graph::{OntologyGraph, Relation};
use ontology_storage::{LogRecord, Store};
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
    sink.finish().await?;
    Ok(stats)
}

#[derive(Debug, Default, Clone)]
pub struct ExportStats {
    pub concepts: u64,
    pub relations: u64,
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

/// Drains `source` into the graph, optionally journaling every applied
/// record to `store`. Maintains a name -> id mapping so `NamedRelation`
/// records can be resolved without imposing an ordering requirement on
/// the source — concepts can come before or after the relations that
/// reference them, as long as both arrive within the same call.
pub async fn ingest_records<S: Source + ?Sized>(
    source: &mut S,
    graph: &Arc<OntologyGraph>,
    store: Option<&dyn Store>,
) -> Result<IngestStats, IngestError> {
    let mut stats = IngestStats::default();
    // Buffer NamedRelations whose endpoints haven't shown up yet.
    let mut deferred: Vec<Record> = Vec::new();

    while let Some(rec) = source.next().await? {
        if !apply_record(&rec, graph, store, &mut stats).await? {
            deferred.push(rec);
        }
    }

    // Retry deferred records once both endpoints should now exist.
    for rec in deferred {
        if !apply_record(&rec, graph, store, &mut stats).await? {
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

    Ok(stats)
}

#[derive(Debug, Default, Clone)]
pub struct IngestStats {
    pub concepts: u64,
    pub relations: u64,
    pub ontology_updates: u64,
}

async fn apply_record(
    rec: &Record,
    graph: &Arc<OntologyGraph>,
    store: Option<&dyn Store>,
    stats: &mut IngestStats,
) -> Result<bool, IngestError> {
    match rec {
        Record::Ontology(o) => {
            graph.extend_ontology(|target| {
                *target = o.clone();
                Ok(())
            })?;
            if let Some(s) = store {
                s.append(&LogRecord::ontology(o.clone())).await?;
            }
            stats.ontology_updates += 1;
            Ok(true)
        }
        Record::Concept(c) => {
            let mut c = c.clone();
            let id = graph.upsert_concept(c.clone())?;
            c.id = id;
            if let Some(s) = store {
                s.append(&LogRecord::concept(c)).await?;
            }
            stats.concepts += 1;
            Ok(true)
        }
        Record::Relation(r) => {
            let id = graph.add_relation(r.clone())?;
            let mut r = r.clone();
            r.id = id;
            if let Some(s) = store {
                s.append(&LogRecord::relation(r)).await?;
            }
            stats.relations += 1;
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
            let src = graph.find_by_name(source_type, source_name);
            let tgt = graph.find_by_name(target_type, target_name);
            match (src, tgt) {
                (Some(s), Some(t)) => {
                    let mut rel = Relation::new(Default::default(), relation_type.clone(), s, t);
                    rel.weight = *weight;
                    let id = graph.add_relation(rel.clone())?;
                    rel.id = id;
                    if let Some(store) = store {
                        store.append(&LogRecord::relation(rel)).await?;
                    }
                    stats.relations += 1;
                    Ok(true)
                }
                _ => Ok(false),
            }
        }
    }
}
