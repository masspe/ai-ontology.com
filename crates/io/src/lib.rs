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

pub mod csv;
pub mod ingest;
pub mod jsonl;
pub mod record;
pub mod triples;

pub use csv::CsvSource;
pub use ingest::{export_graph, ingest_records, ExportStats, Sink, Source};
pub use jsonl::{JsonlSink, JsonlSource};
pub use record::{Record, RecordPayload};
pub use triples::TripleSource;
