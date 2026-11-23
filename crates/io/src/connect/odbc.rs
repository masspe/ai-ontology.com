// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Anything with an ODBC driver — OpenEdge first — through `odbc-api`.
//! The driver manager's calls block, so each one runs on the blocking
//! pool with its own connection. Every value comes back as text (the
//! driver's rendering), which is what a sheet shows anyway. Behind the
//! `odbc` feature: the runtime image needs the driver manager and the
//! engine's driver, mounted by the operator.

use async_trait::async_trait;
use odbc_api::{ConnectionOptions, Cursor, Environment, ResultSetMetadata};
use serde_json::Value;

use super::{quote_ansi, ColumnInfo, ConnectError, Connector, Query, ResultSet, TableInfo};

#[derive(Debug, Clone)]
pub struct OdbcConnector {
    /// `Driver={…};Host=…;Port=…;Database=…;UID=…;PWD=…` as the driver
    /// documents it.
    pub connection_string: String,
    pub timeout_secs: u64,
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
        for i in 1..=n as u16 {
            let name = cursor
                .col_name(i)
                .map_err(|e| ConnectError::Query(e.to_string()))?;
            let data_type = cursor
                .col_data_type(i)
                .map(|t| format!("{t:?}").to_lowercase())
                .unwrap_or_default();
            columns.push(ColumnInfo { name, data_type });
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
                    Value::String(String::from_utf8_lossy(&buf).trim_end().to_string())
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
}
