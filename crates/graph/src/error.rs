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

    /// P1: the concept's payload lives on disk and could not be read back
    /// (no payload source attached, or the source failed).
    #[error("payload of concept {0} unavailable: {1}")]
    PayloadUnavailable(ConceptId, String),

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

    #[error("missing required property `{property}` on concept of type `{concept_type}`")]
    MissingRequiredProperty {
        property: String,
        concept_type: String,
    },

    #[error("disjoint type violation: `{type_a}` is disjoint with `{type_b}`")]
    DisjointTypeViolation { type_a: String, type_b: String },

    #[error("serialization error: {0}")]
    Serde(String),

    /// An `apply_prepared_*` method received an entity whose id was never
    /// allocated by the matching `prepare_*` call (id == 0). Callers must
    /// prepare, persist, then apply — see `STORAGE.md` R8.
    #[error("{0} was not prepared: id is unset")]
    NotPrepared(&'static str),

    /// A concept update tried to change `concept_type` (immutable, H5).
    #[error("concept type is immutable (was `{0}`, got `{1}`)")]
    ImmutableConceptType(String, String),

    /// The storage format packs relation endpoints as `(source << 32) |
    /// target`; a concept id above 2³² cannot be persisted (`STORAGE.md`
    /// D6). Rejected where the id is born, never assumed downstream.
    #[error("concept id {0} exceeds the storage format limit of 2^32 - 1")]
    ConceptIdOutOfRange(ConceptId),

    /// A concept type declares a domain that is not `[a-z0-9_-]{1,32}`.
    #[error("concept type `{concept_type}` declares invalid domain `{ns}` (expected [a-z0-9_-]{{1,32}})")]
    InvalidNamespace { concept_type: String, ns: String },

    /// A child type declares a domain different from its parent's.
    #[error(
        "concept type `{concept_type}` declares domain `{ns}` but its parent `{parent}` is in `{parent_ns}`"
    )]
    NamespaceMismatch {
        concept_type: String,
        ns: String,
        parent: String,
        parent_ns: String,
    },

    /// Moving a type to another domain while instances exist would need
    /// tombstones on the old side (`STORAGE.md` R13); refused for now.
    #[error(
        "concept type `{concept_type}` cannot move from domain `{from}` to `{to}`: {instances} instance(s) exist"
    )]
    NamespaceChangeWithInstances {
        concept_type: String,
        from: String,
        to: String,
        instances: usize,
    },

    /// A type that still has instances was removed from the ontology.
    #[error("{kind} type `{name}` still has {instances} instance(s); remove them before dropping the type")]
    TypeInUse {
        kind: &'static str,
        name: String,
        instances: usize,
    },

    /// A relation type's `domain` / `range` changed while relations of that
    /// type exist: their validity and their storage domain would change
    /// under them.
    #[error(
        "relation type `{relation_type}` cannot change domain/range: {instances} relation(s) exist"
    )]
    RelationTypeChangeWithInstances {
        relation_type: String,
        instances: usize,
    },

    /// `insert_relation_exact` was given an id that is already present.
    #[error("relation {0} already exists")]
    RelationExists(RelationId),

    /// A concept type is its own ancestor through the `parent` chain. Such
    /// a schema could never be replayed (`is_subtype`, `descendants` and
    /// `ns_of_type` would have no fixed point), so it is refused before it
    /// reaches the journal (`STORAGE.md` §10.10).
    #[error("concept type `{concept_type}` is its own ancestor: the parent chain is a cycle")]
    ParentCycle { concept_type: String },
}
