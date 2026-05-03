// SPDX-License-Identifier: LGPL-3.0-or-later
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// It is licensed under the GNU Lesser General Public License v3.0
// or (at your option) any later version. See LICENSE and LICENSE.GPL.

//! Ontology-structured graph database.
//!
//! Stores `Concept` nodes and typed `Relation` edges that conform to an
//! `Ontology` schema. The schema declares which concept types exist, which
//! relation types are permitted, and the (domain, range) constraints between
//! them. All mutating operations validate against the schema so the graph
//! cannot drift away from its ontology.

pub mod error;
pub mod graph;
pub mod id;
pub mod model;
pub mod schema;
pub mod traversal;

pub use error::{GraphError, GraphResult};
pub use graph::OntologyGraph;
pub use id::{ConceptId, RelationId};
pub use model::{Concept, ConceptPatch, Property, PropertyValue, Relation};
pub use schema::{Cardinality, ConceptType, Ontology, RelationType};
pub use traversal::{Direction, Path, PathStep, Subgraph, TraversalSpec};
