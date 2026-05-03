// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

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
