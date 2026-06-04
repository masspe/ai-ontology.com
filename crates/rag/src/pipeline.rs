// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

use futures::stream::{BoxStream, StreamExt};
use ontology_graph::{ConceptId, OntologyGraph, Subgraph};
use ontology_index::{HybridIndex, RetrievalRequest, ScoredConcept};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::debug;

use crate::model::{
    LanguageModel, LlmError, LlmRequest, LlmResponse, Message, StreamChunk, TokenUsage,
};
use crate::prompt::PromptBuilder;

/// End-to-end answer returned by the pipeline. Includes the retrieved
/// context so callers can render citations / "show your work" UIs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagAnswer {
    pub query: String,
    pub answer: String,
    pub retrieved: Vec<ScoredConcept>,
    pub subgraph: Subgraph,
    pub model: String,
    pub stop_reason: Option<String>,
    /// Token usage including prompt-cache hits — non-zero
    /// `usage.cache_read_input_tokens` confirms the ontology was served from
    /// cache on this call.
    #[serde(default)]
    pub usage: TokenUsage,
    /// Concept ids the model cited (parsed from the `Cited: [#…]` line) that
    /// are present in `subgraph.concepts`. Empty when the model declined to
    /// answer or emitted no parseable citations.
    #[serde(default)]
    pub cited: Vec<ConceptId>,
    /// Prose answer with the `Cited:` line stripped. Falls back to `answer`
    /// verbatim when no `Cited:` line was emitted.
    #[serde(default)]
    pub answer_body: String,
}

impl RagAnswer {
    pub fn citations(&self) -> Vec<ConceptId> {
        self.retrieved.iter().map(|s| s.id).collect()
    }
}

/// One frame of a streaming RAG answer.
///
/// Order on the wire is always:
/// 1. Exactly one `Retrieved` (the seeds + subgraph that grounds the answer).
/// 2. Zero or more `Token` chunks as the LLM produces them.
/// 3. Exactly one `End` carrying the final usage and stop reason.
///
/// `Token` is a struct variant (`{ "type": "token", "text": "…" }`) rather
/// than a tuple variant — `#[serde(tag)]` with newtype-of-String produces
/// surprising JSON, and the explicit `text` key plays nicer with TS types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RagStreamEvent {
    Retrieved {
        query: String,
        scored: Vec<ScoredConcept>,
        subgraph: Subgraph,
    },
    Token {
        text: String,
    },
    End {
        #[serde(default)]
        usage: TokenUsage,
        #[serde(default)]
        model: String,
        #[serde(default)]
        stop_reason: Option<String>,
    },
}

pub type RagStream = BoxStream<'static, Result<RagStreamEvent, LlmError>>;

/// Wires together a [`HybridIndex`] and a [`LanguageModel`].
///
/// The pipeline holds `Arc`s so it is cheap to clone and share across
/// async tasks. Both stages are fully `Send + Sync`, so multiple concurrent
/// requests can be handled by a single pipeline instance.
#[derive(Clone)]
pub struct RagPipeline {
    pub index: Arc<HybridIndex>,
    pub llm: Arc<dyn LanguageModel>,
    pub max_tokens: u32,
    pub temperature: f32,
    pub max_context_chars: usize,
}

impl RagPipeline {
    pub fn new(index: Arc<HybridIndex>, llm: Arc<dyn LanguageModel>) -> Self {
        Self {
            index,
            llm,
            max_tokens: 1024,
            temperature: 0.0,
            max_context_chars: 6000,
        }
    }

    pub async fn answer(
        &self,
        query: impl Into<String>,
    ) -> Result<RagAnswer, crate::model::LlmError> {
        let query = query.into();
        let req = RetrievalRequest {
            query: query.clone(),
            ..Default::default()
        };
        self.answer_with(req).await
    }

