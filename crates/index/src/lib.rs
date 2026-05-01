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
pub mod lexical;
pub mod vector;
pub mod hybrid;

pub use embed::{Embedder, HashEmbedder};
pub use lexical::LexicalIndex;
pub use vector::VectorIndex;
pub use hybrid::{HybridIndex, RetrievalRequest, ScoredConcept};
