// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Anything with an ODBC driver — OpenEdge first — through `odbc-api`.
//! The driver manager's calls block, so each one runs on the blocking
//! pool with its own connection. Every value comes back as text (the
//! driver's rendering); integers, floats and bits are read back as what
//! they are, so a watermark compares and binds as a number. Behind the
//! `odbc` feature: the runtime image needs the driver manager and the
//! engine's driver, mounted by the operator.

use async_trait::async_trait;
use odbc_api::{ConnectionOptions, Cursor, DataType, Environment, ResultSetMetadata};
use serde_json::Value;

use super::{quote_ansi, ColumnInfo, ConnectError, Connector, Query, ResultSet, TableInfo};

#[derive(Debug, Clone)]
pub struct OdbcConnector {
    /// `Driver={…};Host=…;Port=…;Database=…;UID=…;PWD=…` as the driver
    /// documents it.
    pub connection_string: String,
    pub timeout_secs: u64,
}

/// Driver names, then `(data source name, driver)` pairs.
pub type Catalog = (Vec<String>, Vec<(String, String)>);

/// What the driver manager of this machine offers, as its ODBC
/// administrator lists it: the installed drivers, and the data source
/// names (user and system) with their driver.
pub fn catalog() -> Result<Catalog, ConnectError> {
    let env = Environment::new().map_err(|e| ConnectError::Connect(e.to_string()))?;
    let drivers = env
        .drivers()
        .map_err(|e| ConnectError::Connect(e.to_string()))?
        .into_iter()
        .map(|d| d.description)
        .collect();
    let dsns = env
        .data_sources()
        .map_err(|e| ConnectError::Connect(e.to_string()))?
        .into_iter()
        .map(|d| (d.server_name, d.driver))
        .collect();
    Ok((drivers, dsns))
}

/// OpenEdge refuses rows wider than the field's SQL width (error 7864):
/// the fix is on the database side, say so.
fn with_hint(e: impl std::fmt::Display) -> ConnectError {
    let text = e.to_string();
    if text.contains("7864") || text.to_ascii_lowercase().contains("too large") {
        return ConnectError::Query(format!(
            "{text} — OpenEdge: a value is wider than the field's SQL width; run `dbtool` option 2 (SQL width scan/fix) or start the broker with `-SQLWidthUpdate ON`"
        ));
    }
    ConnectError::Query(text)
}

/// The driver's text for a cell, as the JSON value its column type says.
fn typed(kind: Option<&DataType>, text: String) -> Value {
    let parsed = match kind {
        Some(DataType::Integer | DataType::SmallInt | DataType::BigInt | DataType::TinyInt) => {
            text.parse::<i64>().ok().map(Value::from)
        }
        Some(DataType::Real | DataType::Double | DataType::Float { .. }) => text
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number),
        Some(DataType::Bit) => match text.as_str() {
            "1" => Some(Value::Bool(true)),
            "0" => Some(Value::Bool(false)),
            _ => None,
        },
        _ => None,
    };
    parsed.unwrap_or(Value::String(text))
}

impl OdbcConnector {
    fn run_blocking(
        cs: &str,
        sql: &str,
        max_rows: usize,
        timeout_secs: u64,
    ) -> Result<ResultSet, ConnectError> {
        let env = Environment::new().map_err(|e| ConnectError::Connect(e.to_string()))?;
        let conn = env
            .connect_with_connection_string(
                cs,
                ConnectionOptions {
                    login_timeout_sec: Some(timeout_secs.clamp(1, 600) as u32),
                    ..Default::default()
                },
            )
            .map_err(|e| ConnectError::Connect(e.to_string()))?;
        // The driver ends a statement that runs past the source's timeout.
        let Some(mut cursor) = conn
            .execute(sql, (), Some(timeout_secs.clamp(1, 600) as usize))
            .map_err(with_hint)?
        else {
            return Ok(ResultSet::default());
        };
        let n = cursor
            .num_result_cols()
            .map_err(|e| ConnectError::Query(e.to_string()))? as usize;
        let mut columns = Vec::with_capacity(n);
        let mut kinds = Vec::with_capacity(n);
        for i in 1..=n as u16 {
            let name = cursor
                .col_name(i)
                .map_err(|e| ConnectError::Query(e.to_string()))?;
            let kind = cursor.col_data_type(i).ok();
            let data_type = kind
                .as_ref()
                .map(|t| format!("{t:?}").to_lowercase())
                .unwrap_or_default();
            columns.push(ColumnInfo { name, data_type });
            kinds.push(kind);
        }
        let mut rows = Vec::new();
        let mut truncated = false;
        let mut buf = Vec::new();
        while let Some(mut row) = cursor.next_row().map_err(with_hint)? {
            if max_rows > 0 && rows.len() >= max_rows {
                truncated = true;
                break;
            }
            let mut cells = Vec::with_capacity(n);
            for i in 1..=n as u16 {
                buf.clear();
                let present = row
                    .get_text(i, &mut buf)
                    .map_err(|e| ConnectError::Query(e.to_string()))?;
                cells.push(if present {
                    typed(
                        kinds[i as usize - 1].as_ref(),
                        String::from_utf8_lossy(&buf).trim_end().to_string(),
                    )
                } else {
                    Value::Null
                });
            }
            rows.push(cells);
        }
        Ok(ResultSet {
            columns,
            rows,
            truncated,
        })
    }

