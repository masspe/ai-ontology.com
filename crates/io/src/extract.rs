// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Deterministic extractor that lifts ontology fragments out of free-text
//! documents.
//!
//! Tag syntax — one directive per line, recognized anywhere in the body.
//! Lines that don't start with a recognized `@…` directive are ignored, so
//! prose mixes freely with annotations.
//!
//! ```text
//! @concept_type Type -- optional description
//! @concept Type:Name -- optional description
//! @relation_type predicate: Source -> Target [symmetric]
//! @relation Type:Source -predicate-> Type:Target
//! @rule [strict] name on Type1,Type2: when <text> then <text>
//! @action name: Subject -> Object [p1, p2] => effect
//! ```
//!
//! The extractor never errors on unrecognized input — malformed directives
//! are skipped silently — so a document can opt-in to whatever subset of
//! the syntax suits it without breaking ingest.

use ontology_graph::{ActionType, Concept, ConceptId, ConceptType, RelationType, RuleType};

use crate::record::Record;

/// Extract structured records from a text document.
///
/// `doc_type` and `doc_name` describe the enclosing document — they are
/// used to emit the document itself as a [`Concept`] (whose `description`
/// is the full body) and to attach a `mentions` named relation from the
/// document to every `@concept` or `@relation` endpoint it references.
/// Documents longer than this many characters are split into fragments
/// (decision G, STORAGE-PLAN.md §8): the document concept keeps a short
/// excerpt, each fragment is a concept of type `<Type>Fragment` linked to the
/// document by `fragment_of`, and retrieval works on fragments — payloads
/// stay in the kilobyte range (`STORAGE.md` H16) and the full text remains
/// searchable instead of being truncated.
pub const DEFAULT_CHUNK_CHARS: usize = 4_000;
/// Excerpt kept on the document concept when the body is fragmented.
pub const EXCERPT_CHARS: usize = 600;

/// Name of the fragment type derived from a document type.
pub fn fragment_type_name(doc_type: &str) -> String {
    format!("{doc_type}Fragment")
}
/// Relation from a fragment to its document, one per document type (like
/// `mentions_<type>`), so two document types never redefine each other's
/// domain/range.
pub const FRAGMENT_OF_PREFIX: &str = "fragment_of_";
pub fn fragment_relation_name(doc_type: &str) -> String {
    format!("{FRAGMENT_OF_PREFIX}{}", doc_type.to_lowercase())
}
/// Upper bound on fragments per document; beyond it, chunks are merged so
/// the count fits (a 50 MB text must not become 12 000 concepts).
pub const MAX_FRAGMENTS_PER_DOCUMENT: usize = 2_000;

/// `extract_from_text_chunked` with [`DEFAULT_CHUNK_CHARS`].
pub fn extract_from_text(doc_type: &str, doc_name: &str, body: &str) -> Vec<Record> {
    extract_from_text_chunked(doc_type, doc_name, body, DEFAULT_CHUNK_CHARS)
}

/// Split `body` into chunks of at most `chunk_chars` characters, cutting at
/// blank lines (paragraphs) when possible and inside a paragraph otherwise.
/// `chunk_chars == 0` disables chunking (one chunk).
pub fn chunk_text(body: &str, chunk_chars: usize) -> Vec<String> {
    if chunk_chars == 0 || body.chars().count() <= chunk_chars {
        return vec![body.to_string()];
    }
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;
    let flush = |chunks: &mut Vec<String>, current: &mut String, len: &mut usize| {
        let t = current.trim();
        if !t.is_empty() {
            chunks.push(t.to_string());
        }
        current.clear();
        *len = 0;
    };
    let body = body.replace("\r\n", "\n");
    for para in body.split("\n\n") {
        let para = para.trim_matches(['\r', '\n']);
        if para.trim().is_empty() {
            continue;
        }
        let plen = para.chars().count();
        if plen > chunk_chars {
            // Oversized paragraph: settle what we have, then hard-split it.
            flush(&mut chunks, &mut current, &mut current_len);
            let chars: Vec<char> = para.chars().collect();
            for piece in chars.chunks(chunk_chars) {
                chunks.push(piece.iter().collect::<String>().trim().to_string());
            }
            continue;
        }
        if current_len > 0 && current_len + 2 + plen > chunk_chars {
            flush(&mut chunks, &mut current, &mut current_len);
        }
        if current_len > 0 {
            current.push_str("\n\n");
            current_len += 2;
        }
        current.push_str(para);
        current_len += plen;
    }
    flush(&mut chunks, &mut current, &mut current_len);
    chunks.retain(|c| !c.is_empty());
    if chunks.is_empty() {
        chunks.push(body.trim().to_string());
    }
    chunks
}

