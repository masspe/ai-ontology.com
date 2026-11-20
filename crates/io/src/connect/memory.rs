// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Tables held in memory: what the sync and the API are tested against
//! without a database. Understands a table name and the watermark; a
//! `SELECT` statement is refused.

use std::collections::BTreeMap;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::Value;

use super::{ColumnInfo, ConnectError, Connector, Query, ResultSet, TableInfo};

/// A table: its columns and its rows.
type Table = (Vec<ColumnInfo>, Vec<Vec<Value>>);

#[derive(Debug, Default)]
pub struct MemoryConnector {
    tables: Mutex<BTreeMap<String, Table>>,
    /// Canned results of `SELECT` statements, by their exact text.
    statements: Mutex<BTreeMap<String, Table>>,
    /// Every `ping` fails with this: a connector that cannot be reached.
    down: Option<String>,
}

impl MemoryConnector {
    pub fn new() -> Self {
        Self::default()
    }

    /// A connector whose every call fails with `why`.
    pub fn down(why: &str) -> Self {
        Self {
            tables: Mutex::new(BTreeMap::new()),
            statements: Mutex::new(BTreeMap::new()),
            down: Some(why.to_string()),
        }
    }

    /// The result of a `SELECT` statement, given by its exact text (what
    /// a named query renders after its parameters are bound).
    pub fn statement(self, sql: &str, columns: &[&str], rows: Vec<Vec<Value>>) -> Self {
        self.statements.lock().insert(
            sql.trim().to_string(),
            (
                columns
                    .iter()
                    .map(|c| ColumnInfo {
                        name: c.to_string(),
                        data_type: "any".into(),
                    })
                    .collect(),
                rows,
            ),
        );
        self
    }

    /// Add (or replace) a table: column names and rows of JSON values.
    pub fn table(self, name: &str, columns: &[&str], rows: Vec<Vec<Value>>) -> Self {
        self.tables.lock().insert(
            name.to_string(),
            (
                columns
                    .iter()
                    .map(|c| ColumnInfo {
                        name: c.to_string(),
                        data_type: "any".into(),
                    })
                    .collect(),
                rows,
            ),
        );
        self
    }

    /// Append rows to an existing table (a later sync sees them).
    pub fn append(&self, name: &str, rows: Vec<Vec<Value>>) {
        if let Some((_, existing)) = self.tables.lock().get_mut(name) {
            existing.extend(rows);
        }
    }

    /// Remove the rows whose first column equals `key`.
    pub fn remove(&self, name: &str, key: &Value) {
        if let Some((_, rows)) = self.tables.lock().get_mut(name) {
            rows.retain(|r| r.first() != Some(key));
        }
    }

    fn check(&self) -> Result<(), ConnectError> {
        match &self.down {
            Some(why) => Err(ConnectError::Connect(why.clone())),
            None => Ok(()),
        }
    }
}

/// `a > b` for the JSON values a watermark can be: numbers by value,
/// strings and the rest by their text.
fn greater(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x > y,
        _ => {
            a.as_str()
                .map(str::to_string)
                .unwrap_or_else(|| a.to_string())
                > b.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| b.to_string())
        }
    }
}

#[async_trait]
impl Connector for MemoryConnector {
    async fn ping(&self) -> Result<(), ConnectError> {
        self.check()
    }

    async fn tables(&self) -> Result<Vec<TableInfo>, ConnectError> {
        self.check()?;
        Ok(self
            .tables
            .lock()
            .keys()
            .map(|n| TableInfo {
                schema: None,
                name: n.clone(),
            })
            .collect())
    }

    async fn columns(&self, table: &str) -> Result<Vec<ColumnInfo>, ConnectError> {
        self.check()?;
        self.tables
            .lock()
            .get(table)
            .map(|(c, _)| c.clone())
            .ok_or_else(|| ConnectError::Query(format!("no table `{table}`")))
    }

