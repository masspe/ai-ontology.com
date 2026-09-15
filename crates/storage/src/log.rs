// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

use ontology_graph::{
    Action, ActionId, Concept, ConceptId, Ontology, Relation, RelationId, Rule, RuleId,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecordKind {
    Ontology(Ontology),
    Concept(Concept),
    Relation(Relation),
    /// Replaces the relation at `id` with the supplied state. Used to log
    /// `update_relation` (weight / properties only) without going through
    /// `add_relation`, which would otherwise reassign a colliding id.
    UpdateRelation(Relation),
    /// Replaces the concept at `id` with the supplied state, including any
    /// rename (which the live `update_concept` handles via the name-index
    /// cleanup; replay needs the same treatment to avoid stale bindings).
    UpdateConcept(Concept),
    DeleteConcept(ConceptId),
    DeleteRelation(RelationId),
    Rule(Rule),
    Action(Action),
    DeleteRule(RuleId),
    DeleteAction(ActionId),
}

/// What a partitioned store needs to route a record whose payload does not
/// name a type: a deletion carries only an id, but its tombstone must land
/// in the **same domain** as the record it cancels (`STORAGE.md` D3, R13),
/// and the domain is a function of the concept type (H13) or of the
/// relation type's `domain` (H14).
///
/// Never persisted: on disk the stream itself says which domain a record
/// belongs to. Callers that know the entity (the server does, before it
/// deletes) fill it in; stores without domains ignore it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteHint {
    ConceptType(String),
    RelationType(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogRecord {
    /// Logical sequence number assigned by the store; 0 means "not yet assigned".
    #[serde(default)]
    pub seq: u64,
    pub kind: RecordKind,
    /// Routing information for records whose payload does not carry a type
    /// (deletions). See [`RouteHint`].
    #[serde(skip)]
    pub route: Option<RouteHint>,
}

impl LogRecord {
    fn new(kind: RecordKind) -> Self {
        Self {
            seq: 0,
            kind,
            route: None,
        }
    }
    pub fn ontology(o: Ontology) -> Self {
        Self::new(RecordKind::Ontology(o))
    }
    pub fn concept(c: Concept) -> Self {
        Self::new(RecordKind::Concept(c))
    }
    pub fn relation(r: Relation) -> Self {
        Self::new(RecordKind::Relation(r))
    }
    pub fn update_relation(r: Relation) -> Self {
        Self::new(RecordKind::UpdateRelation(r))
    }
    pub fn update_concept(c: Concept) -> Self {
        Self::new(RecordKind::UpdateConcept(c))
    }
    /// Tombstone for a concept. `concept_type` routes it to the concept's
    /// domain; pass the type of the concept being deleted.
    pub fn delete_concept(id: ConceptId, concept_type: impl Into<String>) -> Self {
        Self {
            seq: 0,
            kind: RecordKind::DeleteConcept(id),
            route: Some(RouteHint::ConceptType(concept_type.into())),
        }
    }
    /// Tombstone for a relation. `relation_type` routes it to the domain of
    /// the relation's source type.
    pub fn delete_relation(id: RelationId, relation_type: impl Into<String>) -> Self {
        Self {
            seq: 0,
            kind: RecordKind::DeleteRelation(id),
            route: Some(RouteHint::RelationType(relation_type.into())),
        }
    }
    /// A tombstone without routing information — only for stores without
    /// domains (memory, legacy file) and for tests. A partitioned store
    /// rejects it.
    pub fn delete_concept_unrouted(id: ConceptId) -> Self {
        Self::new(RecordKind::DeleteConcept(id))
    }
    pub fn delete_relation_unrouted(id: RelationId) -> Self {
        Self::new(RecordKind::DeleteRelation(id))
    }
    pub fn rule(r: Rule) -> Self {
        Self::new(RecordKind::Rule(r))
    }
    pub fn action(a: Action) -> Self {
        Self::new(RecordKind::Action(a))
    }
    pub fn delete_rule(id: RuleId) -> Self {
        Self::new(RecordKind::DeleteRule(id))
    }
    pub fn delete_action(id: ActionId) -> Self {
        Self::new(RecordKind::DeleteAction(id))
    }
    /// Attach (or replace) the routing hint.
    pub fn with_route(mut self, route: RouteHint) -> Self {
        self.route = Some(route);
        self
    }
}