/// Merge consecutive chunks so at most [`MAX_FRAGMENTS_PER_DOCUMENT`] remain.
fn cap_fragments(chunks: Vec<String>) -> Vec<String> {
    if chunks.len() <= MAX_FRAGMENTS_PER_DOCUMENT {
        return chunks;
    }
    let per = chunks.len().div_ceil(MAX_FRAGMENTS_PER_DOCUMENT);
    chunks.chunks(per).map(|group| group.join("\n\n")).collect()
}

fn excerpt(body: &str) -> String {
    let mut s: String = body.chars().take(EXCERPT_CHARS).collect();
    if body.chars().count() > EXCERPT_CHARS {
        s = s.trim_end().to_string();
        s.push('…');
    }
    s
}

pub fn extract_from_text_chunked(
    doc_type: &str,
    doc_name: &str,
    body: &str,
    chunk_chars: usize,
) -> Vec<Record> {
    let mut out = Vec::new();

    // Ensure the document's own concept type is registered (idempotent).
    out.push(Record::ConceptTypeDecl(ConceptType {
        name: doc_type.to_string(),
        parent: None,
        properties: None,
        description: String::new(),
        ..Default::default()
    }));

    // The document itself. The body is stored as the concept description, but
    // bounded: failed text extraction (e.g. a binary .xlsx/.zip mis-decoded as
    // text) must never persist a multi-megabyte blob — it bloats the snapshot
    // and freezes the UI when rendered. Directive parsing below still runs over
    // the *full* body, so capping the stored description loses no structure.
    let mut doc = Concept::new(ConceptId(0), doc_type.to_string(), doc_name.to_string());
    let mut fragments: Vec<String> = if chunk_chars > 0
        && !crate::charset::looks_binary(body)
        && body.chars().count() > chunk_chars
    {
        cap_fragments(chunk_text(body, chunk_chars))
    } else {
        Vec::new()
    };
    // A body made only of whitespace has nothing to fragment: `chunk_text`
    // falls back to one empty chunk, which must not become an empty
    // fragment concept plus its link.
    if fragments.iter().all(|f| f.trim().is_empty()) {
        fragments.clear();
    }
    if fragments.is_empty() {
        doc.description = document_description(body);
    } else {
        doc.description = excerpt(body);
        doc.properties.insert(
            "fragments".into(),
            ontology_graph::PropertyValue::Number(fragments.len() as f64),
        );
        doc.properties.insert(
            "chars".into(),
            ontology_graph::PropertyValue::Number(body.chars().count() as f64),
        );
    }
    out.push(Record::Concept(doc));

    if !fragments.is_empty() {
        let ftype = fragment_type_name(doc_type);
        let rel = fragment_relation_name(doc_type);
        // The ingester creates `<Type>Fragment` in the document type's
        // domain and the per-type `fragment_of_<type>` relation.
        out.push(Record::FragmentTypeDecl {
            document_type: doc_type.to_string(),
        });
        // All fragment concepts first, then all links: the ingester batches
        // consecutive concepts under one durability barrier, and a relation
        // in between would flush the batch every time.
        for (i, chunk) in fragments.iter().enumerate() {
            let fname = format!("{doc_name}#{:03}", i + 1);
            let mut f = Concept::new(ConceptId(0), ftype.clone(), fname);
            f.description = chunk.clone();
            f.properties.insert(
                "index".into(),
                ontology_graph::PropertyValue::Number((i + 1) as f64),
            );
            f.properties.insert(
                "document".into(),
                ontology_graph::PropertyValue::Text(doc_name.to_string()),
            );
            out.push(Record::Concept(f));
        }
        for i in 0..fragments.len() {
            out.push(Record::NamedRelation {
                relation_type: rel.clone(),
                source_type: ftype.clone(),
                source_name: format!("{doc_name}#{:03}", i + 1),
                target_type: doc_type.to_string(),
                target_name: doc_name.to_string(),
                weight: 1.0,
            });
        }
    }

    let mut mentions: Vec<(String, String)> = Vec::new();

    for raw in body.lines() {
        let line = raw.trim();
        if !line.starts_with('@') {
            continue;
        }
        // Strip a single trailing comma/period that's purely punctuation.
        let line = line.trim_end_matches([',', ';']);

        if let Some(rest) = strip_tag(line, "@concept_type") {
            if let Some(ct) = parse_concept_type(rest) {
                out.push(Record::ConceptTypeDecl(ct));
            }
        } else if let Some(rest) = strip_tag(line, "@concept") {
            if let Some((ty, name, desc)) = parse_concept(rest) {
                out.push(Record::ConceptTypeDecl(ConceptType {
                    name: ty.clone(),
                    parent: None,
                    properties: None,
                    description: String::new(),
                    ..Default::default()
                }));
                let mut c = Concept::new(ConceptId(0), ty.clone(), name.clone());
                if let Some(d) = desc {
                    c.description = d;
                }
                out.push(Record::Concept(c));
                mentions.push((ty, name));
            }
        } else if let Some(rest) = strip_tag(line, "@relation_type") {
            if let Some(rt) = parse_relation_type(rest) {
                out.push(Record::RelationTypeDecl(rt));
            }
        } else if let Some(rest) = strip_tag(line, "@relation") {
            if let Some((rel, st, sn, tt, tn)) = parse_relation(rest) {
                out.push(Record::RelationTypeDecl(RelationType {
                    name: rel.clone(),
                    domain: st.clone(),
                    range: tt.clone(),
                    cardinality: Default::default(),
                    symmetric: false,
                    description: String::new(),
                    ..Default::default()
                }));
                out.push(Record::NamedRelation {
                    relation_type: rel,
                    source_type: st.clone(),
                    source_name: sn.clone(),
                    target_type: tt.clone(),
                    target_name: tn.clone(),
                    weight: 1.0,
                });
                mentions.push((st, sn));
                mentions.push((tt, tn));
            }
        } else if let Some(rest) = strip_tag(line, "@rule") {
            if let Some(rule) = parse_rule(rest) {
                for ct in &rule.applies_to {
                    out.push(Record::ConceptTypeDecl(ConceptType {
                        name: ct.clone(),
                        parent: None,
                        properties: None,
                        description: String::new(),
                        ..Default::default()
                    }));
                }
                out.push(Record::RuleTypeDecl(rule));
            }
        } else if let Some(rest) = strip_tag(line, "@action") {
            if let Some(action) = parse_action(rest) {
                out.push(Record::ConceptTypeDecl(ConceptType {
                    name: action.subject.clone(),
                    parent: None,
                    properties: None,
                    description: String::new(),
                    ..Default::default()
                }));
                if let Some(obj) = &action.object {
                    out.push(Record::ConceptTypeDecl(ConceptType {
                        name: obj.clone(),
                        parent: None,
                        properties: None,
                        description: String::new(),
                        ..Default::default()
                    }));
                }
                out.push(Record::ActionTypeDecl(action));
            }
        }
    }

    // Auto-register a `mentions` relation type and emit one edge per unique
    // mention. Domain is the document type; range varies, so we register a
    // dedicated per-target-type relation `mentions_<Type>` to stay schema-
    // compliant (single domain/range per relation).
    mentions.sort();
    mentions.dedup();
    for (ty, name) in mentions {
        let rel = format!("mentions_{}", ty.to_lowercase());
        out.push(Record::RelationTypeDecl(RelationType {
            name: rel.clone(),
            domain: doc_type.to_string(),
            range: ty.clone(),
            cardinality: Default::default(),
            symmetric: false,
            description: String::new(),
            ..Default::default()
        }));
        out.push(Record::NamedRelation {
            relation_type: rel,
            source_type: doc_type.to_string(),
            source_name: doc_name.to_string(),
            target_type: ty,
            target_name: name,
            weight: 1.0,
        });
    }

    out
}

