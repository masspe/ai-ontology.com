//! Modular data ingest / egress for the ontology graph.
//!
//! This crate exposes two trait-driven seams:
//!
//! * [`Source`] — produces a stream of [`Record`]s to import into the graph.
//! * [`Sink`]   — consumes records produced by export traversals.
//!
//! Built-in adapters: JSONL files, in-memory iterators, and a minimal
//! "subject / predicate / object" triple format. Users can plug in custom
//! sources (e.g. Kafka, S3) by implementing the trait.

pub mod record;
pub mod jsonl;
pub mod triples;
pub mod csv;
pub mod ingest;

pub use record::{Record, RecordPayload};
pub use ingest::{ingest_records, Source, Sink};
pub use jsonl::{JsonlSource, JsonlSink};
pub use triples::TripleSource;
pub use csv::CsvSource;
