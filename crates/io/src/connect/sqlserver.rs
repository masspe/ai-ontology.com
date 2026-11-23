// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! SQL Server through `tiberius` (pure Rust, TLS by rustls): a new
//! connection per call, read-only intent, encryption on unless the
//! source opts out. Behind the `sqlserver` feature.

use async_trait::async_trait;
use serde_json::Value;
use tiberius::{AuthMethod, Client, ColumnData, Config, EncryptionLevel};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

use super::{quote_ansi, ColumnInfo, ConnectError, Connector, Query, ResultSet, TableInfo};

#[derive(Debug, Clone)]
pub struct SqlServerConnector {
    pub host: String,
    /// The address the host policy checked for `host`: connect there, so
    /// a second DNS answer cannot lead elsewhere. The name still goes to TLS.
    pub addr: Option<std::net::IpAddr>,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub password: String,
    /// TLS to the server (on by default); off only for a lab.
    pub encrypt: bool,
    /// Accept the server's certificate without a trusted chain (self-signed).
    pub trust_cert: bool,
    /// Seconds a statement may run.
    pub timeout_secs: u64,
}

impl SqlServerConnector {
    async fn client(&self) -> Result<Client<Compat<TcpStream>>, ConnectError> {
        let mut config = Config::new();
        config.host(&self.host);
        config.port(self.port);
        config.database(&self.database);
        config.authentication(AuthMethod::sql_server(&self.user, &self.password));
        config.application_name("ai-ontology");
        config.readonly(true);
        config.encryption(if self.encrypt {
            EncryptionLevel::Required
        } else {
            EncryptionLevel::NotSupported
        });
        if self.trust_cert {
            config.trust_cert();
        }
        let connect = async {
            let tcp = match self.addr {
                Some(ip) => TcpStream::connect((ip, self.port)).await,
                None => TcpStream::connect(config.get_addr()).await,
            }
            .map_err(|e| ConnectError::Connect(format!("{}:{}: {e}", self.host, self.port)))?;
            tcp.set_nodelay(true)
                .map_err(|e| ConnectError::Connect(e.to_string()))?;
            Client::connect(config, tcp.compat_write())
                .await
                .map_err(|e| ConnectError::Connect(e.to_string()))
        };
        tokio::time::timeout(
            std::time::Duration::from_secs(self.timeout_secs.max(1)),
            connect,
        )
        .await
        .map_err(|_| ConnectError::Connect("timed out connecting".into()))?
    }

    async fn run(&self, sql: &str, max_rows: usize) -> Result<ResultSet, ConnectError> {
        let mut client = self.client().await?;
        let work = async {
            let stream = client
                .simple_query(sql)
                .await
                .map_err(|e| ConnectError::Query(e.to_string()))?;
            let rows = stream
                .into_first_result()
                .await
                .map_err(|e| ConnectError::Query(e.to_string()))?;
            let mut out = ResultSet::default();
            for (n, row) in rows.into_iter().enumerate() {
                if out.columns.is_empty() {
                    out.columns = row
                        .columns()
                        .iter()
                        .map(|c| ColumnInfo {
                            name: c.name().to_string(),
                            data_type: format!("{:?}", c.column_type()).to_lowercase(),
                        })
                        .collect();
                }
                if max_rows > 0 && n >= max_rows {
                    out.truncated = true;
                    break;
                }
                out.rows
                    .push(row.into_iter().map(|cell| cell_to_json(&cell)).collect());
            }
            Ok(out)
        };
        tokio::time::timeout(
            std::time::Duration::from_secs(self.timeout_secs.max(1)),
            work,
        )
        .await
        .map_err(|_| ConnectError::Query("timed out".into()))?
    }
}

fn cell_to_json(cell: &ColumnData<'_>) -> Value {
    match cell {
        ColumnData::U8(v) => v.map(|x| Value::from(x as u64)).unwrap_or(Value::Null),
        ColumnData::I16(v) => v.map(Value::from).unwrap_or(Value::Null),
        ColumnData::I32(v) => v.map(Value::from).unwrap_or(Value::Null),
        ColumnData::I64(v) => v.map(Value::from).unwrap_or(Value::Null),
        ColumnData::F32(v) => v.map(|x| Value::from(x as f64)).unwrap_or(Value::Null),
        ColumnData::F64(v) => v.map(Value::from).unwrap_or(Value::Null),
        ColumnData::Bit(v) => v.map(Value::Bool).unwrap_or(Value::Null),
        ColumnData::String(v) => v
            .as_ref()
            .map(|s| Value::String(s.to_string()))
            .unwrap_or(Value::Null),
        ColumnData::Guid(v) => v
            .map(|g| Value::String(g.to_string()))
            .unwrap_or(Value::Null),
        ColumnData::Numeric(v) => v
            .map(|n| Value::String(n.to_string()))
            .unwrap_or(Value::Null),
        ColumnData::Binary(v) => v
            .as_ref()
            .map(|b| Value::String(format!("<{} bytes>", b.len())))
            .unwrap_or(Value::Null),
        ColumnData::Xml(v) => v
            .as_ref()
            .map(|x| Value::String(x.to_string()))
            .unwrap_or(Value::Null),
        // Dates and times: their debug form is the engine's value, stable
        // enough for a watermark and readable on a sheet.
        other => {
            let s = format!("{other:?}");
            if s.ends_with("(None)") {
                Value::Null
            } else {
                Value::String(s)
            }
        }
    }
}

#[async_trait]
impl Connector for SqlServerConnector {
    async fn ping(&self) -> Result<(), ConnectError> {
        self.run("SELECT 1 AS ok", 1).await.map(|_| ())
    }

    async fn tables(&self) -> Result<Vec<TableInfo>, ConnectError> {
        let rs = self
            .run(
                "SELECT TABLE_SCHEMA, TABLE_NAME FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_TYPE IN ('BASE TABLE', 'VIEW') ORDER BY TABLE_SCHEMA, TABLE_NAME",
                0,
            )
            .await?;
        Ok(rs
            .rows
            .iter()
            .map(|r| TableInfo {
                schema: r[0].as_str().map(str::to_string),
                name: r[1].as_str().unwrap_or("").to_string(),
            })
            .collect())
    }

    async fn columns(&self, table: &str) -> Result<Vec<ColumnInfo>, ConnectError> {
        let (schema, name) = match table.split_once('.') {
            Some((s, n)) => (Some(s), n),
            None => (None, table),
        };
        let mut sql = format!(
            "SELECT COLUMN_NAME, DATA_TYPE FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = {}",
            super::sql_literal(&Value::String(name.into()))
        );
        if let Some(s) = schema {
            sql.push_str(&format!(
                " AND TABLE_SCHEMA = {}",
                super::sql_literal(&Value::String(s.into()))
            ));
        }
        sql.push_str(" ORDER BY ORDINAL_POSITION");
        let rs = self.run(&sql, 0).await?;
        Ok(rs
            .rows
            .iter()
            .map(|r| ColumnInfo {
                name: r[0].as_str().unwrap_or("").to_string(),
                data_type: r[1].as_str().unwrap_or("").to_string(),
            })
            .collect())
    }

    async fn query(&self, q: &Query) -> Result<ResultSet, ConnectError> {
        self.run(&q.render_sql(quote_ansi), q.max_rows).await
    }
}
