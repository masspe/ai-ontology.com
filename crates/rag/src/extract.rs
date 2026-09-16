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

use crate::model::{LanguageModel, LlmError, LlmRequest, LlmResponse, Message, Role};

/// Maximum characters per LLM call. Empirically chosen so a single call
/// stays well under the 12 K input-token budget on `gpt-4o-mini` and
/// leaves room for the schema context. Above this size, the document is
/// chunked on paragraph boundaries.
const CHUNK_BUDGET_CHARS: usize = 12_000;
/// Budget for line-structured inputs (JSONL, CSV, flattened spreadsheets):
/// every line becomes concepts and relations in the answer, so the output
/// grows with the input and a prose-sized chunk overflows `max_tokens`.
const LINE_CHUNK_BUDGET_CHARS: usize = 3_000;
/// Below this size a truncated answer is not retried on halves: the model
/// is not running out of room, it is misbehaving.
const MIN_SPLIT_CHARS: usize = 400;
/// Hard cap on LLM calls per document (bisection included).
const MAX_PIECES: usize = 256;

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
    let budget = if looks_line_structured(text) {
        LINE_CHUNK_BUDGET_CHARS
    } else {
        CHUNK_BUDGET_CHARS
    };
    let schema_block = render_schema(schema);
    let lang_hint = language
        .map(|l| format!("Document language (ISO 639-1): {}.", l.code))
        .unwrap_or_default();

    let mut accumulator = OntologyProposal {
        language: language.cloned(),
        ..Default::default()
    };

    // Work queue of pieces still to extract, in document order. A piece
    // whose answer was cut off at `max_tokens` is split in two at a line
    // boundary and both halves go back to the front of the queue, so the
    // output size adapts to the model's limit instead of failing the file.
    let mut pending: std::collections::VecDeque<String> = chunk_text(text, budget).into();
    let total = pending.len();
    let mut idx = 0usize;
    while let Some(piece) = pending.pop_front() {
        if idx >= MAX_PIECES {
            return Err(ExtractError::Parse(format!(
                "gave up after {MAX_PIECES} LLM calls: answers keep getting truncated"
            )));
        }
        let resp = call_llm(model, &schema_block, &lang_hint, &piece, idx, total).await?;
        let truncated_by_model = matches!(
            resp.stop_reason.as_deref(),
            Some("max_tokens") | Some("length")
        );
        match parse_response(&resp.content, idx) {
            Ok(parsed) if !truncated_by_model => {
                merge_into(&mut accumulator, parsed);
            }
            outcome => {
                let cut_off = truncated_by_model || looks_truncated(&resp.content);
                match (cut_off, split_in_half(&piece)) {
                    (true, Some((a, b))) => {
                        tracing::info!(
                            piece = idx,
                            chars = piece.chars().count(),
                            "LLM answer truncated; retrying on two halves"
                        );
                        pending.push_front(b);
                        pending.push_front(a);
                    }
                    (true, None) => {
                        return Err(ExtractError::Parse(format!(
                            "chunk {idx}: the model's answer was cut off (max_tokens) on a piece \
                             too small to split further"
                        )));
                    }
                    (false, _) => {
                        // A genuine parse failure: surface it.
                        outcome?;
                    }
                }
            }
        }
        idx += 1;
    }

    Ok(accumulator)
}

/// Many short lines and (almost) no blank lines: records, not prose.
fn looks_line_structured(text: &str) -> bool {
    let mut lines = 0usize;
    let mut blank = 0usize;
    let mut chars = 0usize;
    for l in text.lines() {
        lines += 1;
        if l.trim().is_empty() {
            blank += 1;
        }
        chars += l.chars().count();
    }
    lines >= 8 && blank * 8 < lines && chars / lines <= 300
}

/// A JSON object that does not close was cut off mid-answer.
fn looks_truncated(raw: &str) -> bool {
    let cleaned = strip_code_fences(raw).trim_end();
    // A fence that was itself cut off leaves `}` followed by a partial fence.
    !cleaned.ends_with('}') && !cleaned.trim_end_matches('`').trim_end().ends_with('}')
}

