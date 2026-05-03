// SPDX-License-Identifier: LGPL-3.0-or-later
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// It is licensed under the GNU Lesser General Public License v3.0
// or (at your option) any later version. See LICENSE and LICENSE.GPL.

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

pub mod model;
pub mod pipeline;
pub mod prompt;

pub use model::{
    AnthropicModel, EchoModel, LanguageModel, LlmError, LlmRequest, LlmResponse, LlmStream,
    Message, Role, StreamChunk, TokenUsage,
};
pub use pipeline::{RagAnswer, RagPipeline, RagStream, RagStreamEvent};
pub use prompt::PromptBuilder;
