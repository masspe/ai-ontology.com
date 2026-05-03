// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Persistence layer for the ontology graph.
//!
//! Storage is intentionally pluggable through the [`Store`] trait. Two
//! implementations are provided:
//!
//! - [`MemoryStore`] – non-persistent, useful for tests.
//! - [`FileStore`] – append-only, write-ahead log on disk plus periodic
//!   JSON snapshots. Crash-safe for the common single-writer case.
//!
//! The on-disk format is a sequence of length-prefixed [`LogRecord`]s,
//! decoupled from the in-memory graph types so the schema can evolve.

pub mod file;
pub mod log;
pub mod memory;
pub mod periodic;
pub mod snapshot;
pub mod store;

pub use file::FileStore;
pub use log::{LogRecord, RecordKind};
pub use memory::MemoryStore;
pub use periodic::{spawn_snapshotter, SnapshotHandle};
pub use snapshot::Snapshot;
pub use store::{Store, StoreError, StoreResult};
