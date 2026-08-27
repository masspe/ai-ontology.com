// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! User-defined **extraction templates** and their extracted records.
//!
//! A [`ExtractionTemplate`] is a single JSON document that drives two
//! things at once:
//!
//! * the **LLM prompt** — the set of fields to pull out of a document
//!   (e.g. an invoice's number, date, totals and line items); and
//! * the **review UI** — the web client generates a form straight from
//!   `fields`, so customizing the JSON customizes the interface.
//!
//! After the LLM returns an [`ExtractedRecord`], [`record_to_proposal`]
//! maps it onto the generic [`OntologyProposal`] using the template's
//! [`TemplateMapping`], so structured extractions land in the graph
//! through the very same apply pipeline as free-form ingest.

use serde::{Deserialize, Serialize};

use crate::proposal::{
    OntologyProposal, ProposalConcept, ProposalConceptType, ProposalRelation, ProposalRelationType,
};

/// The data type of a template field. Drives both the validation hint sent
/// to the LLM and the input widget rendered by the web client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    /// Free text.
    String,
    /// Plain number (quantity, count).
    Number,
    /// Monetary amount — rendered with a currency-aware widget.
    Currency,
    /// ISO-8601 date (`YYYY-MM-DD`).
    Date,
    /// One of a fixed list of `options`.
    Enum,
    /// `true` / `false`.
    Boolean,
    /// A repeating list of rows, each described by `item_fields`
    /// (e.g. invoice line items).
    Array,
}

impl FieldType {
    /// Short human label used inside the generated LLM prompt.
    pub fn hint(self) -> &'static str {
        match self {
            FieldType::String => "string",
            FieldType::Number => "number",
            FieldType::Currency => "number (monetary amount, no currency symbol)",
            FieldType::Date => "string (ISO-8601 date YYYY-MM-DD)",
            FieldType::Enum => "string (one of the listed options)",
            FieldType::Boolean => "boolean",
            FieldType::Array => "array of objects",
        }
    }
}

/// A single field in an [`ExtractionTemplate`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateField {
    /// Machine key — also the JSON key the LLM must emit and the graph
    /// property name (unless remapped via [`TemplateMapping::field_to_property`]).
    pub key: String,
    /// Human label shown in the form.
    pub label: String,
    #[serde(rename = "type")]
    pub field_type: FieldType,
    /// Whether the reviewer must supply a value before applying.
    #[serde(default)]
    pub required: bool,
    /// Extra guidance handed to the LLM and shown as form helper text.
    #[serde(default)]
    pub hint: String,
    /// Allowed values when `field_type == Enum`.
    #[serde(default)]
    pub options: Vec<String>,
    /// Sub-fields when `field_type == Array`. Empty for scalar fields.
    #[serde(default)]
    pub item_fields: Vec<TemplateField>,
}

impl TemplateField {
    /// `true` for the one array field that maps to line-item concepts.
    pub fn is_array(&self) -> bool {
        matches!(self.field_type, FieldType::Array)
    }
}

/// How an [`ExtractedRecord`] is projected onto graph concepts/relations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateMapping {
    /// Concept type for the record's head entity (e.g. `"Invoice"`).
    pub concept_type: String,
    /// Field key whose value names the head concept (e.g. `"invoice_number"`).
    /// Falls back to the template name + an index when missing/empty.
    #[serde(default)]
    pub key_field: Option<String>,
    /// Concept type minted per array row (e.g. `"LineItem"`). When absent,
    /// array fields are flattened to text properties instead of concepts.
    #[serde(default)]
    pub line_item_concept_type: Option<String>,
    /// Relation type linking the head concept to each line-item concept
    /// (e.g. `"hasLineItem"`).
    #[serde(default)]
    pub line_item_relation: Option<String>,
    /// Field key whose value names each line-item concept (e.g.
    /// `"description"`). Falls back to a positional name.
    #[serde(default)]
    pub line_item_name_field: Option<String>,
    /// Optional rename map: field key → graph property name.
    #[serde(default)]
    pub field_to_property: std::collections::BTreeMap<String, String>,
}

