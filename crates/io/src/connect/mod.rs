// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Connectors: a database or a file seen as tables of rows, read only.
//! One trait, one implementation per engine — `csv` (a file or an HTTPS
//! URL), `sqlserver` (feature), `postgres` (feature), `odbc` (feature:
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
    pub max_rows: usize,
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
        match &self.after {
            Some((col, v)) => format!(
                "{base} WHERE {c} > {v} ORDER BY {c}",
                c = quote(col),
                v = sql_literal(v)
            ),
            None => base,
        }
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

/// A read-only view of a database or a file.
#[async_trait]
pub trait Connector: Send + Sync {
    /// Reach the engine and come back: what "Test connection" runs.
    async fn ping(&self) -> Result<(), ConnectError>;
    async fn tables(&self) -> Result<Vec<TableInfo>, ConnectError>;
    async fn columns(&self, table: &str) -> Result<Vec<ColumnInfo>, ConnectError>;
    async fn query(&self, q: &Query) -> Result<ResultSet, ConnectError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_renders_a_table_or_a_statement_with_its_watermark() {
        let t = Query {
            from: "orders".into(),
            after: None,
            max_rows: 10,
        };
        assert_eq!(t.render_sql(quote_ansi), "SELECT * FROM \"orders\"");
        let wm = Query {
            from: "pub.orders".into(),
            after: Some(("updated_at".into(), Value::String("2026-01-01".into()))),
            max_rows: 10,
        };
        assert_eq!(
            wm.render_sql(quote_ansi),
            "SELECT * FROM \"pub\".\"orders\" WHERE \"updated_at\" > '2026-01-01' ORDER BY \"updated_at\""
        );
        let st = Query {
            from: " select a, b from t where x = 1; ".into(),
            after: Some(("a".into(), Value::Number(5.into()))),
            max_rows: 10,
        };
        assert!(st.is_statement());
        assert_eq!(
            st.render_sql(quote_ansi),
            "SELECT * FROM (select a, b from t where x = 1) AS q WHERE \"a\" > 5 ORDER BY \"a\""
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
}
