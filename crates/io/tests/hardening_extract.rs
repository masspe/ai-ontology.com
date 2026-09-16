// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Decision G (`STORAGE.md` §10.9) at the edges: the 4 000-character
//! boundary, multi-byte text, paragraph-aligned cuts, the 2 000-fragment
//! cap merging rather than dropping text, stable fragment naming, the
//! excerpt kept on the document, and directives parsed over the full body.

use ontology_graph::{Concept, PropertyValue};
use ontology_io::{
    chunk_text, extract_from_text, extract_from_text_chunked, fragment_relation_name,
    fragment_type_name, Record, DEFAULT_CHUNK_CHARS, EXCERPT_CHARS, MAX_FRAGMENTS_PER_DOCUMENT,
};

fn document<'a>(recs: &'a [Record], doc_type: &str, name: &str) -> &'a Concept {
    recs.iter()
        .find_map(|r| match r {
            Record::Concept(c) if c.concept_type == doc_type && c.name == name => Some(c),
            _ => None,
        })
        .expect("document concept")
}

fn fragments<'a>(recs: &'a [Record], doc_type: &str) -> Vec<&'a Concept> {
    let ftype = fragment_type_name(doc_type);
    recs.iter()
        .filter_map(|r| match r {
            Record::Concept(c) if c.concept_type == ftype => Some(c),
            _ => None,
        })
        .collect()
}

fn number(c: &Concept, key: &str) -> f64 {
    match c.properties.get(key) {
        Some(PropertyValue::Number(n)) => *n,
        other => panic!("{key}: expected a number, got {other:?}"),
    }
}

fn has_fragment_decl(recs: &[Record]) -> bool {
    recs.iter()
        .any(|r| matches!(r, Record::FragmentTypeDecl { .. }))
}

/// Exactly `DEFAULT_CHUNK_CHARS` characters stay one document with the full
/// body as description; one more character fragments it (4 000 + 1).
#[test]
fn a_body_of_exactly_chunk_chars_stays_whole_and_one_more_char_is_fragmented() {
    let exact = "a".repeat(DEFAULT_CHUNK_CHARS);
    let recs = extract_from_text("Doc", "exact", &exact);
    assert!(!has_fragment_decl(&recs));
    let doc = document(&recs, "Doc", "exact");
    assert_eq!(doc.description, exact);
    assert!(!doc.properties.contains_key("fragments"));
    assert_eq!(chunk_text(&exact, DEFAULT_CHUNK_CHARS), vec![exact.clone()]);

    let over = "a".repeat(DEFAULT_CHUNK_CHARS + 1);
    let recs = extract_from_text("Doc", "over", &over);
    assert!(has_fragment_decl(&recs));
    let frags = fragments(&recs, "Doc");
    assert_eq!(frags.len(), 2);
    assert_eq!(frags[0].description.chars().count(), DEFAULT_CHUNK_CHARS);
    assert_eq!(frags[1].description, "a");
    let doc = document(&recs, "Doc", "over");
    assert_eq!(number(doc, "fragments"), 2.0);
    assert_eq!(number(doc, "chars"), (DEFAULT_CHUNK_CHARS + 1) as f64);
    assert_eq!(doc.description.chars().count(), EXCERPT_CHARS + 1);
    assert!(doc.description.ends_with('…'));
}