    pub async fn answer_with(
        &self,
        req: RetrievalRequest,
    ) -> Result<RagAnswer, crate::model::LlmError> {
        let (scored, mut subgraph) = self.index.retrieve(&req);
        enrich_with_transitive_closures(self.index.graph(), &mut subgraph, 16);
        debug!(
            seeds = scored.len(),
            context_concepts = subgraph.concepts.len(),
            "retrieved"
        );

        let onto = self.index.graph().ontology();
        let builder = PromptBuilder::new(&onto).with_max_chars(self.max_context_chars);

        // Split context: ontology is stable per-KB → cached system block.
        // Retrieved subgraph is volatile per-query → user message.
        let cached_ontology = builder.render_static_context();
        let query_context = builder.render_query_context(&scored, &subgraph);

        let user_message = format!(
            "Use the context below to answer the question.\n\n\
             ---RETRIEVED---\n{ctx}\n---END RETRIEVED---\n\n\
             Question: {q}",
            ctx = query_context,
            q = req.query,
        );

        let llm_req = LlmRequest {
            system: Some(PromptBuilder::system_message().to_string()),
            cached_context: Some(cached_ontology),
            messages: vec![Message::user(user_message)],
            max_tokens: self.max_tokens,
            temperature: self.temperature,
        };

        let valid_ids: std::collections::HashSet<ConceptId> =
            subgraph.concepts.iter().map(|c| c.id).collect();

        let LlmResponse {
            content,
            model,
            stop_reason,
            usage,
        } = self.llm.generate(&llm_req).await?;

        let parsed = parse_cited_answer(&content);
        let (cited_valid, invalid_cited): (Vec<_>, Vec<_>) = parsed
            .cited
            .iter()
            .copied()
            .partition(|id| valid_ids.contains(id));

        // If the model invented ids, retry ONCE with a stricter reminder
        // listing the allowed id set. Replace the answer only if the retry
        // produces zero invalid ids OR more valid ids than the first try.
        let (final_content, final_model, final_stop, final_usage, cited, body) =
            if !invalid_cited.is_empty() {
                let allowed: Vec<String> = valid_ids
                    .iter()
                    .map(|id| format!("#{}", id.0))
                    .collect();
                let reminder = format!(
                    "Your previous answer cited ids not in the Subgraph: {}. \
                     Valid ids for this question are exactly: [{}]. \
                     Re-answer using the same two-line format and cite ONLY ids \
                     from that set. If none of them support an answer, reply \
                     `I don't know based on the supplied context.`",
                    invalid_cited
                        .iter()
                        .map(|id| format!("#{}", id.0))
                        .collect::<Vec<_>>()
                        .join(", "),
                    allowed.join(", "),
                );
                let mut retry_req = llm_req.clone();
                retry_req.messages.push(Message::assistant(content.clone()));
                retry_req.messages.push(Message::user(reminder));
                match self.llm.generate(&retry_req).await {
                    Ok(retry) => {
                        let retry_parsed = parse_cited_answer(&retry.content);
                        let (retry_valid, retry_invalid): (Vec<_>, Vec<_>) = retry_parsed
                            .cited
                            .iter()
                            .copied()
                            .partition(|id| valid_ids.contains(id));
                        if retry_invalid.is_empty() || retry_valid.len() > cited_valid.len() {
                            (
                                retry.content.clone(),
                                retry.model,
                                retry.stop_reason,
                                retry.usage,
                                retry_valid,
                                retry_parsed.body,
                            )
                        } else {
                            (content, model, stop_reason, usage, cited_valid, parsed.body)
                        }
                    }
                    Err(_) => (content, model, stop_reason, usage, cited_valid, parsed.body),
                }
            } else {
                (content, model, stop_reason, usage, cited_valid, parsed.body)
            };

        Ok(RagAnswer {
            query: req.query,
            answer: final_content,
            retrieved: scored,
            subgraph,
            model: final_model,
            stop_reason: final_stop,
            usage: final_usage,
            cited,
            answer_body: body,
        })
    }

