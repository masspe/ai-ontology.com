// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Background jobs: a sync, a re-index, anything that outlives a request.
//! One list, each job with its status, progress and the tail of its log,
//! persisted in `<data>/jobs.json` so a restart shows what was running
//! (marked *interrupted*) and what finished. A job runs on its own task
//! and checks `cancelled()` between batches; `DELETE /jobs/:id` asks it
//! to stop. Routes: `GET /jobs`, `GET /jobs/:id`, `DELETE /jobs/:id`.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::auth::now_rfc3339;

/// Jobs kept in the list (and the file); older finished ones are dropped.
const KEEP: usize = 200;
/// Log lines kept per job.
const LOG_TAIL: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Running,
    Done,
    Failed,
    Cancelled,
    /// Was running when the process stopped.
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: u64,
    /// What kind of work: `sync`, `reindex`, … (one worker per kind is
    /// the caller's rule, see [`Jobs::running`]).
    pub kind: String,
    /// What it works on, for people: a mapping's name, a file.
    pub label: String,
    pub status: JobStatus,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    /// Units done, and the total when known.
    pub progress: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    /// The last word: an error, a summary.
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub log: VecDeque<String>,
    /// What the job produced, for the API (a sync's counts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// Who started it.
    #[serde(default)]
    pub by: String,
}

#[derive(Default, Serialize, Deserialize)]
struct Inner {
    next_id: u64,
    jobs: VecDeque<Job>,
}

/// The job list: shared by the routes, the workers and the scheduler.
pub struct Jobs {
    file: Option<PathBuf>,
    inner: Mutex<Inner>,
    cancels: Mutex<Vec<(u64, Arc<AtomicBool>)>>,
}

impl std::fmt::Debug for Jobs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Jobs").field("file", &self.file).finish()
    }
}

impl Default for Jobs {
    fn default() -> Self {
        Self::in_memory()
    }
}

impl Jobs {
    /// A list that forgets at exit (tests, the CLI).
    pub fn in_memory() -> Self {
        Self {
            file: None,
            inner: Mutex::new(Inner::default()),
            cancels: Mutex::new(Vec::new()),
        }
    }

