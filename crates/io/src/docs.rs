// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Documents attached to sheets by key.
//!
//! A folder of pages — user help, READMEs, specifications — where each
//! file is *about* one sheet the graph already has: the screen a help page
//! documents, the module a README describes. The file's text becomes a
//! property of that sheet and its title and first lines the sheet's
//! description, so the words users say reach retrieval and the prompt.
//! A file about nothing the graph knows becomes a document sheet of its
//! own when a fallback type is given, or is counted and skipped.

use async_trait::async_trait;
use ontology_graph::{Concept, ConceptId, ConceptType, PropertyValue};
use regex::Regex;
use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::sync::LazyLock;

use crate::charset::decode_to_utf8;
use crate::extract::{extract_from_text_chunked, DEFAULT_CHUNK_CHARS, EXCERPT_CHARS};
use crate::ingest::{IngestError, Source};
use crate::record::Record;
use crate::scan::walk_files;

/// How documents find their sheets.
#[derive(Debug, Clone)]
pub struct DocOptions {
    /// The type of sheet the documents are about.
    pub concept_type: String,
    /// First capture group, applied to the document's title then to its
    /// file name, is the sheet's name; `None` means the file name is.
    pub key: Option<Regex>,
    /// What a document matching no sheet becomes; `None` skips it.
    pub fallback_type: Option<String>,
    /// The property that holds the document's text.
    pub property: String,
    /// The text is cut here: the index reads it, the prompt shows little.
    pub max_chars: usize,
    /// Fragment size of the fallback documents (0 keeps them whole).
    pub chunk_chars: usize,
}

impl DocOptions {
    pub fn new(concept_type: impl Into<String>) -> Self {
        Self {
            concept_type: concept_type.into(),
            key: None,
            fallback_type: None,
            property: "doc".into(),
            max_chars: 8_000,
            chunk_chars: DEFAULT_CHUNK_CHARS,
        }
    }
}

/// What the folder held.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DocReport {
    pub files: usize,
    pub attached: usize,
    /// Documents about no sheet: became fallback sheets, or were skipped.
    pub unmatched: usize,
    pub errors: Vec<String>,
}

static TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<title[^>]*>(.*?)</title>").unwrap());
static DROP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<!--.*?-->|<script\b.*?</script>|<style\b.*?</style>|<noscript\b.*?</noscript>|<head\b.*?</head>").unwrap()
});
static BREAK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)</?(p|div|br|tr|li|h[1-6]|table|ul|ol)\b[^>]*>").unwrap());
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<[^>]*>").unwrap());
static ENTITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"&(#x[0-9a-fA-F]+|#[0-9]+|[a-zA-Z]+);").unwrap());

/// `&#x00E9;`, `&#233;`, and the named entities a help page uses.
pub fn decode_entities(s: &str) -> String {
    ENTITY
        .replace_all(s, |c: &regex::Captures| {
            let e = &c[1];
            let code = if let Some(h) = e.strip_prefix("#x") {
                u32::from_str_radix(h, 16).ok()
            } else if let Some(d) = e.strip_prefix('#') {
                d.parse().ok()
            } else {
                match e {
                    "amp" => Some('&' as u32),
                    "lt" => Some('<' as u32),
                    "gt" => Some('>' as u32),
                    "quot" => Some('"' as u32),
                    "apos" => Some('\'' as u32),
                    "nbsp" => Some(' ' as u32),
                    "eacute" => Some('é' as u32),
                    "egrave" => Some('è' as u32),
                    "ecirc" => Some('ê' as u32),
                    "agrave" => Some('à' as u32),
                    "ccedil" => Some('ç' as u32),
                    "ocirc" => Some('ô' as u32),
                    "ucirc" => Some('û' as u32),
                    "icirc" => Some('î' as u32),
                    "euml" => Some('ë' as u32),
                    "uuml" => Some('ü' as u32),
                    "ouml" => Some('ö' as u32),
                    "auml" => Some('ä' as u32),
                    "laquo" => Some('«' as u32),
                    "raquo" => Some('»' as u32),
                    "times" => Some('×' as u32),
                    "copy" => Some('©' as u32),
                    "hellip" => Some('…' as u32),
                    "rsquo" => Some('’' as u32),
                    "ndash" => Some('–' as u32),
                    _ => None,
                }
            };
            match code.and_then(char::from_u32) {
                Some(ch) => ch.to_string(),
                None => c[0].to_string(),
            }
        })
        .into_owned()
}

