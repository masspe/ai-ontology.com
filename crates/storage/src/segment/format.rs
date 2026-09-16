// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! On-disk primitives of the binary segment format (`STORAGE.md` §4):
//! file headers, record framing and the fixed-size index entry. Pure
//! functions over byte slices — no I/O — so every layout rule is testable
//! byte for byte and the same code serves the writer, the reader and the
//! recovery scan.
//!
//! All integers are little-endian. Offsets in this module are relative to
//! the start of the file they describe.

use crate::log::RecordKind;
use thiserror::Error;

/// Magic of a `.data` file.
pub const DATA_MAGIC: [u8; 4] = *b"GRFD";
/// Magic of an `.idx` file.
pub const IDX_MAGIC: [u8; 4] = *b"GRFI";
/// Current layout version of both files. Bump it — and keep reading the
/// old one — rather than ever rewriting a sealed segment (`STORAGE.md` §11.3).
pub const FORMAT_VERSION: u16 = 1;
/// `MANIFEST.format_version` of a store whose graph streams use a payload
/// codec other than JSON. Builds that only know codec 0 refuse such a store
/// at open instead of mis-reading (or truncating) binary payloads; this
/// build reads both versions. File headers keep `FORMAT_VERSION`.
pub const FORMAT_VERSION_CODECS: u16 = 2;
/// Size of the `.data` and `.idx` file headers.
pub const FILE_HEADER_LEN: usize = 32;
/// Size of the header that precedes every record payload.
pub const RECORD_HEADER_LEN: usize = 32;
/// Size of one index entry (decision D2).
pub const IDX_ENTRY_LEN: usize = 48;
/// Payloads start on an 8-byte boundary; the tail is zero padding.
pub const PAYLOAD_ALIGN: usize = 8;

/// Payload codecs. The container is codec-agnostic (`STORAGE.md` §7.1).
pub const CODEC_JSON: u8 = 0;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FormatError {
    #[error("bad magic: expected {expected:?}, found {found:?}")]
    BadMagic { expected: [u8; 4], found: [u8; 4] },
    #[error("unsupported format version {0} (this build reads {FORMAT_VERSION})")]
    UnsupportedVersion(u16),
    #[error("unexpected index entry size {0} (expected {IDX_ENTRY_LEN})")]
    BadEntrySize(u16),
    #[error("buffer too short: need {need} bytes at offset {at}, have {have}")]
    Truncated { at: usize, need: usize, have: usize },
    #[error("unknown record kind {0}")]
    UnknownKind(u8),
    #[error("unknown codec {0}")]
    UnknownCodec(u8),
    #[error("crc mismatch at offset {at}: header says {expected:#010x}, payload hashes to {actual:#010x}")]
    CrcMismatch {
        at: usize,
        expected: u32,
        actual: u32,
    },
}

// ---------------------------------------------------------------------------
// little-endian helpers
// ---------------------------------------------------------------------------

fn need(buf: &[u8], at: usize, len: usize) -> Result<&[u8], FormatError> {
    match buf.get(at..at + len) {
        Some(s) => Ok(s),
        None => Err(FormatError::Truncated {
            at,
            need: len,
            have: buf.len().saturating_sub(at),
        }),
    }
}
fn u16_at(buf: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(buf[at..at + 2].try_into().unwrap())
}
fn u32_at(buf: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(buf[at..at + 4].try_into().unwrap())
}
fn u64_at(buf: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(buf[at..at + 8].try_into().unwrap())
}

/// Hardware CRC32C (Castagnoli) with software fallback.
pub fn crc32c(bytes: &[u8]) -> u32 {
    crc32c::crc32c(bytes)
}

/// Length of a payload once padded to [`PAYLOAD_ALIGN`].
pub fn padded_len(payload_len: usize) -> usize {
    payload_len.div_ceil(PAYLOAD_ALIGN) * PAYLOAD_ALIGN
}

/// Total bytes a record occupies in a `.data` file: header + padded payload.
pub fn record_span(payload_len: usize) -> usize {
    RECORD_HEADER_LEN + padded_len(payload_len)
}

// ---------------------------------------------------------------------------
// record kinds
// ---------------------------------------------------------------------------

