// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Byte-level front door of the text pipeline: charset sniffing and the
//! binary heuristic, the `.docx` extractor on hand-built archives, and
//! language detection.

mod hardening_common;

use hardening_common::{docx_bytes, zip_bytes};
use ontology_io::{decode_to_utf8, detect_language, extract_docx_text, is_zip, looks_binary};

// -------------------------------------------------------------- charset --

/// UTF-16 big-endian with a BOM is decoded, the BOM stripped and reported.
#[test]
fn utf16_be_with_bom_is_decoded_and_reported() {
    let text = "héllo wörld — ça va";
    let mut buf = vec![0xFE, 0xFF];
    for unit in text.encode_utf16() {
        buf.extend_from_slice(&unit.to_be_bytes());
    }
    let r = decode_to_utf8(&buf);
    assert_eq!(r.text, text);
    assert!(r.had_bom);
    assert!(!r.lossy);
    assert_eq!(r.encoding, "UTF-16BE");
    assert!(!r.text.starts_with('\u{FEFF}'));
}

/// PNG and ZIP headers decoded as text are flagged binary; prose with a few
/// control characters (form feeds, an ANSI escape) is not.
#[test]
fn png_and_zip_look_binary_but_prose_with_a_few_control_chars_does_not() {
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend_from_slice(&[0, 0, 0, 13, b'I', b'H', b'D', b'R', 0, 0, 1, 0, 0, 0, 1, 0]);
    assert!(looks_binary(&decode_to_utf8(&png).text));
    let zip = zip_bytes(&[("a.txt", b"hello")]);
    assert!(looks_binary(&decode_to_utf8(&zip).text));

    let prose = format!(
        "Page one.{}\u{c}Page two.{}\u{c}Page three.{}\u{1b}[0m",
        "Lorem ipsum dolor sit amet. ".repeat(12),
        "Consectetur adipiscing elit. ".repeat(12),
        "Sed do eiusmod tempor. ".repeat(12)
    );
    assert!(!looks_binary(&prose));
    assert!(!looks_binary("tabs\tand\r\nnewlines\nare fine"));
}

/// The heuristic threshold is 10 % of the sampled characters: one control
/// character in ten flags the text, one in eleven does not; a NUL flags it
/// regardless of length.
#[test]
fn the_binary_threshold_is_ten_percent_and_a_nul_is_immediate() {
    assert!(looks_binary(&format!("\u{1}{}", "a".repeat(9))));
    assert!(!looks_binary(&format!("\u{1}{}", "a".repeat(10))));
    assert!(looks_binary(&format!("{}\0", "a".repeat(5_000))));
    // The replacement character counts as suspicious too.
    assert!(looks_binary(&"\u{FFFD}a".repeat(5)));
}

/// Invalid bytes after a UTF-8 BOM are replaced, reported as `lossy`, and
/// never abort the decode.
#[test]
fn invalid_utf8_after_a_utf8_bom_is_lossy_not_fatal() {
    let mut buf = vec![0xEF, 0xBB, 0xBF];
    buf.extend_from_slice(b"ok ");
    buf.extend_from_slice(&[0xFF, 0xFE]);
    buf.extend_from_slice(b" end");
    let r = decode_to_utf8(&buf);
    assert!(r.had_bom);
    assert!(r.lossy);
    assert_eq!(r.encoding, "UTF-8");
    assert!(r.text.starts_with("ok "));
    assert!(r.text.ends_with(" end"));
    assert!(r.text.contains('\u{FFFD}'));
}

/// Empty input and a BOM-only file both decode to empty text without being
/// lossy; only the latter reports a BOM.
#[test]
fn empty_and_bom_only_inputs_decode_to_empty_text() {
    let r = decode_to_utf8(b"");
    assert_eq!(r.text, "");
    assert!(!r.lossy && !r.had_bom);
    let r = decode_to_utf8(&[0xEF, 0xBB, 0xBF]);
    assert_eq!(r.text, "");
    assert!(r.had_bom);
    assert!(!r.lossy);
}

// ----------------------------------------------------------------- docx --

/// Paragraphs are joined with newlines, `w:br` / `w:tab` rendered, XML
/// entities unescaped, runs of blank paragraphs collapsed to one, and the
/// result trimmed.
#[test]
fn docx_paragraphs_are_joined_and_breaks_tabs_entities_rendered() {
    let body = concat!(
        "<w:p><w:r><w:t>Hello &amp; welcome</w:t></w:r></w:p>",
        "<w:p><w:r><w:t>Line</w:t><w:br/><w:t>break</w:t></w:r></w:p>",
        "<w:p><w:r><w:rPr><w:b/></w:rPr><w:t>a</w:t><w:tab/><w:t>b</w:t></w:r></w:p>",
        "<w:p></w:p><w:p></w:p>",
        "<w:p><w:r><w:t xml:space=\"preserve\"> last </w:t></w:r></w:p>",
    );
    let text = extract_docx_text(&docx_bytes(body)).unwrap();
    assert_eq!(text, "Hello & welcome\nLine\nbreak\na\tb\n\n last");
}

/// A ZIP without `word/document.xml` and a non-ZIP buffer are both errors
/// saying which; `is_zip` needs the full 4-byte local-header magic.
#[test]
fn a_zip_without_document_xml_and_a_non_zip_are_errors() {
    let not_docx = zip_bytes(&[("hello.txt", b"hi")]);
    assert!(is_zip(&not_docx));
    let err = extract_docx_text(&not_docx).unwrap_err();
    assert!(err.contains("missing word/document.xml"), "{err}");

    let plain = b"not a zip archive at all";
    assert!(!is_zip(plain));
    let err = extract_docx_text(plain).unwrap_err();
    assert!(err.contains("not a valid docx"), "{err}");

    assert!(!is_zip(b"PK\x03"), "too short");
    assert!(
        !is_zip(b"PK\x05\x06\x00\x00"),
        "end-of-central-directory only"
    );
}

/// Malformed XML (a mismatched end tag) inside `word/document.xml` is an
/// error, not a panic or a silent empty string.
#[test]
fn malformed_document_xml_is_an_error() {
    let bytes = zip_bytes(&[(
        "word/document.xml",
        b"<w:document><w:body><w:p><w:t>oops</w:p></w:t></w:body></w:document>",
    )]);
    let err = extract_docx_text(&bytes).unwrap_err();
    assert!(err.contains("xml parse"), "{err}");
}

// ----------------------------------------------------------------- lang --

/// Detected languages come back as ISO 639-1 codes with a script and a
/// bounded confidence; text without letters yields `None`.
#[test]
fn detect_language_returns_two_letter_codes_and_none_without_letters() {
    let fr = detect_language(
        "Les fragments sont l'unité naturelle du RAG et restent indexés par le retrieval.",
    )
    .expect("french");
    assert_eq!(fr.code, "fr");
    assert_eq!(fr.script, "Latin");
    assert!((0.0..=1.0).contains(&fr.confidence));

    let en =
        detect_language("Fragments are the natural unit of retrieval and stay indexed in full.")
            .expect("english");
    assert_eq!(en.code, "en");
    assert_eq!(en.code.len(), 2);

    assert!(detect_language("1234 5678 --- !!! 42").is_none());
    assert!(detect_language("   \n\t").is_none());
}
