// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

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
                    // A date-formatted cell is a serial number in the file;
                    // keep it readable and sortable as ISO text.
                    calamine::Data::DateTime(dt) => PropertyValue::Text(excel_datetime_text(&dt)),
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

/// Flatten every sheet of a spreadsheet into plain text for the LLM-assisted
/// analysis (`POST /ingest/analyze`): a `# Sheet: <name>` heading per sheet,
/// then one line per non-empty data row as `header: value; header: value`.
/// The first non-empty row of a sheet is its header; a blank header cell is
/// named by position (`col3`). Empty cells are omitted.
pub fn spreadsheet_to_text(path: impl AsRef<Path>) -> Result<String, IngestError> {
    let mut workbook =
        open_workbook_auto(path.as_ref()).map_err(|e| IngestError::Source(e.to_string()))?;
    let names: Vec<String> = workbook.sheet_names().to_vec();
    if names.is_empty() {
        return Err(IngestError::Source(
            "spreadsheet: workbook has no sheets".into(),
        ));
    }
    let mut out = String::new();
    for name in names {
        let range = workbook
            .worksheet_range(&name)
            .map_err(|e| IngestError::Source(format!("spreadsheet: {e}")))?;
        let mut rows = range
            .rows()
            .filter(|r| !r.iter().all(|c| matches!(c, calamine::Data::Empty)));
        let Some(header_row) = rows.next() else {
            continue;
        };
        let header: Vec<String> = header_row
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let h = c.to_string().trim().to_string();
                if h.is_empty() {
                    format!("col{}", i + 1)
                } else {
                    h
                }
            })
            .collect();
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!("# Sheet: {name}\n"));
        for row in rows {
            let cells: Vec<String> = row
                .iter()
                .enumerate()
                .filter_map(|(i, c)| {
                    let v = cell_text(c)?;
                    let h = header
                        .get(i)
                        .cloned()
                        .unwrap_or_else(|| format!("col{}", i + 1));
                    Some(format!("{h}: {v}"))
                })
                .collect();
            if !cells.is_empty() {
                out.push_str(&cells.join("; "));
                out.push('\n');
            }
        }
    }
    Ok(out)
}

/// One cell as text: numbers without a spurious `.0`, dates as ISO
/// (a date cell is a serial number in the file), empty cells as `None`.
fn cell_text(c: &calamine::Data) -> Option<String> {
    let s = match c {
        calamine::Data::Empty => return None,
        calamine::Data::DateTime(dt) => excel_datetime_text(dt),
        calamine::Data::Float(f) => f.to_string(),
        calamine::Data::Int(i) => i.to_string(),
        calamine::Data::Bool(b) => b.to_string(),
        other => other.to_string(),
    };
    let s = s.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// `YYYY-MM-DD`, with `THH:MM:SS` appended when the cell carries a time
/// of day. A value below one day has no date part (a time or a short
/// duration) and is rendered as `HH:MM:SS`. Without calamine's `dates`
/// feature only the raw serial is available, hence the arithmetic below.
fn excel_datetime_text(dt: &calamine::ExcelDateTime) -> String {
    let v = dt.as_f64();
    if (0.0..1.0).contains(&v) {
        let secs = (v * 86_400.0).round() as i64;
        return format!(
            "{:02}:{:02}:{:02}",
            secs / 3600,
            (secs / 60) % 60,
            secs % 60
        );
    }
    excel_serial_to_iso(v)
}

/// Excel's 1900 date system: day 25569 is 1970-01-01. Calendar arithmetic
/// after H. Hinnant's `civil_from_days`, so no date crate is needed.
fn excel_serial_to_iso(serial: f64) -> String {
    let days = serial.floor();
    let frac = serial - days;
    let z = days as i64 - 25569 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let mut out = format!("{y:04}-{m:02}-{d:02}");
    let secs = (frac * 86_400.0).round() as i64;
    if secs > 0 && secs < 86_400 {
        out.push_str(&format!(
            "T{:02}:{:02}:{:02}",
            secs / 3600,
            (secs / 60) % 60,
            secs % 60
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excel_serials_become_iso_dates() {
        assert_eq!(excel_serial_to_iso(45747.0), "2025-03-31");
        assert_eq!(excel_serial_to_iso(45838.0), "2025-06-30");
        assert_eq!(excel_serial_to_iso(25569.0), "1970-01-01");
        assert_eq!(excel_serial_to_iso(60.0), "1900-02-28"); // Excel's phantom leap day is above this
        assert_eq!(excel_serial_to_iso(45747.5), "2025-03-31T12:00:00");
        assert_eq!(excel_serial_to_iso(45747.75), "2025-03-31T18:00:00");
    }

    #[test]
    fn finance_invoices_carry_dates_and_amounts_as_rows() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/finance/invoices.xlsx"
        );
        let mut src = XlsxSource::open(path, "Invoice").unwrap();
        let mut seen = 0;
        while let Some(Record::Concept(c)) = src.pending.pop_front() {
            seen += 1;
            assert!(
                matches!(
                    c.properties.get("amount_eur"),
                    Some(PropertyValue::Number(_))
                ),
                "{}: {:?}",
                c.name,
                c.properties
            );
            let date = c
                .properties
                .get("issue_date")
                .and_then(|v| v.as_text())
                .unwrap();
            assert!(date.starts_with("2025-"), "issue_date as ISO, got {date}");
        }
        assert!(seen >= 3);
    }

    #[test]
    fn finance_invoices_flatten_to_one_line_per_row() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/finance/invoices.xlsx"
        );
        let text = spreadsheet_to_text(path).unwrap();
        assert!(text.starts_with("# Sheet: "), "{text}");
        let rows: Vec<&str> = text.lines().filter(|l| l.contains("name: ")).collect();
        assert!(rows.len() >= 3, "expected the invoice rows, got:\n{text}");
        assert!(
            rows.iter().all(|l| l.contains("; ")),
            "every row carries several `header: value` cells:\n{text}"
        );
        assert!(!text.contains("\u{0}"), "no control characters");
        assert!(
            text.contains("amount_eur: 18000"),
            "amounts as plain numbers:\n{text}"
        );
        assert!(
            text.contains("issue_date: 2025-"),
            "dates as ISO, not serials:\n{text}"
        );
    }

    #[test]
    fn a_non_spreadsheet_is_an_error_not_a_panic() {
        let dir = std::env::temp_dir();
        let p = dir.join(format!("not-a-sheet-{}.xlsx", std::process::id()));
        std::fs::write(&p, b"hello, this is text").unwrap();
        assert!(spreadsheet_to_text(&p).is_err());
        let _ = std::fs::remove_file(&p);
    }
}
