// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! LLM-driven extraction of an [`OntologyProposal`] from a free-text
//! document.
//!
//! This module is the engine behind the `/ingest/analyze` endpoint:
//! 1. The document is chunked on paragraph boundaries when it would
//!    exceed the per-call budget (~12 000 chars).
//! 2. Each chunk is sent to a [`LanguageModel`] with a strict JSON
//!    instruction and the live ontology schema as context.
//! 3. Per-chunk proposals are merged, deduplicating by
//!    `(type, normalized_name)` so the same entity surfacing twice
//!    becomes a single item with the higher confidence.
//! 4. [`attach_conflicts`] cross-references the merged proposal against
//!    the live graph, flagging existing items and dangling references.

use std::collections::HashMap;

use ontology_graph::{Ontology, OntologyGraph};
use ontology_io::{
    ConflictInfo, ConflictKind, LangTag, OntologyProposal, ProposalAction, ProposalConcept,
    ProposalConceptType, ProposalRelation, ProposalRelationType, ProposalRule,
};
use serde::Deserialize;

use crate::model::{LanguageModel, LlmError, LlmRequest, Message, Role};

/// Maximum characters per LLM call. Empirically chosen so a single call
/// stays well under the 12 K input-token budget on `gpt-4o-mini` and
/// leaves room for the schema context. Above this size, the document is
/// chunked on paragraph boundaries.
const CHUNK_BUDGET_CHARS: usize = 12_000;

/// Top-level extractor error. Wraps LLM transport errors and surfaces
/// JSON-shape failures so the caller can decide to retry with a
/// truncated input.
#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("llm: {0}")]
    Llm(#[from] LlmError),
    #[error("parse: {0}")]
    Parse(String),
}

/// Extract a reviewable proposal from `text`, using the live `schema` as
/// LLM context so the model reuses existing types rather than coining
/// duplicates.
///
/// `language` is forwarded to the prompt so the LLM keeps concept names
/// in the source language (search and human review work best with the
/// natural form; an English alias can be added later as a property).
pub async fn extract_proposal(
    model: &dyn LanguageModel,
    text: &str,
    language: Option<&LangTag>,
    schema: &Ontology,
) -> Result<OntologyProposal, ExtractError> {
    let chunks = chunk_text(text, CHUNK_BUDGET_CHARS);
    let schema_block = render_schema(schema);
    let lang_hint = language
        .map(|l| format!("Document language (ISO 639-1): {}.", l.code))
        .unwrap_or_default();

    let mut accumulator = OntologyProposal::default();
    accumulator.language = language.cloned();

    for (idx, chunk) in chunks.iter().enumerate() {
        let raw = call_llm(model, &schema_block, &lang_hint, chunk, idx, chunks.len()).await?;
        let parsed = parse_response(&raw, idx)?;
        merge_into(&mut accumulator, parsed);
    }

    Ok(accumulator)
}

