use ahash::AHashMap;
use serde::{Deserialize, Serialize};

use crate::error::{GraphError, GraphResult};

/// Cardinality constraint for a relation type.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cardinality {
    OneToOne,
    OneToMany,
    ManyToOne,
    #[default]
    ManyToMany,
}

/// A node type in the ontology, e.g. `Person`, `Paper`, `Drug`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConceptType {
    pub name: String,
    /// Names of properties that may appear on instances of this type.
    /// `None` means open-world (any property allowed).
    #[serde(default)]
    pub properties: Option<Vec<String>>,
    /// Optional parent concept type — instances of `name` are also instances
    /// of any ancestor type. Enables simple subtyping for relation domains.
    #[serde(default)]
    pub parent: Option<String>,
    /// Human-readable description used by the RAG prompt builder.
    #[serde(default)]
    pub description: String,
}

/// An edge type in the ontology, e.g. `authored`, `treats`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationType {
    pub name: String,
    pub domain: String,
    pub range: String,
    #[serde(default)]
    pub cardinality: Cardinality,
    #[serde(default)]
    pub symmetric: bool,
    #[serde(default)]
    pub description: String,
}

/// The full ontology: concept types + relation types.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Ontology {
    pub concept_types: AHashMap<String, ConceptType>,
    pub relation_types: AHashMap<String, RelationType>,
}

impl Ontology {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_concept_type(&mut self, ct: ConceptType) {
        self.concept_types.insert(ct.name.clone(), ct);
    }

    pub fn add_relation_type(&mut self, rt: RelationType) -> GraphResult<()> {
        if !self.concept_types.contains_key(&rt.domain) {
            return Err(GraphError::UnknownConceptType(rt.domain));
        }
        if !self.concept_types.contains_key(&rt.range) {
            return Err(GraphError::UnknownConceptType(rt.range));
        }
        self.relation_types.insert(rt.name.clone(), rt);
        Ok(())
    }

    pub fn concept_type(&self, name: &str) -> GraphResult<&ConceptType> {
        self.concept_types
            .get(name)
            .ok_or_else(|| GraphError::UnknownConceptType(name.to_string()))
    }

    pub fn relation_type(&self, name: &str) -> GraphResult<&RelationType> {
        self.relation_types
            .get(name)
            .ok_or_else(|| GraphError::UnknownRelationType(name.to_string()))
    }

    /// Returns true if `child` is `ancestor`, or transitively inherits from it.
    pub fn is_subtype(&self, child: &str, ancestor: &str) -> bool {
        if child == ancestor {
            return true;
        }
        let mut cursor = self.concept_types.get(child);
        while let Some(ct) = cursor {
            match &ct.parent {
                Some(p) if p == ancestor => return true,
                Some(p) => cursor = self.concept_types.get(p),
                None => return false,
            }
        }
        false
    }

    /// Validate that a relation linking the two concept types is permitted.
    pub fn validate_edge(
        &self,
        relation: &str,
        source_type: &str,
        target_type: &str,
    ) -> GraphResult<()> {
        let rt = self.relation_type(relation)?;
        if !self.is_subtype(source_type, &rt.domain) || !self.is_subtype(target_type, &rt.range) {
            return Err(GraphError::SchemaViolation {
                relation: relation.to_string(),
                source_type: source_type.to_string(),
                target_type: target_type.to_string(),
                expected_domain: rt.domain.clone(),
                expected_range: rt.range.clone(),
            });
        }
        Ok(())
    }
}