/// Sizes are counted in characters, not bytes, and a hard split inside an
/// oversized paragraph never lands inside a multi-byte character.
#[test]
fn chunking_counts_characters_not_bytes_and_never_splits_a_multibyte_char() {
    // 2-byte characters: 9 000 chars = 18 000 bytes, one paragraph.
    let body = "é".repeat(9_000);
    let chunks = chunk_text(&body, DEFAULT_CHUNK_CHARS);
    let sizes: Vec<usize> = chunks.iter().map(|c| c.chars().count()).collect();
    assert_eq!(sizes, vec![4_000, 4_000, 1_000]);
    assert!(chunks.iter().all(|c| c.len() == 2 * c.chars().count()));
    assert_eq!(chunks.concat(), body, "every character is kept, in order");

    // 4-byte characters.
    let emoji = "😀".repeat(4_500);
    let chunks = chunk_text(&emoji, DEFAULT_CHUNK_CHARS);
    assert_eq!(
        chunks.iter().map(|c| c.chars().count()).collect::<Vec<_>>(),
        vec![4_000, 500]
    );
    assert_eq!(chunks.concat(), emoji);

    // The excerpt is a character count too.
    let recs = extract_from_text("Doc", "accents", &body);
    let doc = document(&recs, "Doc", "accents");
    assert_eq!(doc.description.chars().count(), EXCERPT_CHARS + 1);
    assert_eq!(number(doc, "chars"), 9_000.0);
}

/// N paragraphs of exactly `chunk_chars` characters yield N fragments of
/// exactly `chunk_chars` — the separator never pushes a paragraph over.
#[test]
fn a_document_of_exactly_n_paragraphs_of_chunk_size_yields_n_full_fragments() {
    for (chunk, n) in [(DEFAULT_CHUNK_CHARS, 3usize), (10, 7)] {
        let paras: Vec<String> = (0..n)
            .map(|i| {
                let head = format!("{i}");
                format!("{head}{}", "x".repeat(chunk - head.chars().count()))
            })
            .collect();
        let body = paras.join("\n\n");
        let chunks = chunk_text(&body, chunk);
        assert_eq!(chunks, paras, "chunk = {chunk}");
        assert!(chunks.iter().all(|c| c.chars().count() == chunk));
    }
}

/// When paragraphs fit, cuts fall on paragraph boundaries only: every
/// fragment is a `\n\n`-join of whole, consecutive paragraphs.
#[test]
fn chunks_cut_at_paragraph_boundaries_when_paragraphs_fit() {
    let paras: Vec<String> = (0..6)
        .map(|i| {
            format!("P{i} {}", "lorem ".repeat(249))
                .trim_end()
                .to_string()
        })
        .collect();
    assert!(paras.iter().all(|p| p.chars().count() == 1_496));
    let body = paras.join("\n\n");
    let chunks = chunk_text(&body, DEFAULT_CHUNK_CHARS);
    // 1 496 + 2 + 1 496 = 2 994 fits; a third paragraph would make 4 492.
    assert_eq!(chunks.len(), 3);
    let mut rebuilt: Vec<String> = Vec::new();
    for c in &chunks {
        let pieces: Vec<&str> = c.split("\n\n").collect();
        assert_eq!(pieces.len(), 2, "two whole paragraphs per chunk");
        rebuilt.extend(pieces.iter().map(|p| p.to_string()));
    }
    assert_eq!(rebuilt, paras, "no paragraph split, none lost, order kept");
    assert_eq!(chunks.join("\n\n"), body);
}