    /// The list kept in `file`; a job that was running when the file was
    /// last written is marked interrupted.
    pub fn open(file: PathBuf) -> std::io::Result<Self> {
        let mut inner: Inner = match std::fs::read(&file) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Inner::default(),
            Err(e) => return Err(e),
        };
        let now = now_rfc3339();
        for j in inner
            .jobs
            .iter_mut()
            .filter(|j| j.status == JobStatus::Running)
        {
            j.status = JobStatus::Interrupted;
            j.finished_at = Some(now.clone());
            j.message = "interrupted: the server stopped while it ran".into();
        }
        let jobs = Self {
            file: Some(file),
            inner: Mutex::new(inner),
            cancels: Mutex::new(Vec::new()),
        };
        jobs.persist();
        Ok(jobs)
    }

    fn persist(&self) {
        let Some(file) = &self.file else {
            return;
        };
        let bytes = serde_json::to_vec_pretty(&*self.inner.lock()).expect("jobs serialise");
        let tmp = file.with_extension("json.tmp");
        let written = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, file));
        if let Err(e) = written {
            tracing::warn!(file = %file.display(), error = %e, "jobs file not written");
        }
    }

    pub fn list(&self) -> Vec<Job> {
        self.inner.lock().jobs.iter().rev().cloned().collect()
    }

    pub fn get(&self, id: u64) -> Option<Job> {
        self.inner.lock().jobs.iter().find(|j| j.id == id).cloned()
    }

    /// The running job of `kind` whose label is `label`, if any: one sync
    /// per mapping at a time.
    pub fn running(&self, kind: &str, label: &str) -> Option<u64> {
        self.inner
            .lock()
            .jobs
            .iter()
            .find(|j| j.status == JobStatus::Running && j.kind == kind && j.label == label)
            .map(|j| j.id)
    }

    /// Register a job and run `work` on its own task. The job's handle
    /// reports progress and log lines and answers `cancelled()`; the
    /// task's outcome sets the final status.
    pub fn start<F, Fut>(self: &Arc<Self>, kind: &str, label: &str, by: &str, work: F) -> u64
    where
        F: FnOnce(JobHandle) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<serde_json::Value, String>> + Send + 'static,
    {
        let cancel = Arc::new(AtomicBool::new(false));
        let id = {
            let mut inner = self.inner.lock();
            inner.next_id += 1;
            let id = inner.next_id;
            inner.jobs.push_back(Job {
                id,
                kind: kind.to_string(),
                label: label.to_string(),
                status: JobStatus::Running,
                created_at: now_rfc3339(),
                finished_at: None,
                progress: 0,
                total: None,
                message: String::new(),
                log: VecDeque::new(),
                result: None,
                by: by.to_string(),
            });
            id
        };
        self.prune();
        self.cancels.lock().push((id, cancel.clone()));
        self.persist();
        let handle = JobHandle {
            id,
            jobs: self.clone(),
            cancel,
        };
        let jobs = self.clone();
        tokio::spawn(async move {
            let cancelled = handle.cancel.clone();
            let outcome = work(handle).await;
            let status = match (&outcome, cancelled.load(Ordering::Relaxed)) {
                (_, true) => JobStatus::Cancelled,
                (Ok(_), false) => JobStatus::Done,
                (Err(_), false) => JobStatus::Failed,
            };
            jobs.finish(id, status, outcome);
        });
        id
    }

    fn finish(&self, id: u64, status: JobStatus, outcome: Result<serde_json::Value, String>) {
        {
            let mut inner = self.inner.lock();
            if let Some(j) = inner.jobs.iter_mut().find(|j| j.id == id) {
                j.status = status;
                j.finished_at = Some(now_rfc3339());
                match outcome {
                    Ok(v) => {
                        if j.message.is_empty() {
                            j.message = match status {
                                JobStatus::Cancelled => "cancelled".into(),
                                _ => "done".into(),
                            };
                        }
                        j.result = Some(v);
                    }
                    Err(e) => j.message = e,
                }
            }
        }
        self.cancels.lock().retain(|(i, _)| *i != id);
        self.prune();
        self.persist();
    }

    /// Drop the oldest finished jobs beyond `KEEP`; a running one stays.
    fn prune(&self) {
        let mut inner = self.inner.lock();
        while inner.jobs.len() > KEEP {
            let Some(pos) = inner
                .jobs
                .iter()
                .position(|j| j.status != JobStatus::Running)
            else {
                break;
            };
            inner.jobs.remove(pos);
        }
    }

    /// Ask a running job to stop; `false` when no such job runs.
    pub fn cancel(&self, id: u64) -> bool {
        match self.cancels.lock().iter().find(|(i, _)| *i == id) {
            Some((_, flag)) => {
                flag.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    fn update(&self, id: u64, f: impl FnOnce(&mut Job)) {
        let mut inner = self.inner.lock();
        if let Some(j) = inner.jobs.iter_mut().find(|j| j.id == id) {
            f(j);
        }
    }
}

/// What a running job holds: report progress, log, learn of a cancel.
#[derive(Clone)]
pub struct JobHandle {
    pub id: u64,
    jobs: Arc<Jobs>,
    cancel: Arc<AtomicBool>,
}

impl JobHandle {
    pub fn progress(&self, done: u64, total: Option<u64>) {
        self.jobs.update(self.id, |j| {
            j.progress = done;
            if total.is_some() {
                j.total = total;
            }
        });
        // shortcut: progress is not persisted on every call (a sync
        // reports per batch); the file catches up at the next log line or
        // at the end. Persist here too if a restart must resume mid-job.
    }

    pub fn log(&self, line: impl Into<String>) {
        let line = line.into();
        tracing::info!(job = self.id, "{line}");
        self.jobs.update(self.id, |j| {
            j.message = line.clone();
            j.log.push_back(line);
            while j.log.len() > LOG_TAIL {
                j.log.pop_front();
            }
        });
        self.jobs.persist();
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

// ---------------------------------------------------------------- routes

#[derive(Deserialize)]
pub struct ListQuery {
    /// Only the running jobs.
    #[serde(default)]
    active: bool,
}

pub fn routes(jobs: Arc<Jobs>) -> Router {
    Router::new()
        .route("/jobs", get(list))
        .route("/jobs/:id", get(one).delete(cancel))
        .with_state(jobs)
}

async fn list(
    State(jobs): State<Arc<Jobs>>,
    Query(q): Query<ListQuery>,
) -> Json<serde_json::Value> {
    let mut all = jobs.list();
    if q.active {
        all.retain(|j| j.status == JobStatus::Running);
    }
    Json(serde_json::json!({ "jobs": all }))
}

async fn one(State(jobs): State<Arc<Jobs>>, Path(id): Path<u64>) -> Response {
    match jobs.get(id) {
        Some(j) => Json(j).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("job {id} not found") })),
        )
            .into_response(),
    }
}

