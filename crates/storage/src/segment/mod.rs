// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Binary segment container (`STORAGE.md` §4): a `.data` file holding
//! framed records and a sibling `.idx` file holding one fixed-size entry
//! per record. The `.data` is the truth; the `.idx` is derived and
//! reconstructible (R7).
//!
//! - [`format`] — byte layouts, pure.
//! - [`active`] — the growing segment: buffered appends, one `fdatasync`
//!   per commit, never mapped (R11).
//! - [`sealed`] — an immutable segment, memory-mapped.
//! - [`recover`] — tail scan, truncation, index rebuild (§9).

pub mod active;
pub mod format;
pub mod recover;
pub mod sealed;
pub mod xref;

pub use active::{data_path, idx_path, segment_stem, ActiveSegment, IndexFields};
pub use format::{
    crc32c, decode_record, encode_record, pack_endpoints, padded_len, record_span,
    unpack_endpoints, DataHeader, FormatError, IdxEntry, IdxHeader, Kind, RecordHeader, RecordMeta,
    RecordView, CODEC_JSON, DATA_MAGIC, FILE_HEADER_LEN, FORMAT_VERSION, FORMAT_VERSION_CODECS,
    IDX_ENTRY_LEN, IDX_MAGIC, PAYLOAD_ALIGN, RECORD_HEADER_LEN,
};
pub use recover::{recover_segment, Recovered};
pub use sealed::SealedSegment;
pub use xref::{read_xref, write_xref, xref_path, XrefEntry, XREF_ENTRY_LEN, XREF_MAGIC};