    /// Streaming variant of [`answer_with`]. Emits one `Retrieved` frame
    /// with the grounding subgraph, then text deltas as the model produces
    /// them, then a final `End` frame with usage totals.
    pub async fn answer_stream(&self, req: RetrievalRequest) -> Result<RagStream, LlmError> {
        let (scored, mut subgraph) = self.index.retrieve(&req);
        enrich_with_transitive_closures(self.index.graph(), &mut subgraph, 16);
        debug!(
            seeds = scored.len(),
            context_concepts = subgraph.concepts.len(),
            "retrieved (stream)",
        );

        let onto = self.index.graph().ontology();
        let builder = PromptBuilder::new(&onto).with_max_chars(self.max_context_chars);
        let cached_ontology = builder.render_static_context();
        let query_context = builder.render_query_context(&scored, &subgraph);

        let user_message = format!(
            "Use the context below to answer the question.\n\n\
             ---RETRIEVED---\n{ctx}\n---END RETRIEVED---\n\n\
             Question: {q}",
            ctx = query_context,
            q = req.query,
        );

        let llm_req = LlmRequest {
            system: Some(PromptBuilder::system_message().to_string()),
            cached_context: Some(cached_ontology),
            messages: vec![Message::user(user_message)],
            max_tokens: self.max_tokens,
            temperature: self.temperature,
        };

        let inner = self.llm.generate_stream(&llm_req).await?;

        let retrieved = futures::stream::iter(vec![Ok(RagStreamEvent::Retrieved {
            query: req.query,
            scored,
            subgraph,
        })]);

        let mapped = inner.filter_map(|r| async move {
            match r {
                Ok(StreamChunk::Text(text)) => Some(Ok(RagStreamEvent::Token { text })),
                Ok(StreamChunk::KeepAlive) => None,
                Ok(StreamChunk::End {
                    usage,
                    stop_reason,
                    model,
                }) => Some(Ok(RagStreamEvent::End {
                    usage,
                    stop_reason,
                    model,
                })),
                Err(e) => Some(Err(e)),
            }
        });

        Ok(retrieved.chain(mapped).boxed())
    }

    /// One-shot natural-language → [`ontology_graph::Ontology`] generator.
    ///
    /// Routes a brief through the configured LLM with a strict-JSON
    /// instruction prompt, then deserializes the response. The function is
    /// tolerant of common LLM tics: a leading "```json" fence, trailing
    /// commentary after the closing brace, or a stray BOM. Anything that
    /// still fails to parse surfaces as [`OntologyGenError::Parse`] with the
    /// raw text so the caller can show it to the user.
    pub async fn generate_ontology(
        &self,
        description: &str,
    ) -> Result<ontology_graph::Ontology, OntologyGenError> {
        let llm_req = LlmRequest {
            system: Some(PromptBuilder::ontology_generation_system_message().to_string()),
            cached_context: None,
            messages: vec![Message::user(PromptBuilder::ontology_generation_user_message(
                description,
            ))],
            max_tokens: self.max_tokens.max(2048),
            temperature: 0.0,
        };
        let resp = self.llm.generate(&llm_req).await.map_err(OntologyGenError::Llm)?;
        let json = extract_json_block(&resp.content)
            .ok_or_else(|| OntologyGenError::Parse {
                raw: resp.content.clone(),
                error: "no JSON object found in response".into(),
            })?;
        serde_json::from_str::<ontology_graph::Ontology>(&json).map_err(|e| {
            OntologyGenError::Parse {
                raw: resp.content,
                error: e.to_string(),
            }
        })
    }

    /// One-shot natural-language → [`GeneratedRule`] generator.
    ///
    /// The caller supplies the rule type and the concept names this rule
    /// scopes to; the LLM fills in `name`, `when`, `then`, `description`
    /// and `strict`. Tolerant of the same LLM tics handled by
    /// [`Pipeline::generate_ontology`].
    pub async fn generate_rule(
        &self,
        description: &str,
        rule_type: &str,
        concept_names: &[String],
    ) -> Result<GeneratedRule, OntologyGenError> {
        let llm_req = LlmRequest {
            system: Some(PromptBuilder::rule_generation_system_message().to_string()),
            cached_context: None,
            messages: vec![Message::user(PromptBuilder::rule_generation_user_message(
                description,
                rule_type,
                concept_names,
            ))],
            max_tokens: self.max_tokens.max(1024),
            temperature: 0.0,
        };
        let resp = self.llm.generate(&llm_req).await.map_err(OntologyGenError::Llm)?;
        let json = extract_json_block(&resp.content)
            .ok_or_else(|| OntologyGenError::Parse {
                raw: resp.content.clone(),
                error: "no JSON object found in response".into(),
            })?;
        serde_json::from_str::<GeneratedRule>(&json).map_err(|e| OntologyGenError::Parse {
            raw: resp.content,
            error: e.to_string(),
        })
    }
}