/// One byte per [`RecordKind`] variant, frozen: values are on disk forever.
/// New variants append; nothing is ever renumbered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Kind {
    Ontology = 1,
    Concept = 2,
    Relation = 3,
    UpdateRelation = 4,
    UpdateConcept = 5,
    DeleteConcept = 6,
    DeleteRelation = 7,
    Rule = 8,
    Action = 9,
    DeleteRule = 10,
    DeleteAction = 11,
    /// A relation written by compaction: inserted exactly as stored, id
    /// kept, no symmetric inverse materialized (`STORAGE.md` §5).
    RelationExact = 12,
}

impl Kind {
    pub const ALL: [Kind; 12] = [
        Kind::Ontology,
        Kind::Concept,
        Kind::Relation,
        Kind::UpdateRelation,
        Kind::UpdateConcept,
        Kind::DeleteConcept,
        Kind::DeleteRelation,
        Kind::Rule,
        Kind::Action,
        Kind::DeleteRule,
        Kind::DeleteAction,
        Kind::RelationExact,
    ];

    pub fn from_u8(v: u8) -> Result<Kind, FormatError> {
        Kind::ALL
            .iter()
            .copied()
            .find(|k| *k as u8 == v)
            .ok_or(FormatError::UnknownKind(v))
    }

    pub fn of(kind: &RecordKind) -> Kind {
        match kind {
            RecordKind::Ontology(_) => Kind::Ontology,
            RecordKind::Concept(_) => Kind::Concept,
            RecordKind::Relation(_) => Kind::Relation,
            RecordKind::UpdateRelation(_) => Kind::UpdateRelation,
            RecordKind::UpdateConcept(_) => Kind::UpdateConcept,
            RecordKind::DeleteConcept(_) => Kind::DeleteConcept,
            RecordKind::DeleteRelation(_) => Kind::DeleteRelation,
            RecordKind::Rule(_) => Kind::Rule,
            RecordKind::Action(_) => Kind::Action,
            RecordKind::DeleteRule(_) => Kind::DeleteRule,
            RecordKind::DeleteAction(_) => Kind::DeleteAction,
            RecordKind::RelationExact(_) => Kind::RelationExact,
        }
    }

    /// Which stream a record of this kind is written to (decision D3):
    /// schema, rules and actions are global (`meta`); concepts and relations
    /// belong to a domain.
    pub fn is_meta(self) -> bool {
        matches!(
            self,
            Kind::Ontology | Kind::Rule | Kind::Action | Kind::DeleteRule | Kind::DeleteAction
        )
    }
}

/// What the index needs to know about a record without reading its payload
/// (`STORAGE.md` §4.3). Derived from the record at write time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordMeta<'a> {
    pub kind: Kind,
    /// Id of the entity (`ConceptId`, `RelationId`, `RuleId`, `ActionId`),
    /// 0 for `Ontology`.
    pub entity_id: u64,
    /// `(source << 32) | target` for a relation payload, 0 otherwise.
    pub endpoints: u64,
    /// Relation type name for a relation payload, resolved to a frozen
    /// symbol by the store; `None` otherwise.
    pub relation_type: Option<&'a str>,
}

impl<'a> RecordMeta<'a> {
    pub fn of(kind: &'a RecordKind) -> Self {
        let k = Kind::of(kind);
        match kind {
            RecordKind::Ontology(_) => Self {
                kind: k,
                entity_id: 0,
                endpoints: 0,
                relation_type: None,
            },
            RecordKind::Concept(c) | RecordKind::UpdateConcept(c) => Self {
                kind: k,
                entity_id: c.id.0,
                endpoints: 0,
                relation_type: None,
            },
            RecordKind::DeleteConcept(id) => Self {
                kind: k,
                entity_id: id.0,
                endpoints: 0,
                relation_type: None,
            },
            RecordKind::Relation(r)
            | RecordKind::UpdateRelation(r)
            | RecordKind::RelationExact(r) => Self {
                kind: k,
                entity_id: r.id.0,
                endpoints: pack_endpoints(r.source.0, r.target.0),
                relation_type: Some(r.relation_type.as_str()),
            },
            RecordKind::DeleteRelation(id) => Self {
                kind: k,
                entity_id: id.0,
                endpoints: 0,
                relation_type: None,
            },
            RecordKind::Rule(r) => Self {
                kind: k,
                entity_id: r.id.0,
                endpoints: 0,
                relation_type: None,
            },
            RecordKind::DeleteRule(id) => Self {
                kind: k,
                entity_id: id.0,
                endpoints: 0,
                relation_type: None,
            },
            RecordKind::Action(a) => Self {
                kind: k,
                entity_id: a.id.0,
                endpoints: 0,
                relation_type: None,
            },
            RecordKind::DeleteAction(id) => Self {
                kind: k,
                entity_id: id.0,
                endpoints: 0,
                relation_type: None,
            },
        }
    }
}

