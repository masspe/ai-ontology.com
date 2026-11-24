// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! PostgreSQL through `tokio-postgres` (pure Rust, TLS by rustls): a new
//! connection per call, read-only transactions (an approved write opens
//! its own read-write one), the statement timeout
//! set on the server, encryption on unless the source opts out. Values
//! travel as text; booleans, integers and floats become JSON values, the
//! rest stays the engine's text (dates sort as written). Behind the
//! `postgres` feature.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use serde_json::Value;
use tokio_postgres::config::SslMode;
use tokio_postgres::types::Type;
use tokio_postgres::{Client, Config, SimpleQueryMessage};
use tokio_postgres_rustls::MakeRustlsConnect;

use super::{quote_ansi, ColumnInfo, ConnectError, Connector, Query, ResultSet, TableInfo};

#[derive(Debug, Clone)]
pub struct PostgresConnector {
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

/// `trust_cert`: the channel is encrypted, the server is whoever answered.
#[derive(Debug)]
struct AnyCertificate(Arc<CryptoProvider>);

impl ServerCertVerifier for AnyCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

impl PostgresConnector {
    fn tls(&self) -> Result<MakeRustlsConnect, ConnectError> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .map_err(|e| ConnectError::Connect(e.to_string()))?;
        let config = if self.trust_cert {
            builder
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(AnyCertificate(provider)))
                .with_no_client_auth()
        } else {
            // The public authorities, and whatever the host trusts (a
            // company's own authority installed by the operator).
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            roots.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
            builder.with_root_certificates(roots).with_no_client_auth()
        };
        Ok(MakeRustlsConnect::new(config))
    }

    async fn client(&self) -> Result<Client, ConnectError> {
        let limit = Duration::from_secs(self.timeout_secs.max(1));
        let mut config = Config::new();
        config
            .host(&self.host)
            .port(self.port)
            .dbname(&self.database)
            .user(&self.user)
            .password(&self.password)
            .application_name("ai-ontology")
            .connect_timeout(limit)
            .ssl_mode(if self.encrypt {
                SslMode::Require
            } else {
                SslMode::Disable
            });
        if let Some(ip) = self.addr {
            config.hostaddr(ip);
        }
        let (client, connection) = tokio::time::timeout(limit, config.connect(self.tls()?))
            .await
            .map_err(|_| ConnectError::Connect("timed out connecting".into()))?
            .map_err(|e| ConnectError::Connect(describe(&e)))?;
        // The connection ends when the client is dropped.
        tokio::spawn(connection);
        client
            .batch_execute(&format!(
                "SET default_transaction_read_only = on; SET statement_timeout = {}",
                limit.as_millis()
            ))
            .await
            .map_err(|e| ConnectError::Connect(describe(&e)))?;
        Ok(client)
    }

    async fn run(&self, sql: &str, max_rows: usize) -> Result<ResultSet, ConnectError> {
        let client = self.client().await?;
        let work = async {
            // The column types come from the statement's description, the
            // values as text from the simple protocol.
            let statement = client
                .prepare(sql)
                .await
                .map_err(|e| ConnectError::Query(describe(&e)))?;
            let types: Vec<Type> = statement
                .columns()
                .iter()
                .map(|c| c.type_().clone())
                .collect();
            let mut out = ResultSet {
                columns: statement
                    .columns()
                    .iter()
                    .map(|c| ColumnInfo {
                        name: c.name().to_string(),
                        data_type: c.type_().name().to_string(),
                    })
                    .collect(),
                ..Default::default()
            };
            let messages = client
                .simple_query(sql)
                .await
                .map_err(|e| ConnectError::Query(describe(&e)))?;
            for message in messages {
                let SimpleQueryMessage::Row(row) = message else {
                    continue;
                };
                if max_rows > 0 && out.rows.len() >= max_rows {
                    out.truncated = true;
                    break;
                }
                out.rows.push(
                    types
                        .iter()
                        .enumerate()
                        .map(|(i, ty)| cell_to_json(ty, row.get(i)))
                        .collect(),
                );
            }
            Ok(out)
        };
        tokio::time::timeout(Duration::from_secs(self.timeout_secs.max(1)), work)
            .await
            .map_err(|_| ConnectError::Query("timed out".into()))?
    }
}

