// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Charset detection + normalization for inbound documents.
//!
//! Pipeline:
//! 1. Detect a BOM (UTF-8 / UTF-16 LE / UTF-16 BE) and decode accordingly.
//! 2. Otherwise let [`chardetng::EncodingDetector`] pick the most likely
//!    legacy or UTF-8 encoding and decode via [`encoding_rs`].
//! 3. Strip a leading U+FEFF (BOM that survived decoding).
//! 4. Apply Unicode Normalization Form C (NFC) so accented characters
//!    have a single canonical representation in the graph.
//!
//! Decoding never fails: malformed bytes get the Unicode replacement
//! character (U+FFFD) rather than aborting the ingest.

use encoding_rs::{Encoding, UTF_16BE, UTF_16LE, UTF_8};
use unicode_normalization::UnicodeNormalization;

/// Outcome of decoding a raw byte slice.
#[derive(Debug, Clone)]
pub struct DecodedText {
    /// Decoded, BOM-stripped, NFC-normalized text.
    pub text: String,
    /// IANA / Whatwg name of the encoding that produced `text`
    /// (e.g. `"UTF-8"`, `"windows-1252"`).
    pub encoding: &'static str,
    /// `true` when the source carried a UTF-8 / UTF-16 BOM.
    pub had_bom: bool,
    /// `true` when at least one byte was replaced by U+FFFD.
    pub lossy: bool,
}

/// Decode arbitrary bytes into a UTF-8 string, normalize to NFC.
///
/// The decoder is infallible: ingestion must never reject a document
/// just because it was saved in the wrong codepage. Replacement
/// characters surface in [`DecodedText::lossy`].
pub fn decode_to_utf8(raw: &[u8]) -> DecodedText {
    let (encoding, had_bom) = sniff_encoding(raw);
    // `encoding_rs::Encoding::decode` strips a BOM matching its encoding
    // automatically, so the leading BOM bytes are dropped here too.
    let (cow, _used, lossy) = encoding.decode(raw);
    let mut text: String = cow.into_owned();

    // Defensive: if a BOM somehow survives (e.g. decoder mismatch), drop it.
    if text.starts_with('\u{FEFF}') {
        text.remove(0);
    }

    // NFC keeps "café" as a single composed codepoint per glyph; both
    // search indexing and exact-name conflict detection rely on this.
    let normalized: String = text.nfc().collect();

    DecodedText {
        text: normalized,
        encoding: encoding.name(),
        had_bom,
        lossy,
    }
}

/// Choose an encoding by sniffing BOM / running `chardetng`.
fn sniff_encoding(raw: &[u8]) -> (&'static Encoding, bool) {
    if raw.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return (UTF_8, true);
    }
    if raw.starts_with(&[0xFF, 0xFE]) {
        return (UTF_16LE, true);
    }
    if raw.starts_with(&[0xFE, 0xFF]) {
        return (UTF_16BE, true);
    }

    let mut det = chardetng::EncodingDetector::new();
    // Feed up to 64 KiB — plenty for legacy single-byte detection and
    // cheap on huge files.
    let sample = &raw[..raw.len().min(64 * 1024)];
    det.feed(sample, true);
    // `tld = None` and `allow_utf8 = true`: trust the detector to upgrade
    // valid UTF-8 even when no BOM is present.
    let encoding = det.guess(None, true);
    (encoding, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_plain_passthrough() {
        let r = decode_to_utf8("hello world".as_bytes());
        assert_eq!(r.text, "hello world");
        assert_eq!(r.encoding, "UTF-8");
        assert!(!r.had_bom);
        assert!(!r.lossy);
    }

    #[test]
    fn utf8_bom_stripped() {
        let mut buf = vec![0xEF, 0xBB, 0xBF];
        buf.extend_from_slice("café".as_bytes());
        let r = decode_to_utf8(&buf);
        assert_eq!(r.text, "café");
        assert!(r.had_bom);
        assert!(!r.text.starts_with('\u{FEFF}'));
    }

    #[test]
    fn windows_1252_decoded() {
        // "naïve résumé" in Windows-1252.
        let bytes: &[u8] = b"na\xEFve r\xE9sum\xE9";
        let r = decode_to_utf8(bytes);
        assert_eq!(r.text, "naïve résumé");
        // chardetng resolves this as windows-1252 (or compatible).
        assert!(
            r.encoding.eq_ignore_ascii_case("windows-1252")
                || r.encoding.eq_ignore_ascii_case("ISO-8859-1"),
            "unexpected encoding {}",
            r.encoding
        );
    }

    #[test]
    fn utf16_le_with_bom() {
        let text = "héllo";
        let mut buf = vec![0xFF, 0xFE];
        for unit in text.encode_utf16() {
            buf.extend_from_slice(&unit.to_le_bytes());
        }
        let r = decode_to_utf8(&buf);
        assert_eq!(r.text, "héllo");
        assert!(r.had_bom);
    }

    #[test]
    fn nfc_normalization_applied() {
        // "café" with combining acute (U+0301) → composed é (U+00E9).
        let decomposed = "cafe\u{0301}";
        let r = decode_to_utf8(decomposed.as_bytes());
        assert_eq!(r.text, "café");
        assert_eq!(r.text.chars().count(), 4);
    }
}
