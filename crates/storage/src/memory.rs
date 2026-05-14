// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

use async_trait::async_trait;
use ontology_graph::OntologyGraph;
use parking_lot::Mutex;
use std::sync::Arc;

use crate::log::{LogRecord, RecordKind};
use crate::store::{Store, StoreResult};

/// Records-in-RAM store. Useful for tests and short-lived sessions.
#[derive(Debug, Default)]
pub struct MemoryStore {
    inner: Mutex<Vec<LogRecord>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.inner.lock().len()
    }
    pub fn is_empty(&self) -> bool {
        self.inner.lock().is_empty()
    }
}

#[async_trait]
impl Store for MemoryStore {
    async fn append(&self, record: &LogRecord) -> StoreResult<()> {
        let mut g = self.inner.lock();
        let mut r = record.clone();
        r.seq = g.len() as u64 + 1;
        g.push(r);
        Ok(())
    }

    async fn load_into(&self, graph: &Arc<OntologyGraph>) -> StoreResult<()> {
        let records = self.inner.lock().clone();
        for r in records {
            apply(graph, r)?;
        }
        Ok(())
    }
}

pub(crate) fn apply(graph: &Arc<OntologyGraph>, r: LogRecord) -> StoreResult<()> {
    match r.kind {
        RecordKind::Ontology(o) => {
            graph.extend_ontology(|target| {
                *target = o;
                Ok(())
            })?;
        }
        RecordKind::Concept(c) => {
            graph.upsert_concept(c)?;
        }
        RecordKind::Relation(rel) => {
            graph.add_relation(rel)?;
        }
        RecordKind::UpdateRelation(rel) => {
            if graph.get_relation(rel.id).is_ok() {
                graph.update_relation(
                    rel.id,
                    ontology_graph::RelationPatch {
                        weight: Some(rel.weight),
                        properties: Some(rel.properties.clone()),
                    },
                )?;
            } else {
                graph.add_relation(rel)?;
            }
        }
        RecordKind::UpdateConcept(c) => {
            // If the concept already exists, drive update_concept so the
            // rename path cleans the name-index. Otherwise treat the update
            // as a create (defensive — shouldn't happen in normal logs).
            if graph.get_concept(c.id).is_ok() {
                graph.update_concept(
                    c.id,
                    ontology_graph::ConceptPatch {
                        name: Some(c.name.clone()),
                        description: Some(c.description.clone()),
                        properties: Some(c.properties.clone()),
                    },
                )?;
            } else {
                graph.upsert_concept(c)?;
            }
        }
        RecordKind::DeleteConcept(id) => {
            // Idempotent — replay over a snapshot may try to delete twice.
            let _ = graph.remove_concept(id);
        }
        RecordKind::DeleteRelation(id) => {
            let _ = graph.remove_relation(id);
        }
        RecordKind::Rule(r) => {
            graph.upsert_rule(r)?;
        }
        RecordKind::Action(a) => {
            graph.upsert_action(a)?;
        }
        RecordKind::DeleteRule(id) => {
            let _ = graph.remove_rule(id);
        }
        RecordKind::DeleteAction(id) => {
            let _ = graph.remove_action(id);
        }
    }
    Ok(())
}
