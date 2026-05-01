//! Persistence layer for the ontology graph.
//!
//! Storage is intentionally pluggable through the [`Store`] trait. Two
//! implementations are provided:
//!
//! - [`MemoryStore`] – non-persistent, useful for tests.
//! - [`FileStore`] – append-only, write-ahead log on disk plus periodic
//!   bincode snapshots. Crash-safe for the common single-writer case.
//!
//! The on-disk format is a sequence of length-prefixed [`LogRecord`]s,
//! decoupled from the in-memory graph types so the schema can evolve.

pub mod log;
pub mod memory;
pub mod file;
pub mod store;
pub mod snapshot;

pub use log::{LogRecord, RecordKind};
pub use memory::MemoryStore;
pub use file::FileStore;
pub use store::{Store, StoreError, StoreResult};
pub use snapshot::Snapshot;
