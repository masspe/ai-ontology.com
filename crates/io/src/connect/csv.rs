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
    /// The address the host policy checked: the URL is fetched there and
    /// nowhere else (no second DNS answer, no redirect).
    pub pin: Option<std::net::IpAddr>,
}

impl CsvConnector {
    pub fn new(location: impl Into<String>) -> Self {
        Self {
            location: location.into(),
            pin: None,
        }
    }

    /// Fetch the URL at `ip` only: see `pin`.
    pub fn pinned(mut self, ip: std::net::IpAddr) -> Self {
        self.pin = Some(ip);
        self
    }

    fn is_url(&self) -> bool {
        let l = self.location.to_ascii_lowercase();
        l.starts_with("https://") || l.starts_with("http://")
    }

    async fn read(&self) -> Result<String, ConnectError> {
        let raw = if self.is_url() {
            let mut client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(60));
            if let Some(ip) = self.pin {
                let host = reqwest::Url::parse(&self.location)
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_string))
                    .unwrap_or_default();
                // Port 0: the scheme's own port.
                client = client
                    .resolve(&host, std::net::SocketAddr::new(ip, 0))
                    .redirect(reqwest::redirect::Policy::none());
            }
            let resp = client
                .build()
                .map_err(|e| ConnectError::Connect(e.to_string()))?
                .get(&self.location)
                .send()
                .await
                .and_then(|r| r.error_for_status())
                .map_err(|e| ConnectError::Connect(e.to_string()))?;
            if resp.status().is_redirection() {
                return Err(ConnectError::Connect(format!(
                    "{} redirects elsewhere ({}): give the source the final address",
                    self.location,
                    resp.status()
                )));
            }
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

    /// Serve one canned HTTP response per connection, in order.
    fn serve(responses: Vec<&'static str>) -> u16 {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for body in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(body.as_bytes());
            }
        });
        port
    }

    #[tokio::test]
    async fn a_pinned_url_is_fetched_at_its_pin_and_a_redirect_is_refused() {
        let port = serve(vec![
            "HTTP/1.1 200 OK\r\ncontent-length: 8\r\nconnection: close\r\n\r\nid\n1\n2\n\n",
            "HTTP/1.1 302 Found\r\nlocation: http://127.0.0.1:1/\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
        ]);
        // A name no resolver knows: only the pin can lead to the listener.
        let c = CsvConnector::new(format!("http://files.invalid:{port}/t.csv"))
            .pinned("127.0.0.1".parse().unwrap());
        let rs = c
            .query(&Query {
                from: "data".into(),
                after: None,
                max_rows: 0,
            })
            .await
            .unwrap();
        assert_eq!(rs.rows.len(), 2);
        let e = c.ping().await.unwrap_err().to_string();
        assert!(e.contains("redirects elsewhere (302"), "{e}");
    }
}
