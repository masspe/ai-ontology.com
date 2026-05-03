use futures::stream::{BoxStream, StreamExt};
use ontology_graph::{ConceptId, Subgraph};
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RagStreamEvent {
    Retrieved {
        query: String,
        scored: Vec<ScoredConcept>,
        subgraph: Subgraph,
    },
    Token(String),
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
        let (scored, subgraph) = self.index.retrieve(&req);
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

        let LlmResponse {
            content,
            model,
            stop_reason,
            usage,
        } = self.llm.generate(&llm_req).await?;

        Ok(RagAnswer {
            query: req.query,
            answer: content,
            retrieved: scored,
            subgraph,
            model,
            stop_reason,
            usage,
        })
    }

    /// Streaming variant of [`answer_with`]. Emits one `Retrieved` frame
    /// with the grounding subgraph, then text deltas as the model produces
    /// them, then a final `End` frame with usage totals.
    pub async fn answer_stream(&self, req: RetrievalRequest) -> Result<RagStream, LlmError> {
        let (scored, subgraph) = self.index.retrieve(&req);
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
                Ok(StreamChunk::Text(t)) => Some(Ok(RagStreamEvent::Token(t))),
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
        });
        o.add_relation_type(RelationType {
            name: "related_to".into(),
            domain: "Topic".into(),
            range: "Topic".into(),
            cardinality: Default::default(),
            symmetric: true,
            description: "".into(),
        })
        .unwrap();
        o
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
