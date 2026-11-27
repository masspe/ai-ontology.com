// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! MySQL and MariaDB through `mysql_async` (pure Rust, TLS by rustls): a
//! new connection per call, a read-only session (an approved write opens
//! its own read-write transaction), the statement timeout set on the
//! server (`max_execution_time` on MySQL, `max_statement_time` on
//! MariaDB — whichever the engine knows), encryption on unless the source
//! opts out. Integers, floats, dates and times become JSON values; a
//! decimal keeps its digits as text; a `tinyint(1)` is the engine's 0 or
//! 1. Behind the `mysql` feature.

use std::time::Duration;

use async_trait::async_trait;
use mysql_async::consts::ColumnType;
use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Opts, OptsBuilder, SslOpts, Value as SqlValue};
use serde_json::Value;

use super::{ColumnInfo, ConnectError, Connector, Query, ResultSet, TableInfo};

#[derive(Debug, Clone)]
pub struct MysqlConnector {
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

/// MySQL quotes identifiers with backticks.
fn quote_backtick(ident: &str) -> String {
    ident
        .split('.')
        .map(|part| format!("`{}`", part.replace('`', "``")))
        .collect::<Vec<_>>()
        .join(".")
}

impl MysqlConnector {
    fn opts(&self) -> Opts {
        let mut b = OptsBuilder::default()
            .ip_or_hostname(
                self.addr
                    .map(|ip| ip.to_string())
                    .unwrap_or_else(|| self.host.clone()),
            )
            .tcp_port(self.port)
            .user(Some(self.user.clone()))
            .pass(Some(self.password.clone()))
            .db_name(Some(self.database.clone()))
            .prefer_socket(false)
            .stmt_cache_size(0)
            // The rows an UPDATE matched, not only the ones it altered:
            // what the write guard counts, as the other engines count.
            .client_found_rows(true)
            .setup(vec!["SET SESSION TRANSACTION READ ONLY".to_string()]);
        if self.encrypt {
            // The driver builds its TLS from the process-wide provider; a
            // build with more than one provider in it has to pick (an
            // earlier pick, the same one, stands).
            let _ = rustls::crypto::ring::default_provider().install_default();
            let mut ssl = SslOpts::default()
                .with_danger_accept_invalid_certs(self.trust_cert)
                .with_danger_skip_domain_validation(self.trust_cert);
            if !self.trust_cert {
                // The public authorities are built in; add whatever the host
                // trusts (a company's own authority installed by the operator).
                ssl = ssl.with_root_certs(
                    rustls_native_certs::load_native_certs()
                        .certs
                        .into_iter()
                        .map(|c| c.to_vec().into())
                        .collect(),
                );
                if self.addr.is_some() {
                    ssl = ssl.with_danger_tls_hostname_override(Some(self.host.clone()));
                }
            }
            b = b.ssl_opts(ssl);
        }
        b.into()
    }

    async fn client(&self) -> Result<Conn, ConnectError> {
        let limit = Duration::from_secs(self.timeout_secs.max(1));
        let mut conn = tokio::time::timeout(limit, Conn::new(self.opts()))
            .await
            .map_err(|_| ConnectError::Connect("timed out connecting".into()))?
            .map_err(|e| ConnectError::Connect(describe(&e)))?;
        // One of the two is the engine's; the other is an unknown variable.
        let ms = limit.as_millis();
        let _ = conn
            .query_drop(format!("SET SESSION max_execution_time = {ms}"))
            .await;
        let _ = conn
            .query_drop(format!(
                "SET SESSION max_statement_time = {}",
                limit.as_secs()
            ))
            .await;
        Ok(conn)
    }

