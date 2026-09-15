// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! `MANIFEST.json` (`STORAGE.md` §4.5): the small, atomically rewritten
//! description of a store — frozen `ns_id`s (R10), frozen relation-type
//! symbols (D2), partition allocator, and per-sealed-partition zone maps.
//!
//! The manifest is **never the source of truth for data**: every sealed
//! partition is rediscovered from the directory at open and its zone map
//! recomputed from the index if missing, so a crash between a seal and the
//! manifest write costs nothing but a recomputation. What *is*
//! authoritative here are the two symbol tables, because an id or symbol
//! must never be reused (R10).

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::segment::{Kind, SealedSegment, FORMAT_VERSION};

pub const MANIFEST_FILE: &str = "MANIFEST.json";
pub const MANIFEST_VERSION: u32 = 1;

/// The global stream: ontology, rules, actions (D3).
pub const META_NS_ID: u16 = 0;
pub const META_NS: &str = "meta";
/// The only graph domain until partitioning by `ns` lands (phase 3).
pub const DEFAULT_NS_ID: u16 = 1;
pub const DEFAULT_NS: &str = "default";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NsEntry {
    pub id: u16,
    pub name: String,
    /// A retired domain keeps its id forever (R10) but accepts no writes.
    #[serde(default)]
    pub retired: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SymEntry {
    pub sym: u32,
    pub name: String,
}

/// Zone map of one sealed partition. Everything here is derivable from the
/// partition's `.idx` (R7); it only saves opening the file.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PartitionEntry {
    pub id: u32,
    pub base_seq: u64,
    pub last_seq: u64,
    pub records: u64,
    pub data_bytes: u64,
    pub payload_bytes: u64,
    /// Smallest / largest concept id touched by a concept-kind record;
    /// `entity_min > entity_max` means none.
    pub entity_min: u64,
    pub entity_max: u64,
    /// Bitmap of the `Kind`s present (bit `k as u8`).
    pub kinds: u16,
    /// Relation records (edges written, not net of deletes).
    pub edges: u64,
}