/// `(source << 32) | target`. Callers guarantee both fit in 32 bits
/// (`ConceptId::fits_storage`, enforced in `prepare_relation`); the mask
/// here only keeps a violation from silently corrupting the high word.
pub fn pack_endpoints(source: u64, target: u64) -> u64 {
    ((source & 0xFFFF_FFFF) << 32) | (target & 0xFFFF_FFFF)
}
pub fn unpack_endpoints(packed: u64) -> (u64, u64) {
    (packed >> 32, packed & 0xFFFF_FFFF)
}

// ---------------------------------------------------------------------------
// file headers
// ---------------------------------------------------------------------------

/// Header of a `.data` file (32 bytes).
///
/// ```text
/// 0  magic "GRFD"      4  version u16     6  codec u8   7  flags u8
/// 8  partition_id u32  12 base_seq u64    20 reserved (12)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataHeader {
    pub version: u16,
    pub codec: u8,
    pub flags: u8,
    pub partition_id: u32,
    pub base_seq: u64,
}

impl DataHeader {
    pub fn new(partition_id: u32, base_seq: u64, codec: u8) -> Self {
        Self {
            version: FORMAT_VERSION,
            codec,
            flags: 0,
            partition_id,
            base_seq,
        }
    }
    pub fn encode(&self) -> [u8; FILE_HEADER_LEN] {
        let mut b = [0u8; FILE_HEADER_LEN];
        b[0..4].copy_from_slice(&DATA_MAGIC);
        b[4..6].copy_from_slice(&self.version.to_le_bytes());
        b[6] = self.codec;
        b[7] = self.flags;
        b[8..12].copy_from_slice(&self.partition_id.to_le_bytes());
        b[12..20].copy_from_slice(&self.base_seq.to_le_bytes());
        b
    }
    pub fn decode(buf: &[u8]) -> Result<Self, FormatError> {
        let b = need(buf, 0, FILE_HEADER_LEN)?;
        let magic: [u8; 4] = b[0..4].try_into().unwrap();
        if magic != DATA_MAGIC {
            return Err(FormatError::BadMagic {
                expected: DATA_MAGIC,
                found: magic,
            });
        }
        let version = u16_at(b, 4);
        if version != FORMAT_VERSION {
            return Err(FormatError::UnsupportedVersion(version));
        }
        Ok(Self {
            version,
            codec: b[6],
            flags: b[7],
            partition_id: u32_at(b, 8),
            base_seq: u64_at(b, 12),
        })
    }
}

/// Header of an `.idx` file (32 bytes).
///
/// ```text
/// 0  magic "GRFI"      4  version u16     6  entry_size u16
/// 8  partition_id u32  12 base_seq u64    20 count u32   24 reserved (8)
/// ```
///
/// `count` is authoritative only for a **sealed** segment; while a segment
/// is active the header is written once and the entry count is derived
/// from the file length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdxHeader {
    pub version: u16,
    pub entry_size: u16,
    pub partition_id: u32,
    pub base_seq: u64,
    pub count: u32,
}

