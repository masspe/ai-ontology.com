//! RAG pipeline: query -> retrieve -> render context -> call LLM -> answer.
//!
//! The pipeline is split into composable stages so callers can swap any of
//! them. The default implementation:
//!
//! 1. Routes the user query to [`HybridIndex::retrieve`].
//! 2. Renders the resulting [`Subgraph`] into a deterministic, ontology-aware
//!    prompt fragment via [`PromptBuilder`].
//! 3. Sends the assembled messages to a [`LanguageModel`] implementation.
//!
//! Two LLM clients are bundled: an in-memory [`EchoModel`] for tests, and an
//! HTTP [`AnthropicModel`] for production use against the Claude Messages API.

pub mod prompt;
pub mod model;
pub mod pipeline;

pub use model::{LanguageModel, LlmRequest, LlmResponse, Message, Role, EchoModel, AnthropicModel};
pub use pipeline::{RagPipeline, RagAnswer};
pub use prompt::PromptBuilder;