/// The engine's own message when there is one (`relation "x" does not
/// exist`), not the driver's "db error".
fn describe(e: &tokio_postgres::Error) -> String {
    match e.as_db_error() {
        Some(db) => format!("{} ({})", db.message(), db.code().code()),
        None => {
            let mut text = e.to_string();
            let mut source = std::error::Error::source(e);
            while let Some(s) = source {
                text.push_str(&format!(": {s}"));
                source = s.source();
            }
            text
        }
    }
}

fn cell_to_json(ty: &Type, text: Option<&str>) -> Value {
    let Some(t) = text else {
        return Value::Null;
    };
    let parsed = match *ty {
        Type::BOOL => Some(Value::Bool(t == "t")),
        Type::INT2 | Type::INT4 | Type::INT8 | Type::OID => t.parse::<i64>().ok().map(Value::from),
        Type::FLOAT4 | Type::FLOAT8 => t
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number),
        _ => None,
    };
    parsed.unwrap_or_else(|| Value::String(t.to_string()))
}

#[async_trait]
impl Connector for PostgresConnector {
    async fn ping(&self) -> Result<(), ConnectError> {
        self.run("SELECT 1 AS ok", 1).await.map(|_| ())
    }

    async fn tables(&self) -> Result<Vec<TableInfo>, ConnectError> {
        let rs = self
            .run(
                "SELECT table_schema, table_name FROM information_schema.tables WHERE table_schema NOT IN ('pg_catalog', 'information_schema') ORDER BY table_schema, table_name",
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
            "SELECT column_name, data_type FROM information_schema.columns WHERE table_name = {}",
            super::sql_literal(&Value::String(name.into()))
        );
        if let Some(s) = schema {
            sql.push_str(&format!(
                " AND table_schema = {}",
                super::sql_literal(&Value::String(s.into()))
            ));
        }
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
        let mut sql = q.render_sql(quote_ansi);
        if q.max_rows > 0 {
            sql.push_str(&format!(" LIMIT {}", q.max_rows + 1));
        }
        self.run(&sql, q.max_rows).await
    }

    async fn execute(&self, sql: &str, max_rows: u64) -> Result<u64, ConnectError> {
        let client = self.client().await?;
        let failed = |e: tokio_postgres::Error| ConnectError::Query(describe(&e));
        let work = async {
            // The session is read-only: this one transaction is not. A
            // failure leaves it open, and the engine rolls it back when
            // the connection is dropped.
            client
                .batch_execute("BEGIN READ WRITE")
                .await
                .map_err(failed)?;
            // The extended protocol takes one statement and counts its rows.
            let rows = client.execute(sql, &[]).await.map_err(failed)?;
            if let Err(over) = super::guard(rows, max_rows) {
                client.batch_execute("ROLLBACK").await.map_err(failed)?;
                return Err(over);
            }
            client.batch_execute("COMMIT").await.map_err(failed)?;
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
    fn text_values_become_json_by_column_type() {
        assert_eq!(cell_to_json(&Type::BOOL, Some("t")), Value::Bool(true));
        assert_eq!(cell_to_json(&Type::BOOL, Some("f")), Value::Bool(false));
        assert_eq!(cell_to_json(&Type::INT8, Some("-42")), Value::from(-42));
        assert_eq!(cell_to_json(&Type::FLOAT8, Some("1.5")), Value::from(1.5));
        assert_eq!(
            cell_to_json(&Type::FLOAT8, Some("NaN")),
            Value::String("NaN".into()),
            "JSON has no NaN"
        );
        assert_eq!(
            cell_to_json(&Type::NUMERIC, Some("12.30")),
            Value::String("12.30".into()),
            "an exact number keeps its digits"
        );
        assert_eq!(
            cell_to_json(&Type::TIMESTAMPTZ, Some("2026-11-24 10:00:00+00")),
            Value::String("2026-11-24 10:00:00+00".into())
        );
        assert_eq!(cell_to_json(&Type::TEXT, None), Value::Null);
    }

    #[tokio::test]
    async fn an_engine_that_is_not_there_is_a_connection_error() {
        let c = PostgresConnector {
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
        let verified = PostgresConnector {
            trust_cert: false,
            ..c
        };
        assert!(matches!(
            verified.tables().await,
            Err(ConnectError::Connect(_))
        ));
        assert!(matches!(
            verified.execute("delete from t", 1).await,
            Err(ConnectError::Connect(_))
        ));
    }
}