impl IdxHeader {
    pub fn new(partition_id: u32, base_seq: u64, count: u32) -> Self {
        Self {
            version: FORMAT_VERSION,
            entry_size: IDX_ENTRY_LEN as u16,
            partition_id,
            base_seq,
            count,
        }
    }
    pub fn encode(&self) -> [u8; FILE_HEADER_LEN] {
        let mut b = [0u8; FILE_HEADER_LEN];
        b[0..4].copy_from_slice(&IDX_MAGIC);
        b[4..6].copy_from_slice(&self.version.to_le_bytes());
        b[6..8].copy_from_slice(&self.entry_size.to_le_bytes());
        b[8..12].copy_from_slice(&self.partition_id.to_le_bytes());
        b[12..20].copy_from_slice(&self.base_seq.to_le_bytes());
        b[20..24].copy_from_slice(&self.count.to_le_bytes());
        b
    }
    pub fn decode(buf: &[u8]) -> Result<Self, FormatError> {
        let b = need(buf, 0, FILE_HEADER_LEN)?;
        let magic: [u8; 4] = b[0..4].try_into().unwrap();
        if magic != IDX_MAGIC {
            return Err(FormatError::BadMagic {
                expected: IDX_MAGIC,
                found: magic,
            });
        }
        let version = u16_at(b, 4);
        if version != FORMAT_VERSION {
            return Err(FormatError::UnsupportedVersion(version));
        }
        let entry_size = u16_at(b, 6);
        if entry_size as usize != IDX_ENTRY_LEN {
            return Err(FormatError::BadEntrySize(entry_size));
        }
        Ok(Self {
            version,
            entry_size,
            partition_id: u32_at(b, 8),
            base_seq: u64_at(b, 12),
            count: u32_at(b, 20),
        })
    }
}

// ---------------------------------------------------------------------------
// records
// ---------------------------------------------------------------------------

/// Header that precedes every payload in a `.data` file (32 bytes).
///
/// ```text
/// 0  seq u64      8  ts_micros u64   16 payload_len u32   20 crc32c u32
/// 24 kind u8      25 codec u8        26 flags u8          27 reserved u8
/// 28 reserved u32
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordHeader {
    pub seq: u64,
    pub ts_micros: u64,
    pub payload_len: u32,
    pub crc: u32,
    pub kind: Kind,
    pub codec: u8,
    pub flags: u8,
}

impl RecordHeader {
    pub fn encode(&self) -> [u8; RECORD_HEADER_LEN] {
        let mut b = [0u8; RECORD_HEADER_LEN];
        b[0..8].copy_from_slice(&self.seq.to_le_bytes());
        b[8..16].copy_from_slice(&self.ts_micros.to_le_bytes());
        b[16..20].copy_from_slice(&self.payload_len.to_le_bytes());
        b[20..24].copy_from_slice(&self.crc.to_le_bytes());
        b[24] = self.kind as u8;
        b[25] = self.codec;
        b[26] = self.flags;
        b
    }
    /// Decode the header at `at`. Does not touch the payload.
    pub fn decode(buf: &[u8], at: usize) -> Result<Self, FormatError> {
        let b = need(buf, at, RECORD_HEADER_LEN)?;
        Ok(Self {
            seq: u64_at(b, 0),
            ts_micros: u64_at(b, 8),
            payload_len: u32_at(b, 16),
            crc: u32_at(b, 20),
            kind: Kind::from_u8(b[24])?,
            codec: b[25],
            flags: b[26],
        })
    }
}

/// A record as laid out on disk: header + payload + zero padding to 8.
/// Appends to `out`; returns the number of bytes appended.
pub fn encode_record(
    out: &mut Vec<u8>,
    seq: u64,
    ts_micros: u64,
    kind: Kind,
    codec: u8,
    payload: &[u8],
) -> usize {
    let header = RecordHeader {
        seq,
        ts_micros,
        payload_len: u32::try_from(payload.len()).expect("payload longer than u32::MAX"),
        crc: crc32c(payload),
        kind,
        codec,
        flags: 0,
    };
    let start = out.len();
    out.extend_from_slice(&header.encode());
    out.extend_from_slice(payload);
    out.resize(start + record_span(payload.len()), 0);
    out.len() - start
}

/// A decoded record view: header plus a borrowed payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordView<'a> {
    pub header: RecordHeader,
    pub payload: &'a [u8],
    /// Offset of the header in the buffer.
    pub offset: usize,
    /// Offset of the next record.
    pub next: usize,
}

