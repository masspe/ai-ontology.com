// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! A CSV file — on disk or at an HTTPS URL, fetched on every read — as
//! one table named `data`, every column text, the first line the
//! header. The separator is `;` when the header has more of them than
//! `,` (Swiss and French exports), `,` otherwise.

use async_trait::async_trait;
use serde_json::Value;

use super::{ColumnInfo, ConnectError, Connector, Query, ResultSet, TableInfo};
use crate::charset::decode_to_utf8;

#[derive(Debug, Clone)]
pub struct CsvConnector {
    /// A path, or `http://` / `https://`.
    pub location: String,
}

impl CsvConnector {
    pub fn new(location: impl Into<String>) -> Self {
        Self {
            location: location.into(),
        }
    }

    fn is_url(&self) -> bool {
        let l = self.location.to_ascii_lowercase();
        l.starts_with("https://") || l.starts_with("http://")
    }

    async fn read(&self) -> Result<String, ConnectError> {
        let raw = if self.is_url() {
            let resp = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .map_err(|e| ConnectError::Connect(e.to_string()))?
                .get(&self.location)
                .send()
                .await
                .and_then(|r| r.error_for_status())
                .map_err(|e| ConnectError::Connect(e.to_string()))?;
            resp.bytes()
                .await
                .map_err(|e| ConnectError::Connect(e.to_string()))?
                .to_vec()
        } else {
            tokio::fs::read(&self.location)
                .await
                .map_err(|e| ConnectError::Connect(format!("{}: {e}", self.location)))?
        };
        Ok(decode_to_utf8(&raw).text)
    }

    fn parse(text: &str) -> (Vec<ColumnInfo>, Vec<Vec<Value>>) {
        let mut lines = text.lines().filter(|l| !l.trim().is_empty());
        let Some(header) = lines.next() else {
            return (Vec::new(), Vec::new());
        };
        let sep = if header.matches(';').count() > header.matches(',').count() {
            ';'
        } else {
            ','
        };
        let columns: Vec<ColumnInfo> = split(header, sep)
            .into_iter()
            .map(|c| ColumnInfo {
                name: c.trim().to_string(),
                data_type: "text".into(),
            })
            .collect();
        let rows = lines
            .map(|l| {
                let mut cells: Vec<Value> = split(l, sep)
                    .into_iter()
                    .map(|c| {
                        if c.is_empty() {
                            Value::Null
                        } else {
                            Value::String(c)
                        }
                    })
                    .collect();
                cells.resize(columns.len(), Value::Null);
                cells
            })
            .collect();
        (columns, rows)
    }
}

/// Split one line on `sep`, honouring `"…"` quotes with `""` escapes.
fn split(line: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            c if c == sep && !quoted => out.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    out.push(cur);
    out.iter_mut().for_each(|s| *s = s.trim().to_string());
    out
}

#[async_trait]
impl Connector for CsvConnector {
    async fn ping(&self) -> Result<(), ConnectError> {
        self.read().await.map(|_| ())
    }

    async fn tables(&self) -> Result<Vec<TableInfo>, ConnectError> {
        self.ping().await?;
        Ok(vec![TableInfo {
            schema: None,
            name: "data".into(),
        }])
    }

    async fn columns(&self, _table: &str) -> Result<Vec<ColumnInfo>, ConnectError> {
        let text = self.read().await?;
        Ok(Self::parse(&text).0)
    }

    async fn query(&self, q: &Query) -> Result<ResultSet, ConnectError> {
        if q.is_statement() {
            return Err(ConnectError::Unsupported(
                "a SELECT statement on a CSV".into(),
            ));
        }
        let text = self.read().await?;
        let (columns, mut rows) = Self::parse(&text);
        if let Some((col, last)) = &q.after {
            let i = columns
                .iter()
                .position(|c| c.name.eq_ignore_ascii_case(col))
                .ok_or_else(|| ConnectError::Query(format!("no column `{col}`")))?;
            let last = last
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| last.to_string());
            rows.retain(|r| r[i].as_str().is_some_and(|v| v > last.as_str()));
            rows.sort_by(|a, b| a[i].as_str().cmp(&b[i].as_str()));
        }
        let truncated = q.max_rows > 0 && rows.len() > q.max_rows;
        if truncated {
            rows.truncate(q.max_rows);
        }
        Ok(ResultSet {
            columns,
            rows,
            truncated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_file_is_one_table_with_quotes_separators_and_a_watermark() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.csv");
        std::fs::write(
            &path,
            "id;name;updated\n1;\"Dupont; Jean\";2026-01-01\n2;\"say \"\"hi\"\"\";2026-01-03\n\n3;;2026-01-02\n",
        )
        .unwrap();
        let c = CsvConnector::new(path.to_string_lossy().into_owned());
        c.ping().await.unwrap();
        assert_eq!(c.tables().await.unwrap()[0].name, "data");
        let cols: Vec<String> = c
            .columns("data")
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(cols, ["id", "name", "updated"]);
        let rs = c
            .query(&Query {
                from: "data".into(),
                after: None,
                max_rows: 0,
            })
            .await
            .unwrap();
        assert_eq!(rs.rows.len(), 3);
        assert_eq!(rs.rows[0][1], Value::String("Dupont; Jean".into()));
        assert_eq!(rs.rows[1][1], Value::String("say \"hi\"".into()));
        assert_eq!(rs.rows[2][1], Value::Null, "an empty cell is null");
        let later = c
            .query(&Query {
                from: "data".into(),
                after: Some(("updated".into(), Value::String("2026-01-01".into()))),
                max_rows: 1,
            })
            .await
            .unwrap();
        assert_eq!(later.rows.len(), 1);
        assert!(later.truncated);
        assert_eq!(
            later.rows[0][0],
            Value::String("3".into()),
            "ordered by the watermark"
        );
        assert!(c
            .query(&Query {
                from: "select 1".into(),
                after: None,
                max_rows: 0
            })
            .await
            .is_err());
        assert!(CsvConnector::new(
            dir.path()
                .join("missing.csv")
                .to_string_lossy()
                .into_owned()
        )
        .ping()
        .await
        .is_err());
        let (cols, rows) = CsvConnector::parse("a,b\n1,2\n");
        assert_eq!(cols.len(), 2);
        assert_eq!(
            rows,
            vec![vec![Value::String("1".into()), Value::String("2".into())]]
        );
        assert!(CsvConnector::parse("").0.is_empty());
    }
}