/// Upper bound on the document body we persist as a concept description.
/// Real documents are far smaller; this only clamps pathological inputs.
const MAX_DOC_DESCRIPTION_BYTES: usize = 64 * 1024;

/// Derive the stored description for a document concept from its raw body.
/// Binary bodies (mis-decoded uploads) are replaced with a short marker
/// rather than a blob; oversized text bodies are truncated on a char
/// boundary so the snapshot stays bounded.
fn document_description(body: &str) -> String {
    if crate::charset::looks_binary(body) {
        return "[binary content — text extraction unavailable]".to_string();
    }
    if body.len() <= MAX_DOC_DESCRIPTION_BYTES {
        return body.to_string();
    }
    let mut end = MAX_DOC_DESCRIPTION_BYTES;
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    let mut s = body[..end].to_string();
    s.push('…');
    s
}

fn strip_tag<'a>(line: &'a str, tag: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(tag)?;
    // Require a space (or end) after the tag so `@concepttype` doesn't
    // match `@concept`.
    match rest.chars().next() {
        Some(c) if c.is_whitespace() => Some(rest.trim_start()),
        None => Some(""),
        _ => None,
    }
}

fn split_desc(s: &str) -> (&str, Option<String>) {
    if let Some(idx) = s.find("--") {
        let (head, tail) = s.split_at(idx);
        (head.trim(), Some(tail[2..].trim().to_string()))
    } else {
        (s.trim(), None)
    }
}

