// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Property tests of the on-disk primitives: whatever goes through
//! `encode_record` comes back byte-identical through `decode_record`, a
//! stream of records decodes to exactly the records written, in order, and
//! no prefix of a stream ever panics the decoder.

use ontology_storage::segment::{
    decode_record, encode_record, record_span, DataHeader, FormatError, IdxEntry, IdxHeader, Kind,
    CODEC_JSON, PAYLOAD_ALIGN, RECORD_HEADER_LEN,
};
use proptest::prelude::*;

fn any_kind() -> impl Strategy<Value = Kind> {
    prop::sample::select(Kind::ALL.to_vec())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn a_record_round_trips(
        seq in any::<u64>(),
        ts in any::<u64>(),
        kind in any_kind(),
        payload in prop::collection::vec(any::<u8>(), 0..2048),
    ) {
        let mut out = Vec::new();
        let n = encode_record(&mut out, seq, ts, kind, CODEC_JSON, &payload);
        prop_assert_eq!(n, record_span(payload.len()));
        prop_assert_eq!(n % PAYLOAD_ALIGN, 0);
        prop_assert!(n >= RECORD_HEADER_LEN + payload.len());
        let v = decode_record(&out, 0, true).unwrap();
        prop_assert_eq!(v.header.seq, seq);
        prop_assert_eq!(v.header.ts_micros, ts);
        prop_assert_eq!(v.header.kind, kind);
        prop_assert_eq!(v.header.codec, CODEC_JSON);
        prop_assert_eq!(v.payload, &payload[..]);
        prop_assert_eq!(v.next, n);
        // Padding is zero.
        for b in &out[RECORD_HEADER_LEN + payload.len()..] {
            prop_assert_eq!(*b, 0);
        }
    }

    #[test]
    fn a_stream_decodes_to_the_records_written_in_order(
        records in prop::collection::vec(
            (any::<u64>(), any_kind(), prop::collection::vec(any::<u8>(), 0..300)),
            0..40,
        ),
    ) {
        let mut buf = Vec::new();
        let mut spans = Vec::new();
        for (seq, kind, payload) in &records {
            spans.push(encode_record(&mut buf, *seq, 0, *kind, CODEC_JSON, payload));
        }
        let mut at = 0;
        let mut decoded = Vec::new();
        while at < buf.len() {
            let v = decode_record(&buf, at, true).unwrap();
            decoded.push((v.header.seq, v.header.kind, v.payload.to_vec()));
            prop_assert_eq!(v.next - v.offset, spans[decoded.len() - 1]);
            at = v.next;
        }
        prop_assert_eq!(at, buf.len());
        prop_assert_eq!(decoded, records);
    }

    #[test]
    fn a_torn_prefix_is_reported_as_truncated_never_as_garbage(
        seq in any::<u64>(),
        kind in any_kind(),
        payload in prop::collection::vec(any::<u8>(), 0..512),
        cut_from_end in 1usize..64,
    ) {
        let mut out = Vec::new();
        let n = encode_record(&mut out, seq, 0, kind, CODEC_JSON, &payload);
        let cut = n.saturating_sub(cut_from_end);
        match decode_record(&out[..cut], 0, true) {
            Err(FormatError::Truncated { .. }) => {}
            other => prop_assert!(false, "cut {cut} of {n}: {other:?}"),
        }
    }

    #[test]
    fn a_corrupted_payload_byte_is_caught_by_the_crc(
        payload in prop::collection::vec(any::<u8>(), 1..512),
        which in any::<prop::sample::Index>(),
        flip in 1u8..=255,
    ) {
        let mut out = Vec::new();
        encode_record(&mut out, 1, 0, Kind::Concept, CODEC_JSON, &payload);
        let i = RECORD_HEADER_LEN + which.index(payload.len());
        out[i] ^= flip;
        let corrupt = decode_record(&out, 0, true);
        prop_assert!(matches!(corrupt, Err(FormatError::CrcMismatch { .. })), "got {:?}", corrupt);
        // The non-verifying path (hot reads) still decodes the frame.
        prop_assert!(decode_record(&out, 0, false).is_ok());
    }

    #[test]
    fn index_entries_and_headers_round_trip(
        seq in any::<u64>(),
        offset in any::<u64>(),
        payload_len in any::<u32>(),
        kind in any_kind(),
        flags in any::<u8>(),
        ns_id in any::<u16>(),
        entity_id in any::<u64>(),
        endpoints in any::<u64>(),
        rtype_sym in any::<u32>(),
        target_ns_id in any::<u16>(),
        partition_id in any::<u32>(),
        base_seq in any::<u64>(),
        count in any::<u32>(),
    ) {
        let e = IdxEntry { seq, offset, payload_len, kind, flags, ns_id, entity_id, endpoints, rtype_sym, target_ns_id };
        prop_assert_eq!(IdxEntry::decode(&e.encode(), 0).unwrap(), e);
        let d = DataHeader::new(partition_id, base_seq, CODEC_JSON);
        prop_assert_eq!(DataHeader::decode(&d.encode()).unwrap(), d);
        let i = IdxHeader::new(partition_id, base_seq, count);
        prop_assert_eq!(IdxHeader::decode(&i.encode()).unwrap(), i);
    }

    #[test]
    fn random_bytes_never_panic_the_decoders(bytes in prop::collection::vec(any::<u8>(), 0..128)) {
        let _ = decode_record(&bytes, 0, true);
        let _ = DataHeader::decode(&bytes);
        let _ = IdxHeader::decode(&bytes);
        let _ = IdxEntry::decode(&bytes, 0);
    }
}