    async fn query(&self, q: &Query) -> Result<ResultSet, ConnectError> {
        self.check()?;
        if q.is_statement() {
            let key = q.from.trim().trim_end_matches(';').to_string();
            let Some((columns, rows)) = self.statements.lock().get(&key).cloned() else {
                return Err(ConnectError::Unsupported(format!(
                    "no canned result for `{key}`"
                )));
            };
            let truncated = q.max_rows > 0 && rows.len() > q.max_rows;
            let mut rows = rows;
            if truncated {
                rows.truncate(q.max_rows);
            }
            return Ok(ResultSet {
                columns,
                rows,
                truncated,
            });
        }
        let tables = self.tables.lock();
        let (columns, rows) = tables
            .get(&q.from)
            .ok_or_else(|| ConnectError::Query(format!("no table `{}`", q.from)))?;
        let mut rows: Vec<Vec<Value>> = match &q.after {
            Some((col, last)) => {
                let i = columns
                    .iter()
                    .position(|c| c.name.eq_ignore_ascii_case(col))
                    .ok_or_else(|| ConnectError::Query(format!("no column `{col}`")))?;
                let mut kept: Vec<Vec<Value>> = rows
                    .iter()
                    .filter(|r| r.get(i).is_some_and(|v| greater(v, last)))
                    .cloned()
                    .collect();
                kept.sort_by(|a, b| {
                    if greater(&a[i], &b[i]) {
                        std::cmp::Ordering::Greater
                    } else if greater(&b[i], &a[i]) {
                        std::cmp::Ordering::Less
                    } else {
                        std::cmp::Ordering::Equal
                    }
                });
                kept
            }
            None => rows.clone(),
        };
        let truncated = q.max_rows > 0 && rows.len() > q.max_rows;
        if truncated {
            rows.truncate(q.max_rows);
        }
        Ok(ResultSet {
            columns: columns.clone(),
            rows,
            truncated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn tables_columns_watermark_and_the_cap() {
        let m = MemoryConnector::new().table(
            "orders",
            &["id", "total", "updated"],
            vec![
                vec![json!(1), json!(10.5), json!("2026-01-01")],
                vec![json!(2), json!(20), json!("2026-01-03")],
                vec![json!(3), json!(5), json!("2026-01-02")],
            ],
        );
        m.ping().await.unwrap();
        assert_eq!(m.tables().await.unwrap()[0].name, "orders");
        assert_eq!(m.columns("orders").await.unwrap().len(), 3);
        assert!(m.columns("nope").await.is_err());
        let all = m
            .query(&Query {
                from: "orders".into(),
                after: None,
                max_rows: 2,
            })
            .await
            .unwrap();
        assert_eq!(all.rows.len(), 2);
        assert!(all.truncated);
        let later = m
            .query(&Query {
                from: "orders".into(),
                after: Some(("updated".into(), json!("2026-01-01"))),
                max_rows: 0,
            })
            .await
            .unwrap();
        let ids: Vec<i64> = later.rows.iter().map(|r| r[0].as_i64().unwrap()).collect();
        assert_eq!(ids, [3, 2], "after the watermark, in watermark order");
        let by_id = m
            .query(&Query {
                from: "orders".into(),
                after: Some(("id".into(), json!(2))),
                max_rows: 0,
            })
            .await
            .unwrap();
        assert_eq!(by_id.rows.len(), 1);
        assert!(m
            .query(&Query {
                from: "select 1".into(),
                after: None,
                max_rows: 0
            })
            .await
            .is_err());
        m.append(
            "orders",
            vec![vec![json!(4), json!(1), json!("2026-02-01")]],
        );
        m.remove("orders", &json!(1));
        assert_eq!(
            m.query(&Query {
                from: "orders".into(),
                after: None,
                max_rows: 0
            })
            .await
            .unwrap()
            .rows
            .len(),
            3
        );
        let down = MemoryConnector::down("refused");
        assert!(matches!(down.ping().await, Err(ConnectError::Connect(_))));
        assert!(down.tables().await.is_err());
    }
}