/// Optional presentation overrides for the generated form. Purely advisory
/// — the client falls back to sensible defaults for any missing key.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TemplateUi {
    /// Number of columns to lay scalar fields out in (default 2).
    #[serde(default)]
    pub columns: Option<u8>,
}

/// A reusable, user-editable extraction specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionTemplate {
    /// Stable slug (`"facture-fr"`). Unique per store.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// ISO-639-1 hint forwarded to extraction when the user leaves it blank.
    #[serde(default)]
    pub language_hint: Option<String>,
    /// `true` for templates shipped with the product; the UI marks these
    /// read-only and offers "duplicate" instead of "edit".
    #[serde(default)]
    pub builtin: bool,
    pub fields: Vec<TemplateField>,
    pub mapping: TemplateMapping,
    #[serde(default)]
    pub ui: TemplateUi,
}

// -- extracted record --------------------------------------------------

/// One scalar value the LLM pulled out of a document, with provenance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedField {
    pub key: String,
    /// The raw extracted value (string / number / bool / null). Kept as
    /// `serde_json::Value` so the form can round-trip the LLM's own typing.
    #[serde(default)]
    pub value: serde_json::Value,
    #[serde(default)]
    pub confidence: f32,
    #[serde(default)]
    pub evidence: Option<String>,
}

/// One repeating array field, expanded into rows of cells.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedArray {
    pub key: String,
    /// Each row holds one [`ExtractedField`] per `item_fields` entry.
    pub rows: Vec<Vec<ExtractedField>>,
}

/// The result of running an [`ExtractionTemplate`] against a document.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtractedRecord {
    pub template_id: String,
    /// Scalar fields, in template order.
    #[serde(default)]
    pub fields: Vec<ExtractedField>,
    /// Array fields, in template order.
    #[serde(default)]
    pub arrays: Vec<ExtractedArray>,
}

impl ExtractedRecord {
    /// Look up a scalar field's value as a display string (`""` if absent).
    pub fn field_str(&self, key: &str) -> String {
        self.fields
            .iter()
            .find(|f| f.key == key)
            .map(|f| value_to_string(&f.value))
            .unwrap_or_default()
    }
}

/// Render a JSON scalar as a plain string for graph properties / names.
/// Objects and arrays are JSON-encoded as a last resort.
pub fn value_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

// -- record -> proposal mapping ----------------------------------------

/// Average a slice of confidences, defaulting to `0.0` when empty.
fn mean_conf(vals: &[f32]) -> f32 {
    if vals.is_empty() {
        0.0
    } else {
        vals.iter().sum::<f32>() / vals.len() as f32
    }
}

