// SPDX-License-Identifier: LGPL-3.0-or-later
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// It is licensed under the GNU Lesser General Public License v3.0
// or (at your option) any later version. See LICENSE and LICENSE.GPL.

use async_trait::async_trait;
use calamine::{open_workbook_auto, Reader};
use ontology_graph::{Concept, ConceptId, PropertyValue};
use std::collections::VecDeque;
use std::path::Path;

use crate::ingest::{IngestError, Source};
use crate::record::Record;

/// Excel ingester. Reads the first sheet of a `.xlsx` / `.xls` / `.ods`
/// workbook (whatever `calamine` understands) and emits one
/// [`Record::Concept`] per row, mirroring the [`CsvSource`] semantics:
///
/// * The first row is the header. One column must be named `name` (case
///   insensitive); it becomes the concept's `name`.
/// * A column named `description` (case insensitive) becomes the concept's
///   `description`.
/// * Every other named column becomes a property:
///     - numeric cells → [`PropertyValue::Number`]
///     - boolean cells → [`PropertyValue::Bool`]
///     - everything else (strings, datetimes, formula results) → coerced
///       to text via `Display` and stored as [`PropertyValue::Text`].
/// * Empty cells are dropped (no property emitted).
///
/// `concept_type` is fixed at construction time, so a single workbook
/// produces a homogeneous batch — load multiple times against different
/// types if you have multi-sheet data.
///
/// [`CsvSource`]: crate::CsvSource
pub struct XlsxSource {
    pending: VecDeque<Record>,
}

impl XlsxSource {
    pub fn open(
        path: impl AsRef<Path>,
        concept_type: impl Into<String>,
    ) -> Result<Self, IngestError> {
        let concept_type = concept_type.into();
        let mut workbook =
            open_workbook_auto(path.as_ref()).map_err(|e| IngestError::Source(e.to_string()))?;
        let sheet_name = workbook
            .sheet_names()
            .first()
            .cloned()
            .ok_or_else(|| IngestError::Source("xlsx: workbook has no sheets".into()))?;
        let range = workbook
            .worksheet_range(&sheet_name)
            .map_err(|e| IngestError::Source(format!("xlsx: {e}")))?;

        let mut rows = range.rows();
        let header_row = rows
            .next()
            .ok_or_else(|| IngestError::Source("xlsx: empty sheet".into()))?;
        let header: Vec<String> = header_row
            .iter()
            .map(|c| c.to_string().trim().to_string())
            .collect();
        let name_col = header
            .iter()
            .position(|h| h.eq_ignore_ascii_case("name"))
            .ok_or_else(|| IngestError::Source("xlsx: expected a `name` header column".into()))?;

        let mut pending: VecDeque<Record> = VecDeque::new();
        for row in rows {
            // Skip rows that are entirely empty.
            if row.iter().all(|c| matches!(c, calamine::Data::Empty)) {
                continue;
            }
            let name_cell = row.get(name_col).cloned().unwrap_or(calamine::Data::Empty);
            let name = match name_cell {
                calamine::Data::Empty => continue, // ignore rows without a name
                other => other.to_string().trim().to_string(),
            };
            if name.is_empty() {
                continue;
            }

            let mut c = Concept::new(ConceptId(0), concept_type.clone(), name);
            for (i, col_name) in header.iter().enumerate() {
                if i == name_col || col_name.is_empty() {
                    continue;
                }
                let cell = row.get(i).cloned().unwrap_or(calamine::Data::Empty);
                let pv = match cell {
                    calamine::Data::Empty => continue,
                    calamine::Data::Bool(b) => PropertyValue::Bool(b),
                    calamine::Data::Float(f) => PropertyValue::Number(f),
                    calamine::Data::Int(i) => PropertyValue::Number(i as f64),
                    other => {
                        let s = other.to_string();
                        if s.trim().is_empty() {
                            continue;
                        }
                        PropertyValue::Text(s)
                    }
                };
                if col_name.eq_ignore_ascii_case("description") {
                    if let PropertyValue::Text(t) = pv {
                        c.description = t;
                    }
                } else {
                    c.properties.insert(col_name.clone(), pv);
                }
            }
            pending.push_back(Record::Concept(c));
        }

        Ok(Self { pending })
    }
}

#[async_trait]
impl Source for XlsxSource {
    async fn next(&mut self) -> Result<Option<Record>, IngestError> {
        Ok(self.pending.pop_front())
    }
}
