// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

use async_trait::async_trait;
use ontology_graph::{Concept, ConceptId, PropertyValue};
use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, BufReader, Lines};

use crate::ingest::{IngestError, Source};
use crate::record::Record;

/// Minimal CSV ingester. The first row is the header; one column must be
/// `name`, every other named column becomes a [`PropertyValue::Text`] on
/// the resulting [`Concept`]. The concept type is fixed at construction
/// time, so a single CSV imports a homogeneous batch.
///
/// Quoted fields with embedded commas are supported (`"a,b"`); embedded
/// quotes use the doubled-quote convention (`""`). Anything beyond that
/// (record-level escaping, alternate delimiters) is out of scope — wire
/// up a real CSV crate if you need it.
pub struct CsvSource {
    lines: Lines<BufReader<File>>,
    concept_type: String,
    header: Option<Vec<String>>,
    name_col: Option<usize>,
}

impl CsvSource {
    pub async fn open(
        path: impl AsRef<Path>,
        concept_type: impl Into<String>,
    ) -> Result<Self, IngestError> {
        let f = File::open(path.as_ref()).await?;
        Ok(Self {
            lines: BufReader::new(f).lines(),
            concept_type: concept_type.into(),
            header: None,
            name_col: None,
        })
    }
}

#[async_trait]
impl Source for CsvSource {
    async fn next(&mut self) -> Result<Option<Record>, IngestError> {
        loop {
            let line = match self.lines.next_line().await? {
                Some(l) => l,
                None => return Ok(None),
            };
            if line.trim().is_empty() {
                continue;
            }
            let row = parse_csv_row(&line);

            if self.header.is_none() {
                let name_col = row
                    .iter()
                    .position(|h| h.eq_ignore_ascii_case("name"))
                    .ok_or_else(|| {
                        IngestError::Source("csv: expected a `name` column in the header".into())
                    })?;
                self.name_col = Some(name_col);
                self.header = Some(row);
                continue;
            }

            let header = self.header.as_ref().unwrap();
            let name_col = self.name_col.unwrap();
            if row.len() != header.len() {
                return Err(IngestError::Source(format!(
                    "csv: row has {} columns, expected {}",
                    row.len(),
                    header.len(),
                )));
            }
            let name = row.get(name_col).cloned().unwrap_or_default();
            if name.is_empty() {
                return Err(IngestError::Source("csv: empty `name` cell".into()));
            }

            let mut c = Concept::new(ConceptId(0), self.concept_type.clone(), name);
            for (i, col) in header.iter().enumerate() {
                if i == name_col {
                    continue;
                }
                let v = row.get(i).cloned().unwrap_or_default();
                if col.eq_ignore_ascii_case("description") {
                    c.description = v;
                } else if !v.is_empty() {
                    c.properties.insert(col.clone(), PropertyValue::Text(v));
                }
            }
            return Ok(Some(Record::Concept(c)));
        }
    }
}

fn parse_csv_row(line: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, in_quotes) {
            ('"', true) => {
                if chars.peek() == Some(&'"') {
                    cur.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            }
            ('"', false) if cur.is_empty() => in_quotes = true,
            (',', false) => {
                out.push(std::mem::take(&mut cur));
            }
            (other, _) => cur.push(other),
        }
    }
    out.push(cur);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_commas() {
        let row = parse_csv_row(r#"a,"b,c","d""e",f"#);
        assert_eq!(row, vec!["a", "b,c", r#"d"e"#, "f"]);
    }
}