/// Project an [`ExtractedRecord`] onto an [`OntologyProposal`] using the
/// template's [`TemplateMapping`].
///
/// The proposal includes:
/// * the head concept type + line-item concept type + link relation type
///   (so a fresh graph gains them; conflict-tagging dedupes later), and
/// * one head concept whose properties are the scalar fields, plus one
///   concept + link relation per array row.
///
/// The result is a normal proposal — the existing `/ingest/apply` pipeline
/// validates and writes it, and `attach_conflicts` still runs on top.
pub fn record_to_proposal(
    template: &ExtractionTemplate,
    record: &ExtractedRecord,
) -> OntologyProposal {
    let mapping = &template.mapping;
    let mut proposal = OntologyProposal::default();

    // Property name for a field key, honouring the rename map.
    let prop_name = |key: &str| -> String {
        mapping
            .field_to_property
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.to_string())
    };

    // --- head concept type ---
    let head_type = mapping.concept_type.clone();
    let scalar_props: Vec<String> = record.fields.iter().map(|f| prop_name(&f.key)).collect();
    proposal.concept_types.push(ProposalConceptType {
        client_ref: "ct-head".into(),
        name: head_type.clone(),
        description: template.description.clone(),
        properties: scalar_props,
        parent: None,
        confidence: 1.0,
        conflict: None,
    });

    // --- head concept ---
    let head_name = mapping
        .key_field
        .as_deref()
        .map(|k| record.field_str(k))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| template.name.clone());

    let head_props: Vec<(String, String)> = record
        .fields
        .iter()
        .map(|f| (prop_name(&f.key), value_to_string(&f.value)))
        .collect();
    let head_conf = mean_conf(&record.fields.iter().map(|f| f.confidence).collect::<Vec<_>>());
    let head_evidence = record.fields.iter().find_map(|f| f.evidence.clone());

    proposal.concepts.push(ProposalConcept {
        client_ref: "head".into(),
        concept_type: head_type.clone(),
        name: head_name,
        description: String::new(),
        properties: head_props,
        evidence: head_evidence,
        confidence: if head_conf > 0.0 { head_conf } else { 1.0 },
        conflict: None,
    });

    // --- line items ---
    if let (Some(li_type), Some(li_rel)) = (
        mapping.line_item_concept_type.clone(),
        mapping.line_item_relation.clone(),
    ) {
        // Line-item concept type: union of all item field property names.
        let li_props: Vec<String> = template
            .fields
            .iter()
            .find(|f| f.is_array())
            .map(|f| f.item_fields.iter().map(|c| prop_name(&c.key)).collect())
            .unwrap_or_default();
        proposal.concept_types.push(ProposalConceptType {
            client_ref: "ct-li".into(),
            name: li_type.clone(),
            description: format!("Line item of {}", head_type),
            properties: li_props,
            parent: None,
            confidence: 1.0,
            conflict: None,
        });
        proposal.relation_types.push(ProposalRelationType {
            client_ref: "rt-li".into(),
            name: li_rel.clone(),
            domain: head_type.clone(),
            range: li_type.clone(),
            symmetric: false,
            description: format!("{} has line item", head_type),
            confidence: 1.0,
            conflict: None,
        });

        for arr in &record.arrays {
            for (ri, row) in arr.rows.iter().enumerate() {
                let li_ref = format!("li-{ri}");
                let name = mapping
                    .line_item_name_field
                    .as_deref()
                    .and_then(|k| row.iter().find(|c| c.key == k))
                    .map(|c| value_to_string(&c.value))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| format!("Line {}", ri + 1));
                let props: Vec<(String, String)> = row
                    .iter()
                    .map(|c| (prop_name(&c.key), value_to_string(&c.value)))
                    .collect();
                let conf = mean_conf(&row.iter().map(|c| c.confidence).collect::<Vec<_>>());
                proposal.concepts.push(ProposalConcept {
                    client_ref: li_ref.clone(),
                    concept_type: li_type.clone(),
                    name,
                    description: String::new(),
                    properties: props,
                    evidence: None,
                    confidence: if conf > 0.0 { conf } else { 1.0 },
                    conflict: None,
                });
                proposal.relations.push(ProposalRelation {
                    client_ref: format!("rel-{ri}"),
                    relation_type: li_rel.clone(),
                    source_ref: "head".into(),
                    target_ref: li_ref,
                    weight: None,
                    evidence: None,
                    confidence: 1.0,
                    conflict: None,
                });
            }
        }
    }

    proposal
}

