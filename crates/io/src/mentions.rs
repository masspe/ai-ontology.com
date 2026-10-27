// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Entity linking: text that names a sheet is linked to it.
//!
//! A help page, a ticket, a README name the things they are about by the
//! names the graph already knows — a screen code, a program path, a
//! table. The linker reads the text of the sheets of some types, finds
//! those names word by word, and writes `mentions_<source>_<target>`
//! links weighted by the number of times the name occurs. A ticket is
//! then one hop from the programs it talks about, and a question about
//! a program reaches the pages that mention it.

use async_trait::async_trait;
use ontology_graph::{Cardinality, Concept, OntologyGraph, PropertyValue, RelationType};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use crate::ingest::{IngestError, Source};
use crate::record::Record;

/// Shorter words are too common to be a name (`ca`, `gl`, `id`).
const MIN_WORD: usize = 4;

/// The names worth finding, lower-cased, with their aliases: a program
/// `gl/w-gl100.w` is also written `w-gl100.w` and `w-gl100`. An alias
/// that could mean two sheets means none.
#[derive(Debug, Default)]
pub struct MentionIndex {
    names: HashMap<String, (String, String)>,
}

impl MentionIndex {
    pub fn from_graph(graph: &OntologyGraph, types: &[String]) -> Self {
        let mut names: HashMap<String, (String, String)> = HashMap::new();
        let mut ambiguous: HashSet<String> = HashSet::new();
        let mut add = |key: String, ty: &str, name: &str| {
            if key.chars().count() < MIN_WORD || ambiguous.contains(&key) {
                return;
            }
            match names.get(&key) {
                Some((t, n)) if t == ty && n == name => {}
                Some(_) => {
                    names.remove(&key);
                    ambiguous.insert(key);
                }
                None => {
                    names.insert(key, (ty.to_string(), name.to_string()));
                }
            }
        };
        for ty in types {
            for c in graph.concepts_of_type(ty, false) {
                let lower = c.name.to_lowercase();
                add(lower.clone(), ty, &c.name);
                if let Some((_, file)) = lower.rsplit_once('/') {
                    add(file.to_string(), ty, &c.name);
                    if let Some((stem, _)) = file.rsplit_once('.') {
                        add(stem.to_string(), ty, &c.name);
                    }
                }
            }
        }
        Self { names }
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// The sheets `text` names, with how many times each.
    pub fn find(&self, text: &str) -> BTreeMap<(String, String), u32> {
        let mut out: BTreeMap<(String, String), u32> = BTreeMap::new();
        for word in words(text) {
            if let Some(hit) = self.names.get(&word) {
                *out.entry(hit.clone()).or_insert(0) += 1;
            }
        }
        out
    }
}

/// Words as a name is written: letters, digits and `_ . / -` inside,
/// surrounding punctuation gone, lower-cased.
fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '.' | '/' | '-')))
        .map(|w| w.trim_matches(|c: char| matches!(c, '.' | '/' | '-')))
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
}

pub fn relation_name(source_type: &str, target_type: &str) -> String {
    format!(
        "mentions_{}_{}",
        source_type.to_lowercase(),
        target_type.to_lowercase()
    )
}

/// What the linker found.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MentionReport {
    pub scanned: usize,
    pub links: usize,
    pub targets: usize,
}

/// The `mentions_*` links of the sheets of `from` types to the sheets of
/// `to` types, read from their description and text properties.
pub struct MentionSource {
    records: VecDeque<Record>,
}

impl MentionSource {
    pub fn from_graph(
        graph: &OntologyGraph,
        from: &[String],
        to: &[String],
    ) -> (Self, MentionReport) {
        let index = MentionIndex::from_graph(graph, to);
        let mut records = VecDeque::new();
        let mut declared: HashSet<(String, String)> = HashSet::new();
        let mut report = MentionReport::default();
        let mut targets: HashSet<(String, String)> = HashSet::new();
        for src_ty in from {
            for c in graph.concepts_of_type(src_ty, false) {
                report.scanned += 1;
                let hits = index.find(&text_of(&c));
                for ((ty, name), count) in hits {
                    if ty == c.concept_type && name == c.name {
                        continue;
                    }
                    let rel = relation_name(src_ty, &ty);
                    if declared.insert((src_ty.clone(), ty.clone())) {
                        records.push_back(Record::RelationTypeDecl(RelationType {
                            name: rel.clone(),
                            domain: src_ty.clone(),
                            range: ty.clone(),
                            cardinality: Cardinality::ManyToMany,
                            description: "mentions".into(),
                            ..Default::default()
                        }));
                    }
                    targets.insert((ty.clone(), name.clone()));
                    report.links += 1;
                    records.push_back(Record::NamedRelation {
                        relation_type: rel,
                        source_type: src_ty.clone(),
                        source_name: c.name.clone(),
                        target_type: ty,
                        target_name: name,
                        weight: count as f32,
                        properties: Default::default(),
                    });
                }
            }
        }
        report.targets = targets.len();
        (Self { records }, report)
    }
}

/// A sheet's words: its description and every text property.
fn text_of(c: &Concept) -> String {
    let mut s = c.description.clone();
    for v in c.properties.values() {
        if let PropertyValue::Text(t) = v {
            s.push(' ');
            s.push_str(t);
        }
    }
    s
}

#[async_trait]
impl Source for MentionSource {
    async fn next(&mut self) -> Result<Option<Record>, IngestError> {
        Ok(self.records.pop_front())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_keep_what_a_name_is_made_of() {
        let w: Vec<String> =
            words("Voir gl/w-gl100.w (gl100), puis appdb.emp.cono ; fin.").collect();
        assert_eq!(
            w,
            [
                "voir",
                "gl/w-gl100.w",
                "gl100",
                "puis",
                "appdb.emp.cono",
                "fin"
            ]
        );
    }

    #[test]
    fn relation_names_are_per_type_pair() {
        assert_eq!(
            relation_name("Document", "Screen"),
            "mentions_document_screen"
        );
    }
}
