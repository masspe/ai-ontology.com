// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

use ontology_graph::{Concept, ConceptId, Ontology, Relation, RelationId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecordKind {
    Ontology(Ontology),
    Concept(Concept),
    Relation(Relation),
    /// Replaces the concept at `id` with the supplied state, including any
    /// rename (which the live `update_concept` handles via the name-index
    /// cleanup; replay needs the same treatment to avoid stale bindings).
    UpdateConcept(Concept),
    DeleteConcept(ConceptId),
    DeleteRelation(RelationId),
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
        Self {
            seq: 0,
            kind: RecordKind::Ontology(o),
        }
    }
    pub fn concept(c: Concept) -> Self {
        Self {
            seq: 0,
            kind: RecordKind::Concept(c),
        }
    }
    pub fn relation(r: Relation) -> Self {
        Self {
            seq: 0,
            kind: RecordKind::Relation(r),
        }
    }
    pub fn update_concept(c: Concept) -> Self {
        Self {
            seq: 0,
            kind: RecordKind::UpdateConcept(c),
        }
    }
    pub fn delete_concept(id: ConceptId) -> Self {
        Self {
            seq: 0,
            kind: RecordKind::DeleteConcept(id),
        }
    }
    pub fn delete_relation(id: RelationId) -> Self {
        Self {
            seq: 0,
            kind: RecordKind::DeleteRelation(id),
        }
    }
}