/// Subset of [`ontology_graph::Rule`] fields produced by
/// [`Pipeline::generate_rule`]. The caller fills in `id`, `rule_type`,
/// `applies_to` and `properties`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedRule {
    pub name: String,
    #[serde(default)]
    pub when: String,
    #[serde(default)]
    pub then: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub strict: bool,
}

/// Strip BOM / leading markdown fence and trim to the outermost balanced
/// `{ … }` block. Returns `None` if no `{` is present.
fn extract_json_block(text: &str) -> Option<String> {
    let s = text.trim_start_matches('\u{feff}').trim();
    // Strip ``` or ```json fences if present.
    let s = if let Some(rest) = s.strip_prefix("```") {
        let rest = rest.trim_start_matches("json").trim_start_matches('\n');
        rest.trim_end_matches("```").trim()
    } else {
        s
    };
    let start = s.find('{')?;
    // Find matching closing brace, accounting for string literals.
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escape = false;
    let mut end = None;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(i + 1);
                    break;
                }
            }
            _ => {}
        }
    }
    end.map(|e| s[start..e].to_string())
}

/// Parsed shape of an LLM answer that follows the `Cited:` / `Answer:`
/// contract from [`PromptBuilder::system_message`].
#[derive(Debug, Clone, Default)]
pub(crate) struct ParsedAnswer {
    pub cited: Vec<ConceptId>,
    /// Prose with the `Cited:` line removed and a leading `Answer:` prefix
    /// stripped. Falls back to the raw text trimmed when no `Cited:` line
    /// is present.
    pub body: String,
}

/// Extract `#<id>` tokens from the first `Cited:` line found in `raw`, and
/// return the remaining prose. Tolerant of: `Cited: [#1, #2]`, `Cited: #1 #2`,
/// `cited: 1, 2`, surrounding whitespace, and a leading `Answer:` on the
/// remaining prose.
pub(crate) fn parse_cited_answer(raw: &str) -> ParsedAnswer {
    let trimmed = raw.trim_start_matches('\u{feff}').trim();
    let mut cited_line: Option<&str> = None;
    let mut cited_idx: Option<usize> = None;
    for (i, line) in trimmed.lines().enumerate().take(5) {
        let l = line.trim_start();
        if l.len() >= 6 && l[..6].eq_ignore_ascii_case("cited:") {
            cited_line = Some(&l[6..]);
            cited_idx = Some(i);
            break;
        }
    }
    let Some(cited_raw) = cited_line else {
        return ParsedAnswer {
            cited: Vec::new(),
            body: trimmed.to_string(),
        };
    };

    let mut ids = Vec::new();
    let mut num = String::new();
    let flush = |num: &mut String, ids: &mut Vec<ConceptId>| {
        if !num.is_empty() {
            if let Ok(n) = num.parse::<u64>() {
                ids.push(ConceptId(n));
            }
            num.clear();
        }
    };
    for ch in cited_raw.chars() {
        if ch.is_ascii_digit() {
            num.push(ch);
        } else {
            flush(&mut num, &mut ids);
        }
    }
    flush(&mut num, &mut ids);
    ids.sort_unstable();
    ids.dedup();

    // Body = everything except the cited line.
    let idx = cited_idx.unwrap();
    let body: String = trimmed
        .lines()
        .enumerate()
        .filter(|(i, _)| *i != idx)
        .map(|(_, l)| l)
        .collect::<Vec<_>>()
        .join("\n");
    let body = body.trim();
    let body = body
        .strip_prefix("Answer:")
        .or_else(|| body.strip_prefix("answer:"))
        .map(|s| s.trim_start())
        .unwrap_or(body)
        .to_string();

    ParsedAnswer { cited: ids, body }
}

