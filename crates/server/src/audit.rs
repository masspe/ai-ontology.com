// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Audit log of the writes: who did what, when, on which
//! path, one JSON line per successful mutating request, appended to a
//! file next to the data. Read back by `GET /audit` (the last N lines).

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::auth::now_rfc3339;
use crate::AuthContext;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// RFC 3339, UTC.
    pub ts: String,
    /// The caller: a user's email, a key's name, or the service token.
    pub who: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

pub struct AuditLog {
    path: PathBuf,
    // (Debug below: the path only.)
    // ponytail: one file, appended under a lock, read whole by `tail`;
    // rotate by size when a tenant's log outgrows a few hundred MB.
    lock: Mutex<()>,
}

impl std::fmt::Debug for AuditLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditLog")
            .field("path", &self.path)
            .finish()
    }
}

impl AuditLog {
    /// Open (create) the log; the directory is created if needed.
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)?;
        Ok(Self {
            path,
            lock: Mutex::new(()),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, entry: &AuditEntry) -> std::io::Result<()> {
        let mut line = serde_json::to_vec(entry).expect("audit entry serialises");
        line.push(b'\n');
        let _g = self.lock.lock();
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&self.path)?;
        f.write_all(&line)
    }

    /// The last `n` entries, oldest first. A line that does not parse is
    /// skipped (a torn last line after a crash).
    pub fn tail(&self, n: usize) -> std::io::Result<Vec<AuditEntry>> {
        let _g = self.lock.lock();
        let f = std::fs::File::open(&self.path)?;
        let entries: Vec<AuditEntry> = std::io::BufReader::new(f)
            .lines()
            .map_while(Result::ok)
            .filter_map(|l| serde_json::from_str(&l).ok())
            .collect();
        let skip = entries.len().saturating_sub(n);
        Ok(entries.into_iter().skip(skip).collect())
    }
}

/// Inside the authenticated router: every successful POST/PUT/PATCH/DELETE
/// is appended with its caller.
/// POST routes that read (a query, an analysis, a probe): not writes.
const READ_ONLY_POSTS: &[&str] = &[
    "/retrieve",
    "/subgraph",
    "/ask",
    "/ask/stream",
    "/path",
    "/ingest/analyze",
    "/settings/llm/test",
    "/auth/logout",
];

pub async fn audit_layer(req: Request, next: Next, log: Arc<AuditLog>) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let mutating = match method {
        http::Method::PUT | http::Method::PATCH | http::Method::DELETE => true,
        http::Method::POST => !READ_ONLY_POSTS.contains(&path.as_str()) && !path.ends_with("/run"),
        _ => false,
    };
    let who = req
        .extensions()
        .get::<AuthContext>()
        .map(|c| {
            c.email
                .clone()
                .or_else(|| c.name.clone())
                .unwrap_or_else(|| c.subject.clone())
        })
        .unwrap_or_else(|| "anonymous".into());
    // Stamped on the request by the outermost layer (the response header
    // is added after this layer returns).
    let request_id = req
        .extensions()
        .get::<crate::RequestId>()
        .map(|r| r.0.clone());
    let resp = next.run(req).await;
    if mutating && resp.status().is_success() {
        let entry = AuditEntry {
            ts: now_rfc3339(),
            who,
            method: method.to_string(),
            path,
            status: resp.status().as_u16(),
            request_id,
        };
        let log = log.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if let Err(e) = log.append(&entry) {
                tracing::warn!(path = %log.path().display(), error = %e, "audit line not written");
            }
        })
        .await;
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_returns_the_last_entries_and_skips_a_torn_line() {
        let dir = tempfile::tempdir().unwrap();
        let log = AuditLog::open(dir.path().join("sub").join("audit.jsonl")).unwrap();
        for i in 0..5 {
            log.append(&AuditEntry {
                ts: "t".into(),
                who: "w".into(),
                method: "POST".into(),
                path: format!("/p{i}"),
                status: 201,
                request_id: None,
            })
            .unwrap();
        }
        std::fs::OpenOptions::new()
            .append(true)
            .open(log.path())
            .unwrap()
            .write_all(b"{\"torn\"")
            .unwrap();
        let last = log.tail(2).unwrap();
        assert_eq!(
            last.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(),
            ["/p3", "/p4"]
        );
        assert_eq!(log.tail(100).unwrap().len(), 5);
    }
}
