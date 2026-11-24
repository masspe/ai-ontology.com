// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! SQL Server through `tiberius` (pure Rust, TLS by rustls): a new
//! connection per call, read-only intent (but for an approved write),
//! encryption on unless the source opts out. Behind the `sqlserver`
//! feature.

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
    async fn client(&self, read_only: bool) -> Result<Client<Compat<TcpStream>>, ConnectError> {
        let mut config = Config::new();
        config.host(&self.host);
        config.port(self.port);
        config.database(&self.database);
        config.authentication(AuthMethod::sql_server(&self.user, &self.password));
        config.application_name("ai-ontology");
        config.readonly(read_only);
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
        let mut client = self.client(true).await?;
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
        // Dates and times in ISO 8601, the one form the engine reads back
        // the same under every language setting: a watermark kept from a
        // sync is a literal the next sync can bind.
        ColumnData::DateTime(v) => text(v.map(|d| {
            // Days from 1900-01-01, the time in 1/300 s.
            let ms = (u64::from(d.seconds_fragments()) * 10 + 1) / 3;
            format!(
                "{}T{}",
                civil(DAYS_TO_1900 + i64::from(d.days())),
                clock(ms, 3)
            )
        })),
        ColumnData::SmallDateTime(v) => text(v.map(|d| {
            // Days from 1900-01-01, the time in minutes.
            format!(
                "{}T{}",
                civil(DAYS_TO_1900 + i64::from(d.days())),
                clock(u64::from(d.seconds_fragments()) * 60, 0)
            )
        })),
        ColumnData::Date(v) => text(v.map(|d| civil(i64::from(d.days())))),
        ColumnData::Time(v) => text(v.map(|t| clock(t.increments(), t.scale()))),
        ColumnData::DateTime2(v) => text(v.map(datetime2)),
        // Stored as UTC beside its offset: the instant, as UTC.
        ColumnData::DateTimeOffset(v) => text(v.map(|d| format!("{}Z", datetime2(d.datetime2())))),
    }
}

fn text(v: Option<String>) -> Value {
    v.map(Value::String).unwrap_or(Value::Null)
}

/// 1900-01-01, the epoch of `datetime`, in days from 0001-01-01.
const DAYS_TO_1900: i64 = 693_595;

/// `YYYY-MM-DD` for a count of days from 0001-01-01.
fn civil(days: i64) -> String {
    // Days-to-civil over 400-year eras that start on a 1 March.
    let z = days + 306;
    let (era, doe) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `hh:mm:ss[.fraction]` for a time of day in units of 10^-scale seconds.
fn clock(increments: u64, scale: u8) -> String {
    let unit = 10u64.pow(u32::from(scale));
    let (secs, fraction) = (increments / unit, increments % unit);
    let mut s = format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60);
    if scale > 0 {
        s.push_str(&format!(".{fraction:0width$}", width = usize::from(scale)));
    }
    s
}

fn datetime2(d: tiberius::time::DateTime2) -> String {
    let (date, time) = (d.date(), d.time());
    format!(
        "{}T{}",
        civil(i64::from(date.days())),
        clock(time.increments(), time.scale())
    )
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
        // One row past the cap says whether the result was cut.
        let mut sql = q.render_sql(quote_ansi);
        if q.max_rows > 0 {
            sql = sql.replacen("SELECT *", &format!("SELECT TOP ({}) *", q.max_rows + 1), 1);
        }
        self.run(&sql, q.max_rows).await
    }

    async fn execute(&self, sql: &str, max_rows: u64) -> Result<u64, ConnectError> {
        let mut client = self.client(false).await?;
        let failed = |e: tiberius::error::Error| ConnectError::Query(e.to_string());
        let work = async {
            // A failure leaves the transaction open, and the engine rolls
            // it back when the connection is dropped.
            client
                .simple_query("BEGIN TRANSACTION")
                .await
                .map_err(failed)?
                .into_results()
                .await
                .map_err(failed)?;
            // Every statement of the batch counts (a trigger's rows too).
            let rows = client.execute(sql, &[]).await.map_err(failed)?.total();
            let end = match super::guard(rows, max_rows) {
                Ok(_) => "COMMIT TRANSACTION",
                Err(_) => "ROLLBACK TRANSACTION",
            };
            client
                .simple_query(end)
                .await
                .map_err(failed)?
                .into_results()
                .await
                .map_err(failed)?;
            super::guard(rows, max_rows)
        };
        tokio::time::timeout(
            std::time::Duration::from_secs(self.timeout_secs.max(1)),
            work,
        )
        .await
        .map_err(|_| ConnectError::Query("timed out".into()))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiberius::time::{Date, DateTime, DateTime2, DateTimeOffset, SmallDateTime, Time};

    #[test]
    fn dates_and_times_are_read_as_iso_8601() {
        assert_eq!(civil(0), "0001-01-01");
        assert_eq!(civil(DAYS_TO_1900), "1900-01-01");
        assert_eq!(civil(719_162), "1970-01-01");
        // A leap day, the day after, and the last day the engine has.
        assert_eq!(civil(730_178), "2000-02-29");
        assert_eq!(civil(730_179), "2000-03-01");
        assert_eq!(civil(3_652_058), "9999-12-31");

        let noon = Time::new(12 * 3600 + 34 * 60 + 56, 0);
        let day = Date::new(739_647);
        assert_eq!(
            cell_to_json(&ColumnData::DateTime2(Some(DateTime2::new(day, noon)))),
            "2026-02-01T12:34:56"
        );
        assert_eq!(
            cell_to_json(&ColumnData::DateTime2(Some(DateTime2::new(
                day,
                Time::new(10_000_001, 7)
            )))),
            "2026-02-01T00:00:01.0000001"
        );
        assert_eq!(cell_to_json(&ColumnData::Date(Some(day))), "2026-02-01");
        assert_eq!(cell_to_json(&ColumnData::Time(Some(noon))), "12:34:56");
        // `datetime` counts thirds of a hundredth: .003 and .007.
        assert_eq!(
            cell_to_json(&ColumnData::DateTime(Some(DateTime::new(46_052, 300 + 2)))),
            "2026-02-01T00:00:01.007"
        );
        assert_eq!(
            cell_to_json(&ColumnData::SmallDateTime(Some(SmallDateTime::new(
                46_052, 61
            )))),
            "2026-02-01T01:01:00"
        );
        assert_eq!(
            cell_to_json(&ColumnData::DateTimeOffset(Some(DateTimeOffset::new(
                DateTime2::new(day, noon),
                120
            )))),
            "2026-02-01T12:34:56Z"
        );
        assert!(cell_to_json(&ColumnData::DateTime2(None)).is_null());
        assert!(cell_to_json(&ColumnData::Date(None)).is_null());
    }
}