    async fn run(&self, sql: &str, max_rows: usize) -> Result<ResultSet, ConnectError> {
        let mut conn = self.client().await?;
        let work = async {
            let mut result = conn
                .query_iter(sql)
                .await
                .map_err(|e| ConnectError::Query(describe(&e)))?;
            // The text protocol sends every value as text: the column
            // types say what the text is.
            let types: Vec<ColumnType> = result
                .columns()
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .map(|c| c.column_type())
                .collect();
            let mut out = ResultSet {
                columns: result
                    .columns()
                    .as_deref()
                    .unwrap_or(&[])
                    .iter()
                    .map(|c| ColumnInfo {
                        name: c.name_str().into_owned(),
                        data_type: format!("{:?}", c.column_type())
                            .trim_start_matches("MYSQL_TYPE_")
                            .to_ascii_lowercase(),
                    })
                    .collect(),
                ..Default::default()
            };
            while let Some(row) = result
                .next()
                .await
                .map_err(|e| ConnectError::Query(describe(&e)))?
            {
                if max_rows > 0 && out.rows.len() >= max_rows {
                    out.truncated = true;
                    break;
                }
                out.rows.push(
                    (0..row.len())
                        .map(|i| cell_to_json(types[i], row.as_ref(i).unwrap_or(&SqlValue::NULL)))
                        .collect(),
                );
            }
            // The rest of the result is read off the wire, then the
            // connection is returned cleanly.
            result
                .drop_result()
                .await
                .map_err(|e| ConnectError::Query(describe(&e)))?;
            conn.disconnect().await.ok();
            Ok(out)
        };
        tokio::time::timeout(Duration::from_secs(self.timeout_secs.max(1)), work)
            .await
            .map_err(|_| ConnectError::Query("timed out".into()))?
    }
}

/// The engine's own message when there is one (`Table 'x' doesn't
/// exist (1146)`), not the driver's wrapping.
fn describe(e: &mysql_async::Error) -> String {
    match e {
        mysql_async::Error::Server(s) => format!("{} ({})", s.message, s.code),
        other => {
            let mut text = other.to_string();
            let mut source = std::error::Error::source(other);
            while let Some(s) = source {
                text.push_str(&format!(": {s}"));
                source = s.source();
            }
            text
        }
    }
}

fn cell_to_json(ty: ColumnType, v: &SqlValue) -> Value {
    match v {
        SqlValue::NULL => Value::Null,
        SqlValue::Bytes(b) => {
            let text = String::from_utf8_lossy(b);
            let parsed = match ty {
                ColumnType::MYSQL_TYPE_TINY
                | ColumnType::MYSQL_TYPE_SHORT
                | ColumnType::MYSQL_TYPE_INT24
                | ColumnType::MYSQL_TYPE_LONG
                | ColumnType::MYSQL_TYPE_LONGLONG
                | ColumnType::MYSQL_TYPE_YEAR => text
                    .parse::<i64>()
                    .map(Value::from)
                    .ok()
                    .or_else(|| text.parse::<u64>().map(Value::from).ok()),
                ColumnType::MYSQL_TYPE_FLOAT | ColumnType::MYSQL_TYPE_DOUBLE => text
                    .parse::<f64>()
                    .ok()
                    .and_then(serde_json::Number::from_f64)
                    .map(Value::Number),
                // A decimal keeps its digits; dates and times stay as the
                // engine writes them.
                _ => None,
            };
            parsed.unwrap_or_else(|| Value::String(text.into_owned()))
        }
        SqlValue::Int(i) => Value::from(*i),
        SqlValue::UInt(u) => Value::from(*u),
        SqlValue::Float(f) => serde_json::Number::from_f64(f64::from(*f))
            .map(Value::Number)
            .unwrap_or_else(|| Value::String(f.to_string())),
        SqlValue::Double(f) => serde_json::Number::from_f64(*f)
            .map(Value::Number)
            .unwrap_or_else(|| Value::String(f.to_string())),
        // A DATE has no time of day; a DATETIME keeps it (and its
        // microseconds when the engine sent some), as the engine writes it.
        SqlValue::Date(y, m, d, 0, 0, 0, 0) => Value::String(format!("{y:04}-{m:02}-{d:02}")),
        SqlValue::Date(y, m, d, h, i, s, 0) => {
            Value::String(format!("{y:04}-{m:02}-{d:02} {h:02}:{i:02}:{s:02}"))
        }
        SqlValue::Date(y, m, d, h, i, s, us) => {
            Value::String(format!("{y:04}-{m:02}-{d:02} {h:02}:{i:02}:{s:02}.{us:06}"))
        }
        SqlValue::Time(neg, days, h, i, s, us) => {
            let hours = u32::from(*h) + days * 24;
            let mut text = format!("{}{hours:02}:{i:02}:{s:02}", if *neg { "-" } else { "" });
            if *us > 0 {
                text.push_str(&format!(".{us:06}"));
            }
            Value::String(text)
        }
    }
}

#[async_trait]
impl Connector for MysqlConnector {
    async fn ping(&self) -> Result<(), ConnectError> {
        self.run("SELECT 1 AS ok", 1).await.map(|_| ())
    }

    async fn tables(&self) -> Result<Vec<TableInfo>, ConnectError> {
        let rs = self
            .run(
                "SELECT table_schema, table_name FROM information_schema.tables WHERE table_schema NOT IN ('mysql', 'information_schema', 'performance_schema', 'sys') ORDER BY table_schema, table_name",
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
            "SELECT column_name, data_type FROM information_schema.columns WHERE table_name = {} AND table_schema = {}",
            super::sql_literal(&Value::String(name.into())),
            match schema {
                Some(s) => super::sql_literal(&Value::String(s.into())),
                None => "DATABASE()".to_string(),
            }
        );
        sql.push_str(" ORDER BY ordinal_position");
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
        // One row past the cap says whether the result was cut.
        let mut sql = q.render_sql(quote_backtick);
        if q.max_rows > 0 {
            sql.push_str(&format!(" LIMIT {}", q.max_rows + 1));
        }
        self.run(&sql, q.max_rows).await
    }