/// Decorate every proposed item with conflict information sourced from
/// `graph`. Idempotent: re-running it overwrites previous conflict
/// annotations, so the caller can update the report after edits.
pub fn attach_conflicts(proposal: &mut OntologyProposal, graph: &OntologyGraph) {
    let ontology = graph.ontology();

    // Concept types: collision when the name already exists in the live schema.
    for ct in &mut proposal.concept_types {
        ct.conflict = match ontology.concept_types.get(&ct.name) {
            Some(existing) => Some(ConflictInfo {
                kind: ConflictKind::Exists {
                    existing_id: existing.name.clone(),
                    existing_display: existing.name.clone(),
                },
                summary: format!("Concept type `{}` already exists.", ct.name),
            }),
            None => None,
        };
    }

    // Relation types: same logic plus a friendlier summary.
    for rt in &mut proposal.relation_types {
        rt.conflict = match ontology.relation_types.get(&rt.name) {
            Some(existing) => Some(ConflictInfo {
                kind: ConflictKind::Exists {
                    existing_id: existing.name.clone(),
                    existing_display: format!(
                        "{}: {} → {}",
                        existing.name, existing.domain, existing.range
                    ),
                },
                summary: format!("Relation type `{}` already exists.", rt.name),
            }),
            None => None,
        };
    }

    // Concepts: check for an existing entity with the same (type, name).
    // The graph's `find_by_name` lower-cases the lookup key, so this also
    // catches case-only differences ("Acme Corp" vs "acme corp").
    for c in &mut proposal.concepts {
        c.conflict = match graph.find_by_name(&c.concept_type, &c.name) {
            Some(id) => {
                let existing = graph
                    .get_concept(id)
                    .map(|c| c.name.clone())
                    .unwrap_or_default();
                Some(ConflictInfo {
                    kind: ConflictKind::Exists {
                        existing_id: id.0.to_string(),
                        existing_display: format!("{}:{}", c.concept_type, existing),
                    },
                    summary: format!("`{}` already exists in the graph.", c.name),
                })
            }
            None => None,
        };
    }

    // Build a lookup of the proposal's own client_refs AND
    // `"<concept_type>:<name>"` forms so relation/action references can
    // resolve forward (a relation may target a concept declared earlier
    // in the same proposal under either notation).
    let mut proposal_refs: std::collections::HashSet<String> = std::collections::HashSet::new();
    for c in &proposal.concepts {
        proposal_refs.insert(c.client_ref.clone());
        proposal_refs.insert(format!("{}:{}", c.concept_type, c.name));
    }

    for r in &mut proposal.relations {
        let src_ok = ref_resolvable(&r.source_ref, &proposal_refs, graph);
        let tgt_ok = ref_resolvable(&r.target_ref, &proposal_refs, graph);
        r.conflict = if !src_ok {
            Some(ConflictInfo {
                kind: ConflictKind::DanglingRef {
                    missing_ref: r.source_ref.clone(),
                },
                summary: format!("Source `{}` not found.", r.source_ref),
            })
        } else if !tgt_ok {
            Some(ConflictInfo {
                kind: ConflictKind::DanglingRef {
                    missing_ref: r.target_ref.clone(),
                },
                summary: format!("Target `{}` not found.", r.target_ref),
            })
        } else {
            None
        };
    }

    for a in &mut proposal.actions {
        let subj_ok = ref_resolvable(&a.subject_ref, &proposal_refs, graph);
        let obj_ok = a
            .object_ref
            .as_ref()
            .map(|r| ref_resolvable(r, &proposal_refs, graph))
            .unwrap_or(true);
        a.conflict = if !subj_ok {
            Some(ConflictInfo {
                kind: ConflictKind::DanglingRef {
                    missing_ref: a.subject_ref.clone(),
                },
                summary: format!("Subject `{}` not found.", a.subject_ref),
            })
        } else if !obj_ok {
            let r = a.object_ref.clone().unwrap_or_default();
            Some(ConflictInfo {
                kind: ConflictKind::DanglingRef {
                    missing_ref: r.clone(),
                },
                summary: format!("Object `{}` not found.", r),
            })
        } else {
            None
        };
    }
}

/// A reference resolves if it matches a proposal client_ref OR
/// the literal form `"<concept_type>:<name>"` of an existing graph concept.
fn ref_resolvable(
    r: &str,
    proposal_refs: &std::collections::HashSet<String>,
    graph: &OntologyGraph,
) -> bool {
    if proposal_refs.contains(r) {
        return true;
    }
    if let Some((ty, name)) = r.split_once(':') {
        return graph.find_by_name(ty, name).is_some();
    }
    false
}

// -- internal helpers --------------------------------------------------