/// Decode the record starting at `at`. `verify_crc` should be `true` on the
/// hydration and recovery paths and `false` on hot reads (`STORAGE.md`
/// §7.3). A short buffer yields [`FormatError::Truncated`] — the signal the
/// recovery scan turns into "torn tail".
pub fn decode_record(
    buf: &[u8],
    at: usize,
    verify_crc: bool,
) -> Result<RecordView<'_>, FormatError> {
    let header = RecordHeader::decode(buf, at)?;
    let payload_at = at + RECORD_HEADER_LEN;
    let payload = need(buf, payload_at, header.payload_len as usize)?;
    let next = at + record_span(header.payload_len as usize);
    // The padding must be present too, otherwise the record is torn.
    need(buf, at, next - at)?;
    if verify_crc {
        let actual = crc32c(payload);
        if actual != header.crc {
            return Err(FormatError::CrcMismatch {
                at,
                expected: header.crc,
                actual,
            });
        }
    }
    Ok(RecordView {
        header,
        payload,
        offset: at,
        next,
    })
}

// ---------------------------------------------------------------------------
// index entries
// ---------------------------------------------------------------------------

/// One fixed-size entry of an `.idx` file (48 bytes, decision D2).
///
/// ```text
/// 0  seq u64        8  offset u64        16 payload_len u32
/// 20 kind u8        21 flags u8          22 ns_id u16
/// 24 entity_id u64  32 endpoints u64     40 rtype_sym u32   44 target_ns_id u16   46 reserved u16
/// ```
///
/// `target_ns_id` (phase 3) is the domain of a relation's **target**
/// concept type; it lets the `.xref` of that domain be rebuilt from indexes
/// alone (`STORAGE.md` §4.4). 0 for non-relation records and for relations
/// whose target is in the record's own domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdxEntry {
    pub seq: u64,
    /// Offset of the record header in the `.data` file.
    pub offset: u64,
    pub payload_len: u32,
    pub kind: Kind,
    pub flags: u8,
    pub ns_id: u16,
    pub entity_id: u64,
    pub endpoints: u64,
    pub rtype_sym: u32,
    /// Domain of the target endpoint when it differs from the record's own
    /// domain (relations only); 0 otherwise.
    pub target_ns_id: u16,
}

