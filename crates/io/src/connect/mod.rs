// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Connectors: a database or a file seen as tables of rows, read only.
//! One trait, one implementation per engine — `csv` (a file or an HTTPS
//! URL), `sqlserver` (feature), `postgres` (feature), `mysql` (feature:
//! MySQL and MariaDB), `odbc` (feature:
//! the route to OpenEdge and anything with a driver), and `memory` for
//! tests. A sync
//! (`RowSource`) turns rows into sheets and links through the regular
//! ingester, so validation, journaling and idempotency are the same as
//! for every other import.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod csv;
pub mod memory;
#[cfg(feature = "mysql")]
pub mod mysql;
#[cfg(feature = "odbc")]
pub mod odbc;
#[cfg(feature = "postgres")]
pub mod postgres;
pub mod rows;
#[cfg(feature = "sqlserver")]
pub mod sqlserver;

pub use csv::CsvConnector;
pub use memory::MemoryConnector;
pub use rows::{Mapping, RelationRule, RowSource};

#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error("connection: {0}")]
    Connect(String),
    #[error("query: {0}")]
    Query(String),
    #[error("not supported by this connector: {0}")]
    Unsupported(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub name: String,
}

impl TableInfo {
    /// `schema.name`, or the name alone.
    pub fn qualified(&self) -> String {
        match &self.schema {
            Some(s) if !s.is_empty() => format!("{s}.{}", self.name),
            _ => self.name.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub name: String,
    /// The engine's type name, for the mapping editor.
    #[serde(default)]
    pub data_type: String,
}

/// What a connector returns: the columns, and the rows as positional
/// JSON values (`Null`, `Bool`, `Number`, `String`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResultSet {
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<Vec<Value>>,
    /// `true` when `max_rows` cut the result.
    #[serde(default)]
    pub truncated: bool,
}

impl ResultSet {
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case(name))
    }
}

/// What a sync asks for: a table (or a `SELECT`), rows past a watermark,
/// at most `max_rows`.
#[derive(Debug, Clone, Default)]
pub struct Query {
    /// A table name (quoted by the connector) — or, when it starts with
    /// `select` (any case), a statement run as is.
    pub from: String,
    /// `(column, last value)`: only rows whose column is greater come,
    /// ordered by that column, so the next sync starts where this one ends.
    pub after: Option<(String, Value)>,
    /// Keyset paging: the rows come ordered by these columns, and only
    /// those past `cursor` (the values of these columns on the last row
    /// read) — each page a short statement on the columns' index.
    pub order: Vec<String>,
    pub cursor: Vec<Value>,
    pub max_rows: usize,
}

/// An index of a table, as the engine's catalog lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
}

/// Rows `(index name, column, unique)` ordered by index then position,
/// grouped into indexes.
pub fn group_indexes(rows: &[Vec<Value>], unique: fn(&Value) -> bool) -> Vec<IndexInfo> {
    let mut out: Vec<IndexInfo> = Vec::new();
    for r in rows {
        let name = r[0].as_str().unwrap_or("").to_string();
        let column = r[1].as_str().unwrap_or("").to_string();
        match out.last_mut() {
            Some(last) if last.name == name => last.columns.push(column),
            _ => out.push(IndexInfo {
                name,
                columns: vec![column],
                unique: unique(&r[2]),
            }),
        }
    }
    out
}

/// `a` against `b` as numbers when both are one (or the text of one),
/// as text otherwise: how a watermark or a key compares.
pub fn compare(a: &Value, b: &Value) -> std::cmp::Ordering {
    let number = |x: &Value| x.as_f64().or_else(|| x.as_str()?.trim().parse().ok());
    match (number(a), number(b)) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
        _ => {
            let text = |x: &Value| {
                x.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| x.to_string())
            };
            text(a).cmp(&text(b))
        }
    }
}