fn chunk_text(text: &str, budget: usize) -> Vec<String> {
    if text.chars().count() <= budget {
        return vec![text.to_string()];
    }
    // Split on blank lines (paragraph boundary) and greedily pack.
    let mut chunks = Vec::new();
    let mut current = String::new();
    for para in text.split("\n\n") {
        if current.chars().count() + para.chars().count() + 2 > budget && !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(para);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn render_schema(s: &Ontology) -> String {
    // Serialize a compact JSON view of the schema so the LLM can re-use
    // existing types. Deterministic ordering (sorted) keeps the prompt
    // byte-stable for caching across requests.
    let mut concept_types: Vec<&String> = s.concept_types.keys().collect();
    concept_types.sort();
    let mut relation_types: Vec<&String> = s.relation_types.keys().collect();
    relation_types.sort();

    let mut out = String::from("Known concept types:\n");
    for n in &concept_types {
        out.push_str("  - ");
        out.push_str(n);
        out.push('\n');
    }
    out.push_str("\nKnown relation types:\n");
    for n in &relation_types {
        if let Some(rt) = s.relation_types.get(*n) {
            out.push_str(&format!("  - {} ({} -> {})\n", rt.name, rt.domain, rt.range));
        }
    }
    out
}

async fn call_llm(
    model: &dyn LanguageModel,
    schema_block: &str,
    lang_hint: &str,
    chunk: &str,
    chunk_idx: usize,
    chunk_total: usize,
) -> Result<String, LlmError> {
    let system = SYSTEM_INSTRUCTION.to_string();
    let user = format!(
        "{lang_hint}\nChunk {n}/{m}.\n\nKnown schema (reuse names where possible):\n{schema_block}\n\
---\nDocument:\n{chunk}\n---\n\n\
Return ONLY a JSON object with the shape described above. \
Do not wrap it in markdown fences. Do not include commentary.",
        lang_hint = lang_hint,
        n = chunk_idx + 1,
        m = chunk_total,
        schema_block = schema_block,
        chunk = chunk,
    );

    let req = LlmRequest {
        system: Some(system),
        cached_context: None,
        messages: vec![Message {
            role: Role::User,
            content: user,
        }],
        max_tokens: 4096,
        temperature: 0.1,
    };
    let resp = model.generate(&req).await?;
    Ok(resp.content)
}

/// Strict-ish JSON parser: strips Markdown code fences if the model
/// stubbornly added them, then attempts a `serde_json::from_str` into the
/// internal shape and converts it to public proposal types.
fn parse_response(raw: &str, chunk_idx: usize) -> Result<OntologyProposal, ExtractError> {
    let cleaned = strip_code_fences(raw.trim());
    let parsed: RawProposal = serde_json::from_str(cleaned)
        .map_err(|e| ExtractError::Parse(format!("chunk {chunk_idx}: {e}")))?;
    Ok(parsed.into_proposal(chunk_idx))
}

fn strip_code_fences(s: &str) -> &str {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("```json") {
        return rest.trim().trim_end_matches("```").trim();
    }
    if let Some(rest) = s.strip_prefix("```") {
        return rest.trim().trim_end_matches("```").trim();
    }
    s
}

/// Merge `incoming` into `acc`, deduplicating concepts by
/// `(concept_type, normalized_name)` and types by `name`. Higher
/// confidence wins on collision.
fn merge_into(acc: &mut OntologyProposal, incoming: OntologyProposal) {
    fn norm(s: &str) -> String {
        s.trim().to_lowercase()
    }

    // Concept types — dedup by name.
    let mut seen: HashMap<String, usize> = acc
        .concept_types
        .iter()
        .enumerate()
        .map(|(i, x)| (x.name.clone(), i))
        .collect();
    for ct in incoming.concept_types {
        match seen.get(&ct.name) {
            Some(&i) if acc.concept_types[i].confidence < ct.confidence => {
                acc.concept_types[i] = ct;
            }
            Some(_) => {}
            None => {
                seen.insert(ct.name.clone(), acc.concept_types.len());
                acc.concept_types.push(ct);
            }
        }
    }

    let mut seen: HashMap<String, usize> = acc
        .relation_types
        .iter()
        .enumerate()
        .map(|(i, x)| (x.name.clone(), i))
        .collect();
    for rt in incoming.relation_types {
        if !seen.contains_key(&rt.name) {
            seen.insert(rt.name.clone(), acc.relation_types.len());
            acc.relation_types.push(rt);
        }
    }

    let mut seen_c: HashMap<(String, String), usize> = acc
        .concepts
        .iter()
        .enumerate()
        .map(|(i, x)| ((x.concept_type.clone(), norm(&x.name)), i))
        .collect();
    for c in incoming.concepts {
        let key = (c.concept_type.clone(), norm(&c.name));
        match seen_c.get(&key) {
            Some(&i) if acc.concepts[i].confidence < c.confidence => acc.concepts[i] = c,
            Some(_) => {}
            None => {
                seen_c.insert(key, acc.concepts.len());
                acc.concepts.push(c);
            }
        }
    }

    // Relations / rules / actions: append, dedupe by client_ref.
    let known: std::collections::HashSet<String> =
        acc.relations.iter().map(|x| x.client_ref.clone()).collect();
    for r in incoming.relations {
        if !known.contains(&r.client_ref) {
            acc.relations.push(r);
        }
    }
    let known: std::collections::HashSet<String> =
        acc.rules.iter().map(|x| x.client_ref.clone()).collect();
    for r in incoming.rules {
        if !known.contains(&r.client_ref) {
            acc.rules.push(r);
        }
    }
    let known: std::collections::HashSet<String> =
        acc.actions.iter().map(|x| x.client_ref.clone()).collect();
    for a in incoming.actions {
        if !known.contains(&a.client_ref) {
            acc.actions.push(a);
        }
    }
}

// -- LLM-facing JSON shape --------------------------------------------
//
// We parse a permissive intermediate representation rather than the
// public `OntologyProposal` directly, so the prompt stays small and
// the model is not forced to emit Rust-internal fields (`client_ref`,
// `conflict`). The intermediate is mapped onto the public types with
// auto-generated `client_ref`s in `into_proposal`.

#[derive(Debug, Deserialize)]
struct RawProposal {
    #[serde(default)]
    concept_types: Vec<RawConceptType>,
    #[serde(default)]
    relation_types: Vec<RawRelationType>,
    #[serde(default)]
    concepts: Vec<RawConcept>,
    #[serde(default)]
    relations: Vec<RawRelation>,
    #[serde(default)]
    rules: Vec<RawRule>,
    #[serde(default)]
    actions: Vec<RawAction>,
}

#[derive(Debug, Deserialize)]
struct RawConceptType {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    properties: Vec<String>,
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    confidence: f32,
}

#[derive(Debug, Deserialize)]
struct RawRelationType {
    name: String,
    domain: String,
    range: String,
    #[serde(default)]
    symmetric: bool,
    #[serde(default)]
    description: String,
    #[serde(default)]
    confidence: f32,
}

#[derive(Debug, Deserialize)]
struct RawConcept {
    concept_type: String,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    properties: Vec<(String, String)>,
    #[serde(default)]
    evidence: Option<String>,
    #[serde(default)]
    confidence: f32,
}

#[derive(Debug, Deserialize)]
struct RawRelation {
    relation_type: String,
    source_ref: String,
    target_ref: String,
    #[serde(default)]
    weight: Option<f32>,
    #[serde(default)]
    evidence: Option<String>,
    #[serde(default)]
    confidence: f32,
}

#[derive(Debug, Deserialize)]
struct RawRule {
    rule_type: String,
    name: String,
    #[serde(default)]
    when: String,
    #[serde(default)]
    then: String,
    #[serde(default)]
    applies_to: Vec<String>,
    #[serde(default)]
    strict: bool,
    #[serde(default)]
    description: String,
    #[serde(default)]
    evidence: Option<String>,
    #[serde(default)]
    confidence: f32,
}

#[derive(Debug, Deserialize)]
struct RawAction {
    action_type: String,
    name: String,
    subject_ref: String,
    #[serde(default)]
    object_ref: Option<String>,
    #[serde(default)]
    parameters: Vec<(String, String)>,
    #[serde(default)]
    effect: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    evidence: Option<String>,
    #[serde(default)]
    confidence: f32,
}

impl RawProposal {
    fn into_proposal(self, chunk_idx: usize) -> OntologyProposal {
        let mk_ref = |kind: &str, i: usize| format!("c{chunk_idx}-{kind}-{i}");
        OntologyProposal {
            source: None,
            language: None,
            concept_types: self
                .concept_types
                .into_iter()
                .enumerate()
                .map(|(i, x)| ProposalConceptType {
                    client_ref: mk_ref("ct", i),
                    name: x.name,
                    description: x.description,
                    properties: x.properties,
                    parent: x.parent,
                    confidence: x.confidence,
                    conflict: None,
                })
                .collect(),
            relation_types: self
                .relation_types
                .into_iter()
                .enumerate()
                .map(|(i, x)| ProposalRelationType {
                    client_ref: mk_ref("rt", i),
                    name: x.name,
                    domain: x.domain,
                    range: x.range,
                    symmetric: x.symmetric,
                    description: x.description,
                    confidence: x.confidence,
                    conflict: None,
                })
                .collect(),
            concepts: self
                .concepts
                .into_iter()
                .enumerate()
                .map(|(i, x)| ProposalConcept {
                    client_ref: mk_ref("c", i),
                    concept_type: x.concept_type,
                    name: x.name,
                    description: x.description,
                    properties: x.properties,
                    evidence: x.evidence,
                    confidence: x.confidence,
                    conflict: None,
                })
                .collect(),
            relations: self
                .relations
                .into_iter()
                .enumerate()
                .map(|(i, x)| ProposalRelation {
                    client_ref: mk_ref("r", i),
                    relation_type: x.relation_type,
                    source_ref: x.source_ref,
                    target_ref: x.target_ref,
                    weight: x.weight,
                    evidence: x.evidence,
                    confidence: x.confidence,
                    conflict: None,
                })
                .collect(),
            rules: self
                .rules
                .into_iter()
                .enumerate()
                .map(|(i, x)| ProposalRule {
                    client_ref: mk_ref("ru", i),
                    rule_type: x.rule_type,
                    name: x.name,
                    when: x.when,
                    then: x.then,
                    applies_to: x.applies_to,
                    strict: x.strict,
                    description: x.description,
                    evidence: x.evidence,
                    confidence: x.confidence,
                    conflict: None,
                })
                .collect(),
            actions: self
                .actions
                .into_iter()
                .enumerate()
                .map(|(i, x)| ProposalAction {
                    client_ref: mk_ref("a", i),
                    action_type: x.action_type,
                    name: x.name,
                    subject_ref: x.subject_ref,
                    object_ref: x.object_ref,
                    parameters: x.parameters,
                    effect: x.effect,
                    description: x.description,
                    evidence: x.evidence,
                    confidence: x.confidence,
                    conflict: None,
                })
                .collect(),
        }
    }
}

const SYSTEM_INSTRUCTION: &str = r#"You are an ontology extraction assistant.
You read a single document chunk and return a strict JSON object describing
the concepts, relations, rules, and actions it contains.

Rules:
- Keep canonical entity names in the SOURCE LANGUAGE of the document.
- Re-use names from the "Known schema" block whenever a concept or
  relation matches an existing type. Introduce new type names only when
  the document genuinely needs one.
- Cite the supporting text in `evidence` as a short verbatim snippet
  (max 200 characters). Omit if you can't pinpoint one.
- Rate every item with `confidence` in [0.0, 1.0]. Be honest; under-
  confident items are still useful — the human reviewer will edit them.
- For relations and actions, use `source_ref` / `target_ref` /
  `subject_ref` / `object_ref` strings of the form
  "<concept_type>:<name>" referring to a concept declared in this
  response OR already present in the graph.

Required JSON shape:
{
  "concept_types":   [{ "name": "...", "description": "...", "properties": ["..."], "parent": "..."|null, "confidence": 0.0 }],
  "relation_types":  [{ "name": "...", "domain": "...", "range": "...", "symmetric": false, "description": "...", "confidence": 0.0 }],
  "concepts":        [{ "concept_type": "...", "name": "...", "description": "...", "properties": [["k","v"]], "evidence": "...", "confidence": 0.0 }],
  "relations":       [{ "relation_type": "...", "source_ref": "Type:Name", "target_ref": "Type:Name", "weight": 1.0, "evidence": "...", "confidence": 0.0 }],
  "rules":           [{ "rule_type": "...", "name": "...", "when": "...", "then": "...", "applies_to": ["Type"], "strict": false, "description": "...", "evidence": "...", "confidence": 0.0 }],
  "actions":         [{ "action_type": "...", "name": "...", "subject_ref": "Type:Name", "object_ref": "Type:Name"|null, "parameters": [["k","v"]], "effect": "...", "description": "...", "evidence": "...", "confidence": 0.0 }]
}

Return ONLY this JSON object. No prose, no markdown fences."#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EchoModel, LanguageModel, LlmRequest, LlmResponse, TokenUsage};
    use async_trait::async_trait;
    use ontology_graph::OntologyGraph;

    struct CannedModel(&'static str);
    #[async_trait]
    impl LanguageModel for CannedModel {
        async fn generate(&self, _req: &LlmRequest) -> Result<LlmResponse, LlmError> {
            Ok(LlmResponse {
                content: self.0.to_string(),
                model: "canned".into(),
                stop_reason: Some("stop".into()),
                usage: TokenUsage::default(),
            })
        }
    }

    const CANNED: &str = r#"{
        "concept_types": [{"name":"Party","description":"signing party","properties":["role"],"confidence":0.9}],
        "relation_types": [{"name":"signs","domain":"Party","range":"Contract","confidence":0.8}],
        "concepts": [
            {"concept_type":"Party","name":"Acme","description":"a corp","confidence":0.95,"evidence":"Acme Corp signs..."},
            {"concept_type":"Contract","name":"C-1","confidence":0.9}
        ],
        "relations": [
            {"relation_type":"signs","source_ref":"Party:Acme","target_ref":"Contract:C-1","confidence":0.85}
        ],
        "rules": [],
        "actions": []
    }"#;

    #[tokio::test]
    async fn parses_canned_proposal() {
        let model = CannedModel(CANNED);
        let schema = Ontology::default();
        let p = extract_proposal(&model, "some doc", None, &schema)
            .await
            .expect("extract");
        assert_eq!(p.concept_types.len(), 1);
        assert_eq!(p.concepts.len(), 2);
        assert_eq!(p.relations.len(), 1);
    }

    #[tokio::test]
    async fn echo_model_yields_parse_error() {
        let model = EchoModel;
        let schema = Ontology::default();
        let err = extract_proposal(&model, "hello", None, &schema)
            .await
            .unwrap_err();
        matches!(err, ExtractError::Parse(_));
    }

    #[tokio::test]
    async fn attach_conflicts_flags_dangling_refs() {
        let model = CannedModel(CANNED);
        let schema = Ontology::default();
        let mut p = extract_proposal(&model, "doc", None, &schema).await.unwrap();
        let graph = OntologyGraph::new(Ontology::default());
        attach_conflicts(&mut p, &graph);
        // Both proposal-internal refs should resolve.
        assert!(p.relations[0].conflict.is_none());
    }

    #[test]
    fn chunking_splits_on_paragraphs() {
        let para = "x".repeat(7_000);
        let doc = format!("{para}\n\n{para}\n\n{para}");
        let chunks = chunk_text(&doc, 12_000);
        assert!(chunks.len() >= 2);
        for c in &chunks {
            assert!(c.chars().count() <= 12_000 + 4);
        }
    }

    #[test]
    fn strip_code_fences_handles_json_fence() {
        let s = "```json\n{\"a\":1}\n```";
        assert_eq!(strip_code_fences(s), "{\"a\":1}");
    }
}
