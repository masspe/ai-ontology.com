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

use sha2::{Digest, Sha256};

use crate::auth::now_rfc3339;
use crate::AuthContext;

/// One line of the log. `prev` is the `hash` of the line before, `hash`
/// the SHA-256 of this line without it: a removed, altered or inserted
/// line breaks the chain (`AuditLog::verify`).
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
    /// What happened, when the method and path do not say: a sign-in and
    /// its provider, a refusal and its code, the domains a question
    /// touched (and the question itself when the operator asked for it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prev: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hash: String,
}

impl AuditEntry {
    fn digest(&self) -> String {
        let mut h = Sha256::new();
        for part in [
            self.ts.as_str(),
            self.who.as_str(),
            self.method.as_str(),
            self.path.as_str(),
            self.request_id.as_deref().unwrap_or(""),
            self.detail.as_deref().unwrap_or(""),
            self.prev.as_str(),
        ] {
            h.update(part.as_bytes());
            h.update([0]);
        }
        h.update(self.status.to_be_bytes());
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// Something a handler wants in the audit log beyond method and path:
/// put on the response, read by [`audit_layer`]. A note is logged even
/// for a read (a question is one).
#[derive(Debug, Clone)]
pub struct AuditNote(pub String);

impl AuditNote {
    /// A question or retrieval: the domains touched, and the text itself
    /// only when the log keeps questions.
    pub fn ask(question: &str, domains: &[String]) -> Self {
        Self(format!(
            "domains={} q={}",
            domains.join(","),
            QuestionText(question)
        ))
    }
}

/// The question text, or a hash of it: the operator decides
/// (`AuditLog::keep_questions`). Rendered when the note is logged.
struct QuestionText<'a>(&'a str);

impl std::fmt::Display for QuestionText<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "\u{1}{}\u{2}", self.0)
    }
}

/// What `GET /audit` may narrow the tail to.
#[derive(Debug, Default, Clone)]
pub struct AuditFilter {
    pub who: Option<String>,
    pub path: Option<String>,
    pub since: Option<String>,
}

