//! Ontology-structured graph database.
//!
//! Stores `Concept` nodes and typed `Relation` edges that conform to an
//! `Ontology` schema. The schema declares which concept types exist, which
//! relation types are permitted, and the (domain, range) constraints between
//! them. All mutating operations validate against the schema so the graph
//! cannot drift away from its ontology.

pub mod id;
pub mod schema;
pub mod model;
pub mod graph;
pub mod traversal;
pub mod error;

pub use error::{GraphError, GraphResult};
pub use id::{ConceptId, RelationId};
pub use schema::{Ontology, ConceptType, RelationType, Cardinality};
pub use model::{Concept, Relation, Property, PropertyValue};
pub use graph::OntologyGraph;
pub use traversal::{Subgraph, TraversalSpec, Direction};
