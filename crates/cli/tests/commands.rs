// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! The remaining commands of the `ontology` binary, end to end: `retrieve`
//! and `ask` with the offline echo model and the settings-file resolution
//! (`--settings`, `<data>/settings.json`, `./.ontology/settings.json`),
//! the ingest formats not driven elsewhere (`.triples`, `.csv`, stdin) and
//! their refusals, and `serve` on a free port with seeding, `/healthz`,
//! `/stats`, one concept, and the auth flags' early refusals.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn tempdir(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "ontology-cmd-{tag}-{}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn finance() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .join("finance")
}

fn p(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

/// The binary with logging silenced so stdout holds only command output.
fn ontology(data: Option<&Path>) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ontology"));
    cmd.env("RUST_LOG", "error");
    for k in ["ONTOLOGY_SEED_DIR", "ONTOLOGY_TIER", "ONTOLOGY_MEMORY_MODE"] {
        cmd.env_remove(k);
    }
    if let Some(d) = data {
        cmd.arg("--data").arg(d);
    }
    cmd
}

fn run(data: Option<&Path>, args: &[&str]) -> Output {
    ontology(data).args(args).output().expect("spawn ontology")
}

fn ok(data: Option<&Path>, args: &[&str]) -> String {
    let out = run(data, args);
    assert!(
        out.status.success(),
        "`ontology {}` failed ({:?})\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        out.status.code(),
        text(&out.stdout),
        text(&out.stderr)
    );
    text(&out.stdout)
}

fn fail(data: Option<&Path>, args: &[&str]) -> String {
    let out = run(data, args);
    assert!(
        !out.status.success(),
        "`ontology {}` unexpectedly succeeded\nstdout:\n{}",
        args.join(" "),
        text(&out.stdout)
    );
    assert_ne!(out.status.code(), Some(0));
    text(&out.stderr)
}

/// A store holding the finance schema and its six seed concepts.
fn seeded(data: &Path) {
    let f = finance();
    let out = ok(
        Some(data),
        &[
            "ingest",
            "--ontology",
            &p(&f.join("ontology.json")),
            &p(&f.join("seed.jsonl")),
        ],
    );
    assert_eq!(
        out.trim(),
        "ingested: 6 concepts, 3 relations, 0 ontology updates"
    );
}

/// A settings file that selects OpenAI without a key: configured, unusable.
const OPENAI_NO_KEY: &str = r#"{"llm":{"active_provider":"openai"}}"#;

/// `retrieve` ranks the matching concept first and prints the expanded
/// subgraph with its edges; `ask` answers through the echo model when no
/// provider is configured (also with `--model`, which only retargets a
/// configured provider) and cites the retrieved concepts.
#[test]
fn retrieve_and_ask_answer_offline_with_the_echo_model() {
    let data = tempdir("ask");
    seeded(&data);

    let out = ok(
        Some(&data),
        &["retrieve", "Globex", "--top-k", "3", "--depth", "1"],
    );
    // The header counts what was scored: at most --top-k, never more.
    let n: usize = out
        .lines()
        .next()
        .and_then(|l| l.strip_prefix("# top-")?.strip_suffix(" concepts"))
        .unwrap_or_else(|| panic!("{out}"))
        .parse()
        .unwrap();
    assert!((1..=3).contains(&n), "{out}");
    let row = out
        .lines()
        .find(|l| l.contains("(Company) Globex"))
        .unwrap_or_else(|| {
            panic!(
                "Globex is retrieved:
{out}"
            )
        });
    assert!(row.contains("[lex=") && row.contains("vec="), "{out}");
    assert!(out.contains("\n# subgraph: "), "{out}");
    assert!(out.contains("]-> "), "expanded edges are listed:\n{out}");

    // A query no concept matches: the headers still print, no rows.
    let out = ok(Some(&data), &["retrieve", "zzqx", "--top-k", "0"]);
    assert_eq!(out.lines().next().unwrap(), "# top-0 concepts");

    // No provider: echo. The answer echoes the prompt, which carries the
    // question and the retrieved context.
    let out = ok(Some(&data), &["ask", "Who is Globex?", "--top-k", "2"]);
    assert!(out.starts_with("--- answer ---\n[echo] "), "{out}");
    assert!(out.contains("Who is Globex?"), "{out}");
    assert!(out.contains("--- citations ---\n"), "{out}");
    assert!(out.contains("  (Company) Globex  score="), "{out}");
    assert!(
        !out.contains("--- usage ---"),
        "the echo model reports no tokens:\n{out}"
    );
    // `--model` alone selects nothing: still the echo model.
    let out = ok(
        Some(&data),
        &[
            "ask",
            "Who is Globex?",
            "--model",
            "gpt-4o-mini",
            "--depth",
            "0",
        ],
    );
    assert!(out.starts_with("--- answer ---\n[echo] "), "{out}");

    // A missing --settings file falls back to the defaults (echo), and a
    // provider selected without its key is an error naming the file, even
    // with a `--model` override.
    let out = ok(
        Some(&data),
        &[
            "--settings",
            &p(&data.join("nope").join("settings.json")),
            "ask",
            "Globex",
        ],
    );
    assert!(out.contains("[echo]"), "{out}");
    let bad = data.join("llm.json");
    std::fs::write(&bad, OPENAI_NO_KEY).unwrap();
    let err = fail(Some(&data), &["--settings", &p(&bad), "ask", "Globex"]);
    assert!(err.contains("configuration LLM inutilisable"), "{err}");
    assert!(err.contains("llm.json"), "{err}");
    assert!(err.contains("OpenAI"), "{err}");
    let err = fail(
        Some(&data),
        &["--settings", &p(&bad), "ask", "Globex", "--model", "gpt-4o"],
    );
    assert!(err.contains("configuration LLM inutilisable"), "{err}");
    let _ = std::fs::remove_dir_all(&data);
}