    async fn execute(&self, sql: &str, max_rows: u64) -> Result<u64, ConnectError> {
        let mut conn = self.client().await?;
        let failed = |e: mysql_async::Error| ConnectError::Query(describe(&e));
        let work = async {
            // The session is read-only: this one transaction is not. A
            // failure leaves it open, and the engine rolls it back when
            // the connection is dropped.
            conn.query_drop("START TRANSACTION READ WRITE")
                .await
                .map_err(failed)?;
            // Multi-statement is off on the connection: one statement runs.
            conn.query_drop(sql).await.map_err(failed)?;
            let rows = conn.affected_rows();
            if let Err(over) = super::guard(rows, max_rows) {
                conn.query_drop("ROLLBACK").await.map_err(failed)?;
                return Err(over);
            }
            conn.query_drop("COMMIT").await.map_err(failed)?;
            conn.disconnect().await.ok();
            Ok(rows)
        };
        tokio::time::timeout(Duration::from_secs(self.timeout_secs.max(1)), work)
            .await
            .map_err(|_| ConnectError::Query("timed out".into()))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_become_json_by_kind() {
        let text =
            |ty: ColumnType, s: &str| cell_to_json(ty, &SqlValue::Bytes(s.as_bytes().to_vec()));
        // The text protocol: typed by the column.
        assert_eq!(text(ColumnType::MYSQL_TYPE_LONG, "3"), Value::from(3));
        assert_eq!(text(ColumnType::MYSQL_TYPE_TINY, "-1"), Value::from(-1));
        assert_eq!(
            text(ColumnType::MYSQL_TYPE_LONGLONG, "18446744073709551615"),
            Value::from(u64::MAX),
            "an unsigned bigint past i64"
        );
        assert_eq!(text(ColumnType::MYSQL_TYPE_DOUBLE, "1.5"), Value::from(1.5));
        assert_eq!(
            text(ColumnType::MYSQL_TYPE_NEWDECIMAL, "12.30"),
            Value::String("12.30".into()),
            "a decimal keeps its digits"
        );
        assert_eq!(
            text(ColumnType::MYSQL_TYPE_DATETIME, "2026-03-01 10:00:00"),
            Value::String("2026-03-01 10:00:00".into())
        );
        assert_eq!(
            text(ColumnType::MYSQL_TYPE_LONG, "not a number"),
            Value::String("not a number".into())
        );
        // The binary protocol's values, typed by the driver.
        let ty = ColumnType::MYSQL_TYPE_VARCHAR;
        let cell_to_json = |v: &SqlValue| cell_to_json(ty, v);
        assert_eq!(cell_to_json(&SqlValue::NULL), Value::Null);
        assert_eq!(cell_to_json(&SqlValue::Int(-42)), Value::from(-42));
        assert_eq!(cell_to_json(&SqlValue::UInt(7)), Value::from(7));
        assert_eq!(cell_to_json(&SqlValue::Double(1.5)), Value::from(1.5));
        assert_eq!(
            cell_to_json(&SqlValue::Double(f64::NAN)),
            Value::String("NaN".into()),
            "JSON has no NaN"
        );
        assert_eq!(
            cell_to_json(&SqlValue::Date(2026, 2, 1, 0, 0, 0, 0)),
            Value::String("2026-02-01".into())
        );
        assert_eq!(
            cell_to_json(&SqlValue::Date(2026, 2, 1, 10, 0, 5, 0)),
            Value::String("2026-02-01 10:00:05".into())
        );
        assert_eq!(
            cell_to_json(&SqlValue::Date(2026, 2, 1, 10, 0, 5, 250)),
            Value::String("2026-02-01 10:00:05.000250".into())
        );
        assert_eq!(
            cell_to_json(&SqlValue::Time(true, 1, 2, 3, 4, 0)),
            Value::String("-26:03:04".into())
        );
        assert_eq!(
            cell_to_json(&SqlValue::Time(false, 0, 2, 3, 4, 5)),
            Value::String("02:03:04.000005".into())
        );
    }

    #[test]
    fn identifiers_are_backticked() {
        assert_eq!(quote_backtick("glpi.glpi_tickets"), "`glpi`.`glpi_tickets`");
        assert_eq!(quote_backtick("we`ird"), "`we``ird`");
    }

    #[tokio::test]
    async fn an_engine_that_is_not_there_is_a_connection_error() {
        let c = MysqlConnector {
            host: "127.0.0.1".into(),
            addr: Some("127.0.0.1".parse().unwrap()),
            // A port nothing listens on.
            port: 1,
            database: "db".into(),
            user: "u".into(),
            password: "p".into(),
            encrypt: true,
            trust_cert: true,
            timeout_secs: 2,
        };
        assert!(matches!(c.ping().await, Err(ConnectError::Connect(_))));
        let verified = MysqlConnector {
            trust_cert: false,
            ..c.clone()
        };
        assert!(matches!(
            verified.tables().await,
            Err(ConnectError::Connect(_))
        ));
        let plain = MysqlConnector {
            encrypt: false,
            ..c
        };
        assert!(matches!(
            plain.execute("delete from t", 1).await,
            Err(ConnectError::Connect(_))
        ));
    }
}
