// SPDX-License-Identifier: LGPL-3.0-or-later
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// It is licensed under the GNU Lesser General Public License v3.0
// or (at your option) any later version. See LICENSE and LICENSE.GPL.

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
