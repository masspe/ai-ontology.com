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
        let (final_content, final_model, final_stop, final_usage, cited, body) = if !invalid_cited
            .is_empty()
        {
            let allowed: Vec<String> = valid_ids.iter().map(|id| format!("#{}", id.0)).collect();
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
            messages: vec![Message::user(
                PromptBuilder::ontology_generation_user_message(description),
            )],
            max_tokens: self.max_tokens.max(2048),
            temperature: 0.0,
        };
        let resp = self
            .llm
            .generate(&llm_req)
            .await
            .map_err(OntologyGenError::Llm)?;
        let json = extract_json_block(&resp.content).ok_or_else(|| OntologyGenError::Parse {
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
        let resp = self
            .llm
            .generate(&llm_req)
            .await
            .map_err(OntologyGenError::Llm)?;
        let json = extract_json_block(&resp.content).ok_or_else(|| OntologyGenError::Parse {
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
        o.add_relation_type(RelationType {
            name: "leads_to".into(),
            domain: "Topic".into(),
            range: "Topic".into(),
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

    // -- hardening: citation parsing -----------------------------------

    /// `Cited: []`, an upper-case label, a leading BOM and an `Answer:`
    /// prefix on the body are all handled; the body keeps its own lines.
    #[test]
    fn parse_cited_handles_empty_brackets_uppercase_bom_and_answer_prefix() {
        let p =
            parse_cited_answer("Cited: []\nAnswer: I don't know based on the supplied context.");
        assert!(p.cited.is_empty());
        assert_eq!(p.body, "I don't know based on the supplied context.");

        let p = parse_cited_answer("\u{feff}  CITED: [#5]\nanswer: Yes.\nSecond line.");
        assert_eq!(p.cited, vec![ConceptId(5)]);
        assert_eq!(p.body, "Yes.\nSecond line.");

        // A cited line that is not the first line, with prose before it:
        // the `Answer:` label is only stripped when it opens the body.
        let p = parse_cited_answer("Sure!\nCited: [#2]\nAnswer: ok");
        assert_eq!(p.cited, vec![ConceptId(2)]);
        assert_eq!(p.body, "Sure!\nAnswer: ok");
    }

    /// Pins the contract: only the first five lines are scanned for the
    /// `Cited:` label, and digits in the prose are never taken as ids.
    #[test]
    fn parse_cited_only_scans_the_first_five_lines_and_ignores_body_digits() {
        let late = "a\nb\nc\nd\ne\nCited: [#1]\nAnswer: x";
        let p = parse_cited_answer(late);
        assert!(p.cited.is_empty());
        assert_eq!(p.body, late);

        let p = parse_cited_answer("Cited: [#3]\nAnswer: Built in 1999 by #42.");
        assert_eq!(p.cited, vec![ConceptId(3)]);
        assert_eq!(p.body, "Built in 1999 by #42.");
    }

    /// Fences, commentary around the object, braces inside string literals
    /// and escaped quotes do not confuse the block extractor.
    #[test]
    fn extract_json_block_handles_fences_commentary_and_braces_in_strings() {
        let raw = "Sure, here it is:\n{\"a\": \"}{\\\"\", \"b\": {\"c\": 1}}\nHope this helps.";
        assert_eq!(
            extract_json_block(raw).as_deref(),
            Some("{\"a\": \"}{\\\"\", \"b\": {\"c\": 1}}")
        );
        assert_eq!(
            extract_json_block("```json\n{\"a\":1}\n```").as_deref(),
            Some("{\"a\":1}")
        );
        assert_eq!(
            extract_json_block("\u{feff}```\n{\"a\":1}\n```\ntrailing").as_deref(),
            Some("{\"a\":1}")
        );
        assert!(extract_json_block("no object here").is_none());
        assert!(
            extract_json_block("{\"a\": {\"b\": 1}").is_none(),
            "unbalanced"
        );
        assert!(extract_json_block("{\"a\": \"unterminated}").is_none());
    }

    // -- hardening: pipeline behaviour ---------------------------------

    /// Records every request and answers from a script, one entry per call
    /// (the last entry repeats). `{first_allowed}` in a scripted answer is
    /// replaced by the last `#<id>` found in the last user message — a
    /// subgraph id on the first call, an allowed id on a retry.
    struct ScriptedModel {
        script: Vec<&'static str>,
        requests: std::sync::Mutex<Vec<LlmRequest>>,
    }
    impl ScriptedModel {
        fn new(script: &[&'static str]) -> Arc<Self> {
            Arc::new(Self {
                script: script.to_vec(),
                requests: Default::default(),
            })
        }
        fn calls(&self) -> usize {
            self.requests.lock().unwrap().len()
        }
        fn request(&self, i: usize) -> LlmRequest {
            self.requests.lock().unwrap()[i].clone()
        }
    }
    #[async_trait::async_trait]
    impl LanguageModel for ScriptedModel {
        async fn generate(&self, req: &LlmRequest) -> Result<LlmResponse, LlmError> {
            let mut reqs = self.requests.lock().unwrap();
            let idx = reqs.len().min(self.script.len() - 1);
            reqs.push(req.clone());
            let last_user = req
                .messages
                .iter()
                .rev()
                .find(|m| m.role == crate::model::Role::User)
                .map(|m| m.content.as_str())
                .unwrap_or("");
            let first_id = last_user
                .split('#')
                .skip(1)
                .map(|s| {
                    s.chars()
                        .take_while(char::is_ascii_digit)
                        .collect::<String>()
                })
                .filter(|d| !d.is_empty())
                .last()
                .unwrap_or_default();
            Ok(LlmResponse {
                content: self.script[idx].replace("{first_allowed}", &first_id),
                model: "scripted".into(),
                stop_reason: Some("end_turn".into()),
                usage: TokenUsage::default(),
            })
        }
    }

    /// Three chained topics so retrieval depth and top-k are observable.
    fn chained_graph() -> Arc<OntologyGraph> {
        let g = OntologyGraph::with_arc(ont());
        let a = g
            .upsert_concept(
                Concept::new(Default::default(), "Topic", "Alpha")
                    .with_description("alpha is the first topic"),
            )
            .unwrap();
        let b = g
            .upsert_concept(
                Concept::new(Default::default(), "Topic", "Beta")
                    .with_description("beta is the second topic"),
            )
            .unwrap();
        let c = g
            .upsert_concept(
                Concept::new(Default::default(), "Topic", "Gamma")
                    .with_description("gamma is the third topic"),
            )
            .unwrap();
        g.add_relation(Relation::new(Default::default(), "leads_to", a, b))
            .unwrap();
        g.add_relation(Relation::new(Default::default(), "leads_to", b, c))
            .unwrap();
        g
    }

    fn pipeline_over(g: Arc<OntologyGraph>, llm: Arc<dyn LanguageModel>) -> RagPipeline {
        let idx = Arc::new(HybridIndex::with_default_embedder(g));
        idx.reindex_all();
        RagPipeline::new(idx, llm)
    }

    /// An empty knowledge base still answers: nothing retrieved, no
    /// citations, and the body is the raw answer when no `Cited:` line exists.
    #[tokio::test]
    async fn empty_graph_answers_without_citations() {
        let g = OntologyGraph::with_arc(ont());
        let pipe = pipeline_over(g, Arc::new(EchoModel));
        let ans = pipe.answer("anything?").await.unwrap();
        assert!(ans.retrieved.is_empty());
        assert!(ans.subgraph.concepts.is_empty());
        assert!(ans.cited.is_empty());
        assert!(ans.answer.starts_with("[echo] Use the context below"));
        assert!(ans.answer.ends_with("Question: anything?"));
        assert_eq!(ans.answer_body, ans.answer);
        assert_eq!(ans.model, "echo");
    }

    /// `top_k` bounds the seeds and `expansion.max_depth` bounds the
    /// subgraph, both flowing from the `RetrievalRequest` unchanged.
    #[tokio::test]
    async fn retrieval_request_top_k_and_depth_are_honored() {
        let g = chained_graph();
        let pipe = pipeline_over(g.clone(), Arc::new(EchoModel));
        let alpha = g.find_by_name("Topic", "Alpha").unwrap();

        let mut req = RetrievalRequest {
            query: "alpha first topic".into(),
            top_k: 1,
            ..Default::default()
        };
        req.expansion.max_depth = 0;
        let ans = pipe.answer_with(req.clone()).await.unwrap();
        assert_eq!(ans.retrieved.len(), 1);
        assert_eq!(ans.retrieved[0].id, alpha);
        assert_eq!(ans.subgraph.concepts.len(), 1, "depth 0 = seeds only");
        assert!(ans.subgraph.relations.is_empty());
        assert_eq!(ans.query, "alpha first topic");

        req.expansion.max_depth = 2;
        let ans = pipe.answer_with(req).await.unwrap();
        assert_eq!(ans.retrieved.len(), 1);
        assert_eq!(ans.subgraph.concepts.len(), 3, "two hops reach Gamma");
        assert_eq!(ans.subgraph.relations.len(), 2);
    }

    /// The prompt carries the cached ontology as `cached_context`, the
    /// pipeline's sampling settings, and one user turn with the question.
    #[tokio::test]
    async fn answer_request_carries_cached_ontology_and_pipeline_settings() {
        let model = ScriptedModel::new(&["Cited: []\nAnswer: none"]);
        let mut pipe = pipeline_over(chained_graph(), model.clone());
        pipe.max_tokens = 55;
        pipe.temperature = 0.25;
        pipe.max_context_chars = 4000;
        pipe.answer("beta").await.unwrap();
        let req = model.request(0);
        assert_eq!(req.max_tokens, 55);
        assert_eq!(req.temperature, 0.25);
        assert_eq!(req.system.as_deref(), Some(PromptBuilder::system_message()));
        let cached = req
            .cached_context
            .expect("ontology is sent as cached context");
        assert!(cached.starts_with("# Ontology\n"));
        assert!(cached.contains("- Topic :: subject of study"));
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.messages[0].role, crate::model::Role::User);
        assert!(req.messages[0].content.contains("---RETRIEVED---"));
        assert!(req.messages[0].content.ends_with("Question: beta"));
    }

    /// Citing an id outside the subgraph triggers exactly one retry whose
    /// reminder names the bad id and the allowed set; a clean retry wins.
    #[tokio::test]
    async fn invalid_citations_trigger_one_retry_with_the_allowed_ids() {
        let model = ScriptedModel::new(&[
            "Cited: [#999]\nAnswer: made up",
            "Cited: [#{first_allowed}]\nAnswer: grounded",
        ]);
        let pipe = pipeline_over(chained_graph(), model.clone());
        let ans = pipe.answer("alpha").await.unwrap();
        assert_eq!(model.calls(), 2);
        assert_eq!(ans.answer_body, "grounded");
        assert_eq!(ans.cited.len(), 1);
        assert!(ans.subgraph.concepts.iter().any(|c| c.id == ans.cited[0]));

        let retry = model.request(1);
        assert_eq!(retry.messages.len(), 3, "original, first answer, reminder");
        assert_eq!(retry.messages[1].role, crate::model::Role::Assistant);
        assert_eq!(retry.messages[1].content, "Cited: [#999]\nAnswer: made up");
        let reminder = &retry.messages[2].content;
        assert!(reminder.contains("#999"), "{reminder}");
        for c in &ans.subgraph.concepts {
            assert!(reminder.contains(&format!("#{}", c.id.0)), "{reminder}");
        }
    }

    /// When the retry is no better the first answer is kept, with the
    /// invented ids filtered out of `cited`.
    #[tokio::test]
    async fn retry_keeps_the_first_answer_when_the_retry_is_no_better() {
        let model = ScriptedModel::new(&["Cited: [#999]\nAnswer: made up"]);
        let pipe = pipeline_over(chained_graph(), model.clone());
        let ans = pipe.answer("alpha").await.unwrap();
        assert_eq!(model.calls(), 2, "exactly one retry, never more");
        assert!(ans.cited.is_empty());
        assert_eq!(ans.answer, "Cited: [#999]\nAnswer: made up");
        assert_eq!(ans.answer_body, "made up");
    }

    /// Valid citations are kept in ascending order and no retry happens.
    #[tokio::test]
    async fn valid_citations_need_no_retry() {
        let model =
            ScriptedModel::new(&["Cited: [#{first_allowed}, #{first_allowed}]\nAnswer: ok"]);
        let pipe = pipeline_over(chained_graph(), model.clone());
        let ans = pipe.answer("gamma").await.unwrap();
        assert_eq!(model.calls(), 1);
        assert_eq!(ans.cited.len(), 1, "duplicates collapse");
        assert_eq!(ans.answer_body, "ok");
    }

    /// Transitive relation types widen the subgraph along their closure
    /// even when the expansion depth is zero, with depths marked seed + 1.
    #[tokio::test]
    async fn transitive_relations_enrich_the_subgraph_beyond_expansion_depth() {
        let mut o = Ontology::new();
        o.add_concept_type(ConceptType {
            name: "Company".into(),
            ..Default::default()
        });
        o.add_relation_type(RelationType {
            name: "parent_company".into(),
            domain: "Company".into(),
            range: "Company".into(),
            transitive: true,
            ..Default::default()
        })
        .unwrap();
        let g = OntologyGraph::with_arc(o);
        let a = g
            .upsert_concept(Concept::new(Default::default(), "Company", "Acme Robotics"))
            .unwrap();
        let b = g
            .upsert_concept(Concept::new(Default::default(), "Company", "Acme Holding"))
            .unwrap();
        let c = g
            .upsert_concept(Concept::new(
                Default::default(),
                "Company",
                "Umbrella Group",
            ))
            .unwrap();
        g.add_relation(Relation::new(Default::default(), "parent_company", a, b))
            .unwrap();
        g.add_relation(Relation::new(Default::default(), "parent_company", b, c))
            .unwrap();
        let pipe = pipeline_over(g, Arc::new(EchoModel));
        let mut req = RetrievalRequest {
            query: "Acme Robotics".into(),
            top_k: 1,
            ..Default::default()
        };
        req.expansion.max_depth = 0;
        let ans = pipe.answer_with(req).await.unwrap();
        assert_eq!(ans.retrieved[0].id, a);
        let ids: std::collections::HashSet<ConceptId> =
            ans.subgraph.concepts.iter().map(|c| c.id).collect();
        assert!(ids.contains(&b) && ids.contains(&c), "{ids:?}");
        assert_eq!(ans.subgraph.depth_of.get(&c), Some(&1));
    }

    // -- hardening: generation helpers ---------------------------------

    /// No `{` in the answer and a JSON object that is not an `Ontology`
    /// are both `Parse` errors carrying the raw answer for display.
    #[tokio::test]
    async fn generate_ontology_maps_non_json_and_bad_schema_to_parse_errors() {
        let pipe = pipeline_over(chained_graph(), Arc::new(EchoModel));
        match pipe.generate_ontology("a brief").await {
            Err(OntologyGenError::Parse { raw, error }) => {
                assert!(raw.starts_with("[echo] Brief:\na brief"), "{raw}");
                assert_eq!(error, "no JSON object found in response");
            }
            other => panic!("expected Parse, got {other:?}"),
        }
        let model = ScriptedModel::new(&[r#"{"concept_types": []}"#]);
        let pipe = pipeline_over(chained_graph(), model);
        match pipe.generate_ontology("a brief").await {
            Err(OntologyGenError::Parse { raw, error }) => {
                assert_eq!(raw, r#"{"concept_types": []}"#);
                assert_ne!(error, "no JSON object found in response");
            }
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    /// A fenced ontology with commentary parses; the request uses the
    /// strict-JSON system prompt, at least 2 048 tokens and temperature 0.
    #[tokio::test]
    async fn generate_ontology_accepts_fenced_json_and_pins_request_shape() {
        let model = ScriptedModel::new(&["Here you go:\n```json\n{\n  \"concept_types\": {\"Person\": {\"name\": \"Person\", \"parent\": null, \"description\": \"a human\", \"properties\": null}},\n  \"relation_types\": {\"knows\": {\"name\": \"knows\", \"domain\": \"Person\", \"range\": \"Person\", \"cardinality\": \"ManyToMany\", \"symmetric\": true, \"description\": \"\"}},\n  \"rule_types\": {},\n  \"action_types\": {}\n}\n```\nLet me know if you want changes."]);
        let mut pipe = pipeline_over(chained_graph(), model.clone());
        pipe.max_tokens = 100;
        pipe.temperature = 0.9;
        let onto = pipe
            .generate_ontology("  people who know each other  ")
            .await
            .unwrap();
        assert!(onto.concept_types.contains_key("Person"));
        assert!(onto.relation_types["knows"].symmetric);
        let req = model.request(0);
        assert_eq!(req.max_tokens, 2048, "raised to the generation floor");
        assert_eq!(req.temperature, 0.0);
        assert!(req.cached_context.is_none());
        assert_eq!(
            req.system.as_deref(),
            Some(PromptBuilder::ontology_generation_system_message())
        );
        assert_eq!(
            req.messages[0].content,
            "Brief:\npeople who know each other\n\nReturn the JSON ontology document now."
        );
    }

    /// `generate_rule` fills defaults for optional fields, requires `name`,
    /// and floors `max_tokens` at 1 024.
    #[tokio::test]
    async fn generate_rule_parses_the_documented_shape_and_requires_name() {
        let model = ScriptedModel::new(&[r#"{"name": "InvoiceNeedsContract", "strict": true}"#]);
        let mut pipe = pipeline_over(chained_graph(), model.clone());
        pipe.max_tokens = 10;
        let rule = pipe
            .generate_rule(
                "every invoice bills a contract",
                "constraint",
                &["Invoice".into()],
            )
            .await
            .unwrap();
        assert_eq!(rule.name, "InvoiceNeedsContract");
        assert!(rule.strict);
        assert_eq!((rule.when.as_str(), rule.then.as_str()), ("", ""));
        let req = model.request(0);
        assert_eq!(req.max_tokens, 1024);
        assert_eq!(
            req.messages[0].content,
            "Rule type: constraint\nApplies to concepts: Invoice\n\nPrompt:\nevery invoice bills a contract\n\nReturn the JSON rule object now."
        );

        let model = ScriptedModel::new(&[r#"{"when": "x", "then": "y"}"#]);
        let pipe = pipeline_over(chained_graph(), model);
        match pipe.generate_rule("p", "t", &[]).await {
            Err(OntologyGenError::Parse { error, .. }) => {
                assert!(error.contains("name"), "{error}")
            }
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    // -- hardening: streaming ------------------------------------------

    /// The stream is exactly `Retrieved`, then tokens, then one `End`.
    #[tokio::test]
    async fn answer_stream_emits_retrieved_then_tokens_then_end() {
        let pipe = pipeline_over(chained_graph(), Arc::new(EchoModel));
        let stream = pipe
            .answer_stream(RetrievalRequest {
                query: "alpha".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let events: Vec<RagStreamEvent> = stream.map(|e| e.unwrap()).collect().await;
        assert!(events.len() >= 3, "{events:?}");
        match &events[0] {
            RagStreamEvent::Retrieved {
                query,
                scored,
                subgraph,
            } => {
                assert_eq!(query, "alpha");
                assert!(!scored.is_empty());
                assert!(!subgraph.concepts.is_empty());
            }
            other => panic!("first frame must be Retrieved, got {other:?}"),
        }
        let middle = &events[1..events.len() - 1];
        assert!(middle
            .iter()
            .all(|e| matches!(e, RagStreamEvent::Token { .. })));
        let text: String = middle
            .iter()
            .map(|e| match e {
                RagStreamEvent::Token { text } => text.as_str(),
                _ => "",
            })
            .collect();
        assert!(text.starts_with("[echo] "));
        match events.last().unwrap() {
            RagStreamEvent::End {
                model, stop_reason, ..
            } => {
                assert_eq!(model, "echo");
                assert_eq!(stop_reason.as_deref(), Some("end_turn"));
            }
            other => panic!("last frame must be End, got {other:?}"),
        }
    }

    /// The wire shape the web client depends on: a `type` tag and a `text`
    /// key for tokens, snake_case tags everywhere.
    #[test]
    fn rag_stream_event_serializes_with_snake_case_tags() {
        let v = serde_json::to_value(RagStreamEvent::Token { text: "hi".into() }).unwrap();
        assert_eq!(v, serde_json::json!({"type": "token", "text": "hi"}));
        let v = serde_json::to_value(RagStreamEvent::End {
            usage: TokenUsage::default(),
            model: "m".into(),
            stop_reason: None,
        })
        .unwrap();
        assert_eq!(v["type"], "end");
        assert_eq!(v["model"], "m");
        let back: RagStreamEvent = serde_json::from_str(r#"{"type":"end"}"#).unwrap();
        assert!(matches!(back, RagStreamEvent::End { model, .. } if model.is_empty()));
    }
}