/// Without `--settings` the file is `<data>/settings.json`, and without
/// `--data` it is `./.ontology/settings.json` in the working directory.
#[test]
fn settings_file_defaults_to_data_dir_then_dot_ontology() {
    let data = tempdir("settings-data");
    std::fs::write(data.join("settings.json"), OPENAI_NO_KEY).unwrap();
    let err = fail(Some(&data), &["ask", "anything"]);
    assert!(err.contains("configuration LLM inutilisable"), "{err}");
    assert!(
        err.contains(&p(&data.join("settings.json"))),
        "names <data>/settings.json:\n{err}"
    );

    let cwd = tempdir("settings-cwd");
    std::fs::create_dir_all(cwd.join(".ontology")).unwrap();
    std::fs::write(cwd.join(".ontology").join("settings.json"), OPENAI_NO_KEY).unwrap();
    let out = ontology(None)
        .current_dir(&cwd)
        .args(["ask", "anything"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = text(&out.stderr);
    assert!(err.contains("configuration LLM inutilisable"), "{err}");
    assert!(
        err.contains(
            &Path::new(".ontology")
                .join("settings.json")
                .to_string_lossy()
                .into_owned()
        ),
        "names ./.ontology/settings.json:\n{err}"
    );
    // Nothing configured in the working directory: the echo model answers
    // on the empty in-memory graph.
    let empty = tempdir("settings-none");
    let out = ontology(None)
        .current_dir(&empty)
        .args(["ask", "anything"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    assert!(stdout.starts_with("--- answer ---\n[echo] "), "{stdout}");
    assert_eq!(
        stdout.trim_end().lines().last().unwrap(),
        "--- citations ---"
    );
    for d in [data, cwd, empty] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// `.triples` lines create both endpoints and the relation; `.csv` rows
/// become concepts of `--csv-type` with the other columns as properties;
/// `-` reads JSONL from stdin. Each refusal exits non-zero with its cause
/// and leaves the store as it was.
#[test]
fn ingest_triples_csv_and_stdin() {
    let data = tempdir("formats");
    let f = finance();
    let onto = p(&f.join("ontology.json"));
    let none = data.join("none.jsonl");
    std::fs::write(&none, "").unwrap();
    let out = ok(Some(&data), &["ingest", "--ontology", &onto, &p(&none)]);
    assert!(out.starts_with("ingested: 0 concepts"), "{out}");

    let triples = data.join("orgs.triples");
    std::fs::write(
        &triples,
        "# comment\nPerson:Zoe employed_by Company:Umbrella\n\nCompany:Umbrella parent_company Company:Hooli\n",
    )
    .unwrap();
    let out = ok(Some(&data), &["ingest", &p(&triples)]);
    assert_eq!(
        out.trim(),
        "ingested: 3 concepts, 2 relations, 0 ontology updates"
    );

    let csv = data.join("people.csv");
    std::fs::write(&csv, "name,role\nYann,CFO\n\"Lee, Ann\",CTO\n").unwrap();
    let err = fail(Some(&data), &["ingest", &p(&csv)]);
    assert!(err.contains("--csv-type required"), "{err}");
    let out = ok(Some(&data), &["ingest", "--csv-type", "Person", &p(&csv)]);
    assert_eq!(
        out.trim(),
        "ingested: 2 concepts, 0 relations, 0 ontology updates"
    );

    let out = ontology(Some(&data))
        .args(["ingest", "-"])
        .stdin(Stdio::from(
            std::fs::File::open(f.join("seed.jsonl")).unwrap(),
        ))
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout).trim(),
        "ingested: 6 concepts, 3 relations, 0 ontology updates"
    );

    // Everything landed and is read back: 3 + 2 + 6 concepts, 2 + 3 relations,
    // and the CSV properties survive the round trip.
    let stats = ok(Some(&data), &["stats"]);
    assert!(stats.contains("concepts: 11\nrelations: 5\n"), "{stats}");
    let export = data.join("out.jsonl");
    let out = ok(Some(&data), &["export", &p(&export)]);
    assert!(
        out.starts_with("exported: 11 concepts, 5 relations -> "),
        "{out}"
    );
    let dump = std::fs::read_to_string(&export).unwrap();
    assert!(dump.contains(r#""name":"Lee, Ann""#), "{dump}");
    assert!(dump.contains(r#""role":"CTO""#), "{dump}");
    assert!(
        dump.contains(r#""relation_type":"parent_company""#),
        "{dump}"
    );

    // Refusals: a path that does not exist, a malformed ontology file, a
    // malformed triple line. The store is unchanged.
    let err = fail(Some(&data), &["ingest", &p(&data.join("missing.jsonl"))]);
    assert!(!err.trim().is_empty());
    let broken = data.join("broken.json");
    std::fs::write(&broken, "{ not json").unwrap();
    let err = fail(
        Some(&data),
        &["ingest", "--ontology", &p(&broken), &p(&triples)],
    );
    assert!(
        err.contains("key must be a string") || err.contains("expected"),
        "{err}"
    );
    std::fs::write(&triples, "Person:Ann owns\n").unwrap();
    let err = fail(Some(&data), &["ingest", &p(&triples)]);
    assert!(err.contains("expected `Subj predicate Obj`"), "{err}");
    assert!(ok(Some(&data), &["stats"]).contains("concepts: 11\nrelations: 5\n"));
    let _ = std::fs::remove_dir_all(&data);
}

/// A port the OS just handed out and we released: free at this instant.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// One HTTP/1.1 GET over a raw socket: `(status, body)`.
fn http_get(port: u16, path: &str) -> Option<(u16, String)> {
    let addr = format!("127.0.0.1:{port}").parse().unwrap();
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(500)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(10))).ok()?;
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut raw = String::new();
    s.read_to_string(&mut raw).ok()?;
    let status = raw.split_whitespace().nth(1)?.parse().ok()?;
    let body = raw.split_once("\r\n\r\n").map(|(_, b)| b.to_string())?;
    Some((status, body))
}

/// Ask the server to stop the way an operator does: SIGTERM on Unix,
/// Ctrl+Break on Windows (the child runs in its own console process group
/// so the signal reaches it alone). A killed process would also lose its
/// coverage profile.
fn request_stop(child: &std::process::Child) -> bool {
    #[cfg(unix)]
    {
        Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn GenerateConsoleCtrlEvent(ctrl_event: u32, process_group_id: u32) -> i32;
        }
        const CTRL_BREAK_EVENT: u32 = 1;
        // SAFETY: plain Win32 call with two integers; no memory is shared.
        unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, child.id()) != 0 }
    }
}

/// A server child; `stop` shuts it down and returns its log.
struct Server {
    child: std::process::Child,
    port: u16,
}

impl Server {
    /// Start `serve` and wait for `/healthz`, at most 30 s.
    fn start(cmd: &mut Command, serve_args: &[&str]) -> Self {
        let port = free_port();
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
        }
        let child = cmd
            .arg("serve")
            .arg("--bind")
            .arg(format!("127.0.0.1:{port}"))
            .args(serve_args)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn serve");
        let mut srv = Server { child, port };
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some((200, body)) = http_get(port, "/healthz") {
                assert_eq!(body, "ok");
                return srv;
            }
            if let Some(status) = srv.child.try_wait().unwrap() {
                let out = srv.child.wait_with_output().unwrap();
                panic!("serve exited early ({status}):\n{}", text(&out.stderr));
            }
            assert!(Instant::now() < deadline, "serve not healthy after 30 s");
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Signal the server, require a clean exit (code 0) within 30 s and
    /// return its stderr (the tracing log).
    fn stop(mut self) -> String {
        assert!(request_stop(&self.child), "could not signal the server");
        let deadline = Instant::now() + Duration::from_secs(30);
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                self.child.kill().unwrap();
                panic!("serve ignored the shutdown signal for 30 s");
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let log = text(&self.child.wait_with_output().unwrap().stderr);
        assert!(status.success(), "serve exited with {status}:\n{log}");
        assert!(log.contains("shutdown signal received"), "{log}");
        assert!(log.contains("server stopped"), "{log}");
        log
    }
}

/// `serve` binds the requested port, seeds an empty store from `--seed`
/// (schema, JSONL, text directory, spreadsheets, relations last), answers
/// `/healthz`, `/stats` and `/concepts/:id`, and holds the store LOCK. A
/// restart with a seed configured skips seeding because the store has
/// data; `<data>/seed` is found without any flag.
#[test]
fn serve_seeds_an_empty_store_and_answers_http() {
    let data = tempdir("serve");
    let mut cmd = ontology(Some(&data));
    cmd.env("RUST_LOG", "info");
    let srv = Server::start(&mut cmd, &["--seed", &p(&finance())]);

    let (status, body) = http_get(srv.port, "/stats").unwrap();
    assert_eq!(status, 200);
    let stats: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(stats["concepts"], 22, "{stats}");
    assert_eq!(stats["relations"], 38, "{stats}");
    assert_eq!(stats["concept_types"], 6, "{stats}");
    assert!(
        stats["memory"].is_object(),
        "a persistent store has a plan: {stats}"
    );

    let (status, body) = http_get(srv.port, "/concepts/1").unwrap();
    assert_eq!(status, 200, "{body}");
    let c: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(c["id"], 1);
    assert_eq!(c["concept_type"], "Company", "{c}");
    let (status, _) = http_get(srv.port, "/concepts/999999").unwrap();
    assert_eq!(status, 404);

    // The running server holds the LOCK: a second process is refused.
    let err = fail(Some(&data), &["stats"]);
    assert!(err.contains("locked by another process"), "{err}");

    let log = srv.stop();
    assert!(log.contains("auto-seeding empty graph"), "{log}");
    assert!(log.contains("seed: complete"), "{log}");
    assert!(log.contains("server listening"), "{log}");

    // The seed was persisted; the lock is gone with the process.
    let stats = ok(Some(&data), &["stats"]);
    assert!(stats.contains("concepts: 22\nrelations: 38\n"), "{stats}");

    // Restart with the seed from the environment: skipped, counts unchanged.
    let mut cmd = ontology(Some(&data));
    cmd.env("RUST_LOG", "info")
        .env("ONTOLOGY_SEED_DIR", finance());
    let srv = Server::start(&mut cmd, &[]);
    let (_, body) = http_get(srv.port, "/stats").unwrap();
    assert!(body.contains(r#""concepts":22"#), "{body}");
    let log = srv.stop();
    assert!(
        log.contains("skipping seed: graph already has data"),
        "{log}"
    );
    assert!(!log.contains("auto-seeding"), "{log}");

    // `<data>/seed` is picked up by itself on a fresh store.
    let data2 = tempdir("serve-data-seed");
    copy_dir(&finance(), &data2.join("seed"));
    let mut cmd = ontology(Some(&data2));
    cmd.env("RUST_LOG", "info");
    let srv = Server::start(&mut cmd, &[]);
    let (_, body) = http_get(srv.port, "/stats").unwrap();
    assert!(body.contains(r#""concepts":22"#), "{body}");
    let log = srv.stop();
    assert!(log.contains("auto-seeding empty graph"), "{log}");

    for d in [data, data2] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dst = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dst);
        } else {
            std::fs::copy(e.path(), dst).unwrap();
        }
    }
}

/// The auth flags name environment variables, never secrets: an unset or
/// empty variable is refused before the port is bound; a bad `--seed`
/// path is refused too. In-memory, no `--data` needed.
#[test]
fn serve_refuses_bad_auth_env_and_seed_before_binding() {
    let port = free_port();
    let bind = format!("127.0.0.1:{port}");
    let mut cmd = ontology(None);
    cmd.env_remove("ONTOLOGY_CLI_TEST_UNSET");
    let out = cmd
        .args([
            "serve",
            "--bind",
            &bind,
            "--auth-env",
            "ONTOLOGY_CLI_TEST_UNSET",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("env var `ONTOLOGY_CLI_TEST_UNSET` is unset"),
        "{}",
        text(&out.stderr)
    );

    let out = ontology(None)
        .env("ONTOLOGY_CLI_TEST_EMPTY", "  ")
        .args([
            "serve",
            "--bind",
            &bind,
            "--jwt-secret-env",
            "ONTOLOGY_CLI_TEST_EMPTY",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("env var `ONTOLOGY_CLI_TEST_EMPTY` is empty"),
        "{}",
        text(&out.stderr)
    );
    let mut cmd = ontology(None);
    cmd.env_remove("ONTOLOGY_CLI_TEST_UNSET");
    let out = cmd
        .args([
            "serve",
            "--bind",
            &bind,
            "--jwt-secret-env",
            "ONTOLOGY_CLI_TEST_UNSET",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("is unset"),
        "{}",
        text(&out.stderr)
    );

    let missing = std::env::temp_dir().join("ontology-cmd-no-such-seed-dir");
    let out = ontology(None)
        .args(["serve", "--bind", &bind, "--seed", &p(&missing)])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("is not a directory"),
        "{}",
        text(&out.stderr)
    );
    // Nothing is listening: every refusal happened before `bind`.
    assert!(http_get(port, "/healthz").is_none());
}
