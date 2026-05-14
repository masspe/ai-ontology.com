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

use ontology_graph::{
    ActionType, Concept, ConceptId, ConceptType, RelationType, RuleType,
};

use crate::record::Record;

/// Extract structured records from a text document.
///
/// `doc_type` and `doc_name` describe the enclosing document — they are
/// used to emit the document itself as a [`Concept`] (whose `description`
/// is the full body) and to attach a `mentions` named relation from the
/// document to every `@concept` or `@relation` endpoint it references.
pub fn extract_from_text(doc_type: &str, doc_name: &str, body: &str) -> Vec<Record> {
    let mut out = Vec::new();

    // Ensure the document's own concept type is registered (idempotent).
    out.push(Record::ConceptTypeDecl(ConceptType {
        name: doc_type.to_string(),
        parent: None,
        properties: None,
        description: String::new(),
    }));

    // The document itself.
    let mut doc = Concept::new(ConceptId(0), doc_type.to_string(), doc_name.to_string());
    doc.description = body.to_string();
    out.push(Record::Concept(doc));

    let mut mentions: Vec<(String, String)> = Vec::new();

    for raw in body.lines() {
        let line = raw.trim();
        if !line.starts_with('@') {
            continue;
        }
        // Strip a single trailing comma/period that's purely punctuation.
        let line = line.trim_end_matches(|c: char| c == ',' || c == ';');

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
                }));
                if let Some(obj) = &action.object {
                    out.push(Record::ConceptTypeDecl(ConceptType {
                        name: obj.clone(),
                        parent: None,
                        properties: None,
                        description: String::new(),
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
    })
}

fn parse_relation(s: &str) -> Option<(String, String, String, String, String)> {
    // `Type:Source -predicate-> Type:Target`
    let (left, rest) = s.split_once(" -")?;
    let (predicate, right) = rest.split_once("-> ")?;
    let (st, sn) = left.trim().split_once(':')?;
    let (tt, tn) = right.trim().split_once(':')?;
    let predicate = predicate.trim();
    if predicate.is_empty()
        || st.is_empty()
        || sn.is_empty()
        || tt.is_empty()
        || tn.is_empty()
    {
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
    fn parses_symmetric_relation_type() {
        let rt = parse_relation_type("knows: Person -> Person [symmetric]").unwrap();
        assert_eq!(rt.name, "knows");
        assert!(rt.symmetric);
    }
}