fn parse_concept_type(s: &str) -> Option<ConceptType> {
    let (head, desc) = split_desc(s);
    if head.is_empty() {
        return None;
    }
    Some(ConceptType {
        name: head.to_string(),
        parent: None,
        properties: None,
        description: desc.unwrap_or_default(),
        ..Default::default()
    })
}

fn parse_concept(s: &str) -> Option<(String, String, Option<String>)> {
    let (head, desc) = split_desc(s);
    let (ty, name) = head.split_once(':')?;
    let ty = ty.trim();
    let name = name.trim();
    if ty.is_empty() || name.is_empty() {
        return None;
    }
    Some((ty.to_string(), name.to_string(), desc))
}

fn parse_relation_type(s: &str) -> Option<RelationType> {
    // `predicate: Source -> Target [symmetric]`
    let (name, body) = s.split_once(':')?;
    let name = name.trim();
    let mut body = body.trim().to_string();
    let symmetric = if let Some(stripped) = body.strip_suffix("[symmetric]") {
        body = stripped.trim().to_string();
        true
    } else {
        false
    };
    let (src, tgt) = body.split_once("->")?;
    let src = src.trim();
    let tgt = tgt.trim();
    if name.is_empty() || src.is_empty() || tgt.is_empty() {
        return None;
    }
    Some(RelationType {
        name: name.to_string(),
        domain: src.to_string(),
        range: tgt.to_string(),
        cardinality: Default::default(),
        symmetric,
        description: String::new(),
        ..Default::default()
    })
}

fn parse_relation(s: &str) -> Option<(String, String, String, String, String)> {
    // `Type:Source -predicate-> Type:Target`
    let (left, rest) = s.split_once(" -")?;
    let (predicate, right) = rest.split_once("-> ")?;
    let (st, sn) = left.trim().split_once(':')?;
    let (tt, tn) = right.trim().split_once(':')?;
    let predicate = predicate.trim();
    if predicate.is_empty() || st.is_empty() || sn.is_empty() || tt.is_empty() || tn.is_empty() {
        return None;
    }
    Some((
        predicate.to_string(),
        st.trim().to_string(),
        sn.trim().to_string(),
        tt.trim().to_string(),
        tn.trim().to_string(),
    ))
}

