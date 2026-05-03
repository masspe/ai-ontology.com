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