impl IdxEntry {
    pub fn encode(&self) -> [u8; IDX_ENTRY_LEN] {
        let mut b = [0u8; IDX_ENTRY_LEN];
        b[0..8].copy_from_slice(&self.seq.to_le_bytes());
        b[8..16].copy_from_slice(&self.offset.to_le_bytes());
        b[16..20].copy_from_slice(&self.payload_len.to_le_bytes());
        b[20] = self.kind as u8;
        b[21] = self.flags;
        b[22..24].copy_from_slice(&self.ns_id.to_le_bytes());
        b[24..32].copy_from_slice(&self.entity_id.to_le_bytes());
        b[32..40].copy_from_slice(&self.endpoints.to_le_bytes());
        b[40..44].copy_from_slice(&self.rtype_sym.to_le_bytes());
        b[44..46].copy_from_slice(&self.target_ns_id.to_le_bytes());
        b
    }
    pub fn decode(buf: &[u8], at: usize) -> Result<Self, FormatError> {
        let b = need(buf, at, IDX_ENTRY_LEN)?;
        Ok(Self {
            seq: u64_at(b, 0),
            offset: u64_at(b, 8),
            payload_len: u32_at(b, 16),
            kind: Kind::from_u8(b[20])?,
            flags: b[21],
            ns_id: u16_at(b, 22),
            entity_id: u64_at(b, 24),
            endpoints: u64_at(b, 32),
            rtype_sym: u32_at(b, 40),
            target_ns_id: u16_at(b, 44),
        })
    }
    /// Byte offset of entry `i` in an `.idx` file (positional, decision D1).
    pub fn file_offset(i: usize) -> usize {
        FILE_HEADER_LEN + i * IDX_ENTRY_LEN
    }
    /// Number of complete entries in an `.idx` file of `file_len` bytes.
    pub fn count_in(file_len: u64) -> usize {
        (file_len.saturating_sub(FILE_HEADER_LEN as u64) / IDX_ENTRY_LEN as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ontology_graph::{
        Action, ActionId, Concept, ConceptId, Ontology, Relation, RelationId, Rule, RuleId,
    };

    fn every_kind() -> Vec<RecordKind> {
        vec![
            RecordKind::Ontology(Ontology::new()),
            RecordKind::Concept(Concept::new(ConceptId(7), "T", "n")),
            RecordKind::Relation(Relation::new(
                RelationId(9),
                "rt",
                ConceptId(1),
                ConceptId(2),
            )),
            RecordKind::UpdateRelation(Relation::new(
                RelationId(9),
                "rt",
                ConceptId(1),
                ConceptId(2),
            )),
            RecordKind::UpdateConcept(Concept::new(ConceptId(7), "T", "n")),
            RecordKind::DeleteConcept(ConceptId(7)),
            RecordKind::DeleteRelation(RelationId(9)),
            RecordKind::Rule(Rule::new(RuleId(3), "rule", "r")),
            RecordKind::Action(Action::new(ActionId(4), "act", "a", ConceptId(1))),
            RecordKind::DeleteRule(RuleId(3)),
            RecordKind::DeleteAction(ActionId(4)),
            RecordKind::RelationExact(Relation::new(
                RelationId(9),
                "rt",
                ConceptId(1),
                ConceptId(2),
            )),
        ]
    }

    #[test]
    fn every_record_kind_has_a_frozen_byte_and_round_trips() {
        let kinds = every_kind();
        assert_eq!(
            kinds.len(),
            Kind::ALL.len(),
            "a RecordKind variant is missing a Kind"
        );
        let mut seen = std::collections::HashSet::new();
        for k in &kinds {
            let b = Kind::of(k) as u8;
            assert!(seen.insert(b), "duplicate kind byte {b}");
            assert_eq!(Kind::from_u8(b).unwrap(), Kind::of(k));
        }
        // Frozen values: renumbering would corrupt every existing store.
        assert_eq!(Kind::Ontology as u8, 1);
        assert_eq!(Kind::Concept as u8, 2);
        assert_eq!(Kind::Relation as u8, 3);
        assert_eq!(Kind::DeleteAction as u8, 11);
        assert_eq!(Kind::RelationExact as u8, 12);
        assert_eq!(Kind::from_u8(0), Err(FormatError::UnknownKind(0)));
        assert_eq!(Kind::from_u8(200), Err(FormatError::UnknownKind(200)));
    }

    #[test]
    fn record_meta_extracts_ids_endpoints_and_type() {
        let rel = RecordKind::Relation(Relation::new(
            RelationId(9),
            "knows",
            ConceptId(5),
            ConceptId(6),
        ));
        let m = RecordMeta::of(&rel);
        assert_eq!(m.kind, Kind::Relation);
        assert_eq!(m.entity_id, 9);
        assert_eq!(unpack_endpoints(m.endpoints), (5, 6));
        assert_eq!(m.relation_type, Some("knows"));

        let concept = RecordKind::Concept(Concept::new(ConceptId(7), "T", "n"));
        let c = RecordMeta::of(&concept);
        assert_eq!(
            (c.kind, c.entity_id, c.endpoints, c.relation_type),
            (Kind::Concept, 7, 0, None)
        );
        let onto = RecordKind::Ontology(Ontology::new());
        let o = RecordMeta::of(&onto);
        assert_eq!(o.entity_id, 0);
        let del = RecordKind::DeleteRelation(RelationId(9));
        let d = RecordMeta::of(&del);
        assert_eq!((d.kind, d.entity_id), (Kind::DeleteRelation, 9));
    }

    #[test]
    fn meta_kinds_are_schema_rules_and_actions() {
        for k in Kind::ALL {
            let expect = matches!(
                k,
                Kind::Ontology | Kind::Rule | Kind::Action | Kind::DeleteRule | Kind::DeleteAction
            );
            assert_eq!(k.is_meta(), expect, "{k:?}");
        }
    }

    #[test]
    fn headers_round_trip_and_reject_foreign_files() {
        let d = DataHeader::new(42, 1000, CODEC_JSON);
        let b = d.encode();
        assert_eq!(&b[0..4], b"GRFD");
        assert_eq!(DataHeader::decode(&b).unwrap(), d);
        assert!(matches!(
            IdxHeader::decode(&b),
            Err(FormatError::BadMagic { .. })
        ));

        let i = IdxHeader::new(42, 1000, 17);
        let b = i.encode();
        assert_eq!(&b[0..4], b"GRFI");
        assert_eq!(IdxHeader::decode(&b).unwrap(), i);
        assert!(matches!(
            DataHeader::decode(&b),
            Err(FormatError::BadMagic { .. })
        ));

        let mut bad_version = d.encode();
        bad_version[4..6].copy_from_slice(&99u16.to_le_bytes());
        assert_eq!(
            DataHeader::decode(&bad_version),
            Err(FormatError::UnsupportedVersion(99))
        );
        let mut bad_entry = i.encode();
        bad_entry[6..8].copy_from_slice(&32u16.to_le_bytes());
        assert_eq!(
            IdxHeader::decode(&bad_entry),
            Err(FormatError::BadEntrySize(32))
        );
        assert!(matches!(
            DataHeader::decode(&b[..10]),
            Err(FormatError::Truncated { .. })
        ));
    }

    #[test]
    fn record_encoding_pads_to_eight_and_verifies_crc() {
        let mut out = Vec::new();
        let n = encode_record(&mut out, 5, 123, Kind::Concept, CODEC_JSON, b"hello");
        assert_eq!(n, RECORD_HEADER_LEN + 8);
        assert_eq!(out.len(), n);
        assert_eq!(&out[RECORD_HEADER_LEN + 5..], &[0, 0, 0]);

        let v = decode_record(&out, 0, true).unwrap();
        assert_eq!(v.header.seq, 5);
        assert_eq!(v.header.ts_micros, 123);
        assert_eq!(v.header.kind, Kind::Concept);
        assert_eq!(v.payload, b"hello");
        assert_eq!(v.next, n);

        // Flip a payload byte: CRC catches it on the verifying path only.
        let mut bad = out.clone();
        bad[RECORD_HEADER_LEN] ^= 0xFF;
        assert!(matches!(
            decode_record(&bad, 0, true),
            Err(FormatError::CrcMismatch { .. })
        ));
        assert!(decode_record(&bad, 0, false).is_ok());

        // Empty payload is a header alone.
        let mut out = Vec::new();
        assert_eq!(
            encode_record(&mut out, 1, 0, Kind::Ontology, CODEC_JSON, b""),
            RECORD_HEADER_LEN
        );
        assert_eq!(decode_record(&out, 0, true).unwrap().payload, b"");
    }

    #[test]
    fn truncated_records_are_reported_not_panicked() {
        let mut out = Vec::new();
        encode_record(&mut out, 1, 0, Kind::Concept, CODEC_JSON, b"0123456789");
        for cut in 0..out.len() {
            let err = decode_record(&out[..cut], 0, true).unwrap_err();
            assert!(
                matches!(err, FormatError::Truncated { .. }),
                "cut {cut}: {err}"
            );
        }
        assert!(decode_record(&out, 0, true).is_ok());
    }

    #[test]
    fn idx_entry_round_trips_and_is_positional() {
        let e = IdxEntry {
            seq: 77,
            offset: 4096,
            payload_len: 1300,
            kind: Kind::Relation,
            flags: 0,
            ns_id: 3,
            entity_id: 9,
            endpoints: pack_endpoints(5, 6),
            rtype_sym: 12,
            target_ns_id: 4,
        };
        let b = e.encode();
        assert_eq!(b.len(), IDX_ENTRY_LEN);
        assert_eq!(IdxEntry::decode(&b, 0).unwrap(), e);
        assert_eq!(IdxEntry::file_offset(0), FILE_HEADER_LEN);
        assert_eq!(
            IdxEntry::file_offset(3),
            FILE_HEADER_LEN + 3 * IDX_ENTRY_LEN
        );
        assert_eq!(IdxEntry::count_in(FILE_HEADER_LEN as u64), 0);
        assert_eq!(
            IdxEntry::count_in((FILE_HEADER_LEN + 2 * IDX_ENTRY_LEN + 7) as u64),
            2
        );
        assert_eq!(IdxEntry::count_in(0), 0);
        assert!(matches!(
            IdxEntry::decode(&b[..20], 0),
            Err(FormatError::Truncated { .. })
        ));
    }

    #[test]
    fn endpoints_pack_and_unpack() {
        assert_eq!(unpack_endpoints(pack_endpoints(0, 0)), (0, 0));
        assert_eq!(
            unpack_endpoints(pack_endpoints(u32::MAX as u64, 1)),
            (u32::MAX as u64, 1)
        );
        assert_eq!(
            unpack_endpoints(pack_endpoints(1, u32::MAX as u64)),
            (1, u32::MAX as u64)
        );
    }
}
