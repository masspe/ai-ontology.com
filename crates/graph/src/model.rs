use ahash::AHashMap;
use serde::{Deserialize, Serialize};

use crate::id::{ConceptId, RelationId};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PropertyValue {
    Text(String),
    Number(f64),
    Bool(bool),
    List(Vec<PropertyValue>),
}

impl PropertyValue {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            PropertyValue::Text(t) => Some(t),
            _ => None,
        }
    }
}

pub type Property = (String, PropertyValue);

/// A node in the graph. `name` is a human-readable label, unique per
/// concept type, used as the natural-language anchor in RAG retrieval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Concept {
    pub id: ConceptId,
    pub concept_type: String,
    pub name: String,
    /// Optional free-text description; embedded by the index for vector search.
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub properties: AHashMap<String, PropertyValue>,
}

impl Concept {
    pub fn new(id: ConceptId, concept_type: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id,
            concept_type: concept_type.into(),
            name: name.into(),
            description: String::new(),
            properties: AHashMap::new(),
        }
    }
    pub fn with_description(mut self, d: impl Into<String>) -> Self {
        self.description = d.into();
        self
    }
    pub fn with_property(mut self, k: impl Into<String>, v: PropertyValue) -> Self {
        self.properties.insert(k.into(), v);
        self
    }

    /// Concatenated text used for indexing (lexical + vector).
    pub fn indexable_text(&self) -> String {
        let mut s = String::with_capacity(self.name.len() + self.description.len() + 32);
        s.push_str(&self.name);
        if !self.description.is_empty() {
            s.push_str(". ");
            s.push_str(&self.description);
        }
        for (k, v) in &self.properties {
            if let PropertyValue::Text(t) = v {
                s.push_str(". ");
                s.push_str(k);
                s.push_str(": ");
                s.push_str(t);
            }
        }
        s
    }
}

/// Partial update to an existing concept. Each `Some` field replaces the
/// corresponding field; `None` leaves it untouched. `concept_type` is
/// intentionally absent — changing it would invalidate incident edges.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConceptPatch {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// When `Some`, replaces the property map wholesale. To merge instead,
    /// fetch the concept, modify the map, and pass the merged result.
    #[serde(default)]
    pub properties: Option<AHashMap<String, PropertyValue>>,
}

/// A directed, typed edge between two concepts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relation {
    pub id: RelationId,
    pub relation_type: String,
    pub source: ConceptId,
    pub target: ConceptId,
    /// Provenance / weight attached to the edge.
    #[serde(default)]
    pub weight: f32,
    #[serde(default)]
    pub properties: AHashMap<String, PropertyValue>,
}

impl Relation {
    pub fn new(
        id: RelationId,
        relation_type: impl Into<String>,
        source: ConceptId,
        target: ConceptId,
    ) -> Self {
        Self {
            id,
            relation_type: relation_type.into(),
            source,
            target,
            weight: 1.0,
            properties: AHashMap::new(),
        }
    }
}
