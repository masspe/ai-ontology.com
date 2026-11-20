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
    /// Rotate once the file is this big; 0 never rotates.
    rotate_bytes: u64,
    /// Rotated files kept beside the current one; older ones are deleted.
    keep: usize,
}

/// Default rotation: 64 MB, the last 10 files kept.
pub const ROTATE_BYTES: u64 = 64 * 1024 * 1024;
pub const ROTATE_KEEP: usize = 10;

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
        // The chain continues from the last line already there — in the
        // current file, else in the newest rotated one.
        let mut last = Self::read_all(&path)?
            .last()
            .map(|e| e.hash.clone())
            .unwrap_or_default();
        if last.is_empty() {
            if let Some(newest) = rotated_files(&path).pop() {
                last = Self::read_all(&newest)?
                    .last()
                    .map(|e| e.hash.clone())
                    .unwrap_or_default();
            }
        }
        Ok(Self {
            path,
            lock: Mutex::new(last),
            keep_questions: false,
            rotate_bytes: ROTATE_BYTES,
            keep: ROTATE_KEEP,
        })
    }

    /// Rotate the file past `bytes` (0: never) and keep the last `keep`
    /// rotated files. The chain runs across files: the first line of a
    /// new file follows the last line of the one before.
    pub fn rotate(mut self, bytes: u64, keep: usize) -> Self {
        self.rotate_bytes = bytes;
        self.keep = keep;
        self
    }

    /// The rotated files, oldest first.
    fn rotated(&self) -> Vec<PathBuf> {
        rotated_files(&self.path)
    }

    /// Called with the lock held, after a write: move the file aside
    /// once it is big enough, and drop the oldest rotated files.
    fn maybe_rotate(&self, written: u64) {
        if self.rotate_bytes == 0 || written < self.rotate_bytes {
            return;
        }
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let stem = self
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("audit");
        let mut aside = self.path.with_file_name(format!("{stem}.{secs}.jsonl"));
        let mut n = secs;
        while aside.exists() {
            n += 1;
            aside = self.path.with_file_name(format!("{stem}.{n}.jsonl"));
        }
        if let Err(e) = std::fs::rename(&self.path, &aside) {
            tracing::warn!(path = %self.path.display(), error = %e, "audit log not rotated");
            return;
        }
        let rotated = self.rotated();
        for old in rotated.iter().take(rotated.len().saturating_sub(self.keep)) {
            let _ = std::fs::remove_file(old);
        }
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

    /// The entries of one file; a file that is not there (the current one,
    /// just rotated) holds none.
    fn read_all(path: &Path) -> std::io::Result<Vec<AuditEntry>> {
        let f = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
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
        let written = f.metadata().map(|m| m.len()).unwrap_or(0);
        drop(f);
        *last = entry.hash;
        self.maybe_rotate(written);
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
        // The rotated files first, oldest first, then the current one:
        // one chain.
        let mut entries = Vec::new();
        for p in self
            .rotated()
            .into_iter()
            .chain(std::iter::once(self.path.clone()))
        {
            entries.extend(Self::read_all(&p).map_err(|e| format!("{}: {e}", p.display()))?);
        }
        let mut prev = String::new();
        let mut chained = false;
        for (i, e) in entries.iter().enumerate() {
            if e.hash.is_empty() {
                if chained {
                    return Err(format!("line {}: unchained line after chained ones", i + 1));
                }
                continue;
            }
            // The oldest kept file may start mid-chain: its first line
            // follows a line that rotation has since deleted.
            if !chained {
                prev = e.prev.clone();
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

/// The rotated files beside `path`, oldest first: `<stem>.<unix seconds>.jsonl`.
fn rotated_files(path: &Path) -> Vec<PathBuf> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("audit")
        .to_string();
    let Some(dir) = path.parent() else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                        n.len() > stem.len() + 7
                            && n.starts_with(&format!("{stem}."))
                            && n.ends_with(".jsonl")
                            && n[stem.len() + 1..n.len() - 6]
                                .chars()
                                .all(|c| c.is_ascii_digit())
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
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
    fn the_log_rotates_past_the_size_and_the_chain_runs_across_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        // Tiny files: every line rotates; keep the last two.
        let log = AuditLog::open(&path).unwrap().rotate(1, 2);
        for i in 0..5 {
            log.event("w", "POST", &format!("/p{i}"), 200, "e");
        }
        let rotated = log.rotated();
        assert_eq!(rotated.len(), 2, "{rotated:?}");
        assert!(!path.exists() || std::fs::metadata(&path).unwrap().len() == 0);
        assert_eq!(log.verify().unwrap(), 2, "the two kept files chain");
        // A reopened log continues from the last kept line.
        let again = AuditLog::open(&path).unwrap().rotate(0, 2);
        again.event("w", "POST", "/p5", 200, "e");
        assert_eq!(again.verify().unwrap(), 3);
        // Never rotating keeps one file.
        let never = AuditLog::open(dir.path().join("never.jsonl"))
            .unwrap()
            .rotate(0, 1);
        for _ in 0..3 {
            never.event("w", "POST", "/x", 200, "e");
        }
        assert!(never.rotated().is_empty());
        assert_eq!(never.tail(10).unwrap().len(), 3);
    }

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
