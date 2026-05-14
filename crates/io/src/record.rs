// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

use ontology_graph::{
    Action, ActionType, Concept, ConceptType, Ontology, Relation, RelationType, Rule, RuleType,
};
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
    /// Ontology extension produced by the document extractor — register
    /// (or refresh) a single concept type without rewriting the whole
    /// schema. Idempotent.
    ConceptTypeDecl(ConceptType),
    /// Ontology extension — register (or refresh) a single relation type.
    RelationTypeDecl(RelationType),
    /// Ontology extension — register (or refresh) a single rule.
    RuleTypeDecl(RuleType),
    /// Ontology extension — register (or refresh) a single action.
    ActionTypeDecl(ActionType),
    /// Concrete rule instance.
    Rule(Rule),
    /// Concrete action instance.
    Action(Action),
}

fn default_weight() -> f32 {
    1.0
}

/// Convenience alias used by some adapters to avoid quoting `Record`.
pub type RecordPayload = Record;
