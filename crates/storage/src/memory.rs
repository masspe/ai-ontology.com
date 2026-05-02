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
    pub fn new() -> Self { Self::default() }
    pub fn len(&self) -> usize { self.inner.lock().len() }
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
        RecordKind::Concept(c) => { graph.upsert_concept(c)?; }
        RecordKind::Relation(rel) => { graph.add_relation(rel)?; }
        RecordKind::DeleteConcept(id) => {
            // Idempotent — replay over a snapshot may try to delete twice.
            let _ = graph.remove_concept(id);
        }
        RecordKind::DeleteRelation(id) => {
            let _ = graph.remove_relation(id);
        }
    }
    Ok(())
}