async fn cancel(State(jobs): State<Arc<Jobs>>, Path(id): Path<u64>) -> Response {
    if jobs.cancel(id) {
        Json(serde_json::json!({ "ok": true, "id": id })).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("job {id} is not running") })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn a_job_reports_progress_finishes_and_is_listed_newest_first() {
        let jobs = Arc::new(Jobs::in_memory());
        let id = jobs.start("sync", "orders", "ann", |h| async move {
            h.progress(1, Some(3));
            h.log("one");
            h.progress(3, None);
            Ok(serde_json::json!({ "rows": 3 }))
        });
        for _ in 0..100 {
            if jobs.get(id).unwrap().status != JobStatus::Running {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let j = jobs.get(id).unwrap();
        assert_eq!(j.status, JobStatus::Done);
        assert_eq!((j.progress, j.total), (3, Some(3)));
        assert_eq!(j.result, Some(serde_json::json!({ "rows": 3 })));
        assert_eq!(j.log, ["one"]);
        assert_eq!(j.by, "ann");
        let failed = jobs.start("sync", "orders", "ann", |_| async {
            Err("boom".to_string())
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(jobs.get(failed).unwrap().status, JobStatus::Failed);
        assert_eq!(jobs.get(failed).unwrap().message, "boom");
        assert_eq!(
            jobs.list().iter().map(|j| j.id).collect::<Vec<_>>(),
            [failed, id]
        );
        assert!(jobs.running("sync", "orders").is_none());
        assert!(!jobs.cancel(id), "a finished job cannot be cancelled");
    }

    #[tokio::test]
    async fn a_cancel_reaches_the_job_and_a_restart_marks_running_ones_interrupted() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("jobs.json");
        let jobs = Arc::new(Jobs::open(file.clone()).unwrap());
        let id = jobs.start("sync", "slow", "ann", |h| async move {
            for i in 0..200 {
                if h.cancelled() {
                    return Ok(serde_json::json!({ "stopped_at": i }));
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            Ok(serde_json::json!({}))
        });
        assert_eq!(jobs.running("sync", "slow"), Some(id));
        assert!(jobs.cancel(id));
        for _ in 0..100 {
            if jobs.get(id).unwrap().status != JobStatus::Running {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(jobs.get(id).unwrap().status, JobStatus::Cancelled);
        // A job still "running" in the file (the process died) is interrupted at open.
        let raw = std::fs::read_to_string(&file).unwrap();
        std::fs::write(&file, raw.replace("\"cancelled\"", "\"running\"")).unwrap();
        let again = Jobs::open(file).unwrap();
        let j = again.get(id).unwrap();
        assert_eq!(j.status, JobStatus::Interrupted);
        assert!(j.message.contains("interrupted"));
        assert!(again.get(99).is_none());
    }

    #[tokio::test]
    async fn the_list_keeps_the_last_jobs_and_never_drops_a_running_one() {
        let jobs = Arc::new(Jobs::in_memory());
        let long = jobs.start("sync", "long", "ann", |h| async move {
            while !h.cancelled() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            Ok(serde_json::json!({}))
        });
        for i in 0..(KEEP + 20) {
            jobs.start("sync", &format!("j{i}"), "ann", |_| async {
                Ok(serde_json::json!({}))
            });
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        let list = jobs.list();
        assert!(list.len() <= KEEP + 1, "{}", list.len());
        assert!(list.iter().any(|j| j.id == long), "the running job stays");
        jobs.cancel(long);
    }
}
