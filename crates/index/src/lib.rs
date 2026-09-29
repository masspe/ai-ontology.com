// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A.. See LICENSE and LICENSE-COMMERCIAL.md.

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
pub use hybrid::{HybridIndex, ReindexReport, RetrievalRequest, ScoredConcept};
pub use lexical::LexicalIndex;
pub use vector::VectorIndex;
