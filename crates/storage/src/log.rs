use ontology_graph::{Concept, Ontology, Relation};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecordKind {
    Ontology(Ontology),
    Concept(Concept),
    Relation(Relation),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogRecord {
    /// Logical sequence number assigned by the store; 0 means "not yet assigned".
    #[serde(default)]
    pub seq: u64,
    pub kind: RecordKind,
}

impl LogRecord {
    pub fn ontology(o: Ontology) -> Self {
        Self { seq: 0, kind: RecordKind::Ontology(o) }
    }
    pub fn concept(c: Concept) -> Self {
        Self { seq: 0, kind: RecordKind::Concept(c) }
    }
    pub fn relation(r: Relation) -> Self {
        Self { seq: 0, kind: RecordKind::Relation(r) }
    }
}
