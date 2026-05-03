use ontology_graph::{Concept, Ontology, Relation};
use serde::{Deserialize, Serialize};

/// Tagged input record. Sources emit one of these per item.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Record {
    Ontology(Ontology),
    Concept(Concept),
    Relation(Relation),
    /// Lighter-weight relation expressed by concept name + type.
    NamedRelation {
        relation_type: String,
        source_type: String,
        source_name: String,
        target_type: String,
        target_name: String,
        #[serde(default = "default_weight")]
        weight: f32,
    },
}

fn default_weight() -> f32 {
    1.0
}

/// Convenience alias used by some adapters to avoid quoting `Record`.
pub type RecordPayload = Record;