/// Split at the line break nearest the middle; fall back to a character
/// boundary for a single huge line. `None` when the piece is already small.
fn split_in_half(piece: &str) -> Option<(String, String)> {
    let n = piece.chars().count();
    if n < MIN_SPLIT_CHARS {
        return None;
    }
    let mid_byte = piece
        .char_indices()
        .nth(n / 2)
        .map(|(b, _)| b)
        .unwrap_or(piece.len());
    let before = piece[..mid_byte].rfind('\n');
    let after = piece[mid_byte..].find('\n').map(|i| mid_byte + i);
    let cut = match (before, after) {
        (Some(b), Some(a)) => {
            if mid_byte - b <= a - mid_byte {
                b
            } else {
                a
            }
        }
        (Some(b), None) => b,
        (None, Some(a)) => a,
        (None, None) => mid_byte,
    };
    let (a, b) = piece.split_at(cut);
    let (a, b) = (a.trim_end(), b.trim_start());
    if a.is_empty() || b.is_empty() {
        return None;
    }
    Some((a.to_string(), b.to_string()))
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
    // Split on blank lines (paragraph boundary) and greedily pack. A
    // paragraph over budget (a JSONL or CSV file has no blank lines at all)
    // is split on single line breaks first, and a single oversize line is
    // cut at the budget.
    let mut chunks = Vec::new();
    let mut current = String::new();
    for para in text.split("\n\n") {
        for piece in split_oversize(para, budget) {
            if current.chars().count() + piece.chars().count() + 2 > budget && !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            if !current.is_empty() {
                current.push_str("\n\n");
            }
            current.push_str(&piece);
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn split_oversize(para: &str, budget: usize) -> Vec<String> {
    if para.chars().count() <= budget {
        return vec![para.to_string()];
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    for line in para.split('\n') {
        for piece in hard_split(line, budget) {
            if cur.chars().count() + piece.chars().count() + 1 > budget && !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            if !cur.is_empty() {
                cur.push('\n');
            }
            cur.push_str(&piece);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn hard_split(line: &str, budget: usize) -> Vec<String> {
    if line.chars().count() <= budget {
        return vec![line.to_string()];
    }
    let chars: Vec<char> = line.chars().collect();
    chars
        .chunks(budget.max(1))
        .map(|c| c.iter().collect())
        .collect()
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
            out.push_str(&format!(
                "  - {} ({} -> {})\n",
                rt.name, rt.domain, rt.range
            ));
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
) -> Result<LlmResponse, LlmError> {
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
    model.generate(&req).await
}

/// Lenient JSON parser: strips Markdown code fences if the model stubbornly
/// added them, normalizes the shapes models get wrong (see
/// [`normalize_llm_json`]), then deserializes into the internal shape and
/// converts it to public proposal types.
fn parse_response(raw: &str, chunk_idx: usize) -> Result<OntologyProposal, ExtractError> {
    let cleaned = strip_code_fences(raw.trim());
    let mut value: serde_json::Value = serde_json::from_str(cleaned)
        .map_err(|e| ExtractError::Parse(format!("chunk {chunk_idx}: {e}")))?;
    normalize_llm_json(&mut value);
    let parsed: RawProposal = serde_json::from_value(value)
        .map_err(|e| ExtractError::Parse(format!("chunk {chunk_idx}: {e}")))?;
    Ok(parsed.into_proposal(chunk_idx))
}

/// Make a model's JSON fit `RawProposal` where the intent is unambiguous:
///
/// * `null` on an optional field means "absent" — the key is removed so the
///   `#[serde(default)]` applies (serde rejects `null` for a `String`);
/// * `properties` / `parameters` written as an object `{"k": v}` become the
///   expected `[["k", "v"], …]` pairs;
/// * pair values that are numbers or booleans (a spreadsheet's amounts,
///   dates as numbers) are stringified; pairs with a `null` value are dropped.
///
/// Required fields (`name`, `client_ref`s…) are left alone: a `null` there is
/// a real error and still fails deserialization.
fn normalize_llm_json(v: &mut serde_json::Value) {
    use serde_json::Value;
    match v {
        Value::Object(map) => {
            map.retain(|_, val| !val.is_null());
            for (key, val) in map.iter_mut() {
                if key == "properties" || key == "parameters" {
                    normalize_pairs(val);
                } else {
                    normalize_llm_json(val);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(normalize_llm_json),
        _ => {}
    }
}

fn scalar_to_string(v: &serde_json::Value) -> Option<String> {
    use serde_json::Value;
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

/// `properties` on a concept type is a list of names; on a concept it is a
/// list of `[name, value]` pairs. Both are handled: an object becomes pairs,
/// a list keeps its strings and stringifies pair values.
fn normalize_pairs(v: &mut serde_json::Value) {
    use serde_json::Value;
    let pairs: Vec<Value> = match v {
        Value::Object(map) => map
            .iter()
            .filter_map(|(k, val)| {
                scalar_to_string(val)
                    .map(|s| Value::Array(vec![Value::String(k.clone()), Value::String(s)]))
            })
            .collect(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| match item {
                Value::Array(pair) if pair.len() == 2 => {
                    let k = scalar_to_string(&pair[0])?;
                    let val = scalar_to_string(&pair[1])?;
                    Some(Value::Array(vec![Value::String(k), Value::String(val)]))
                }
                Value::Object(map) if map.len() == 1 => {
                    let (k, val) = map.iter().next()?;
                    let val = scalar_to_string(val)?;
                    Some(Value::Array(vec![
                        Value::String(k.clone()),
                        Value::String(val),
                    ]))
                }
                Value::String(s) => Some(Value::String(s.clone())),
                Value::Null => None,
                other => scalar_to_string(other).map(Value::String),
            })
            .collect(),
        _ => return,
    };
    *v = Value::Array(pairs);
}

/// Strip a Markdown code fence wrapping the answer. The opening fence may
/// carry a language tag (```` ```json ````); the closing fence may be
/// followed by commentary, which is dropped. Without a closing fence (an
/// answer cut off mid-JSON) the remainder is returned as is, so
/// [`looks_truncated`] still sees the unterminated object.
fn strip_code_fences(s: &str) -> &str {
    let s = s.trim();
    let Some(rest) = s.strip_prefix("```") else {
        return s;
    };
    let rest = rest.trim_start_matches(|c: char| c.is_ascii_alphabetic());
    match rest.find("```") {
        Some(end) => rest[..end].trim(),
        None => rest.trim(),
    }
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
- TABULAR INPUT: when the document is a table flattened as lines of
  `header: value; header: value` (a `# Sheet:` heading, CSV or JSON
  records), every line is one concept. Use the identifier column
  (`name`, `id`, code…) as `name` and carry EVERY other column as a
  property pair [header, value] — amounts, dates, quantities, references
  included, values verbatim. Never drop a column because it is numeric.
  Columns that reference other rows or entities (issued_by, contract,
  invoice…) also become relations when a matching relation type exists.

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
    use crate::model::{EchoModel, TokenUsage};
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
        let mut p = extract_proposal(&model, "doc", None, &schema)
            .await
            .unwrap();
        let graph = OntologyGraph::new(Ontology::default());
        attach_conflicts(&mut p, &graph);
        // Both proposal-internal refs should resolve.
        assert!(p.relations[0].conflict.is_none());
    }

    /// What models actually return for a spreadsheet: `null` for empty
    /// cells and optional fields, numbers for amounts, properties as an
    /// object. None of that is a reason to reject the whole proposal.
    #[test]
    fn parse_tolerates_nulls_numbers_and_object_properties() {
        let raw = r#"{
            "concept_types": [
                {"name": "Invoice", "description": null, "properties": ["amount_eur", null], "parent": null}
            ],
            "relation_types": [],
            "concepts": [
                {"concept_type": "Invoice", "name": "INV-2025-001", "description": null,
                 "properties": {"amount_eur": 1200.5, "paid": false, "note": null, "issued_to": "Globex"},
                 "evidence": null, "confidence": 0.9},
                {"concept_type": "Invoice", "name": "INV-2025-002",
                 "properties": [["amount_eur", 80], ["currency", "EUR"], ["due", null]]}
            ],
            "relations": [
                {"relation_type": "issued_to", "source_ref": "c0", "target_ref": "c1", "weight": null, "evidence": null}
            ],
            "rules": null,
            "actions": []
        }"#;
        let p = parse_response(raw, 0).expect("lenient parse");
        assert_eq!(p.concept_types.len(), 1);
        assert_eq!(
            p.concept_types[0].properties,
            vec!["amount_eur".to_string()]
        );
        assert_eq!(p.concepts.len(), 2);
        let first = &p.concepts[0];
        assert_eq!(first.description, "");
        let props: std::collections::BTreeMap<_, _> = first.properties.iter().cloned().collect();
        assert_eq!(props.get("amount_eur").map(String::as_str), Some("1200.5"));
        assert_eq!(props.get("paid").map(String::as_str), Some("false"));
        assert_eq!(props.get("issued_to").map(String::as_str), Some("Globex"));
        assert!(!props.contains_key("note"), "null-valued pair dropped");
        let second: std::collections::BTreeMap<_, _> =
            p.concepts[1].properties.iter().cloned().collect();
        assert_eq!(second.get("amount_eur").map(String::as_str), Some("80"));
        assert!(!second.contains_key("due"));
        assert_eq!(p.relations.len(), 1);
        assert!(p.relations[0].weight.is_none());
        assert!(p.rules.is_empty());
    }

    /// A `null` where the model must speak (a concept's name) is still an error.
    #[test]
    fn parse_still_rejects_null_required_fields() {
        let raw = r#"{"concepts": [{"concept_type": "Invoice", "name": null}]}"#;
        let err = parse_response(raw, 0).unwrap_err();
        assert!(matches!(err, ExtractError::Parse(_)));
    }

    /// A model whose answer is cut off whenever the piece has more than
    /// `max_lines` lines, and otherwise returns one concept per line.
    struct TruncatingModel {
        max_lines: usize,
        calls: std::sync::atomic::AtomicUsize,
    }
    #[async_trait]
    impl LanguageModel for TruncatingModel {
        async fn generate(&self, req: &LlmRequest) -> Result<LlmResponse, LlmError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let content = &req.messages[0].content;
            let doc = content
                .split("Document:\n")
                .nth(1)
                .and_then(|s| s.split("\n---\n").next())
                .unwrap_or("");
            let lines: Vec<&str> = doc.lines().filter(|l| !l.trim().is_empty()).collect();
            if lines.len() > self.max_lines {
                return Ok(LlmResponse {
                    content: r#"{"concepts": [{"concept_type": "Row", "name": "cut off mid"#.into(),
                    model: "trunc".into(),
                    stop_reason: Some("max_tokens".into()),
                    usage: TokenUsage::default(),
                });
            }
            let concepts: Vec<String> = lines
                .iter()
                .map(|l| {
                    format!(
                        r#"{{"concept_type":"Row","name":"{}"}}"#,
                        l.trim().replace('"', "'")
                    )
                })
                .collect();
            Ok(LlmResponse {
                content: format!(r#"{{"concepts": [{}]}}"#, concepts.join(",")),
                model: "trunc".into(),
                stop_reason: Some("stop".into()),
                usage: TokenUsage::default(),
            })
        }
    }

    fn records(n: usize) -> String {
        (0..n)
            .map(|i| format!(r#"{{"kind":"Relation","source":"S{i:03}","target":"T{i:03}","relation_type":"between","weight":1.0,"padding":"{}"}}"#, "x".repeat(60)))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A truncated answer is retried on halves until every piece fits; the
    /// document is never rejected for being dense.
    #[tokio::test]
    async fn truncated_answers_are_retried_on_halves() {
        let model = TruncatingModel {
            max_lines: 3,
            calls: Default::default(),
        };
        let doc = records(16);
        assert!(looks_line_structured(&doc));
        let p = extract_proposal(&model, &doc, None, &Ontology::default())
            .await
            .expect("bisection recovers");
        assert_eq!(
            p.concepts.len(),
            16,
            "one concept per record: {:?}",
            p.concepts.iter().map(|c| &c.name).collect::<Vec<_>>()
        );
        let calls = model.calls.load(std::sync::atomic::Ordering::SeqCst);
        // 16 records fit in one 3 000-char chunk: 1 + 2 + 4 + 8 calls.
        assert_eq!(calls, 15, "calls");
        let refs: std::collections::HashSet<_> =
            p.concepts.iter().map(|c| c.client_ref.clone()).collect();
        assert_eq!(refs.len(), 16, "client refs stay unique across pieces");
    }

    /// A piece too small to split that still gets cut off is a real error.
    #[tokio::test]
    async fn truncation_on_a_tiny_piece_is_an_error() {
        let model = TruncatingModel {
            max_lines: 0,
            calls: Default::default(),
        };
        let err = extract_proposal(&model, "a\nb", None, &Ontology::default())
            .await
            .unwrap_err();
        assert!(matches!(err, ExtractError::Parse(_)), "{err:?}");
        assert_eq!(model.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn line_structured_inputs_get_the_small_budget() {
        assert!(looks_line_structured(&records(20)));
        let prose = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. ".repeat(40);
        let doc = format!("{prose}\n\n{prose}\n\n{prose}");
        assert!(!looks_line_structured(&doc));
    }

    #[test]
    fn chunking_splits_an_oversize_paragraph_on_lines() {
        let doc = records(200); // ~150 chars per line, no blank line
        let chunks = chunk_text(&doc, 3_000);
        assert!(chunks.len() >= 8, "{}", chunks.len());
        for c in &chunks {
            assert!(c.chars().count() <= 3_000, "chunk over budget");
            assert!(
                c.starts_with('{') && c.ends_with('}'),
                "lines are never cut: {c:?}"
            );
        }
        let rejoined: Vec<&str> = chunks.iter().flat_map(|c| c.lines()).collect();
        assert_eq!(rejoined.len(), 200);
        assert_eq!(rejoined.join("\n"), doc);
    }

    #[test]
    fn split_in_half_prefers_a_line_break() {
        let doc = records(4);
        let (a, b) = split_in_half(&doc).unwrap();
        assert_eq!(a.lines().count(), 2);
        assert_eq!(b.lines().count(), 2);
        assert!(split_in_half("short").is_none());
        let huge = "y".repeat(1_000);
        let (a, b) = split_in_half(&huge).unwrap();
        assert_eq!(a.len() + b.len(), 1_000);
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

    // -- hardening: proposal parsing -----------------------------------

    /// A language-tagged fence followed by commentary still yields the JSON
    /// object; a fence with no closing marker keeps the (truncated) body.
    #[test]
    fn strip_code_fences_drops_language_tag_and_trailing_prose() {
        let s = "```json\n{\"a\":1}\n```\nHere is the extraction you asked for.";
        assert_eq!(strip_code_fences(s), "{\"a\":1}");
        assert_eq!(strip_code_fences("```JSON\n{\"a\":1}\n```"), "{\"a\":1}");
        assert_eq!(strip_code_fences("```\n{\"a\":1}\n```  "), "{\"a\":1}");
        assert_eq!(strip_code_fences("```json\n{\"a\": ["), "{\"a\": [");
        assert_eq!(strip_code_fences("  {\"a\":1}  "), "{\"a\":1}");
        // Through the full parser: the commentary must not poison the parse.
        let p = parse_response(
            "```json\n{\"concepts\":[{\"concept_type\":\"T\",\"name\":\"n\"}]}\n```\nDone.",
            0,
        )
        .expect("fenced answer with trailing prose parses");
        assert_eq!(p.concepts.len(), 1);
    }

    /// Extra keys at any level are ignored; nested objects / arrays used as
    /// property values are stringified rather than rejected.
    #[test]
    fn parse_ignores_unknown_fields_and_stringifies_nested_property_values() {
        let raw = r#"{
            "summary": "ignored top-level key",
            "concepts": [
                {"concept_type": "Company", "name": "Acme", "source_line": 3,
                 "properties": {"address": {"city": "Lyon", "zip": 69001}, "tags": ["a", "b"]}}
            ]
        }"#;
        let p = parse_response(raw, 0).expect("unknown fields are tolerated");
        let props: std::collections::BTreeMap<_, _> =
            p.concepts[0].properties.iter().cloned().collect();
        assert_eq!(
            props.get("address").map(String::as_str),
            Some(r#"{"city":"Lyon","zip":69001}"#)
        );
        assert_eq!(props.get("tags").map(String::as_str), Some(r#"["a","b"]"#));
    }

    /// `parameters` on an action gets the same object → pairs treatment as
    /// `properties`, and a `null` `object_ref` means "no object".
    #[test]
    fn parse_normalizes_action_parameters_object_into_pairs() {
        let raw = r#"{"actions": [
            {"action_type": "sign", "name": "Sign C-1", "subject_ref": "Person:Alice",
             "object_ref": null, "parameters": {"date": "2025-01-15", "copies": 2, "witness": null}}
        ]}"#;
        let p = parse_response(raw, 2).unwrap();
        let a = &p.actions[0];
        assert_eq!(a.client_ref, "c2-a-0");
        assert!(a.object_ref.is_none());
        let params: std::collections::BTreeMap<_, _> = a.parameters.iter().cloned().collect();
        assert_eq!(params.get("date").map(String::as_str), Some("2025-01-15"));
        assert_eq!(params.get("copies").map(String::as_str), Some("2"));
        assert!(
            !params.contains_key("witness"),
            "null-valued parameter dropped"
        );
    }

    /// Pins current behaviour: `confidence` must be a number; a quoted
    /// number is a parse error naming the chunk.
    #[test]
    fn parse_rejects_confidence_given_as_a_string() {
        let raw = r#"{"concepts": [{"concept_type": "T", "name": "n", "confidence": "0.9"}]}"#;
        let err = parse_response(raw, 4).unwrap_err();
        match err {
            ExtractError::Parse(msg) => assert!(msg.starts_with("chunk 4:"), "{msg}"),
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    /// `client_ref`s are `c<chunk>-<kind>-<index>`: unique across chunks
    /// and across kinds within a chunk.
    #[test]
    fn client_refs_encode_chunk_index_and_are_unique_per_kind() {
        let raw = r#"{
            "concept_types": [{"name":"A"}],
            "relation_types": [{"name":"r","domain":"A","range":"A"}],
            "concepts": [{"concept_type":"A","name":"x"},{"concept_type":"A","name":"y"}],
            "relations": [{"relation_type":"r","source_ref":"A:x","target_ref":"A:y"}],
            "rules": [{"rule_type":"k","name":"R"}],
            "actions": [{"action_type":"k","name":"Do","subject_ref":"A:x"}]
        }"#;
        let p = parse_response(raw, 7).unwrap();
        let mut refs: Vec<String> = Vec::new();
        refs.extend(p.concept_types.iter().map(|x| x.client_ref.clone()));
        refs.extend(p.relation_types.iter().map(|x| x.client_ref.clone()));
        refs.extend(p.concepts.iter().map(|x| x.client_ref.clone()));
        refs.extend(p.relations.iter().map(|x| x.client_ref.clone()));
        refs.extend(p.rules.iter().map(|x| x.client_ref.clone()));
        refs.extend(p.actions.iter().map(|x| x.client_ref.clone()));
        assert_eq!(
            refs,
            ["c7-ct-0", "c7-rt-0", "c7-c-0", "c7-c-1", "c7-r-0", "c7-ru-0", "c7-a-0"]
        );
        let unique: std::collections::HashSet<_> = refs.iter().collect();
        assert_eq!(unique.len(), refs.len());
    }

    // -- hardening: merge ---------------------------------------------

    fn concept(ty: &str, name: &str, conf: f32, r: &str) -> ProposalConcept {
        ProposalConcept {
            client_ref: r.into(),
            concept_type: ty.into(),
            name: name.into(),
            description: String::new(),
            properties: vec![],
            evidence: None,
            confidence: conf,
            conflict: None,
        }
    }

    fn concept_type(name: &str, conf: f32, r: &str) -> ProposalConceptType {
        ProposalConceptType {
            client_ref: r.into(),
            name: name.into(),
            description: String::new(),
            properties: vec![],
            parent: None,
            confidence: conf,
            conflict: None,
        }
    }

    fn relation_type(name: &str, conf: f32, r: &str) -> ProposalRelationType {
        ProposalRelationType {
            client_ref: r.into(),
            name: name.into(),
            domain: "A".into(),
            range: "A".into(),
            symmetric: false,
            description: String::new(),
            confidence: conf,
            conflict: None,
        }
    }

    fn relation(src: &str, tgt: &str, r: &str) -> ProposalRelation {
        ProposalRelation {
            client_ref: r.into(),
            relation_type: "rel".into(),
            source_ref: src.into(),
            target_ref: tgt.into(),
            weight: None,
            evidence: None,
            confidence: 0.5,
            conflict: None,
        }
    }

    fn action(subject: &str, object: Option<&str>, r: &str) -> ProposalAction {
        ProposalAction {
            client_ref: r.into(),
            action_type: "sign".into(),
            name: "Sign".into(),
            subject_ref: subject.into(),
            object_ref: object.map(str::to_string),
            parameters: vec![],
            effect: String::new(),
            description: String::new(),
            evidence: None,
            confidence: 0.5,
            conflict: None,
        }
    }

    /// Same `(type, name)` modulo case and surrounding whitespace is one
    /// concept; the higher-confidence version wins, the first one stays on
    /// a tie, and the same name under another type is a different concept.
    #[test]
    fn merge_dedupes_concepts_case_insensitively_keeping_higher_confidence() {
        let mut acc = OntologyProposal {
            concepts: vec![
                concept("Party", "Acme", 0.5, "c0-c-0"),
                concept("Party", "Globex", 0.9, "c0-c-1"),
            ],
            ..Default::default()
        };
        let incoming = OntologyProposal {
            concepts: vec![
                concept("Party", "  acme ", 0.9, "c1-c-0"),
                concept("Party", "GLOBEX", 0.1, "c1-c-1"),
                concept("Contract", "Acme", 0.2, "c1-c-2"),
            ],
            ..Default::default()
        };
        merge_into(&mut acc, incoming);
        let names: Vec<(String, String, f32)> = acc
            .concepts
            .iter()
            .map(|c| (c.concept_type.clone(), c.name.clone(), c.confidence))
            .collect();
        assert_eq!(
            names,
            vec![
                ("Party".into(), "  acme ".into(), 0.9),
                ("Party".into(), "Globex".into(), 0.9),
                ("Contract".into(), "Acme".into(), 0.2),
            ]
        );
        // The winner keeps its own client_ref (no ref rewriting).
        assert_eq!(acc.concepts[0].client_ref, "c1-c-0");
        assert_eq!(acc.concepts[1].client_ref, "c0-c-1");
    }

    /// Concept types dedupe by exact name with higher confidence winning;
    /// relation types dedupe by name, first one wins; relations, rules and
    /// actions dedupe by `client_ref` only, so two chunks may each keep a
    /// relation to the same concept.
    #[test]
    fn merge_dedupes_types_by_name_and_keeps_all_relations() {
        let mut acc = OntologyProposal {
            concept_types: vec![concept_type("Party", 0.4, "c0-ct-0")],
            relation_types: vec![relation_type("signs", 0.4, "c0-rt-0")],
            relations: vec![relation("Party:Acme", "Contract:C-1", "c0-r-0")],
            ..Default::default()
        };
        let incoming = OntologyProposal {
            concept_types: vec![
                concept_type("Party", 0.8, "c1-ct-0"),
                concept_type("party", 0.1, "c1-ct-1"),
            ],
            relation_types: vec![relation_type("signs", 0.9, "c1-rt-0")],
            relations: vec![
                relation("Party:Acme", "Contract:C-2", "c1-r-0"),
                relation("Party:Acme", "Contract:C-1", "c0-r-0"),
            ],
            ..Default::default()
        };
        merge_into(&mut acc, incoming);
        assert_eq!(acc.concept_types.len(), 2, "type names are case-sensitive");
        assert_eq!(acc.concept_types[0].confidence, 0.8);
        assert_eq!(acc.concept_types[0].client_ref, "c1-ct-0");
        assert_eq!(acc.relation_types.len(), 1);
        assert_eq!(
            acc.relation_types[0].client_ref, "c0-rt-0",
            "relation types: first declaration wins regardless of confidence"
        );
        let rels: Vec<&str> = acc
            .relations
            .iter()
            .map(|r| r.client_ref.as_str())
            .collect();
        assert_eq!(rels, ["c0-r-0", "c1-r-0"]);
    }

    // -- hardening: conflicts -----------------------------------------

    fn graph_with_party_acme() -> (OntologyGraph, ontology_graph::ConceptId) {
        let mut o = Ontology::new();
        o.add_concept_type(ontology_graph::ConceptType {
            name: "Party".into(),
            description: "signing party".into(),
            ..Default::default()
        });
        o.add_concept_type(ontology_graph::ConceptType {
            name: "Contract".into(),
            ..Default::default()
        });
        o.add_relation_type(ontology_graph::RelationType {
            name: "signs".into(),
            domain: "Party".into(),
            range: "Contract".into(),
            ..Default::default()
        })
        .unwrap();
        let g = OntologyGraph::new(o);
        let id = g
            .upsert_concept(ontology_graph::Concept::new(
                Default::default(),
                "Party",
                "Acme Corp",
            ))
            .unwrap();
        (g, id)
    }

    /// Existing schema names and existing concepts (matched case-
    /// insensitively by name) are flagged `Exists` with the live identity.
    #[test]
    fn attach_conflicts_flags_existing_types_and_concepts() {
        let (g, acme_id) = graph_with_party_acme();
        let mut p = OntologyProposal {
            concept_types: vec![
                concept_type("Party", 0.9, "c0-ct-0"),
                concept_type("Invoice", 0.9, "c0-ct-1"),
            ],
            relation_types: vec![
                relation_type("signs", 0.9, "c0-rt-0"),
                relation_type("owns", 0.9, "c0-rt-1"),
            ],
            concepts: vec![
                concept("Party", "acme corp", 0.9, "c0-c-0"),
                concept("Party", "Globex", 0.9, "c0-c-1"),
                concept("Contract", "Acme Corp", 0.9, "c0-c-2"),
            ],
            ..Default::default()
        };
        attach_conflicts(&mut p, &g);

        match &p.concept_types[0].conflict {
            Some(ConflictInfo {
                kind: ConflictKind::Exists { existing_id, .. },
                summary,
            }) => {
                assert_eq!(existing_id, "Party");
                assert!(summary.contains("`Party`"), "{summary}");
            }
            other => panic!("expected Exists on Party, got {other:?}"),
        }
        assert!(p.concept_types[1].conflict.is_none());

        match &p.relation_types[0].conflict {
            Some(ConflictInfo {
                kind:
                    ConflictKind::Exists {
                        existing_display, ..
                    },
                ..
            }) => assert_eq!(existing_display, "signs: Party → Contract"),
            other => panic!("expected Exists on signs, got {other:?}"),
        }
        assert!(p.relation_types[1].conflict.is_none());

        match &p.concepts[0].conflict {
            Some(ConflictInfo {
                kind:
                    ConflictKind::Exists {
                        existing_id,
                        existing_display,
                    },
                ..
            }) => {
                assert_eq!(existing_id, &acme_id.0.to_string());
                assert_eq!(existing_display, "Party:Acme Corp");
            }
            other => panic!("expected Exists on acme corp, got {other:?}"),
        }
        assert!(p.concepts[1].conflict.is_none(), "unknown name");
        assert!(
            p.concepts[2].conflict.is_none(),
            "same name under another type is not a collision"
        );
    }

    /// Relation and action endpoints must resolve to a proposal concept
    /// (by `client_ref` or `Type:Name`) or a live graph concept; the first
    /// unresolved endpoint is reported, and a missing `object_ref` is fine.
    #[test]
    fn attach_conflicts_flags_dangling_relation_and_action_refs() {
        let (g, _) = graph_with_party_acme();
        let mut p = OntologyProposal {
            concepts: vec![concept("Contract", "C-1", 0.9, "c0-c-0")],
            relations: vec![
                // source by Type:Name of a live concept, target by client_ref.
                relation("Party:Acme Corp", "c0-c-0", "c0-r-0"),
                // source by Type:Name of a proposal concept, target unknown.
                relation("Contract:C-1", "Party:Nobody", "c0-r-1"),
                // both unknown: the source is reported first.
                relation("c9-c-9", "Party:Nobody", "c0-r-2"),
                // no colon and not a client_ref: dangling.
                relation("Acme Corp", "c0-c-0", "c0-r-3"),
            ],
            actions: vec![
                action("Party:acme corp", None, "c0-a-0"),
                action("c0-c-0", Some("Contract:C-404"), "c0-a-1"),
            ],
            ..Default::default()
        };
        attach_conflicts(&mut p, &g);

        fn missing(c: &Option<ConflictInfo>) -> Option<&str> {
            match c {
                Some(ConflictInfo {
                    kind: ConflictKind::DanglingRef { missing_ref },
                    ..
                }) => Some(missing_ref.as_str()),
                _ => None,
            }
        }
        fn summary(c: &Option<ConflictInfo>) -> &str {
            c.as_ref().map(|c| c.summary.as_str()).unwrap_or("")
        }
        assert!(p.relations[0].conflict.is_none());
        assert_eq!(missing(&p.relations[1].conflict), Some("Party:Nobody"));
        assert!(summary(&p.relations[1].conflict).starts_with("Target"));
        assert_eq!(missing(&p.relations[2].conflict), Some("c9-c-9"));
        assert!(summary(&p.relations[2].conflict).starts_with("Source"));
        assert_eq!(missing(&p.relations[3].conflict), Some("Acme Corp"));
        assert!(
            p.actions[0].conflict.is_none(),
            "live lookup is case-insensitive and a missing object is allowed"
        );
        assert_eq!(missing(&p.actions[1].conflict), Some("Contract:C-404"));
        assert!(summary(&p.actions[1].conflict).starts_with("Object"));
    }

    /// Re-running `attach_conflicts` after the proposal was edited replaces
    /// stale annotations instead of accumulating them.
    #[test]
    fn attach_conflicts_is_idempotent_and_clears_resolved_refs() {
        let (g, _) = graph_with_party_acme();
        let mut p = OntologyProposal {
            relations: vec![relation("Contract:C-1", "Party:Acme Corp", "c0-r-0")],
            ..Default::default()
        };
        attach_conflicts(&mut p, &g);
        assert!(matches!(
            p.relations[0].conflict,
            Some(ConflictInfo {
                kind: ConflictKind::DanglingRef { .. },
                ..
            })
        ));
        // The reviewer adds the missing concept: the relation now resolves.
        p.concepts.push(concept("Contract", "C-1", 0.9, "c0-c-0"));
        attach_conflicts(&mut p, &g);
        assert!(p.relations[0].conflict.is_none());
        attach_conflicts(&mut p, &g);
        assert!(p.relations[0].conflict.is_none());
    }

    // -- hardening: chunking ------------------------------------------

    /// A single line over budget is cut on character boundaries: no
    /// panic inside a multi-byte sequence, no character lost or reordered.
    #[test]
    fn hard_split_is_lossless_on_multibyte_text() {
        let line: String = "é漢😀x".repeat(250); // 1 000 chars, 2 500 bytes
        let pieces = hard_split(&line, 300);
        assert_eq!(pieces.len(), 4);
        assert!(pieces.iter().all(|p| p.chars().count() <= 300));
        assert_eq!(pieces.concat(), line);
        // Same guarantee through the public chunker on a blank-line-free text.
        let chunks = chunk_text(&line, 300);
        assert_eq!(chunks.concat(), line);
        // And the bisection fallback on a huge single line.
        let (a, b) = split_in_half(&line).unwrap();
        assert_eq!(format!("{a}{b}"), line);
    }

    /// Prose paragraphs mixed with a blank-line-free table: no chunk over
    /// budget, no line ever cut, and document order preserved.
    #[test]
    fn chunking_keeps_mixed_prose_and_table_lines_intact_and_ordered() {
        let prose = |i: usize| format!("Paragraph {i}: ") + &"lorem ipsum ".repeat(30);
        let table: Vec<String> = (0..60)
            .map(|i| format!("row: R{i:03}; amount: {}; currency: EUR", i * 10))
            .collect();
        let doc = format!(
            "{}\n\n{}\n\n{}\n\n{}",
            prose(1),
            prose(2),
            table.join("\n"),
            prose(3)
        );
        assert!(
            looks_line_structured(&doc),
            "a table-dominated document counts as line-structured"
        );
        let budget = 1_000;
        let chunks = chunk_text(&doc, budget);
        assert!(chunks.len() >= 3, "{}", chunks.len());
        for c in &chunks {
            assert!(
                c.chars().count() <= budget,
                "chunk over budget: {}",
                c.len()
            );
        }
        let original: Vec<&str> = doc.lines().filter(|l| !l.is_empty()).collect();
        let rejoined: Vec<&str> = chunks
            .iter()
            .flat_map(|c| c.lines())
            .filter(|l| !l.is_empty())
            .collect();
        assert_eq!(rejoined, original);
    }

    /// Fenced answers: an unterminated object is truncated, a complete one
    /// is not — even when the closing fence itself was cut off.
    #[test]
    fn looks_truncated_recognises_cut_off_fenced_output() {
        assert!(looks_truncated("```json\n{\"concepts\": [{\"name\": \"x"));
        assert!(looks_truncated("{\"concepts\": ["));
        assert!(!looks_truncated("```json\n{\"concepts\": []}\n```"));
        assert!(!looks_truncated("```json\n{\"concepts\": []}\n``"));
        assert!(!looks_truncated("{\"concepts\": []}\n\n"));
        assert!(!looks_truncated("```json\n{}\n```\nThat is all."));
    }

    // -- hardening: work queue ----------------------------------------

    /// Answers a syntactically complete but partial object with an OpenAI
    /// `length` stop reason whenever the piece has more than `max_lines`.
    struct PartialOnLengthModel {
        max_lines: usize,
        calls: std::sync::atomic::AtomicUsize,
    }
    #[async_trait]
    impl LanguageModel for PartialOnLengthModel {
        async fn generate(&self, req: &LlmRequest) -> Result<LlmResponse, LlmError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let doc = req.messages[0]
                .content
                .split("Document:\n")
                .nth(1)
                .and_then(|s| s.split("\n---\n").next())
                .unwrap_or("");
            let lines: Vec<&str> = doc.lines().filter(|l| !l.trim().is_empty()).collect();
            if lines.len() > self.max_lines {
                return Ok(LlmResponse {
                    content: r#"{"concepts": [{"concept_type": "Row", "name": "PARTIAL"}]}"#.into(),
                    model: "m".into(),
                    stop_reason: Some("length".into()),
                    usage: TokenUsage::default(),
                });
            }
            let concepts: Vec<String> = lines
                .iter()
                .map(|l| format!(r#"{{"concept_type":"Row","name":"{}"}}"#, l.trim()))
                .collect();
            Ok(LlmResponse {
                content: format!(r#"{{"concepts": [{}]}}"#, concepts.join(",")),
                model: "m".into(),
                stop_reason: Some("stop".into()),
                usage: TokenUsage::default(),
            })
        }
    }

    /// A `length` / `max_tokens` stop reason marks the answer as cut off
    /// even when the JSON happens to parse: the partial result is dropped
    /// and the piece is bisected.
    #[tokio::test]
    async fn truncation_stop_reason_discards_a_parseable_partial_answer() {
        let model = PartialOnLengthModel {
            max_lines: 2,
            calls: Default::default(),
        };
        let doc: String = (0..4)
            .map(|i| format!("line{i} {}", "x".repeat(150)))
            .collect::<Vec<_>>()
            .join("\n");
        let p = extract_proposal(&model, &doc, None, &Ontology::default())
            .await
            .expect("halves fit");
        let names: Vec<&str> = p.concepts.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names.len(), 4);
        assert!(names.iter().all(|n| n.starts_with("line")), "{names:?}");
        assert!(
            !names.contains(&"PARTIAL"),
            "partial answer must not be merged"
        );
        assert_eq!(model.calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    /// Bisection is bounded: once `MAX_PIECES` calls were spent the
    /// extraction fails with a `Parse` error instead of running on.
    #[tokio::test]
    async fn max_pieces_cap_fails_fast_with_a_parse_error() {
        // ~130 chars per record, ~22 per 3 000-char chunk; every chunk is
        // answered truncated once, then its halves succeed: 3 calls per
        // chunk, far more than the cap over 2 000 records.
        let model = TruncatingModel {
            max_lines: 12,
            calls: Default::default(),
        };
        let doc = records(2_000);
        let err = extract_proposal(&model, &doc, None, &Ontology::default())
            .await
            .unwrap_err();
        match &err {
            ExtractError::Parse(msg) => {
                assert!(msg.contains("gave up after"), "{msg}");
                assert!(msg.contains(&MAX_PIECES.to_string()), "{msg}");
            }
            other => panic!("expected Parse, got {other:?}"),
        }
        assert_eq!(
            model.calls.load(std::sync::atomic::Ordering::SeqCst),
            MAX_PIECES,
            "exactly MAX_PIECES calls are spent before giving up"
        );
    }

    /// Halves go back to the front of the queue, so the merged concepts
    /// come out in document order even after several levels of bisection.
    #[tokio::test]
    async fn bisection_preserves_document_order() {
        let model = TruncatingModel {
            max_lines: 3,
            calls: Default::default(),
        };
        let doc = records(16);
        let p = extract_proposal(&model, &doc, None, &Ontology::default())
            .await
            .unwrap();
        let expected: Vec<String> = doc.lines().map(|l| l.replace('"', "'")).collect();
        let got: Vec<String> = p.concepts.iter().map(|c| c.name.clone()).collect();
        assert_eq!(got, expected);
    }
}
