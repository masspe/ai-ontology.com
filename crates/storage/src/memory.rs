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
    /// Snapshot of every record appended so far, in order.
    pub fn records(&self) -> Vec<LogRecord> {
        self.inner.lock().clone()
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

    async fn reset(&self) -> StoreResult<()> {
        self.inner.lock().clear();
        Ok(())
    }

    async fn load_into(&self, graph: &Arc<OntologyGraph>) -> StoreResult<()> {
        let records = self.inner.lock().clone();
        let bulk = graph.begin_bulk();
        for r in records {
            apply(graph, r)?;
        }
        bulk.finish();
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
        RecordKind::RelationExact(rel) => {
            graph.insert_relation_exact(rel)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use ontology_graph::{Concept, ConceptId, ConceptType, Ontology};

    fn schema() -> Ontology {
        let mut o = Ontology::new();
        o.add_concept_type(ConceptType {
            name: "Person".into(),
            ..Default::default()
        });
        o
    }

    /// `reset` empties the store durably (here: in RAM) and the store keeps
    /// accepting records that replay on their own afterwards.
    #[tokio::test]
    async fn reset_clears_every_record_and_the_store_stays_usable() {
        let store = MemoryStore::new();
        store.append(&LogRecord::ontology(schema())).await.unwrap();
        store
            .append(&LogRecord::concept(Concept::new(
                ConceptId(1),
                "Person",
                "a",
            )))
            .await
            .unwrap();
        assert_eq!(store.len(), 2);
        assert_eq!(store.records()[1].seq, 2);

        store.reset().await.unwrap();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
        let g = OntologyGraph::with_arc(Ontology::new());
        store.load_into(&g).await.unwrap();
        assert_eq!(g.concept_count(), 0);
        assert!(g.with_ontology(|o| o.concept_types.is_empty()));

        store.append(&LogRecord::ontology(schema())).await.unwrap();
        store
            .append(&LogRecord::concept(Concept::new(
                ConceptId(1),
                "Person",
                "b",
            )))
            .await
            .unwrap();
        let g = OntologyGraph::with_arc(Ontology::new());
        store.load_into(&g).await.unwrap();
        assert_eq!(g.concept_count(), 1);
        assert!(g.find_by_name("Person", "b").is_some());
        // Idempotent.
        store.reset().await.unwrap();
        store.reset().await.unwrap();
        assert!(store.is_empty());
    }

    /// Replay is idempotent for tombstones: a `DeleteConcept` of an unknown
    /// id (already gone, or never there) is a no-op, not an error.
    #[tokio::test]
    async fn replaying_a_tombstone_of_an_unknown_entity_is_a_no_op() {
        let store = MemoryStore::new();
        store.append(&LogRecord::ontology(schema())).await.unwrap();
        store
            .append(&LogRecord::concept(Concept::new(
                ConceptId(1),
                "Person",
                "a",
            )))
            .await
            .unwrap();
        store
            .append(&LogRecord::delete_concept_unrouted(ConceptId(1)))
            .await
            .unwrap();
        store
            .append(&LogRecord::delete_concept_unrouted(ConceptId(1)))
            .await
            .unwrap();
        store
            .append(&LogRecord::delete_concept_unrouted(ConceptId(42)))
            .await
            .unwrap();
        let g = OntologyGraph::with_arc(Ontology::new());
        store.load_into(&g).await.unwrap();
        assert_eq!(g.concept_count(), 0);
    }
}