/// The page's title and its text: scripts, styles and tags gone, block
/// ends as line breaks, entities decoded, whitespace folded.
pub fn html_to_text(html: &str) -> (Option<String>, String) {
    let title = TITLE
        .captures(html)
        .map(|c| fold_spaces(&decode_entities(&c[1])))
        .filter(|t| !t.is_empty());
    let body = DROP.replace_all(html, " ");
    let body = BREAK.replace_all(&body, "\n");
    let body = TAG.replace_all(&body, " ");
    let text = decode_entities(&body);
    (title, fold_lines(&text))
}

fn fold_spaces(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Lines folded to one space each, blank runs to one blank line.
fn fold_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank = 0;
    for line in s.lines() {
        let line = fold_spaces(line);
        if line.is_empty() {
            blank += 1;
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blank > 0 { "\n\n" } else { "\n" });
        }
        blank = 0;
        out.push_str(&line);
    }
    out
}

/// The first `max` characters, cut at a character boundary.
fn head(s: &str, max: usize) -> String {
    let mut it = s.char_indices();
    match it.nth(max) {
        Some((i, _)) => format!("{}…", s[..i].trim_end()),
        None => s.to_string(),
    }
}

/// The sheet's description: the title, then the text from where the
/// title last appears in it (its heading — past a page's own header,
/// breadcrumb and navigation), or from its start.
fn summary(title: Option<&str>, text: &str) -> String {
    let from = title
        .and_then(|t| text.rfind(t).map(|i| i + t.len()))
        .unwrap_or(0);
    let body = fold_spaces(&text[from..]);
    match title {
        Some(t) if body.is_empty() => t.to_string(),
        Some(t) => format!("{t}. {}", head(&body, EXCERPT_CHARS)),
        None => head(&body, EXCERPT_CHARS),
    }
}

fn is_html(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("htm") || e.eq_ignore_ascii_case("html"))
}

/// One document per file, attached to its sheet.
pub struct DocSource {
    records: VecDeque<Record>,
}

impl DocSource {
    /// `names`: the sheets of the type, lower-cased name → name as stored;
    /// `declared`: the properties the type declares today, kept beside the
    /// document's (a declaration that names properties replaces the list).
    pub fn from_dir(
        dir: &Path,
        opts: &DocOptions,
        names: &BTreeMap<String, String>,
        declared: &[String],
    ) -> Result<(Self, DocReport), IngestError> {
        let mut files = walk_files(dir, |p| {
            p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                ["htm", "html", "txt", "md"]
                    .iter()
                    .any(|w| w.eq_ignore_ascii_case(e))
            })
        })
        .map_err(|e| IngestError::Source(format!("{}: {e}", dir.display())))?;
        files.sort();
        let mut report = DocReport {
            files: files.len(),
            ..Default::default()
        };
        let mut records = VecDeque::new();
        // Two pages may share a title ("Comptabilisations" in two chapters):
        // the second fallback sheet carries its file name too.
        let mut fallback_names: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        // Sheet name → (description, the pages' text), in file order.
        let mut attached: BTreeMap<String, (String, String)> = BTreeMap::new();
        // The property the texts land in, added to the type (a merge: the
        // type keeps everything it already declares).
        let mut properties: Vec<String> = declared.to_vec();
        if !properties.contains(&opts.property) {
            properties.push(opts.property.clone());
        }
        records.push_back(Record::ConceptTypeDecl(ConceptType {
            name: opts.concept_type.clone(),
            properties: Some(properties),
            ..Default::default()
        }));
        for path in files {
            let raw = match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    report.errors.push(format!("{}: {e}", path.display()));
                    continue;
                }
            };
            let decoded = decode_to_utf8(&raw).text;
            let (title, text) = if is_html(&path) {
                html_to_text(&decoded)
            } else {
                (None, fold_lines(&decoded))
            };
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            let key = match &opts.key {
                Some(re) => title
                    .as_deref()
                    .and_then(|t| re.captures(t))
                    .or_else(|| re.captures(&stem))
                    .and_then(|c| c.get(1).map(|m| m.as_str().to_lowercase())),
                None => Some(stem.to_lowercase()),
            };
            match key.and_then(|k| names.get(&k)) {
                Some(name) => {
                    // Several pages about one sheet: the first gives the
                    // description, the texts follow each other.
                    let entry = attached
                        .entry(name.clone())
                        .or_insert_with(|| (summary(title.as_deref(), &text), String::new()));
                    if !entry.1.is_empty() {
                        entry.1.push_str("\n\n");
                    }
                    entry.1.push_str(&text);
                    report.attached += 1;
                }
                None => {
                    report.unmatched += 1;
                    if let Some(ty) = &opts.fallback_type {
                        let mut name = title.clone().unwrap_or_else(|| stem.clone());
                        if !fallback_names.insert(name.to_lowercase()) {
                            name = format!("{name} — {stem}");
                            fallback_names.insert(name.to_lowercase());
                        }
                        records.extend(extract_from_text_chunked(
                            ty,
                            &name,
                            &text,
                            opts.chunk_chars,
                        ));
                    }
                }
            }
        }
        for (name, (description, text)) in attached {
            let mut c = Concept::new(ConceptId(0), &opts.concept_type, &name);
            c.description = description;
            c = c.with_property(
                &opts.property,
                PropertyValue::from(head(&text, opts.max_chars)),
            );
            records.push_back(Record::Concept(c));
        }
        Ok((Self { records }, report))
    }

    /// The sheets of the type taken from the graph.
    pub fn from_graph(
        dir: &Path,
        opts: &DocOptions,
        graph: &ontology_graph::OntologyGraph,
    ) -> Result<(Self, DocReport), IngestError> {
        let names = graph
            .concepts_of_type(&opts.concept_type, false)
            .into_iter()
            .map(|c| (c.name.to_lowercase(), c.name))
            .collect();
        let declared = graph.with_ontology(|o| {
            o.concept_types
                .get(&opts.concept_type)
                .and_then(|t| t.properties.clone())
                .unwrap_or_default()
        });
        Self::from_dir(dir, opts, &names, &declared)
    }
}