fn parse_rule(s: &str) -> Option<RuleType> {
    // `[strict] name [on Type1,Type2]: when <text> then <text>`
    let mut s = s.trim();
    let mut strict = false;
    if let Some(rest) = s.strip_prefix("strict ") {
        strict = true;
        s = rest.trim();
    }
    let (header, body) = s.split_once(':')?;
    let header = header.trim();
    let (name, applies_to) = if let Some((nm, scope)) = header.split_once(" on ") {
        let scope: Vec<String> = scope
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
        (nm.trim().to_string(), scope)
    } else {
        (header.to_string(), Vec::new())
    };
    if name.is_empty() {
        return None;
    }
    let body = body.trim();
    // Tolerate either order; split on " then " case-insensitively.
    let lower = body.to_ascii_lowercase();
    let (when, then) = match lower.find(" then ") {
        Some(idx) => {
            let when_part = body[..idx].trim();
            let then_part = body[idx + " then ".len()..].trim();
            let when_part = when_part.strip_prefix("when ").unwrap_or(when_part);
            (when_part.to_string(), then_part.to_string())
        }
        None => (String::new(), body.to_string()),
    };
    Some(RuleType {
        name,
        when,
        then,
        applies_to,
        strict,
        description: String::new(),
    })
}

fn parse_action(s: &str) -> Option<ActionType> {
    // `name: Subject [-> Object] [(p1, p2)] [=> effect]`
    let (name, rest) = s.split_once(':')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let mut rest = rest.trim().to_string();

    // Extract effect after `=>`.
    let effect = if let Some(idx) = rest.find("=>") {
        let e = rest[idx + 2..].trim().to_string();
        rest.truncate(idx);
        e
    } else {
        String::new()
    };
    let mut rest = rest.trim().to_string();

    // Extract parameters `(...)`.
    let parameters = if let (Some(o), Some(c)) = (rest.find('('), rest.rfind(')')) {
        if o < c {
            let p: Vec<String> = rest[o + 1..c]
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            rest.replace_range(o..=c, "");
            p
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    let rest = rest.trim();

    let (subject, object) = if let Some((s, o)) = rest.split_once("->") {
        (s.trim().to_string(), Some(o.trim().to_string()))
    } else {
        (rest.to_string(), None)
    };
    if subject.is_empty() {
        return None;
    }

    Some(ActionType {
        name: name.to_string(),
        subject,
        object: object.filter(|s| !s.is_empty()),
        parameters,
        effect,
        description: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_concepts_rules_and_actions() {
        let body = "\
Some preamble.

@concept_type Party -- a contracting party
@concept Party:Acme -- buyer
@concept Party:Globex -- seller
@relation Party:Acme -buys_from-> Party:Globex
@rule strict signed_by_employee on Contract: when Contract.signed_by then Person.employed_by one of parties
@action sign: Person -> Contract (date) => creates signed_by edge
";
        let recs = extract_from_text("Contract", "C-001", body);
        // The document concept must appear.
        assert!(
            recs.iter()
                .any(|r| matches!(r, Record::Concept(c) if c.name == "C-001")),
            "document concept missing"
        );
        // Two extracted Party concepts.
        let party_concepts: Vec<_> = recs
            .iter()
            .filter_map(|r| match r {
                Record::Concept(c) if c.concept_type == "Party" => Some(c.name.as_str()),
                _ => None,
            })
            .collect();
        assert!(party_concepts.contains(&"Acme"));
        assert!(party_concepts.contains(&"Globex"));
        // Relation type declared.
        assert!(recs.iter().any(|r| matches!(
            r,
            Record::RelationTypeDecl(rt) if rt.name == "buys_from"
        )));
        // Rule extracted.
        assert!(recs.iter().any(|r| matches!(
            r,
            Record::RuleTypeDecl(ru) if ru.name == "signed_by_employee" && ru.strict
        )));
        // Action extracted.
        assert!(recs.iter().any(|r| matches!(
            r,
            Record::ActionTypeDecl(a) if a.name == "sign"
                && a.subject == "Person"
                && a.object.as_deref() == Some("Contract")
                && a.parameters == vec!["date".to_string()]
        )));
    }

    #[test]
    fn ignores_prose_lines() {
        let recs = extract_from_text("Doc", "D", "plain text without any tags\n");
        // Just the doc concept type + the doc concept, nothing else.
        assert_eq!(recs.len(), 2);
    }

    #[test]
    fn binary_body_does_not_persist_a_blob() {
        // A 2 MB mis-decoded blob (NUL-laden) must not become the description.
        let blob = "\0".repeat(2 * 1024 * 1024);
        let recs = extract_from_text("Doc", "huge.xlsx", &blob);
        let doc = recs
            .iter()
            .find_map(|r| match r {
                Record::Concept(c) if c.name == "huge.xlsx" => Some(c),
                _ => None,
            })
            .expect("document concept missing");
        assert!(doc.description.len() < 256, "binary blob was persisted");
        assert!(doc.description.contains("binary content"));
    }

    #[test]
    fn long_documents_are_split_into_fragments_linked_to_the_document() {
        let para = "lorem ipsum ".repeat(120); // ~1 440 chars
        let body = std::iter::repeat_n(para.as_str(), 6)
            .collect::<Vec<_>>()
            .join("\n\n"); // ~8 650 chars → 3 chunks of ≤ 4 000
        let recs = extract_from_text("Contract", "C-9", &body);
        let doc = recs
            .iter()
            .find_map(|r| match r {
                Record::Concept(c) if c.concept_type == "Contract" => Some(c),
                _ => None,
            })
            .unwrap();
        assert!(doc.description.chars().count() <= EXCERPT_CHARS + 1);
        assert!(doc.description.ends_with('…'));
        let fragments: Vec<_> = recs
            .iter()
            .filter_map(|r| match r {
                Record::Concept(c) if c.concept_type == "ContractFragment" => Some(c),
                _ => None,
            })
            .collect();
        assert_eq!(
            fragments.len(),
            3,
            "{:?}",
            fragments
                .iter()
                .map(|f| f.description.chars().count())
                .collect::<Vec<_>>()
        );
        assert!(fragments
            .iter()
            .all(|f| f.description.chars().count() <= DEFAULT_CHUNK_CHARS));
        assert_eq!(fragments[0].name, "C-9#001");
        // Nothing lost: the fragments together carry the whole text.
        let joined: usize = fragments
            .iter()
            .map(|f| f.description.chars().count())
            .sum();
        assert!(
            joined >= body.chars().count() - 3 * 2 - 6,
            "joined {joined} vs {}",
            body.chars().count()
        );
        let links = recs
            .iter()
            .filter(|r| matches!(r, Record::NamedRelation { relation_type, .. } if *relation_type == fragment_relation_name("Contract")))
            .count();
        assert_eq!(links, 3);
        assert!(recs
            .iter()
            .any(|r| matches!(r, Record::FragmentTypeDecl { document_type } if document_type == "Contract")));
        // Concepts first, links last.
        let first_link = recs
            .iter()
            .position(|r| matches!(r, Record::NamedRelation { .. }))
            .unwrap();
        let last_fragment = recs
            .iter()
            .rposition(|r| matches!(r, Record::Concept(c) if c.concept_type == "ContractFragment"))
            .unwrap();
        assert!(
            last_fragment < first_link,
            "fragments must precede their links"
        );
        match doc.properties.get("fragments") {
            Some(ontology_graph::PropertyValue::Number(n)) => assert_eq!(*n, 3.0),
            other => panic!("fragments property missing: {other:?}"),
        }
    }

    #[test]
    fn short_documents_and_disabled_chunking_keep_the_full_body() {
        let body = "short body\n\nsecond paragraph";
        let recs = extract_from_text("Doc", "s", body);
        assert!(!recs
            .iter()
            .any(|r| matches!(r, Record::FragmentTypeDecl { .. })));
        let long = "x".repeat(10_000);
        let recs = extract_from_text_chunked("Doc", "l", &long, 0);
        let doc = recs
            .iter()
            .find_map(|r| match r {
                Record::Concept(c) => Some(c),
                _ => None,
            })
            .unwrap();
        assert_eq!(doc.description.len(), 10_000);
        assert_eq!(
            recs.iter()
                .filter(|r| matches!(r, Record::Concept(_)))
                .count(),
            1
        );
    }

    #[test]
    fn fragment_count_is_capped_and_crlf_is_normalized() {
        let many: Vec<String> = (0..5_000).map(|i| format!("p{i}")).collect();
        let capped = cap_fragments(many);
        assert!(capped.len() <= MAX_FRAGMENTS_PER_DOCUMENT);
        assert!(capped.len() >= MAX_FRAGMENTS_PER_DOCUMENT / 2);
        assert!(capped[0].starts_with("p0\n\np1"));
        let crlf = "aaa\r\n\r\nbbb";
        assert_eq!(chunk_text(crlf, 5), vec!["aaa", "bbb"]);
    }

    #[test]
    fn chunk_text_respects_paragraphs_and_splits_giant_ones() {
        let body = "aaa\n\nbbb\n\nccc";
        assert_eq!(chunk_text(body, 8), vec!["aaa\n\nbbb", "ccc"]);
        assert_eq!(chunk_text(body, 0), vec![body.to_string()]);
        let giant = "z".repeat(25);
        let c = chunk_text(&giant, 10);
        assert_eq!(c.len(), 3);
        assert_eq!(c[2].len(), 5);
        assert_eq!(chunk_text("", 5), vec!["".to_string()]);
    }

    #[test]
    fn oversized_text_body_is_truncated() {
        let big = "a".repeat(MAX_DOC_DESCRIPTION_BYTES + 5_000);
        let recs = extract_from_text_chunked("Doc", "big.txt", &big, 0);
        let doc = recs
            .iter()
            .find_map(|r| match r {
                Record::Concept(c) if c.name == "big.txt" => Some(c),
                _ => None,
            })
            .expect("document concept missing");
        assert!(doc.description.len() <= MAX_DOC_DESCRIPTION_BYTES + 4);
        assert!(doc.description.ends_with('…'));
    }

    /// `cap_fragments` is the identity up to the cap, and above it merges
    /// consecutive chunks so nothing is dropped and order is kept; the
    /// last group may be short.
    #[test]
    fn cap_fragments_is_identity_up_to_the_cap_and_merges_losslessly_above_it() {
        let at_cap: Vec<String> = (0..MAX_FRAGMENTS_PER_DOCUMENT)
            .map(|i| format!("c{i}"))
            .collect();
        assert_eq!(cap_fragments(at_cap.clone()), at_cap);

        let over: Vec<String> = (0..MAX_FRAGMENTS_PER_DOCUMENT + 1)
            .map(|i| format!("c{i}"))
            .collect();
        let capped = cap_fragments(over.clone());
        assert_eq!(capped.len(), MAX_FRAGMENTS_PER_DOCUMENT / 2 + 1);
        assert_eq!(capped[0], "c0\n\nc1");
        assert_eq!(capped.last().unwrap(), "c2000", "odd one out stays alone");
        assert_eq!(capped.join("\n\n"), over.join("\n\n"));

        let many: Vec<String> = (0..4_501).map(|i| format!("c{i}")).collect();
        let capped = cap_fragments(many.clone());
        assert_eq!(capped.len(), 1_501, "ceil(4501 / 2000) = 3 per group");
        assert!(capped.len() <= MAX_FRAGMENTS_PER_DOCUMENT);
        assert_eq!(capped.join("\n\n"), many.join("\n\n"));
    }

    #[test]
    fn parses_symmetric_relation_type() {
        let rt = parse_relation_type("knows: Person -> Person [symmetric]").unwrap();
        assert_eq!(rt.name, "knows");
        assert!(rt.symmetric);
    }
}
