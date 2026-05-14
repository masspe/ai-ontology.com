// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

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
pub mod extract;
pub mod ingest;
pub mod jsonl;
pub mod record;
pub mod text;
pub mod triples;
pub mod xlsx;

pub use csv::CsvSource;
pub use extract::extract_from_text;
pub use ingest::{export_graph, ingest_records, ExportStats, IngestStats, Sink, Source};
pub use jsonl::{JsonlSink, JsonlSource};
pub use record::{Record, RecordPayload};
pub use text::TextDocumentSource;
pub use triples::TripleSource;
pub use xlsx::XlsxSource;