#[async_trait]
impl Source for DocSource {
    async fn next(&mut self) -> Result<Option<Record>, IngestError> {
        Ok(self.records.pop_front())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<html><head><title>1 - Param&#x00E9;trage CA (gl100)</title>
<script>var x = "<p>not text</p>";</script><style>.a{}</style></head>
<body><!-- a comment --><div>MS2000 - Guide de l'utilisateur</div>
<table><tr><td>Menu</td><td>&nbsp;</td></tr></table>
<h1>1 - Param&#x00E9;trage CA (gl100)</h1>
<p>Cet &eacute;cran permet de <b>param&#233;trer</b> la comptabilit&#x00E9; analytique.</p>
<p>Deuxi&egrave;me paragraphe &amp; fin.</p></body></html>"#;

    #[test]
    fn html_becomes_title_and_text() {
        let (title, text) = html_to_text(PAGE);
        assert_eq!(title.as_deref(), Some("1 - Paramétrage CA (gl100)"));
        assert!(!text.contains("not text") && !text.contains(".a{}") && !text.contains("comment"));
        assert!(
            text.contains("Cet écran permet de paramétrer la comptabilité analytique."),
            "{text}"
        );
        assert!(text.contains("Deuxième paragraphe & fin."));
        assert!(
            text.starts_with("MS2000 - Guide de l'utilisateur\n\nMenu"),
            "{text:?}"
        );
    }

    #[test]
    fn the_summary_starts_after_the_title_reappears() {
        let (title, text) = html_to_text(PAGE);
        let s = summary(title.as_deref(), &text);
        assert!(
            s.starts_with("1 - Paramétrage CA (gl100). Cet écran permet"),
            "{s}"
        );
        assert!(!s.contains("Guide de l'utilisateur"));
        assert_eq!(summary(Some("T"), ""), "T");
        assert_eq!(summary(None, "a  b\n\nc"), "a b c");
        let long = "x".repeat(700);
        assert_eq!(summary(None, &long).chars().count(), EXCERPT_CHARS + 1);
    }

    #[test]
    fn entities_decode_and_unknown_ones_stay() {
        assert_eq!(
            decode_entities("&eacute;&#233;&#xE9;&laquo;&amp;&bogus;"),
            "ééé«&&bogus;"
        );
        assert_eq!(head("héllo", 2), "hé…");
        assert_eq!(head("hé", 5), "hé");
    }
}