/// The rows of an in-memory table under `q`'s watermark and keyset,
/// ordered as the query asks (what a file or a canned table does in
/// Rust, where an engine runs SQL).
pub fn page_rows(
    columns: &[ColumnInfo],
    mut rows: Vec<Vec<Value>>,
    q: &Query,
) -> Result<Vec<Vec<Value>>, ConnectError> {
    let index = |col: &str| {
        columns
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case(col))
            .ok_or_else(|| ConnectError::Query(format!("no column `{col}`")))
    };
    let mut sort_by: Vec<usize> = Vec::new();
    if let Some((col, last)) = &q.after {
        let i = index(col)?;
        rows.retain(|r| r.get(i).is_some_and(|v| compare(v, last).is_gt()));
        sort_by.push(i);
    }
    let order: Vec<usize> = q.order.iter().map(|c| index(c)).collect::<Result<_, _>>()?;
    if !q.cursor.is_empty() {
        let n = order.len().min(q.cursor.len());
        rows.retain(|r| {
            let row: Vec<&Value> = order[..n].iter().map(|&i| &r[i]).collect();
            row.iter()
                .zip(&q.cursor)
                .map(|(a, b)| compare(a, b))
                .find(|o| !o.is_eq())
                .is_some_and(|o| o.is_gt())
        });
    }
    for i in order {
        if !sort_by.contains(&i) {
            sort_by.push(i);
        }
    }
    if !sort_by.is_empty() {
        rows.sort_by(|a, b| {
            sort_by
                .iter()
                .map(|&i| compare(&a[i], &b[i]))
                .find(|o| !o.is_eq())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }
    Ok(rows)
}

impl Query {
    pub fn is_statement(&self) -> bool {
        self.from
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("select")
    }

    /// The SQL for engines that take `"` as the identifier quote and
    /// `'…'` literals: `SELECT * FROM t WHERE wm > '…' ORDER BY wm`.
    pub fn render_sql(&self, quote: fn(&str) -> String) -> String {
        let base = if self.is_statement() {
            format!(
                "SELECT * FROM ({}) AS q",
                self.from.trim().trim_end_matches(';')
            )
        } else {
            format!("SELECT * FROM {}", quote(&self.from))
        };
        let mut conds = Vec::new();
        let mut order: Vec<String> = Vec::new();
        if let Some((col, v)) = &self.after {
            conds.push(format!("{} > {}", quote(col), sql_literal(v)));
            order.push(quote(col));
        }
        let n = self.order.len().min(self.cursor.len());
        if n > 0 {
            // Lexicographic: (a > va) OR (a = va AND b > vb) OR …
            let mut ors = Vec::new();
            for k in 0..n {
                let mut ands: Vec<String> = (0..k)
                    .map(|j| {
                        format!(
                            "{} = {}",
                            quote(&self.order[j]),
                            sql_literal(&self.cursor[j])
                        )
                    })
                    .collect();
                ands.push(format!(
                    "{} > {}",
                    quote(&self.order[k]),
                    sql_literal(&self.cursor[k])
                ));
                ors.push(format!("({})", ands.join(" AND ")));
            }
            conds.push(format!("({})", ors.join(" OR ")));
        }
        for c in &self.order {
            let q = quote(c);
            if !order.contains(&q) {
                order.push(q);
            }
        }
        let mut sql = base;
        if !conds.is_empty() {
            sql.push_str(&format!(" WHERE {}", conds.join(" AND ")));
        }
        if !order.is_empty() {
            sql.push_str(&format!(" ORDER BY {}", order.join(", ")));
        }
        sql
    }
}

/// A value as a SQL literal: numbers and booleans as they are, strings
/// with their quotes doubled. Only ever a watermark the engine gave us.
pub fn sql_literal(v: &Value) -> String {
    match v {
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => if *b { "1" } else { "0" }.to_string(),
        Value::Null => "NULL".to_string(),
        other => {
            let s = match other {
                Value::String(s) => s.clone(),
                o => o.to_string(),
            };
            format!("'{}'", s.replace('\'', "''"))
        }
    }
}

/// `"name"` with inner quotes doubled: ANSI SQL, SQL Server and OpenEdge.
pub fn quote_ansi(ident: &str) -> String {
    ident
        .split('.')
        .map(|part| format!("\"{}\"", part.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(".")
}

/// The row-count guard of a write: `rows` when it is within `max_rows`,
/// else the refusal the caller rolls back on.
pub fn guard(rows: u64, max_rows: u64) -> Result<u64, ConnectError> {
    if rows > max_rows {
        return Err(ConnectError::Query(format!(
            "would change {rows} rows, at most {max_rows} allowed: rolled back"
        )));
    }
    Ok(rows)
}

/// A view of a database or a file: read-only but for `execute`, which
/// only an approved write-back calls.
#[async_trait]
pub trait Connector: Send + Sync {
    /// Reach the engine and come back: what "Test connection" runs.
    async fn ping(&self) -> Result<(), ConnectError>;
    async fn tables(&self) -> Result<Vec<TableInfo>, ConnectError>;
    async fn columns(&self, table: &str) -> Result<Vec<ColumnInfo>, ConnectError>;
    async fn query(&self, q: &Query) -> Result<ResultSet, ConnectError>;

    /// One `UPDATE`, `INSERT` or `DELETE` in a transaction: committed when
    /// it changed at most `max_rows` rows, rolled back otherwise. The rows
    /// changed.
    async fn execute(&self, _sql: &str, _max_rows: u64) -> Result<u64, ConnectError> {
        Err(ConnectError::Unsupported(
            "this kind of source does not write".into(),
        ))
    }

    /// The indexes of `table`, for choosing a key; none where the engine
    /// keeps no catalog of them.
    async fn indexes(&self, _table: &str) -> Result<Vec<IndexInfo>, ConnectError> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_paged_in_rust_as_an_engine_would() {
        let columns: Vec<ColumnInfo> = ["a", "b", "wm"]
            .iter()
            .map(|c| ColumnInfo {
                name: c.to_string(),
                data_type: String::new(),
            })
            .collect();
        let rows = vec![
            vec![Value::from(2), Value::from("y"), Value::from(1)],
            vec![Value::from(1), Value::from("z"), Value::from(2)],
            vec![Value::from(2), Value::from("x"), Value::from(3)],
            vec![Value::from(10), Value::from("a"), Value::from(0)],
        ];
        let q = Query {
            from: "t".into(),
            order: vec!["a".into(), "b".into()],
            ..Default::default()
        };
        let paged = page_rows(&columns, rows.clone(), &q).unwrap();
        let keys: Vec<(i64, &str)> = paged
            .iter()
            .map(|r| (r[0].as_i64().unwrap(), r[1].as_str().unwrap()))
            .collect();
        assert_eq!(
            keys,
            [(1, "z"), (2, "x"), (2, "y"), (10, "a")],
            "numbers as numbers"
        );
        let next = Query {
            cursor: vec![Value::from(2), Value::from("x")],
            ..q.clone()
        };
        let paged = page_rows(&columns, rows.clone(), &next).unwrap();
        assert_eq!(paged.len(), 2);
        assert_eq!(paged[0][1], "y");
        // The watermark filters first and leads the order.
        let wm = Query {
            after: Some(("wm".into(), Value::from(0))),
            order: vec!["a".into()],
            ..Default::default()
        };
        let paged = page_rows(&columns, rows.clone(), &wm).unwrap();
        assert_eq!(
            paged
                .iter()
                .map(|r| r[2].as_i64().unwrap())
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert!(page_rows(
            &columns,
            rows,
            &Query {
                order: vec!["nope".into()],
                ..Default::default()
            }
        )
        .is_err());
        let grouped = group_indexes(
            &[
                vec![Value::from("pk"), Value::from("a"), Value::from(true)],
                vec![Value::from("pk"), Value::from("b"), Value::from(true)],
                vec![Value::from("ix"), Value::from("wm"), Value::from(false)],
            ],
            |v| v.as_bool() == Some(true),
        );
        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped[0].columns, ["a", "b"]);
        assert!(grouped[0].unique && !grouped[1].unique);
    }

    #[test]
    fn a_query_renders_a_table_or_a_statement_with_its_watermark() {
        let t = Query {
            from: "orders".into(),
            after: None,
            max_rows: 10,
            ..Default::default()
        };
        assert_eq!(t.render_sql(quote_ansi), "SELECT * FROM \"orders\"");
        let wm = Query {
            from: "pub.orders".into(),
            after: Some(("updated_at".into(), Value::String("2026-01-01".into()))),
            max_rows: 10,
            ..Default::default()
        };
        assert_eq!(
            wm.render_sql(quote_ansi),
            "SELECT * FROM \"pub\".\"orders\" WHERE \"updated_at\" > '2026-01-01' ORDER BY \"updated_at\""
        );
        let st = Query {
            from: " select a, b from t where x = 1; ".into(),
            after: Some(("a".into(), Value::Number(5.into()))),
            max_rows: 10,
            ..Default::default()
        };
        assert!(st.is_statement());
        assert_eq!(
            st.render_sql(quote_ansi),
            "SELECT * FROM (select a, b from t where x = 1) AS q WHERE \"a\" > 5 ORDER BY \"a\""
        );
        // Keyset paging: the first page orders, the next ones start past
        // the last row read, lexicographically over the key.
        let first = Query {
            from: "t".into(),
            order: vec!["a".into(), "b".into()],
            ..Default::default()
        };
        assert_eq!(
            first.render_sql(quote_ansi),
            "SELECT * FROM \"t\" ORDER BY \"a\", \"b\""
        );
        let next = Query {
            cursor: vec![Value::Number(5.into()), Value::String("x".into())],
            ..first.clone()
        };
        assert_eq!(
            next.render_sql(quote_ansi),
            "SELECT * FROM \"t\" WHERE ((\"a\" > 5) OR (\"a\" = 5 AND \"b\" > 'x')) ORDER BY \"a\", \"b\""
        );
        // With a watermark too: the watermark filters and orders first.
        let both = Query {
            after: Some(("wm".into(), Value::Number(9.into()))),
            order: vec!["wm".into(), "a".into()],
            cursor: vec![Value::Number(9.into()), Value::Number(3.into())],
            ..Default::default()
        };
        assert_eq!(
            both.render_sql(quote_ansi),
            "SELECT * FROM \"\" WHERE \"wm\" > 9 AND ((\"wm\" > 9) OR (\"wm\" = 9 AND \"a\" > 3)) ORDER BY \"wm\", \"a\""
        );
        assert_eq!(sql_literal(&Value::String("it's".into())), "'it''s'");
        assert_eq!(sql_literal(&Value::Bool(true)), "1");
        assert_eq!(sql_literal(&Value::Null), "NULL");
        assert_eq!(quote_ansi("a\"b"), "\"a\"\"b\"");
        assert_eq!(
            TableInfo {
                schema: Some("pub".into()),
                name: "t".into()
            }
            .qualified(),
            "pub.t"
        );
        let rs = ResultSet {
            columns: vec![ColumnInfo {
                name: "Id".into(),
                data_type: "int".into(),
            }],
            rows: vec![],
            truncated: false,
        };
        assert_eq!(rs.column_index("id"), Some(0));
        assert_eq!(rs.column_index("nope"), None);
    }

    #[tokio::test]
    async fn a_write_is_guarded_by_its_row_count_and_a_file_does_not_write() {
        assert_eq!(guard(2, 2).unwrap(), 2);
        let over = guard(3, 2).unwrap_err().to_string();
        assert!(over.contains("would change 3 rows, at most 2"), "{over}");
        let file = CsvConnector::new("nowhere.csv");
        assert!(matches!(
            file.execute("delete from t", 1).await,
            Err(ConnectError::Unsupported(_))
        ));
    }
}
