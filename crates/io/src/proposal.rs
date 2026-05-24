// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Shared types for the LLM-assisted ingest review workflow.
//!
//! The flow is stateless on the server side:
//! 1. Client uploads a document → server calls the LLM → returns
//!    [`OntologyProposal`] with conflict information attached.
//! 2. Client walks the proposal in a wizard, lets the user edit / accept
//!    / merge / skip each item, then posts a list of [`ApplyDecision`]s
//!    alongside the (possibly edited) proposal.
//! 3. Server validates the decisions against the live graph and writes
//!    the accepted records.
//!
//! Each proposed item carries a `client_ref`: a stable string id the
//! client uses to cross-reference its decisions and the response report.
//! Server-side ids are allocated only on apply.

use serde::{Deserialize, Serialize};

use crate::lang::LangTag;

/// Provenance for an [`OntologyProposal`] — useful when the same proposal
/// payload might be produced by different upstreams.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposalSource {
    /// Original filename or upstream identifier.
    pub name: String,
    /// MIME type if known, else best-effort label (e.g. `"text/plain"`).
    #[serde(default)]
    pub kind: String,
    /// Encoding the document was decoded from (`"UTF-8"`, `"windows-1252"`, …).
    #[serde(default)]
    pub encoding: String,
    /// `true` when the original carried a BOM.
    #[serde(default)]
    pub had_bom: bool,
    /// LLM provider that produced the proposal (`"openai"`, `"anthropic"`, …).
    #[serde(default)]
    pub provider: String,
    /// Model name as reported by the provider.
    #[serde(default)]
    pub model: String,
}

/// LLM-generated, human-reviewable extraction from a single document.
///
/// All collections use `client_ref` strings rather than allocated graph
/// ids — the apply step resolves refs to ids in topological order.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OntologyProposal {
    pub source: Option<ProposalSource>,
    /// Dominant document language detected by `whatlang`.
    #[serde(default)]
    pub language: Option<LangTag>,
    #[serde(default)]
    pub concept_types: Vec<ProposalConceptType>,
    #[serde(default)]
    pub relation_types: Vec<ProposalRelationType>,
    #[serde(default)]
    pub concepts: Vec<ProposalConcept>,
    #[serde(default)]
    pub relations: Vec<ProposalRelation>,
    #[serde(default)]
    pub rules: Vec<ProposalRule>,
    #[serde(default)]
    pub actions: Vec<ProposalAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposalConceptType {
    pub client_ref: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub properties: Vec<String>,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub confidence: f32,
    #[serde(default)]
    pub conflict: Option<ConflictInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposalRelationType {
    pub client_ref: String,
    pub name: String,
    pub domain: String,
    pub range: String,
    #[serde(default)]
    pub symmetric: bool,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub confidence: f32,
    #[serde(default)]
    pub conflict: Option<ConflictInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposalConcept {
    pub client_ref: String,
    pub concept_type: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Free-form property bag — values rendered as text in the UI.
    #[serde(default)]
    pub properties: Vec<(String, String)>,
    /// Verbatim snippet from the source document supporting this concept.
    #[serde(default)]
    pub evidence: Option<String>,
    #[serde(default)]
    pub confidence: f32,
    #[serde(default)]
    pub conflict: Option<ConflictInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposalRelation {
    pub client_ref: String,
    pub relation_type: String,
    /// Either a `client_ref` of another `ProposalConcept` or the string
    /// `"<type>:<name>"` of an existing graph concept.
    pub source_ref: String,
    pub target_ref: String,
    #[serde(default)]
    pub weight: Option<f32>,
    #[serde(default)]
    pub evidence: Option<String>,
    #[serde(default)]
    pub confidence: f32,
    #[serde(default)]
    pub conflict: Option<ConflictInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposalRule {
    pub client_ref: String,
    pub rule_type: String,
    pub name: String,
    #[serde(default)]
    pub when: String,
    #[serde(default)]
    pub then: String,
    #[serde(default)]
    pub applies_to: Vec<String>,
    #[serde(default)]
    pub strict: bool,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub evidence: Option<String>,
    #[serde(default)]
    pub confidence: f32,
    #[serde(default)]
    pub conflict: Option<ConflictInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposalAction {
    pub client_ref: String,
    pub action_type: String,
    pub name: String,
    pub subject_ref: String,
    #[serde(default)]
    pub object_ref: Option<String>,
    #[serde(default)]
    pub parameters: Vec<(String, String)>,
    #[serde(default)]
    pub effect: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub evidence: Option<String>,
    #[serde(default)]
    pub confidence: f32,
    #[serde(default)]
    pub conflict: Option<ConflictInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ConflictKind {
    /// A live graph item with the same identity already exists.
    Exists {
        existing_id: String,
        existing_display: String,
    },
    /// A live item with the same name exists but at a different type.
    TypeMismatch {
        existing_type: String,
        existing_id: String,
    },
    /// A `source_ref` / `target_ref` does not resolve to any concept in
    /// the proposal or the live graph.
    DanglingRef { missing_ref: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictInfo {
    pub kind: ConflictKind,
    /// Short human summary suitable for a UI badge / tooltip.
    pub summary: String,
}

/// Wizard verdict for a single proposal item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionAction {
    /// Patch the existing graph item with the proposed fields.
    Merge,
    /// Create a fresh graph item even if a same-name match exists.
    CreateNew,
    /// Drop the proposed item.
    Skip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyDecision {
    pub client_ref: String,
    pub action: DecisionAction,
}

/// Per-item outcome of an apply request, keyed by `client_ref`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum ApplyOutcome {
    Created { id: String },
    Merged { id: String },
    Skipped,
    Failed { error: String },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApplyReport {
    pub concept_types: Vec<(String, ApplyOutcome)>,
    pub relation_types: Vec<(String, ApplyOutcome)>,
    pub concepts: Vec<(String, ApplyOutcome)>,
    pub relations: Vec<(String, ApplyOutcome)>,
    pub rules: Vec<(String, ApplyOutcome)>,
    pub actions: Vec<(String, ApplyOutcome)>,
    /// Aggregated counts for UI summary.
    pub created: u32,
    pub merged: u32,
    pub skipped: u32,
    pub failed: u32,
}

impl OntologyProposal {
    /// Returns an iterator over every client_ref present in the proposal,
    /// preserving insertion order. Used by the apply step to detect
    /// duplicate refs in the incoming payload.
    pub fn iter_refs(&self) -> impl Iterator<Item = &str> {
        let ct = self.concept_types.iter().map(|x| x.client_ref.as_str());
        let rt = self.relation_types.iter().map(|x| x.client_ref.as_str());
        let c = self.concepts.iter().map(|x| x.client_ref.as_str());
        let r = self.relations.iter().map(|x| x.client_ref.as_str());
        let ru = self.rules.iter().map(|x| x.client_ref.as_str());
        let a = self.actions.iter().map(|x| x.client_ref.as_str());
        ct.chain(rt).chain(c).chain(r).chain(ru).chain(a)
    }
}
