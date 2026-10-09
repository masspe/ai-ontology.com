# ai-ontology

**Turn the files your organisation already has into a knowledge graph it can question, and get answers that cite their sources.**

[![CI](https://github.com/masspe/ai-ontology.com/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/masspe/ai-ontology.com/actions/workflows/ci.yml)
[![Rust line coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fmasspe%2Fai-ontology.com%2Fcoverage-badges%2Frust.json)](docs/ARCHITECTURE.md#tests-and-coverage)
[![Web line coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fmasspe%2Fai-ontology.com%2Fcoverage-badges%2Fweb.json)](docs/ARCHITECTURE.md#tests-and-coverage)
[![Image build and end-to-end test](https://img.shields.io/badge/image-built%20%26%20tested%20in%20CI-2ea44f)](.github/workflows/ci.yml)
[![Upgrade tested](https://img.shields.io/badge/upgrade-previous%20main%20%E2%86%92%20current-2ea44f)](scripts/upgrade_check.sh)
[![OpenAPI](https://img.shields.io/badge/API-OpenAPI%203-6BA539)](crates/server/src/openapi.rs)
[![Rust](https://img.shields.io/badge/Rust-2021%20edition-dea584)](Cargo.toml)
[![License: AGPL-3.0 or commercial](https://img.shields.io/badge/license-AGPL--3.0%20or%20commercial-blue)](#license)
[![PRs welcome](https://img.shields.io/badge/PRs-welcome-brightgreen)](CONTRIBUTING.md)

Contracts, invoices, quotes, site reports, procedures, spreadsheets: most of
what a company knows sits in files that nobody can query. `ai-ontology`
reads those files, proposes typed **sheets** (a contract, a company, an
invoice line) and the **links** between them, lets you review what it found,
and then answers questions in plain language from that graph. Every answer
lists the sheets it was built from, so you can check it. Everything runs on
your own machine or server, in one container per client.

https://github.com/user-attachments/assets/c3a237bc-b532-47c3-b48e-d834c7e500e9

*An empty base, the finance example loaded in one click, a question answered
from the graph with its citations, the graph filtered to companies, people
and contracts, one contract opened with its links.*

<details>
<summary>Same walkthrough as a GIF</summary>

![Demo: load the finance example, ask which invoices were issued to a company and under which contracts, explore the graph](docs/demo.gif)

</details>

## Who it is for

- **A small or medium business** whose contracts, invoices and project files
  live in folders and mailboxes, and who wants to ask "which invoices did we
  issue to this client, under which contract?" and trust the answer.
- **A larger organisation** that needs the same thing per subsidiary,
  department or client, with accounts, API keys, an audit trail, metrics,
  backups and a memory budget per deployment, on infrastructure it controls.
- **A developer** who wants a graph database with typed relations, hybrid
  retrieval (lexical + vector + graph expansion) and a retrieval-augmented
  generation pipeline behind one HTTP API, in Rust, with no external
  database to run.

## What problem it solves

A chatbot over PDFs gives fluent answers that nobody can verify. Full-text
search finds documents, not facts. Both leave the knowledge where it was:
unstructured. `ai-ontology` takes a different route:

1. **Describe your data** in everyday words, or pick a ready-made model
   (contracts and invoices, people and organisations, equipment and
   procedures). The model is a small ontology: sheet types, their fields,
   the link types between them.
2. **Drop your files.** Word, Excel, PDF, CSV, JSONL, plain text. The import
   proposes sheets and links that match the model; you review, correct and
   confirm. Nothing enters the graph unreviewed unless you say so.
3. **Ask.** A question is answered from the sheets and links retrieved for
   it, by the language model of your choice, and comes back with the list
   of sheets it used. The model is held to the graph: it cannot cite a sheet
   that does not exist.

The result is a knowledge base that stays structured, that people can browse
as a graph or sheet by sheet, and that an application can query through the
API.

## Try it in five minutes

```bash
docker compose up -d --build
# open http://localhost:5001 — create the first account (it becomes the administrator),
# then click « Essayer avec l'exemple finance » (try the finance example) and ask a question.
```

The example (three contracts, invoices and line items in Excel, companies and
people) and the questions it answers are in [`examples/finance`](examples/finance/README.md).
To answer questions with a real language model, open **Settings** and enter
a key for Anthropic, OpenAI, DeepSeek or Infomaniak; without one, an offline
echo model shows the retrieved context instead of an answer.

Without Docker: `cargo build --release`, then `./target/release/ontology --data ./data serve --web web/dist --login`
after `cd web && npm ci && npm run build`. The full developer setup is in
[docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).

## What you get

| | |
|---|---|
| **Import** | Word, Excel, PDF (with OCR for scans), CSV, JSONL, triples, plain text; an import assistant that proposes sheets and links for review before anything is written. |
| **Data model** | Sheet types with fields and parent types, link types with source and target types, generated from a description or chosen from ready-made models, editable with an impact preview before a change. |
| **Explore** | A graph view with type filters and focus, a sheet page with links both ways and origin documents, saved questions, a home page with what to do next. |
| **Ask** | Questions in plain language answered from the retrieved subgraph, with citations, streamed; lexical and vector retrieval fused per request, bounded graph expansion. |
| **Rules and actions** | Rules and actions attached to sheet types, managed in the interface and through the API. |
| **Accounts and keys** | Built-in sign-up and login (the first account is the administrator), named API keys for machine callers, audit log of every write. |
| **API** | REST with an OpenAPI description served at `/docs`; Server-Sent Events for streamed answers; Prometheus metrics at `/metrics`. |
| **Operations** | One container per client, health probe, clean stop, incremental backup and verified restore, store format migrated on open, memory budget read from the container limit. |
| **Language models** | Anthropic, OpenAI, DeepSeek, Infomaniak (Swiss-hosted open models), or an offline echo model; keys stored in the data directory, never in the environment; prompt caching where the provider supports it. |

## Built to run in production

The storage and memory layers were designed and measured for stores well
beyond what a laptop holds. The figures below were measured on a 16 GB
GitHub runner and on the development machine; the method is in
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#benchmarks).

- **Nothing is lost on a crash.** Every write goes to disk before it changes
  the graph in memory, in a segmented store with CRC-checked records and one
  `fsync` per batch. A torn tail is truncated on restart; a lost positional
  index is rebuilt from the data.
- **It refuses rather than dies.** Before loading, the binary estimates each
  storage domain from the store's manifest and compares it with the budget
  it reads from the container limit. In `strict` mode an oversized store
  refuses to start and prints both figures, instead of being killed later.
- **Capacity.** 2 million sheets and 10 million links on a 16 GB node with
  payloads kept on disk (`--tier p1`, 6.4 GB of heap after loading); about
  9.5 million sheets and 47 million links on a 64 GB node, 13 million and
  63 million when the node is dedicated.
- **Retrieval stays fast at that size.** At 2 million sheets, `/retrieve`
  answers in 66 ms at the 99th percentile and a full reindex takes 38 s.
  Traversals are capped by depth and by node count, so the cost of a question
  depends on the question, not on the size of the graph.
- **Backups are cheap and verified.** Sealed segments are immutable, so a
  repeated backup copies only what changed (a 819 MB store: 30 s the first
  time, 45 MB the second). A restore replays the whole store and is compared
  with the original in the test suite and in the container end-to-end test.
- **Upgrades are tested before they are merged.** The CI builds the previous
  `main`, writes two stores with it, and makes the new version open, extend,
  compact, back up and restore them.
- **Tested.** Rust and web suites with a 90 % line-coverage floor enforced
  per crate by the CI; the container image is built and exercised end to end
  (sign-up, login, API, UI, backup, restore, restart) on every push.

## For larger organisations

- **One container per entity.** Each subsidiary, department or client gets its
  own container, volume, accounts, memory limit and backups: isolation by
  construction, routing by hostname at your reverse proxy.
- **Your data stays with you.** Nothing leaves your infrastructure except the
  prompts you send to the language model you chose; a Swiss-hosted provider
  is supported out of the box, and the model can be changed per request.
- **Accountability.** Named API keys per integration, an audit log of every
  write with its caller, Prometheus metrics per storage domain, an OpenAPI
  description for your integration team.
- **A licence that fits.** AGPL for open use, or a commercial licence with
  support for organisations that cannot publish their modifications.

## HTTP API

Everything the UI does goes through the API; the full description is served
by the binary at `/docs` (Swagger UI) and `/openapi.json`.

| Method | Path | What it does |
|---|---|---|
| `GET` | `/concepts`, `/concepts/:id`, `/relations` | Sheets and links, paginated by cursor, filtered by type and text. |
| `POST` | `/concepts`, `/relations`, `/upload` | Create a sheet or a link; import a file (multipart). |
| `POST` | `/retrieve`, `/ask`, `/ask/stream` | Ranked sheets and subgraph for a question; the answer with citations; the same streamed. |
| `GET` / `PUT` | `/ontology` | Read or replace the data model. |
| `POST` | `/ontology/generate`, `/ingest/analyze` | Draft a model from a description; propose sheets and links for a file. |
| `POST` | `/backup`, `/compact` | Incremental backup to the configured directory; rewrite the store from the live graph. |
| `*` | `/auth/*`, `/audit`, `/metrics`, `/healthz`, `/stats` | Accounts and keys, audit log, Prometheus metrics, probes. |

Command line, for scripts and batch jobs:

```bash
ontology --data ./data ingest --ontology examples/sample-ontology.json examples/sample.triples
ontology --data ./data retrieve "retrieval augmented generation"
ontology --data ./data ask "Who wrote about RAG?"
ontology --data ./data backup ./backups/today
ontology --data ./data serve --bind 0.0.0.0:5000 --web web/dist --login
```

## Documentation

| Document | What it covers |
|---|---|
| [docs/DEPLOY.md](docs/DEPLOY.md) | Installing with the image and compose, first account, API keys, memory, backup and restore, upgrading. |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Crates, request path, storage, retrieval and ranking, graph algorithms, concurrency, memory tiers, observability, benchmarks. |
| [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) | Running the dev stack, tests, coverage, the things that cost an hour if you do not know them. |
| [docs/LLM-PROVIDERS.md](docs/LLM-PROVIDERS.md) | Configuring a provider, Infomaniak AI Tools, prompt caching. |
| [docs/STORAGE.md](docs/STORAGE.md), [docs/PERFORMANCE.md](docs/PERFORMANCE.md) | The store format and the read-path rules, in French. |
| [examples/finance](examples/finance/README.md), [examples/models](examples/models/README.md) | The demo data set and the ready-made models. |
| [auth-server/README.md](auth-server/README.md) | The optional Node server, only needed for Google / Microsoft sign-in. |

## Security

The CI refuses files that hide executable code, pins every action to a
commit, runs `cargo audit` and `npm audit` on every push, and installs npm
packages with scripts disabled. Vulnerabilities go through GitHub's private
reporting: see [SECURITY.md](SECURITY.md).

## License

Copyright © 2026 Mediasoft & Cie S.A.

Dual-licensed: **AGPL-3.0-or-later** ([LICENSE](LICENSE)) for open use, where
running a modified version as a network service means publishing its source;
or a **commercial licence** ([LICENSE-COMMERCIAL.md](LICENSE-COMMERCIAL.md))
that removes that obligation and comes with support. Contributions are
welcome under the terms in [CONTRIBUTING.md](CONTRIBUTING.md).