impl PartitionEntry {
    /// Compute the zone map of a sealed segment from its index.
    pub fn of(seg: &SealedSegment) -> Self {
        let mut e = PartitionEntry {
            id: seg.partition_id(),
            base_seq: seg.base_seq(),
            last_seq: seg.last_seq().unwrap_or(seg.base_seq().saturating_sub(1)),
            records: seg.record_count() as u64,
            data_bytes: seg.data_len(),
            entity_min: u64::MAX,
            entity_max: 0,
            ..Default::default()
        };
        for x in seg.entries() {
            e.payload_bytes += x.payload_len as u64;
            e.kinds |= 1 << (x.kind as u8);
            match x.kind {
                Kind::Concept | Kind::UpdateConcept | Kind::DeleteConcept => {
                    e.entity_min = e.entity_min.min(x.entity_id);
                    e.entity_max = e.entity_max.max(x.entity_id);
                }
                Kind::Relation => e.edges += 1,
                _ => {}
            }
        }
        e
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StreamEntry {
    pub ns_id: u16,
    /// Directory of the stream, relative to the store root.
    pub dir: String,
    pub sealed: Vec<PartitionEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub version: u32,
    pub format_version: u16,
    /// Codec of newly written records (existing segments keep theirs).
    pub codec: u8,
    /// Next partition id to hand out. Partition ids are store-wide.
    pub next_partition_id: u32,
    pub ns: Vec<NsEntry>,
    pub relation_types: Vec<SymEntry>,
    pub streams: Vec<StreamEntry>,
}

impl Manifest {
    pub fn new(codec: u8) -> Self {
        Self {
            version: MANIFEST_VERSION,
            format_version: FORMAT_VERSION,
            codec,
            next_partition_id: 1,
            ns: vec![
                NsEntry {
                    id: META_NS_ID,
                    name: META_NS.into(),
                    retired: false,
                },
                NsEntry {
                    id: DEFAULT_NS_ID,
                    name: DEFAULT_NS.into(),
                    retired: false,
                },
            ],
            relation_types: Vec::new(),
            streams: vec![
                StreamEntry {
                    ns_id: META_NS_ID,
                    dir: META_NS.into(),
                    sealed: Vec::new(),
                },
                StreamEntry {
                    ns_id: DEFAULT_NS_ID,
                    dir: format!("graph/{DEFAULT_NS}"),
                    sealed: Vec::new(),
                },
            ],
        }
    }

    pub fn path(root: &Path) -> PathBuf {
        root.join(MANIFEST_FILE)
    }

    /// `Ok(None)` when the store has no manifest yet.
    pub fn load(root: &Path) -> io::Result<Option<Self>> {
        let p = Self::path(root);
        if !p.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&p)?;
        let m: Manifest = serde_json::from_slice(&bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("MANIFEST: {e}")))?;
        if m.version != MANIFEST_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("MANIFEST version {} not supported", m.version),
            ));
        }
        Ok(Some(m))
    }

    /// Write + fsync a temp file, rename over the manifest, fsync the
    /// directory where the platform allows.
    pub fn save(&self, root: &Path) -> io::Result<()> {
        let dest = Self::path(root);
        let tmp = root.join("MANIFEST.json.tmp");
        {
            let mut f = File::create(&tmp)?;
            f.write_all(&serde_json::to_vec_pretty(self).map_err(io::Error::other)?)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, &dest)?;
        #[cfg(unix)]
        if let Ok(d) = File::open(root) {
            let _ = d.sync_all();
        }
        Ok(())
    }

    /// Frozen symbol of a relation type, allocating one if unseen. Returns
    /// `(sym, newly_allocated)`; a new symbol must be saved **before** the
    /// first record that uses it is written (D2).
    pub fn intern_relation_type(&mut self, name: &str) -> (u32, bool) {
        if let Some(e) = self.relation_types.iter().find(|e| e.name == name) {
            return (e.sym, false);
        }
        let sym = self.relation_types.iter().map(|e| e.sym).max().unwrap_or(0) + 1;
        self.relation_types.push(SymEntry {
            sym,
            name: name.to_string(),
        });
        (sym, true)
    }

    /// Frozen id of a domain, allocating one (and its stream) if unseen.
    /// Returns `(ns_id, newly_allocated)`. Ids are never reused, even after a
    /// domain is retired (R10). Must be saved before the first record of a
    /// new domain is written.
    pub fn intern_ns(&mut self, name: &str) -> (u16, bool) {
        if let Some(e) = self.ns.iter().find(|e| e.name == name && !e.retired) {
            return (e.id, false);
        }
        let id = self.ns.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        self.ns.push(NsEntry {
            id,
            name: name.to_string(),
            retired: false,
        });
        self.streams.push(StreamEntry {
            ns_id: id,
            dir: format!("graph/{name}"),
            sealed: Vec::new(),
        });
        (id, true)
    }

    /// Id of a live domain by name.
    pub fn ns_id(&self, name: &str) -> Option<u16> {
        self.ns
            .iter()
            .find(|e| e.name == name && !e.retired)
            .map(|e| e.id)
    }

    /// Name of a domain by id (retired ones included).
    pub fn ns_name(&self, id: u16) -> Option<&str> {
        self.ns.iter().find(|e| e.id == id).map(|e| e.name.as_str())
    }

    /// Live graph domains (everything but `meta`), by id.
    pub fn graph_ns_ids(&self) -> Vec<u16> {
        let mut v: Vec<u16> = self
            .ns
            .iter()
            .filter(|e| e.id != META_NS_ID && !e.retired)
            .map(|e| e.id)
            .collect();
        v.sort_unstable();
        v
    }

    /// Mark a domain retired: its id and directory are kept, no record is
    /// routed to it any more. A later domain with the same name gets a new id.
    pub fn retire_ns(&mut self, id: u16) {
        if let Some(e) = self.ns.iter_mut().find(|e| e.id == id) {
            e.retired = true;
        }
    }

    pub fn relation_type_sym(&self, name: &str) -> Option<u32> {
        self.relation_types
            .iter()
            .find(|e| e.name == name)
            .map(|e| e.sym)
    }

    pub fn relation_type_name(&self, sym: u32) -> Option<&str> {
        self.relation_types
            .iter()
            .find(|e| e.sym == sym)
            .map(|e| e.name.as_str())
    }

    pub fn stream(&self, ns_id: u16) -> Option<&StreamEntry> {
        self.streams.iter().find(|s| s.ns_id == ns_id)
    }

    pub fn stream_mut(&mut self, ns_id: u16) -> Option<&mut StreamEntry> {
        self.streams.iter_mut().find(|s| s.ns_id == ns_id)
    }

    /// Total records across every sealed partition (the active ones are
    /// not counted here).
    pub fn sealed_records(&self) -> u64 {
        self.streams
            .iter()
            .flat_map(|s| s.sealed.iter())
            .map(|p| p.records)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_are_frozen_and_never_reused() {
        let mut m = Manifest::new(0);
        assert_eq!(m.intern_relation_type("knows"), (1, true));
        assert_eq!(m.intern_relation_type("knows"), (1, false));
        assert_eq!(m.intern_relation_type("owns"), (2, true));
        assert_eq!(m.relation_type_sym("owns"), Some(2));
        assert_eq!(m.relation_type_name(2), Some("owns"));
        assert_eq!(m.relation_type_sym("nope"), None);
        // Even if a name disappeared from the middle, the next symbol
        // continues from the max — never fills the hole.
        m.relation_types.retain(|e| e.name != "knows");
        assert_eq!(m.intern_relation_type("later"), (3, true));
    }

    #[test]
    fn ns_ids_are_frozen_and_never_reused() {
        let mut m = Manifest::new(0);
        assert_eq!(m.ns_id("default"), Some(DEFAULT_NS_ID));
        assert_eq!(m.intern_ns("parties"), (2, true));
        assert_eq!(m.intern_ns("parties"), (2, false));
        assert_eq!(m.stream(2).unwrap().dir, "graph/parties");
        assert_eq!(m.graph_ns_ids(), vec![1, 2]);
        m.retire_ns(2);
        assert_eq!(m.ns_id("parties"), None);
        assert_eq!(m.ns_name(2), Some("parties"));
        assert_eq!(m.graph_ns_ids(), vec![1]);
        // Same name again: a fresh id, the retired one stays frozen.
        assert_eq!(m.intern_ns("parties"), (3, true));
        assert_eq!(m.ns.len(), 4);
    }

    #[test]
    fn new_manifest_declares_meta_and_default() {
        let m = Manifest::new(0);
        assert_eq!(m.stream(META_NS_ID).unwrap().dir, "meta");
        assert_eq!(m.stream(DEFAULT_NS_ID).unwrap().dir, "graph/default");
        assert_eq!(m.ns.len(), 2);
        assert_eq!(m.next_partition_id, 1);
        assert_eq!(m.format_version, FORMAT_VERSION);
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = std::env::temp_dir().join(format!(
            "ontology-manifest-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(Manifest::load(&dir).unwrap().is_none());
        let mut m = Manifest::new(0);
        m.intern_relation_type("knows");
        m.next_partition_id = 5;
        m.stream_mut(DEFAULT_NS_ID)
            .unwrap()
            .sealed
            .push(PartitionEntry {
                id: 3,
                records: 10,
                ..Default::default()
            });
        m.save(&dir).unwrap();
        let back = Manifest::load(&dir).unwrap().unwrap();
        assert_eq!(back, m);
        assert!(!dir.join("MANIFEST.json.tmp").exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