/// The built-in French invoice template. Shipped so a fresh install has a
/// working structured-extraction example out of the box.
pub fn builtin_invoice_template() -> ExtractionTemplate {
    let f = |key: &str, label: &str, ty: FieldType, required: bool| TemplateField {
        key: key.into(),
        label: label.into(),
        field_type: ty,
        required,
        hint: String::new(),
        options: Vec::new(),
        item_fields: Vec::new(),
    };
    ExtractionTemplate {
        id: "facture-fr".into(),
        name: "Facture".into(),
        description: "Extraction de factures fournisseurs".into(),
        language_hint: Some("fr".into()),
        builtin: true,
        fields: vec![
            f("invoice_number", "N° de facture", FieldType::String, true),
            f("issue_date", "Date d'émission", FieldType::Date, true),
            f("due_date", "Échéance", FieldType::Date, false),
            f("vendor_name", "Fournisseur", FieldType::String, true),
            f("customer_name", "Client", FieldType::String, false),
            TemplateField {
                key: "currency".into(),
                label: "Devise".into(),
                field_type: FieldType::Enum,
                required: false,
                hint: String::new(),
                options: vec!["EUR".into(), "CHF".into(), "USD".into()],
                item_fields: Vec::new(),
            },
            f("total_ht", "Total HT", FieldType::Currency, false),
            f("vat_amount", "TVA", FieldType::Currency, false),
            f("total_ttc", "Total TTC", FieldType::Currency, true),
            TemplateField {
                key: "line_items".into(),
                label: "Lignes".into(),
                field_type: FieldType::Array,
                required: false,
                hint: "Une ligne par produit ou service facturé".into(),
                options: Vec::new(),
                item_fields: vec![
                    f("description", "Désignation", FieldType::String, false),
                    f("qty", "Qté", FieldType::Number, false),
                    f("unit_price", "PU HT", FieldType::Currency, false),
                    f("amount", "Montant", FieldType::Currency, false),
                ],
            },
        ],
        mapping: TemplateMapping {
            concept_type: "Invoice".into(),
            key_field: Some("invoice_number".into()),
            line_item_concept_type: Some("LineItem".into()),
            line_item_relation: Some("hasLineItem".into()),
            line_item_name_field: Some("description".into()),
            field_to_property: std::collections::BTreeMap::new(),
        },
        ui: TemplateUi { columns: Some(2) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_template_round_trips() {
        let t = builtin_invoice_template();
        let json = serde_json::to_string(&t).unwrap();
        let back: ExtractionTemplate = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "facture-fr");
        assert!(back.fields.iter().any(|f| f.is_array()));
    }

    #[test]
    fn record_maps_head_and_line_items() {
        let t = builtin_invoice_template();
        let record = ExtractedRecord {
            template_id: t.id.clone(),
            fields: vec![
                ExtractedField {
                    key: "invoice_number".into(),
                    value: serde_json::json!("F-2024-001"),
                    confidence: 0.95,
                    evidence: Some("Facture N° F-2024-001".into()),
                },
                ExtractedField {
                    key: "total_ttc".into(),
                    value: serde_json::json!(120.0),
                    confidence: 0.9,
                    evidence: None,
                },
            ],
            arrays: vec![ExtractedArray {
                key: "line_items".into(),
                rows: vec![vec![
                    ExtractedField {
                        key: "description".into(),
                        value: serde_json::json!("Widget"),
                        confidence: 0.9,
                        evidence: None,
                    },
                    ExtractedField {
                        key: "amount".into(),
                        value: serde_json::json!(120.0),
                        confidence: 0.9,
                        evidence: None,
                    },
                ]],
            }],
        };

        let p = record_to_proposal(&t, &record);
        // Head concept named from invoice_number.
        let head = p.concepts.iter().find(|c| c.client_ref == "head").unwrap();
        assert_eq!(head.concept_type, "Invoice");
        assert_eq!(head.name, "F-2024-001");
        // One line item + one link relation.
        assert!(p.concepts.iter().any(|c| c.concept_type == "LineItem"));
        assert_eq!(p.relations.len(), 1);
        assert_eq!(p.relations[0].relation_type, "hasLineItem");
        // Concept/relation types declared for a fresh graph.
        assert!(p.concept_types.iter().any(|c| c.name == "Invoice"));
        assert!(p.relation_types.iter().any(|r| r.name == "hasLineItem"));
    }
}
