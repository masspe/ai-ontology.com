// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

use crate::id::{ConceptId, RelationId};
use thiserror::Error;

pub type GraphResult<T> = Result<T, GraphError>;

#[derive(Debug, Error)]
pub enum GraphError {
    #[error("unknown concept {0}")]
    UnknownConcept(ConceptId),

    #[error("unknown relation {0}")]
    UnknownRelation(RelationId),

    #[error("concept type `{0}` is not defined in the ontology")]
    UnknownConceptType(String),

    #[error("relation type `{0}` is not defined in the ontology")]
    UnknownRelationType(String),

    #[error(
        "relation `{relation}` between `{source_type}` and `{target_type}` violates schema \
         (expected domain `{expected_domain}`, range `{expected_range}`)"
    )]
    SchemaViolation {
        relation: String,
        source_type: String,
        target_type: String,
        expected_domain: String,
        expected_range: String,
    },

    #[error("cardinality violated for relation `{relation}` on concept {concept}")]
    CardinalityViolation {
        relation: String,
        concept: ConceptId,
    },

    #[error("duplicate concept name `{0}` for type `{1}`")]
    DuplicateConcept(String, String),

    #[error("invalid property `{property}` on concept of type `{concept_type}`")]
    InvalidProperty {
        property: String,
        concept_type: String,
    },

    #[error("serialization error: {0}")]
    Serde(String),
}
