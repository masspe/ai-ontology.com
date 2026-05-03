// SPDX-License-Identifier: LGPL-3.0-or-later
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// It is licensed under the GNU Lesser General Public License v3.0
// or (at your option) any later version. See LICENSE and LICENSE.GPL.

//! Indexing & retrieval over an [`OntologyGraph`].
//!
//! Three index types are layered together by [`HybridIndex`]:
//!
//! * **Lexical** — TF-IDF over the concatenated text of each concept.
//! * **Vector** — cosine similarity over hashed-bag-of-words embeddings,
//!   pluggable via the [`Embedder`] trait so users can swap in real models.
//! * **Graph** — subgraph expansion from seed concepts (provided by the
//!   `ontology-graph` crate; we just orchestrate it here).
//!
//! Retrieval is a two-stage process: rank concepts → expand the top-k into
//! a bounded subgraph that's ready to ship to a language model.

pub mod embed;
pub mod hybrid;
pub mod lexical;
pub mod vector;

pub use embed::{Embedder, HashEmbedder};
pub use hybrid::{HybridIndex, RetrievalRequest, ScoredConcept};
pub use lexical::LexicalIndex;
pub use vector::VectorIndex;
