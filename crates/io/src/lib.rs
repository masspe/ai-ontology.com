// SPDX-License-Identifier: LGPL-3.0-or-later
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// It is licensed under the GNU Lesser General Public License v3.0
// or (at your option) any later version. See LICENSE and LICENSE.GPL.

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
pub mod text;
pub mod triples;
pub mod xlsx;

pub use csv::CsvSource;
pub use ingest::{export_graph, ingest_records, ExportStats, IngestStats, Sink, Source};
pub use jsonl::{JsonlSink, JsonlSource};
pub use record::{Record, RecordPayload};
pub use text::TextDocumentSource;
pub use triples::TripleSource;
pub use xlsx::XlsxSource;