/// Expand the retrieved subgraph along ontology-declared transitive relations
/// (e.g. `partOf`, `locatedIn`). Capped at `max_extra` new concepts. Bypassed
/// when `ONTOLOGY_DISABLE_TRANSITIVE_CLOSURE=1`.
fn enrich_with_transitive_closures(
    graph: &OntologyGraph,
    subgraph: &mut Subgraph,
    max_extra: usize,
) {
    if std::env::var("ONTOLOGY_DISABLE_TRANSITIVE_CLOSURE").as_deref() == Ok("1") {
        debug!("transitive closure disabled via env");
        return;
    }
    let onto = graph.ontology();
    let transitive: Vec<String> = onto
        .relation_types
        .values()
        .filter(|rt| rt.transitive)
        .map(|rt| rt.name.clone())
        .collect();
    if transitive.is_empty() {
        return;
    }
    let seeds: Vec<ConceptId> = subgraph.seeds.clone();
    let mut existing: std::collections::HashSet<ConceptId> =
        subgraph.concepts.iter().map(|c| c.id).collect();
    let mut added = 0usize;
    for seed in seeds {
        for t in &transitive {
            if added >= max_extra {
                return;
            }
            let extras = graph.closure(seed, t, 3).unwrap_or_default();
            for id in extras {
                if !existing.insert(id) {
                    continue;
                }
                if let Ok(c) = graph.get_concept(id) {
                    let depth = subgraph.depth_of.get(&seed).copied().unwrap_or(0) + 1;
                    subgraph.depth_of.entry(id).or_insert(depth);
                    subgraph.concepts.push(c);
                    added += 1;
                    if added >= max_extra {
                        return;
                    }
                }
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OntologyGenError {
    #[error("llm: {0}")]
    Llm(crate::model::LlmError),
    #[error("parse: {error}")]
    Parse { raw: String, error: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EchoModel;
    use ontology_graph::{Concept, ConceptType, Ontology, OntologyGraph, Relation, RelationType};

    fn ont() -> Ontology {
        let mut o = Ontology::new();
        o.add_concept_type(ConceptType {
            name: "Topic".into(),
            parent: None,
            properties: None,
            description: "subject of study".into(),
            ..Default::default()
        });
        o.add_relation_type(RelationType {
            name: "related_to".into(),
            domain: "Topic".into(),
            range: "Topic".into(),
            cardinality: Default::default(),
            symmetric: true,
            description: "".into(),
            ..Default::default()
        })
        .unwrap();
        o
    }

    #[test]
    fn parse_cited_bracketed() {
        let p = parse_cited_answer("Cited: [#1, #42]\nAnswer: hello world");
        assert_eq!(p.cited, vec![ConceptId(1), ConceptId(42)]);
        assert_eq!(p.body, "hello world");
    }

    #[test]
    fn parse_cited_loose() {
        let p = parse_cited_answer("cited: 7 9 9\nthe answer is X");
        assert_eq!(p.cited, vec![ConceptId(7), ConceptId(9)]);
        assert_eq!(p.body, "the answer is X");
    }

    #[test]
    fn parse_cited_missing() {
        let p = parse_cited_answer("I don't know based on the supplied context.");
        assert!(p.cited.is_empty());
        assert_eq!(p.body, "I don't know based on the supplied context.");
    }

    #[tokio::test]
    async fn pipeline_runs_end_to_end() {
        let g = OntologyGraph::with_arc(ont());
        let a = g
            .upsert_concept(
                Concept::new(Default::default(), "Topic", "Vector Search")
                    .with_description("approximate nearest-neighbor retrieval over embeddings"),
            )
            .unwrap();
        let b = g
            .upsert_concept(
                Concept::new(Default::default(), "Topic", "RAG")
                    .with_description("retrieval augmented generation grounds LLMs"),
            )
            .unwrap();
        g.add_relation(Relation::new(Default::default(), "related_to", a, b))
            .unwrap();

        let idx = Arc::new(HybridIndex::with_default_embedder(g.clone()));
        idx.reindex_all();

        let pipe = RagPipeline::new(idx, Arc::new(EchoModel));
        let ans = pipe.answer("explain RAG and vector search").await.unwrap();
        assert!(!ans.retrieved.is_empty());
        assert!(ans.answer.starts_with("[echo]"));
        assert!(!ans.subgraph.concepts.is_empty());
    }
}