    /// One write in a transaction of its own: committed within the guard,
    /// rolled back past it or when the driver does not count the rows.
    fn execute_blocking(
        cs: &str,
        sql: &str,
        max_rows: u64,
        timeout_secs: u64,
    ) -> Result<u64, ConnectError> {
        let failed = |e: odbc_api::Error| ConnectError::Query(e.to_string());
        let env = Environment::new().map_err(|e| ConnectError::Connect(e.to_string()))?;
        let conn = env
            .connect_with_connection_string(
                cs,
                ConnectionOptions {
                    login_timeout_sec: Some(timeout_secs.clamp(1, 600) as u32),
                    ..Default::default()
                },
            )
            .map_err(|e| ConnectError::Connect(e.to_string()))?;
        conn.set_autocommit(false).map_err(failed)?;
        let mut statement = conn.preallocate().map_err(failed)?;
        statement
            .set_query_timeout_sec(timeout_secs.clamp(1, 600) as usize)
            .map_err(failed)?;
        let outcome = statement
            .execute(sql, ())
            .map(|_| ())
            .map_err(with_hint)
            .and_then(|()| statement.row_count().map_err(failed))
            .and_then(|rows| {
                rows.ok_or_else(|| {
                    ConnectError::Unsupported(
                        "the driver does not say how many rows changed: rolled back".into(),
                    )
                })
            })
            .and_then(|rows| super::guard(rows as u64, max_rows));
        match outcome {
            Ok(rows) => conn.commit().map(|()| rows).map_err(failed),
            Err(e) => {
                conn.rollback().map_err(failed)?;
                Err(e)
            }
        }
    }

    async fn run(&self, sql: &str, max_rows: usize) -> Result<ResultSet, ConnectError> {
        let (cs, sql, t) = (
            self.connection_string.clone(),
            sql.to_string(),
            self.timeout_secs,
        );
        tokio::task::spawn_blocking(move || Self::run_blocking(&cs, &sql, max_rows, t))
            .await
            .map_err(|e| ConnectError::Query(e.to_string()))?
    }
}

#[async_trait]
impl Connector for OdbcConnector {
    async fn ping(&self) -> Result<(), ConnectError> {
        // `SELECT 1` is not universal (OpenEdge wants a FROM); the driver
        // manager's own catalogue call reaches the engine just as well.
        self.tables().await.map(|_| ())
    }

    async fn tables(&self) -> Result<Vec<TableInfo>, ConnectError> {
        let cs = self.connection_string.clone();
        let t = self.timeout_secs;
        tokio::task::spawn_blocking(move || -> Result<Vec<TableInfo>, ConnectError> {
            let env = Environment::new().map_err(|e| ConnectError::Connect(e.to_string()))?;
            let conn = env
                .connect_with_connection_string(
                    &cs,
                    ConnectionOptions {
                        login_timeout_sec: Some(t.clamp(1, 600) as u32),
                        ..Default::default()
                    },
                )
                .map_err(|e| ConnectError::Connect(e.to_string()))?;
            let cursor = conn
                .tables("", "", "", "TABLE,VIEW")
                .map_err(|e| ConnectError::Query(e.to_string()))?;
            let text = |v: &odbc_api::parameter::VarCharArray<128>| -> Option<String> {
                v.as_str()
                    .ok()
                    .flatten()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            };
            let mut out = Vec::new();
            for row in cursor {
                let row = row.map_err(|e| ConnectError::Query(e.to_string()))?;
                let Some(name) = row
                    .table
                    .as_str()
                    .ok()
                    .flatten()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                else {
                    continue;
                };
                out.push(TableInfo {
                    schema: text(&row.schema),
                    name,
                });
            }
            Ok(out)
        })
        .await
        .map_err(|e| ConnectError::Query(e.to_string()))?
    }

    async fn columns(&self, table: &str) -> Result<Vec<ColumnInfo>, ConnectError> {
        // One empty page of the table: the driver describes the columns.
        let sql = format!("SELECT * FROM {} WHERE 1 = 0", quote_ansi(table));
        Ok(self.run(&sql, 0).await?.columns)
    }

    async fn query(&self, q: &Query) -> Result<ResultSet, ConnectError> {
        self.run(&q.render_sql(quote_ansi), q.max_rows).await
    }

    async fn execute(&self, sql: &str, max_rows: u64) -> Result<u64, ConnectError> {
        let (cs, sql, t) = (
            self.connection_string.clone(),
            sql.to_string(),
            self.timeout_secs,
        );
        tokio::task::spawn_blocking(move || Self::execute_blocking(&cs, &sql, max_rows, t))
            .await
            .map_err(|e| ConnectError::Query(e.to_string()))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cell_is_read_back_as_its_column_type() {
        assert_eq!(
            typed(Some(&DataType::Integer), "42".into()),
            Value::from(42)
        );
        assert_eq!(typed(Some(&DataType::BigInt), "-7".into()), Value::from(-7));
        assert_eq!(
            typed(Some(&DataType::Double), "1.5".into()),
            Value::from(1.5)
        );
        assert_eq!(typed(Some(&DataType::Bit), "1".into()), Value::Bool(true));
        assert_eq!(typed(Some(&DataType::Bit), "0".into()), Value::Bool(false));
        assert_eq!(
            typed(Some(&DataType::Bit), "yes".into()),
            Value::String("yes".into())
        );
        assert_eq!(
            typed(Some(&DataType::Integer), "n/a".into()),
            Value::String("n/a".into()),
            "what does not read as its type stays the driver's text"
        );
        assert_eq!(
            typed(
                Some(&DataType::Decimal {
                    precision: 10,
                    scale: 2
                }),
                "12.30".into()
            ),
            Value::String("12.30".into()),
            "an exact number keeps its digits"
        );
        assert_eq!(typed(None, "x".into()), Value::String("x".into()));
    }
}