pub struct AuditLog {
    path: PathBuf,
    // (Debug below: the path only.)
    // ponytail: one file, appended under a lock, read whole by `tail`;
    // rotate by size when a tenant's log outgrows a few hundred MB.
    lock: Mutex<String>,
    keep_questions: bool,
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
        // The chain continues from the last line already there.
        let last = Self::read_all(&path)?
            .last()
            .map(|e| e.hash.clone())
            .unwrap_or_default();
        Ok(Self {
            path,
            lock: Mutex::new(last),
            keep_questions: false,
        })
    }

    /// Keep the text of questions in the log (off by default: the
    /// domains touched are logged, the words are not).
    pub fn keep_questions(mut self, yes: bool) -> Self {
        self.keep_questions = yes;
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read_all(path: &Path) -> std::io::Result<Vec<AuditEntry>> {
        let f = std::fs::File::open(path)?;
        Ok(std::io::BufReader::new(f)
            .lines()
            .map_while(Result::ok)
            .filter_map(|l| serde_json::from_str(&l).ok())
            .collect())
    }

    /// Append one line, chained to the previous one.
    pub fn append(&self, entry: &AuditEntry) -> std::io::Result<()> {
        let mut last = self.lock.lock();
        let mut entry = entry.clone();
        if let Some(d) = entry.detail.as_deref() {
            entry.detail = Some(self.render_detail(d));
        }
        entry.prev = last.clone();
        entry.hash = entry.digest();
        let mut line = serde_json::to_vec(&entry).expect("audit entry serialises");
        line.push(b'\n');
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&self.path)?;
        f.write_all(&line)?;
        *last = entry.hash;
        Ok(())
    }

    /// A question's text stays, or becomes its hash, as configured.
    fn render_detail(&self, d: &str) -> String {
        let (Some(a), Some(b)) = (d.find('\u{1}'), d.rfind('\u{2}')) else {
            return d.to_string();
        };
        let q = &d[a + 1..b];
        let shown = if self.keep_questions {
            q.to_string()
        } else {
            let h = Sha256::digest(q.as_bytes());
            format!(
                "sha256:{}",
                h.iter()
                    .take(8)
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            )
        };
        format!("{}{}{}", &d[..a], shown, &d[b + 1..])
    }

    /// An event outside a request (a sign-in, a refusal): logged as a line
    /// of its own.
    pub fn event(
        &self,
        who: &str,
        method: &str,
        path: &str,
        status: u16,
        detail: impl Into<String>,
    ) {
        let entry = AuditEntry {
            ts: now_rfc3339(),
            who: who.to_string(),
            method: method.to_string(),
            path: path.to_string(),
            status,
            request_id: None,
            detail: Some(detail.into()),
            prev: String::new(),
            hash: String::new(),
        };
        if let Err(e) = self.append(&entry) {
            tracing::warn!(path = %self.path.display(), error = %e, "audit line not written");
        }
    }

    /// The last `n` entries, oldest first. A line that does not parse is
    /// skipped (a torn last line after a crash).
    pub fn tail(&self, n: usize) -> std::io::Result<Vec<AuditEntry>> {
        self.tail_where(n, &AuditFilter::default())
    }

    /// `tail` narrowed to a caller, a path prefix, an instant.
    pub fn tail_where(&self, n: usize, filter: &AuditFilter) -> std::io::Result<Vec<AuditEntry>> {
        let _g = self.lock.lock();
        let entries: Vec<AuditEntry> = Self::read_all(&self.path)?
            .into_iter()
            .filter(|e| filter.who.as_deref().is_none_or(|w| e.who == w))
            .filter(|e| filter.path.as_deref().is_none_or(|p| e.path.starts_with(p)))
            .filter(|e| filter.since.as_deref().is_none_or(|s| e.ts.as_str() >= s))
            .collect();
        let skip = entries.len().saturating_sub(n);
        Ok(entries.into_iter().skip(skip).collect())
    }

    /// Walk the chain: `Ok(lines)` when every line's hash is right and
    /// follows the one before; `Err` names the first line that does not
    /// (1-based, among the parsable lines). Lines written before the
    /// chain existed (no hash) are accepted only at the start.
    pub fn verify(&self) -> Result<usize, String> {
        let _g = self.lock.lock();
        let entries = Self::read_all(&self.path).map_err(|e| e.to_string())?;
        let mut prev = String::new();
        let mut chained = false;
        for (i, e) in entries.iter().enumerate() {
            if e.hash.is_empty() {
                if chained {
                    return Err(format!("line {}: unchained line after chained ones", i + 1));
                }
                continue;
            }
            chained = true;
            if e.prev != prev {
                return Err(format!("line {}: does not follow the line before", i + 1));
            }
            if e.digest() != e.hash {
                return Err(format!("line {}: altered", i + 1));
            }
            prev = e.hash.clone();
        }
        Ok(entries.len())
    }
}

/// A request that changes something: every PUT, PATCH and DELETE, and
/// every POST that is not a read in disguise (a retrieval, a question,
/// a path, an analysis, a connection test, a logout, a saved query run).
pub fn is_mutating(method: &http::Method, path: &str) -> bool {
    match *method {
        http::Method::PUT | http::Method::PATCH | http::Method::DELETE => true,
        http::Method::POST => !READ_ONLY_POSTS.contains(&path) && !path.ends_with("/run"),
        _ => false,
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
    let mutating = is_mutating(&method, &path);
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
    let note = resp.extensions().get::<AuditNote>().map(|n| n.0.clone());
    if (mutating || note.is_some()) && resp.status().is_success() {
        let entry = AuditEntry {
            ts: now_rfc3339(),
            who,
            method: method.to_string(),
            path,
            status: resp.status().as_u16(),
            request_id,
            detail: note,
            prev: String::new(),
            hash: String::new(),
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
                detail: None,
                prev: String::new(),
                hash: String::new(),
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
