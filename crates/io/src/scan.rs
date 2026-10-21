// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! The directory walk and the worker pool shared by the file readers
//! (`xref`, `abl`): one place for the thread default, the poisoned-lock
//! handling and the rule that a reader's panic costs that file only.

use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};

/// Every file under `root` (searched recursively) that `keep` accepts, or
/// `root` itself when it is a file. Sorted, so whatever follows is
/// deterministic.
pub fn walk_files(root: &Path, keep: impl Fn(&Path) -> bool) -> std::io::Result<Vec<PathBuf>> {
    fn walk(
        dir: &Path,
        keep: &dyn Fn(&Path) -> bool,
        out: &mut Vec<PathBuf>,
    ) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let p = entry?.path();
            if p.is_dir() {
                walk(&p, keep, out)?;
            } else if keep(&p) {
                out.push(p);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    if root.is_file() {
        out.push(root.to_path_buf());
    } else {
        walk(root, &keep, &mut out)?;
    }
    out.sort();
    Ok(out)
}

/// `threads`, or the number of cores; never less than one.
pub fn thread_count(threads: Option<usize>) -> usize {
    threads
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()))
        .max(1)
}

/// Apply `read` to every path on `threads` threads. `read` returns the
/// file's size and its result; a panic inside it becomes an `Err` for
/// that file — never a silently missing one. Results come back in
/// completion order.
pub fn map_files<T: Send + 'static>(
    paths: Vec<PathBuf>,
    threads: Option<usize>,
    read: impl Fn(&Path) -> (u64, Result<T, String>) + Send + Sync + 'static,
) -> Vec<(PathBuf, u64, Result<T, String>)> {
    let read = Arc::new(read);
    let queue = Arc::new(Mutex::new(paths.into_iter()));
    let (tx, rx) = mpsc::channel::<(PathBuf, u64, Result<T, String>)>();
    let workers: Vec<_> = (0..thread_count(threads))
        .map(|_| {
            let queue = Arc::clone(&queue);
            let tx = tx.clone();
            let read = Arc::clone(&read);
            std::thread::spawn(move || loop {
                let next = queue.lock().unwrap_or_else(|e| e.into_inner()).next();
                let Some(p) = next else { break };
                let (size, result) =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| read(&p)))
                        .unwrap_or_else(|_| (0, Err("the reader panicked on this file".into())));
                if tx.send((p, size, result)).is_err() {
                    break;
                }
            })
        })
        .collect();
    drop(tx);
    let out: Vec<_> = rx.into_iter().collect();
    for w in workers {
        let _ = w.join();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panicking_reader_costs_that_file_only() {
        let dir = std::env::temp_dir().join(format!("scan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(dir.join(name), name).unwrap();
        }
        let paths = walk_files(&dir, |p| p.extension().is_some_and(|e| e == "txt")).unwrap();
        assert_eq!(paths.len(), 3);
        let results = map_files(paths, Some(1), |p| {
            if p.ends_with("b.txt") {
                panic!("bad file");
            }
            (1, Ok(p.file_name().unwrap().to_string_lossy().into_owned()))
        });
        assert_eq!(
            results.len(),
            3,
            "the panic did not lose the rest of the queue"
        );
        let failed: Vec<_> = results.iter().filter(|(_, _, r)| r.is_err()).collect();
        assert_eq!(failed.len(), 1);
        assert!(failed[0].0.ends_with("b.txt"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_single_file_root_is_walked_as_itself() {
        let dir = std::env::temp_dir().join(format!("scan1-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("only.xml");
        std::fs::write(&f, "x").unwrap();
        assert_eq!(walk_files(&f, |_| false).unwrap(), vec![f.clone()]);
        assert_eq!(thread_count(Some(0)), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