/// `MAX_FRAGMENTS_PER_DOCUMENT` merges consecutive chunks instead of
/// dropping text: the fragments joined back give the exact body, the
/// document counts them, and names / links follow the same order.
#[test]
fn the_fragment_cap_merges_chunks_and_preserves_every_character() {
    let paras: Vec<String> = (0..4_500).map(|i| format!("p{i:04}")).collect();
    let body = paras.join("\n\n");
    // chunk_chars = 5: every paragraph is its own chunk → 4 500 chunks,
    // merged 3 by 3 into 1 500 fragments.
    let recs = extract_from_text_chunked("Doc", "big", &body, 5);
    let frags = fragments(&recs, "Doc");
    assert_eq!(frags.len(), 1_500);
    assert!(frags.len() <= MAX_FRAGMENTS_PER_DOCUMENT);
    assert_eq!(frags[0].description, "p0000\n\np0001\n\np0002");
    let joined = frags
        .iter()
        .map(|f| f.description.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    assert_eq!(joined, body, "merging drops nothing");

    let doc = document(&recs, "Doc", "big");
    assert_eq!(number(doc, "fragments"), 1_500.0);
    assert_eq!(number(doc, "chars"), body.chars().count() as f64);

    // Names are unique and zero-padded to three digits, indexes 1-based,
    // and the fragment_of links come in the same order as the fragments.
    assert_eq!(frags[0].name, "big#001");
    assert_eq!(frags[999].name, "big#1000");
    assert_eq!(frags[1_499].name, "big#1500");
    let mut names: Vec<&str> = frags.iter().map(|f| f.name.as_str()).collect();
    names.dedup();
    assert_eq!(names.len(), 1_500);
    let rel = fragment_relation_name("Doc");
    let link_sources: Vec<&str> = recs
        .iter()
        .filter_map(|r| match r {
            Record::NamedRelation {
                relation_type,
                source_name,
                target_name,
                ..
            } if *relation_type == rel => {
                assert_eq!(target_name, "big");
                Some(source_name.as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(link_sources, names);
}

/// Every fragment carries its 1-based `index`, the `document` it belongs
/// to, and the per-type fragment type / relation names of decision G.
#[test]
fn fragments_are_numbered_in_order_and_point_back_to_their_document() {
    assert_eq!(fragment_type_name("Contract"), "ContractFragment");
    assert_eq!(fragment_relation_name("Contract"), "fragment_of_contract");
    let body = (0..3)
        .map(|i| format!("{i}{}", "z".repeat(99)))
        .collect::<Vec<_>>()
        .join("\n\n");
    let recs = extract_from_text_chunked("Contract", "C-7", &body, 100);
    let frags = fragments(&recs, "Contract");
    assert_eq!(frags.len(), 3);
    for (i, f) in frags.iter().enumerate() {
        assert_eq!(f.name, format!("C-7#{:03}", i + 1));
        assert_eq!(number(f, "index"), (i + 1) as f64);
        assert_eq!(
            f.properties.get("document").and_then(|v| v.as_text()),
            Some("C-7")
        );
        assert!(f.description.starts_with(&i.to_string()));
    }
    let decl = recs
        .iter()
        .filter(|r| matches!(r, Record::FragmentTypeDecl { document_type } if document_type == "Contract"))
        .count();
    assert_eq!(decl, 1, "one fragment type declaration per document");
    let first_fragment = recs
        .iter()
        .position(|r| matches!(r, Record::Concept(c) if c.concept_type == "ContractFragment"))
        .unwrap();
    let decl_pos = recs
        .iter()
        .position(|r| matches!(r, Record::FragmentTypeDecl { .. }))
        .unwrap();
    assert!(
        decl_pos < first_fragment,
        "the type is declared before its instances"
    );
}

/// Directives are recognised anywhere in the body, including far past the
/// 600-character excerpt kept on the document concept.
#[test]
fn directives_are_parsed_over_the_full_body_not_the_excerpt() {
    let filler = std::iter::repeat_n("prose ".repeat(200).trim_end().to_string(), 8)
        .collect::<Vec<_>>()
        .join("\n\n");
    assert!(filler.chars().count() > 2 * DEFAULT_CHUNK_CHARS);
    let body = format!(
        "{filler}\n\n@concept_type Party -- a contracting party\n\
         @concept Party:Acme -- buyer\n\
         @concept Party:Globex -- seller\n\
         @relation_type supplies: Party -> Party\n\
         @relation Party:Globex -supplies-> Party:Acme\n"
    );
    let recs = extract_from_text("Doc", "D-1", &body);
    let doc = document(&recs, "Doc", "D-1");
    assert!(
        !doc.description.contains('@'),
        "the excerpt stops long before the directives"
    );
    assert!(has_fragment_decl(&recs));
    let parties: Vec<&str> = recs
        .iter()
        .filter_map(|r| match r {
            Record::Concept(c) if c.concept_type == "Party" => Some(c.name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(parties, vec!["Acme", "Globex"]);
    assert!(recs.iter().any(|r| matches!(
        r,
        Record::ConceptTypeDecl(ct) if ct.name == "Party" && ct.description == "a contracting party"
    )));
    assert!(recs.iter().any(|r| matches!(
        r,
        Record::RelationTypeDecl(rt) if rt.name == "supplies" && rt.domain == "Party" && rt.range == "Party"
    )));
    assert!(recs.iter().any(|r| matches!(
        r,
        Record::NamedRelation { relation_type, source_name, target_name, .. }
            if relation_type == "supplies" && source_name == "Globex" && target_name == "Acme"
    )));
    // One `mentions_party` edge per referenced party, from the document.
    let mentions: Vec<&str> = recs
        .iter()
        .filter_map(|r| match r {
            Record::NamedRelation {
                relation_type,
                source_type,
                source_name,
                target_name,
                ..
            } if relation_type == "mentions_party" => {
                assert_eq!((source_type.as_str(), source_name.as_str()), ("Doc", "D-1"));
                Some(target_name.as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(mentions, vec!["Acme", "Globex"]);
    assert!(recs.iter().any(|r| matches!(
        r,
        Record::RelationTypeDecl(rt) if rt.name == "mentions_party" && rt.domain == "Doc" && rt.range == "Party"
    )));
}

/// A binary body (mis-decoded upload) is never fragmented: the document
/// keeps the short marker and no fragment records are emitted.
#[test]
fn a_binary_body_is_never_fragmented() {
    let blob = format!("PK\u{3}\u{4}{}", "\u{FFFD}\u{1}\u{2}".repeat(3_000));
    let recs = extract_from_text("Doc", "blob.xlsx", &blob);
    assert!(!has_fragment_decl(&recs));
    assert!(fragments(&recs, "Doc").is_empty());
    assert_eq!(recs.len(), 2, "type declaration + document only");
    assert!(document(&recs, "Doc", "blob.xlsx")
        .description
        .contains("binary content"));
}

/// A body made only of whitespace, however long, is not fragmented into an
/// empty fragment; an empty body yields the document alone.
#[test]
fn whitespace_only_and_empty_bodies_are_never_fragmented() {
    let blank = "\n\n \n\n".repeat(2_000);
    assert!(blank.chars().count() > DEFAULT_CHUNK_CHARS);
    let recs = extract_from_text("Doc", "blank", &blank);
    assert!(!has_fragment_decl(&recs), "no fragment for whitespace");
    assert!(fragments(&recs, "Doc").is_empty());
    let doc = document(&recs, "Doc", "blank");
    assert!(!doc.properties.contains_key("fragments"));

    let recs = extract_from_text("Doc", "empty", "");
    assert_eq!(recs.len(), 2);
    assert_eq!(document(&recs, "Doc", "empty").description, "");
    assert_eq!(chunk_text("", DEFAULT_CHUNK_CHARS), vec![String::new()]);
}

/// CRLF bodies chunk on the same paragraph boundaries as LF bodies, and
/// directives are still recognised on CRLF lines.
#[test]
fn crlf_bodies_chunk_and_parse_like_lf_bodies() {
    let paras: Vec<String> = (0..4).map(|i| format!("{i}{}", "y".repeat(59))).collect();
    let lf = paras.join("\n\n");
    let crlf = paras.join("\r\n\r\n");
    assert_eq!(chunk_text(&crlf, 125), chunk_text(&lf, 125));
    assert_eq!(chunk_text(&crlf, 125).len(), 2);

    let body = "intro\r\n@concept_type Party -- p\r\n@concept Party:Acme\r\n";
    let recs = extract_from_text("Doc", "crlf", body);
    assert!(recs.iter().any(|r| matches!(
        r,
        Record::Concept(c) if c.concept_type == "Party" && c.name == "Acme"
    )));
}
