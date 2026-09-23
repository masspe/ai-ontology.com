# ai-ontology.com

[![CI](https://github.com/masspe/ai-ontology.com/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/masspe/ai-ontology.com/actions/workflows/ci.yml)
[![Rust line coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fmasspe%2Fai-ontology.com%2Fcoverage-badges%2Frust.json)](#test-coverage)
[![Web line coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fmasspe%2Fai-ontology.com%2Fcoverage-badges%2Fweb.json)](#test-coverage)

A Rust workspace implementing an ontology-structured graph database with a
hybrid retrieval layer and a RAG pipeline that grounds language-model
answers in retrieved subgraphs.

**Where the project stands and what comes next:** [docs/ROADMAP.md](docs/ROADMAP.md)
(state, dated decisions, next steps with acceptance criteria, working process).

The coverage badges are **live**: after every push to `main` the CI measures
line coverage of the whole Rust suite and of the web UI and publishes the
figures (see [Test coverage](#test-coverage)). Green is ≥ 80 %, bright green
≥ 90 %, red < 30 %. The bar is **90 % minimum**, enforced on the web suite
by `vitest.config.ts` thresholds.

## Crates

| Crate              | Lines covered | Role |
| ------------------ | ------------- | ---- |
| `ontology-graph`   | ![graph line coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fmasspe%2Fai-ontology.com%2Fcoverage-badges%2Fcrate-graph.json&label=lines&style=flat-square) | Concepts, typed relations, schema validation, traversals. |
| `ontology-storage` | ![storage line coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fmasspe%2Fai-ontology.com%2Fcoverage-badges%2Fcrate-storage.json&label=lines&style=flat-square) | Binary segmented store (`<data>/store/`): framed records with CRC, positional index, per-batch fsync, torn-tail recovery, memory-mapped sealed segments; automatic migration from the legacy `graph.log`. Pluggable `Store` trait. Format in [docs/STORAGE.md](docs/STORAGE.md), plan in [docs/STORAGE-PLAN.md](docs/STORAGE-PLAN.md). |
| `ontology-index`   | ![index line coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fmasspe%2Fai-ontology.com%2Fcoverage-badges%2Fcrate-index.json&label=lines&style=flat-square) | Lexical (TF-IDF) + vector (cosine) + graph-expansion retrieval. |
| `ontology-io`      | ![io line coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fmasspe%2Fai-ontology.com%2Fcoverage-badges%2Fcrate-io.json&label=lines&style=flat-square) | `Source` / `Sink` traits with JSONL and triples adapters. |
| `ontology-rag`     | ![rag line coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fmasspe%2Fai-ontology.com%2Fcoverage-badges%2Fcrate-rag.json&label=lines&style=flat-square) | Prompt builder + `LanguageModel` trait (echo, Anthropic, OpenAI, DeepSeek; with prompt caching). |
| `ontology-server`  | ![server line coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fmasspe%2Fai-ontology.com%2Fcoverage-badges%2Fcrate-server.json&label=lines&style=flat-square) | axum HTTP server exposing `/concepts`, `/relations`, `/retrieve`, `/ask`, `/ontology`. |
| `ontology-cli`     | ![cli line coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fmasspe%2Fai-ontology.com%2Fcoverage-badges%2Fcrate-cli.json&label=lines&style=flat-square) | `ontology` binary tying it all together. |
| `web/`             | ![web line coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fmasspe%2Fai-ontology.com%2Fcoverage-badges%2Fweb.json&label=lines&style=flat-square) | Vite + React UI (Ask · Browse · Upload tabs). |

## Architecture

```
┌──────────────┐    ┌──────────────┐    ┌──────────────┐
│  Source       │───▶│  ingest_*    │───▶│ OntologyGraph│
│  (jsonl, …)   │    │  (validate)  │    │  + WAL Store │
└──────────────┘    └──────────────┘    └─────┬────────┘
                                              │
                              ┌───────────────┴───────────────┐
                              │           HybridIndex          │
                              │  ┌─────────┐  ┌─────────┐      │
                              │  │ Lexical │  │ Vector  │      │
                              │  └─────────┘  └─────────┘      │
                              │      ▲            ▲           │
                              │      └─────┬──────┘           │
                              │            ▼                   │
                              │     subgraph expansion         │
                              └───────────────┬────────────────┘
                                              │ ScoredConcept[] + Subgraph
                                              ▼
                              ┌────────────────────────────────┐
                              │  PromptBuilder → LanguageModel │
                              │  (Anthropic / OpenAI /         │
                              │   DeepSeek / Echo)             │
                              └────────────────────────────────┘
```

## Concurrency model

* The graph stores nodes/edges in `DashMap`s (sharded, lock-free reads).
* Schema mutations take a short `parking_lot::RwLock` write.
* `IdAllocator` is lock-free (`AtomicU64`).
* `HybridIndex` uses `RwLock`-protected inverted lists; reads are concurrent.
* The pipeline is `Send + Sync + Clone`, so a single `RagPipeline` value can
  serve many concurrent answers.

## Graph algorithms & optimizations

The graph layer keeps traversal **bounded, typed, and read-lock-free** so that
retrieval stays predictable even on dense ontologies. Everything below is
implemented in `ontology-graph` and `ontology-index`.

### Traversal & pathfinding (`crates/graph/src/traversal.rs`)

| Algorithm | Function | What it does |
| --------- | -------- | ------------ |
| **Bounded BFS shortest path** | `shortest_path` | Undirected breadth-first search between two concepts with unit edge weights, capped by `max_depth`. Tracks `parent` + incident `RelationId` to reconstruct the full edge-labelled path. Exposed as `POST /path`. |
| **N-hop subgraph expansion** | `expand` | Level-by-level BFS from a set of seeds, hard-bounded by both `max_depth` **and** `max_nodes`. Optional filters on relation type, concept type, and direction (`outgoing` / `incoming` / `both`). Returns the `Subgraph` fed to the prompt builder. |
| **Typed relation closure** | `closure` | BFS along a single relation type (in *and* out edges) to compute transitive chains such as `partOf` / `locatedIn`, used by the RAG layer to pull in implied context. |

All three are **depth- and node-capped on entry**, so a pathological "expand the
whole graph" request degrades gracefully instead of blowing up — the cost of a
retrieval is a function of `top_k` and `TraversalSpec`, not of total graph size.

### Indexing & ranking optimizations (`crates/index`)

* **Inverted-index lexical search** (`lexical.rs`) — a TF-IDF / BM25-style
  inverted index. Per-term IDF is `ln((n − df + 0.5) / (df + 0.5) + 1.0)`; the
  per-document weight is length-normalized as `sqrt(tf / doc_len) · idf`. Only
  documents that actually contain a query term are scored (no full scan).
* **Flat cosine vector search** (`vector.rs`, `embed.rs`) — vectors are
  **L2-normalized at insertion**, so similarity collapses to a plain dot
  product `Σ q[i]·doc[i]` — no per-query normalization in the hot loop.
* **Hybrid fusion with min-max normalization** (`hybrid.rs::rank`) — lexical and
  vector scores are each normalized to `[0,1]` against their own max, then
  blended:

  ```text
  score = lexical_weight · (lex / lex_max) + (1 − lexical_weight) · (vec / vec_max)
  ```

  `lexical_weight` is a per-request knob (default `0.5`).
* **Relevance-ratio noise floor** — candidates scoring below `0.4 ×` the top
  hit are dropped, so weak tail matches never reach the LLM prompt.
* **Adaptive candidate pools** — when a request adds type filters the candidate
  pool is widened (`4×` → `16×` `top_k`) so enough survivors remain after
  post-filtering, without paying that cost on unfiltered queries.
* **Trigram substring index** (`graph.rs`) — concept-name substring search uses
  a `[char; 3]` trigram inverted index; matching intersects the candidate sets
  for the query's trigrams instead of scanning every name (falls back to a
  linear scan only for queries shorter than 3 chars).

### Concurrency & memory optimizations

* **Sharded, lock-free reads** — concepts, relations, and typed in/out adjacency
  lists live in `DashMap`s; reads never take a global lock.
* **Generation-counter cache invalidation** — `AtomicU64` generation counters
  (`concepts_gen` / `relations_gen`) bump on any mutation and invalidate the
  bounded (capacity 256) pagination caches in one cheap compare, avoiding
  per-entry eviction bookkeeping.
* **Lock-free ID allocation** — `IdAllocator` hands out ids with a single
  `AtomicU64::fetch_add`; restore reconciles the watermark with
  `compare_exchange_weak`.
* **Typed adjacency** — edges are bucketed by relation type, so a
  relation-filtered expansion walks only the relevant bucket instead of every
  incident edge.

> Out of scope by design: there is **no** weighted shortest path (Dijkstra/A\*),
> centrality/PageRank, or approximate-nearest-neighbor (ANN) vector index. Edges
> are unit-weight and the vector index is exact/flat. These are natural
> extension points rather than current behavior.

## Why this solution (advantages)

* **Grounded answers, not hallucinations.** Every `/ask` response is built from
  a retrieved, type-validated subgraph, so the LLM cites concepts that actually
  exist in your knowledge base instead of inventing them.
* **Hybrid retrieval beats either half alone.** Lexical (exact-term) and vector
  (semantic) signals are fused per request; `lexical_weight` lets you dial
  between keyword precision and semantic recall without redeploying.
* **Structure-aware context.** Graph expansion and typed closures pull in
  *related* concepts (parties of a contract, line items of an invoice), giving
  the model context a flat vector store would miss.
* **Predictable, bounded cost.** Depth/node caps mean retrieval latency tracks
  `top_k` and traversal spec — not the size of the graph — so it scales to large
  ontologies without runaway queries.
* **Built for concurrency.** Lock-free sharded reads and a
  `Send + Sync + Clone` pipeline let one process serve many simultaneous
  answers; no per-request graph cloning.
* **Schema-validated ingestion.** Concepts and relations are checked against the
  ontology schema on the way in, so the graph stays consistent and the index
  never sees malformed nodes.
* **Durable + crash-safe storage.** Every write is journaled *before* the
  in-memory graph changes and `fsync`ed (one sync per batch), into a
  binary segmented store under `<data>/store/`: CRC-checked records, a
  positional index rebuilt from the data if lost, a torn tail truncated on
  restart, sealed segments memory-mapped. A legacy `graph.log` is migrated
  automatically (and verifiably) on first start. All behind a pluggable
  `Store` trait; format in [docs/STORAGE.md](docs/STORAGE.md), plan in
  [docs/STORAGE-PLAN.md](docs/STORAGE-PLAN.md).
* **Provider-agnostic LLM layer with caching.** Anthropic, OpenAI, DeepSeek, or
  an offline echo model behind one `LanguageModel` trait — with prompt/prefix
  caching that drops repeat-query input cost to ≈10% on a stable knowledge base.
* **Production-ready surface.** Bearer-auth middleware (constant-time, env-fed),
  Prometheus `/metrics`, SSE streaming, multipart upload, and a React UI ship in
  the box.
* **No lock-in, dual-licensed.** Pure Rust workspace, open formats (JSONL /
  triples / CSV / XLSX), AGPL **or** commercial — adopt on whichever track fits.

## Quickstart

### POSIX shell

```bash
cargo build --release
DATA=./data
./target/release/ontology --data $DATA ingest \
    --ontology examples/sample-ontology.json examples/sample.triples
./target/release/ontology --data $DATA stats
./target/release/ontology --data $DATA retrieve "retrieval augmented generation"
# Answers through the provider configured in $DATA/settings.json — see
# "Configuring an LLM provider" below. With none configured: echo model.
./target/release/ontology --data $DATA ask "Who wrote about RAG?"
# One-off model override, not written back to the config:
./target/release/ontology --data $DATA ask --model gpt-4o-mini "Who wrote about RAG?"
./target/release/ontology --data $DATA migrate           # legacy graph.log -> store/ (also automatic on start)
./target/release/ontology --data $DATA compact           # rewrite the store from the live graph (verified)
./target/release/ontology --data $DATA path \
    --from-type Person --from-name Alice \
    --to-type   Person --to-name   Bob
./target/release/ontology --data $DATA export out.jsonl  # round-trips through `ingest`

# HTTP API
./target/release/ontology --data $DATA serve --bind 127.0.0.1:5000 &
# dev on a subset: load only some storage domains (see `ns` on concept types)
# ./target/release/ontology --data $DATA serve --ns parties,contrats
curl -s localhost:5000/stats | jq
curl -s -XPOST localhost:5000/retrieve -H 'content-type: application/json' \
  -d '{"query":"retrieval augmented generation","top_k":4,"lexical_weight":0.5,"expansion":{"max_depth":2}}'
```

### PowerShell (Windows)

```powershell
cargo build --release
$env:DATA = ".\data"
.\target\release\ontology.exe --data $env:DATA ingest `
    --ontology examples/sample-ontology.json examples/sample.triples
.\target\release\ontology.exe --data $env:DATA stats
.\target\release\ontology.exe --data $env:DATA retrieve "retrieval augmented generation"
# Answers through the provider configured in $env:DATA\settings.json — see
# "Configuring an LLM provider" below. With none configured: echo model.
.\target\release\ontology.exe --data $env:DATA ask "Who wrote about RAG?"
# One-off model override, not written back to the config:
.\target\release\ontology.exe --data $env:DATA ask --model gpt-4o-mini "Who wrote about RAG?"
.\target\release\ontology.exe --data $env:DATA migrate           # legacy graph.log -> store/ (also automatic on start)
.\target\release\ontology.exe --data $env:DATA path `
    --from-type Person --from-name Alice `
    --to-type   Person --to-name   Bob
.\target\release\ontology.exe --data $env:DATA export out.jsonl  # round-trips through `ingest`

# HTTP API
$server = Start-Process -FilePath .\target\release\ontology.exe -ArgumentList @('--data', $env:DATA, 'serve', '--bind', '127.0.0.1:5000') -PassThru
Invoke-RestMethod http://127.0.0.1:5000/stats
Invoke-RestMethod -Method Post -Uri http://127.0.0.1:5000/retrieve -ContentType 'application/json' -Body '{"query":"retrieval augmented generation","top_k":4,"lexical_weight":0.5,"expansion":{"max_depth":2}}'
Stop-Process -Id $server.Id
```

## HTTP API

| Method | Path                | Body / Returns                                          |
|--------|---------------------|----------------------------------------------------------|
| GET    | `/healthz`          | `"ok"`                                                   |
| GET    | `/stats`            | counts of concepts, relations, types                     |
| GET    | `/metrics`          | Prometheus-format gauges                                 |
| GET    | `/ontology`         | full schema (concept types + relation types)             |
| GET    | `/concepts`         | paginated list; query: `type`, `q`, `limit`, `cursor` (from the previous `next_cursor`; `offset` deprecated) |
| POST   | `/concepts`         | create a `Concept`, returns `{id}`                       |
| GET    | `/concepts/:id`     | fetch one concept                                        |
| PATCH  | `/concepts/:id`     | partial update (`ConceptPatch`)                          |
| DELETE | `/concepts/:id`     | remove concept and cascade incident edges                |
| POST   | `/concepts/delete`  | `{ids}` → remove many concepts under one barrier; returns `{deleted, relations, missing}` |
| POST   | `/relations`        | create a `Relation`, returns `{id}`                      |
| POST   | `/retrieve`         | `RetrievalRequest` → ranked seeds + subgraph             |
| POST   | `/ask`              | `RetrievalRequest` → full `RagAnswer`                    |
| POST   | `/ask/stream`       | same, streamed as Server-Sent Events                     |
| POST   | `/path`             | shortest path between two named concepts                 |
| POST   | `/upload`           | multipart ingest (`kind`, `file`, optional `concept_type`) |
| POST   | `/compact`          | whole-store compaction (rewrite from the live graph)     |
| POST   | `/reset`            | start over: empty store, graph, schema and index (irreversible) |

## Web UI

A Vite + React SPA under [`web/`](./web) consumes the HTTP API and adds
a guided **Ontology Builder** that drafts a schema from a natural-language
description (plus optional seed files) via the RAG backend.

Pages:

* **Builder** (`/builder`) — describe your domain in plain English, optionally
  attach seed files (PDF, text, CSV, JSONL, triples), click **Generate
  Ontology** to call `POST /ontology/generate`, review the proposed concept
  types / relation types in the live graph view, then **Save Ontology** to
  persist via `PUT /ontology`.
* **Graph** (`/graph`) — interactive subgraph viewer (ReactFlow + dagre).
* **Concepts** (`/concepts`) — paginated listing of every node grouped by
  type, with a type filter and name search.
* **Files** (`/files`) — multipart ingest (`/upload`) for ontology JSON,
  JSONL, triples, CSV, XLSX, or text documents.
* **Rules** / **Queries** / **Actions** / **Settings** / **Dashboard**.

### Running the full stack locally

Three services must all be up for the app to work:

| Service | Port | Started by |
| ------- | ---- | ---------- |
| Vite dev server (SPA) | 5173 | `npm run dev:web` |
| Ontology API (Rust) | 5000 | `npm run dev:server` |
| Auth server (Node) | 4000 | `npm run dev:auth` |

**The normal way — one command, all three:**

```bash
cd web
npm install                       # first time only
npm run dev
```

That runs the three scripts above in parallel through `concurrently`, tagged
`server` / `auth` / `web` in the output. Open <http://localhost:5173>.

First-time setup for the auth server (it will not boot without its `.env`):

```bash
cd auth-server
npm install
cp .env.example .env              # then set JWT_SECRET and OAUTH_STATE_SECRET
```

See [auth-server/README.md](./auth-server/README.md) for Google / Microsoft
OAuth setup. The ontology API needs no environment variable at all — the LLM
provider is configured in the UI (**Settings → Configuration**) and stored in
`data/settings.json`.

#### Things that will cost you an hour if you do not know them

* **`npm run dev:web` alone gives you a broken app.** It starts only Vite, so
  every request fails with `NetworkError` / `ECONNREFUSED 127.0.0.1:5000` (or
  `:4000` on login). The page loads; nothing in it works. Use `npm run dev`.
* **`concurrently` runs with `-k`: if one service dies, it kills the other
  two.** So a Rust compile error or a crashed auth server takes the whole
  stack down, and the error you see may be on a service you never touched.
  Scroll up to the first failure rather than debugging the last one.
* **`EADDRINUSE` means an orphan is holding the port.** Usually a backend left
  running from a previous session or started by hand. Find and kill it:

  ```powershell
  Get-NetTCPConnection -State Listen | Where-Object LocalPort -in 4000,5000,5173
  Stop-Process -Id <OwningProcess> -Force
  ```

  ```bash
  lsof -ti:4000,5000,5173 | xargs kill -9
  ```

* **`node --watch` does not recover from a failed boot.** After an
  `EADDRINUSE`, the auth server prints *"Waiting for file changes before
  restarting"* and stays down even once the port frees up. Restart
  `npm run dev`.
* **The Rust binary locks itself while running.** `cargo test` and
  `cargo build` fail with `Accès refusé (os error 5)` / `Permission denied` on
  `target/debug/ontology.exe` while a server is up. Stop it first.

#### Running the services separately

Useful when you want a release build of the backend, or a service in its own
terminal:

```bash
# Ontology API — release build
cargo build --release
./target/release/ontology --data ./data serve --bind 127.0.0.1:5000

# Auth server
npm --prefix auth-server run dev

# Frontend only (backends must already be up)
npm --prefix web run dev:web
```

Vite proxies `/settings`, `/ask`, `/ingest`, `/auth`, … to the two backends
(see [`web/vite.config.ts`](./web/vite.config.ts)), so the SPA needs no URL
configuration in development. `VITE_API_BASE` and `VITE_AUTH_API_BASE` only
matter when serving the built bundle from a different host. Overriding the
proxy targets is possible with `ONTOLOGY_API_URL` and `AUTH_API_URL`.

#### Seeding the demo graph

The dev scripts do **not** seed anything. To start from the bundled finance
example, pass `--seed` yourself on a fresh `data/` directory:

```bash
cargo run -p ontology-cli -- --data ./data serve --bind 127.0.0.1:5000 \
  --seed ./examples/finance
```

That loads the finance demo — Companies, People, Contracts, Invoices,
LineItems and their relations, visible at `GET /stats`. Seeding is skipped
when the persistent store already contains concepts, so restarts are
idempotent; delete the `data/` folder to force a re-seed.

PowerShell equivalent:

```powershell
.\target\release\ontology.exe --data .\data serve `
    --bind 127.0.0.1:5000 --seed .\examples\finance
```

The seeder walks the directory in dependency order: `ontology.json`
first, then `seed.jsonl`, other `*.jsonl` files, `*.triples`,
subdirectories (each file becomes a concept whose type is inferred from
the directory name — `contracts/` → `Contract`), `*.xlsx` spreadsheets
(concept type inferred from the file stem — `invoices.xlsx` → `Invoice`)
and finally `relations.jsonl`.

### Generating an ontology from the UI

1. Open `http://localhost:5173`, sign up or log in.
2. Navigate to **Builder** in the sidebar (`/builder`).
3. Type a description of your domain — e.g. *“Contract management for a
   law firm: parties, clauses, obligations, effective dates, jurisdictions.”*
4. (Optional) Attach one or more seed files. Their text is included as
   context for the LLM.
5. Click **Generate Ontology**. The SPA calls `POST /ontology/generate`;
   the backend asks the configured `LanguageModel` to draft concept types,
   relation types, and example seed concepts, then returns a preview.
6. Inspect the proposed schema in the live graph. Edit names / properties
   inline if needed.
7. Click **Save Ontology** to `PUT /ontology` and persist it (WAL +
   snapshot). The schema is then visible across **Graph**, **Concepts**,
   **Rules**, etc., and ready to receive ingested data via **Files**.

> If **Generate Ontology** reports that no model is configured, open
> **Settings → Configuration**, enter a provider key, load the model list,
> pick a model and click **Appliquer**. It takes effect immediately — no
> restart.

## Observability

`GET /metrics` returns Prometheus-format gauges (`ontology_concepts`,
`ontology_relations`, `ontology_concept_types`, `ontology_relation_types`).
Wire it into your Prometheus scrape config alongside the bearer token.

## Ingest formats

* `*.jsonl` / `*.ndjson` — one tagged `Record` per line.
* `*.triples` / `*.txt`  — `Type:Name predicate Type:Name`, `#` comments.
* `*.csv` — header row with a `name` column; `--csv-type <Type>` required.
* `-` (literal hyphen) — read JSONL from stdin: `cat data.jsonl | ontology ingest -`.

## LLM providers

`ontology-rag` ships these `LanguageModel` implementations:

| Backend             | Constructor                             | Default model     |
| ------------------- | --------------------------------------- | ----------------- |
| Echo (offline fake) | `EchoModel`                             | `echo`            |
| Anthropic Messages  | `AnthropicModel::new(key)`              | `claude-opus-4-7` |
| OpenAI Chat         | `OpenAiModel::new(key)`                 | `gpt-4o-mini`     |
| DeepSeek Chat       | `OpenAiModel::deepseek(key)`            | `deepseek-chat`   |
| Infomaniak AI Tools | `OpenAiModel::infomaniak(key, product)` | *(none — pick one)* |

DeepSeek and Infomaniak are OpenAI-compatible byte-for-byte (streaming SSE
and the `usage` block included), so they share `OpenAiModel` with a different
base URL. Every HTTP client supports streaming, retries 408 / 409 / 429 / 5xx
with full-jitter exponential backoff (3 retries by default), and honors
server-sent `retry-after`.

`v1_api_url(base, path)` builds every endpoint and inserts the `/v1` segment
only when the base URL does not already carry one, so both documented
spellings of a base URL work.

## Configuring an LLM provider

**Credentials live in the settings file, never in the environment.** There is
no `*_API_KEY` variable and no provider flag on `ontology serve`.

* The file is `<data>/settings.json`, or `./.ontology/settings.json` when no
  `--data` directory is given. Override with `--settings <path>`.
* It is created on the first save and holds the raw keys, so it survives
  restarts. On Unix it is created mode 0600; on Windows it inherits the
  directory, so keep the data directory out of shared locations.
* Keys never travel back out: `GET /settings` strips every secret-shaped
  field and returns a masked `*_api_key_hint` instead.
* Applying a provider or model takes effect on the next request — `/ask`,
  `/ask/stream` and `/ingest/analyze` all resolve it per call.

Configure it in the UI (**Settings → Configuration**), or over the API:

```bash
curl -s -XPATCH localhost:5000/settings -H 'content-type: application/json' -d '{
  "llm": {
    "active_provider": "infomaniak",
    "infomaniak_api_key": "…",
    "infomaniak_product_id": "101112",
    "infomaniak_model": "mixtral"
  }
}'
```

Supporting endpoints, all of which accept unsaved credentials in the body so
you can validate before saving:

| Endpoint | Purpose |
| -------- | ------- |
| `POST /settings/llm/test` | Probe the provider; echoes the resolved endpoint. |
| `GET  /settings/llm/models?provider=…` | Model catalogue for the stored config. |
| `POST /settings/llm/models` | Same, with credentials supplied in the body. |
| `POST /settings/llm/infomaniak/products` | Resolve the AI Tools `product_id`. |

### Infomaniak AI Tools

Swiss-hosted open-source models behind an OpenAI-compatible API.

1. Create an API token in the Infomaniak manager with the **`ai-tools`**
   scope.
2. Paste it into **Settings → Configuration** and click **Détecter** to read
   the `product_id` from `GET https://api.infomaniak.com/1/ai`.
3. Click **Charger les modèles** (`GET {base}/models`), pick one, then
   **Appliquer**.

Requests then go to
`https://api.infomaniak.com/2/ai/{product_id}/openai/v1/chat/completions`.
The base URL is derived from the product id; the "base URL" field is an
advanced override for proxies and staging hosts, and is normally left empty.

The CLI uses whatever the settings file selects; `--model` overrides the
model for one run:

```bash
ontology --data $DATA ask                    "Who wrote about RAG?"
ontology --data $DATA ask --model gpt-4o     "Who wrote about RAG?"
ontology --settings ./staging/settings.json ask "Who wrote about RAG?"
```

## Prompt caching

The Anthropic client routes the ontology (stable per knowledge base) into a
separately-cached `system` block via `cache_control: {"type": "ephemeral"}`,
so repeated queries against the same KB pay roughly 10% of the input price
for the cached prefix on subsequent requests within the TTL (5 min default).
Verify hits via `RagAnswer.usage.cache_read_input_tokens`. The minimum
cacheable prefix on Claude Opus 4.7 is 4096 tokens; below that the
breakpoint is silently ignored — no error.

`temperature` is automatically omitted on Claude Opus 4.7 (the API rejects
it with a 400). Older models still receive it.

OpenAI and DeepSeek both perform **automatic** server-side prefix caching
on identical leading content — the `OpenAiModel` folds `cached_context`
into the leading `system` message so byte-stable prefixes pay the cached
rate without any client opt-in. Cache hits are exposed via
`usage.cache_read_input_tokens` (mapped from OpenAI's
`prompt_tokens_details.cached_tokens` and DeepSeek's
`prompt_cache_hit_tokens`).

All HTTP clients retry 408 / 409 / 429 / 5xx with full-jitter
exponential backoff (default 3 retries; configurable via
`with_max_retries`). When the server sends a `retry-after`
header it's honored verbatim.

## Authentication

`build_router_with_auth(state, Some(token))` (and `ontology serve --auth-env
NAME`) wrap every route except `/healthz` with a bearer-token middleware.
The CLI flag reads from a named environment variable rather than taking
the literal value, so the token never appears in process listings or
shell history. Comparison is constant-time.

## Memory budget (`--memory-mode`)

The graph lives in memory (P0). Before loading anything the binary estimates
each domain's cost from the store's MANIFEST and compares the sum with a
budget read from the execution environment (cgroup v2/v1 limit, else free
memory, else Windows available memory), times a fraction. Nothing is read
from `MemTotal`, so a container limit is respected.

```bash
ontology --data $DATA serve                                  # adaptive: under a cgroup/explicit limit, load what fits (smallest first); else load all and warn
ontology --data $DATA --memory-mode strict serve             # refuse to start if the estimate exceeds the budget
ontology --data $DATA --heap-fraction 0.5 serve              # share of available memory (default 0.6)
ontology --data $DATA --memory-budget-mb 2048 serve          # explicit budget, detection and fraction ignored
ontology --data $DATA serve --ns parties,contrats            # an explicit list always wins over the plan
ontology --data $DATA --tier p1 serve                        # P1: concept payloads stay on disk, read back on demand (auto = the plan decides)
ONTOLOGY_MEMORY_MODE=strict ONTOLOGY_MEMORY_BUDGET_MB=2048 ontology --data $DATA serve   # same, for Docker
```

The plan taken is logged at startup and visible in `GET /stats` (`memory`) and
`GET /metrics` (`ontology_memory_*`, `ontology_domains_*`). A partial load is
a `warn` naming the domains left out. Details in
[docs/STORAGE.md §8.1](docs/STORAGE.md#81-le-budget-pas-la-ram-totale).

**Sizing** (measured coefficients, 1.3 KB payloads, 5 relations per concept;
extrapolated, see STORAGE.md §8.1 for the assumptions):

| Node | Today (P0) | With P1 (payloads on disk, next storage step) |
|---|---|---|
| 16 GB, default fraction 0.6 | ~1.8 M concepts / 9 M relations | ~2.4 M / 12 M (P1 delivered 2026-09-23: private heap ÷1.57 measured at 500 k) |
| 64 GB, fraction 0.6 | ~7 M / 35 M | ~9.5 M / 47 M |
| 64 GB, `--heap-fraction 0.8` (dedicated node) | ~9.6 M / 48 M | **~13 M / 63 M** |

The design target of 10 M concepts / 50 M relations per store is therefore
served by a 64 GB node; on 16 GB the guarantee is 2 M / 10 M. Use `--memory-mode
strict` on a sized deployment so an oversized store fails at startup with both
figures instead of being killed later.

## Benchmarks (`ontology bench`)

The measurements of `docs/STORAGE-PLAN.md` phase 4 are reproducible with a
dedicated data directory (never point it at real data):

```bash
DATA=/tmp/bench-1m
ontology --data $DATA bench gen --concepts 1000000 --relations 5000000 --ns 5 --payload 1300
ontology --data $DATA bench hydrate           # open + load_into, then decode-only scan, RSS
ontology --data $DATA bench query             # page 200, ?q= trigram, expand depth 2
ontology --data $DATA bench append --n 2000 --batch 100
ontology --data $DATA bench compact --codec postcard
ontology --data $DATA bench hydrate --json    # same store, binary codec
cargo bench -p ontology-storage --bench codec # per-record encode/decode, JSON vs postcard
```

`--json` prints one object per run. `ontology --data D compact --codec postcard`
switches an existing store to the binary codec (whole-store compaction; every
record header carries its codec, so JSON and postcard segments coexist until
then).

## Test coverage

[![Rust line coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fmasspe%2Fai-ontology.com%2Fcoverage-badges%2Frust.json)](#test-coverage)
[![Web line coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fmasspe%2Fai-ontology.com%2Fcoverage-badges%2Fweb.json)](#test-coverage)

Line coverage, measured by the CI on Linux after every push to `main` with
the full suite (unit, integration and end-to-end tests that spawn the
`ontology` binary). `cargo llvm-cov` for Rust, V8 through vitest for the
web. Source files only: test code is not in the denominator.

Snapshot of 2026-09-23 (the badges above are the live figures; `server` and `cli` measured on Windows before the CI run):

| Crate | Line coverage | Lines |
|---|---|---|
| `ontology-index` | 99.7 % | 387 / 388 |
| `ontology-graph` | 97.0 % | 3 171 / 3 269 |
| `ontology-rag` | 96.7 % | 3 217 / 3 326 |
| `ontology-io` | 93.4 % | 1 924 / 2 060 |
| `ontology-storage` | 91.6 % | 3 871 / 4 228 |
| `ontology-server` | 96.6 % | — |
| `ontology-cli` | 95.1 % | — |
| **Rust total** | **≥ 93 %** (CI publishes the exact figure) | — |
| **Web (`web/src`, JS and TS)** | **99.7 %** | 3 210 / 3 221 |

What the gaps are, so the numbers are read for what they mean:

- **`server`** — `ingest_review.rs` (LLM-assisted ingest review) went from
  53 % to 95 % with a scripted language model (truncated or malformed
  answers, `null` fields, provider errors, every apply branch). What is
  left needs binaries (`tesseract`, `ocrmypdf`) or the real Infomaniak API.
- **`cli`** — `main.rs` went from 64 % to 93 %: `ask` / `retrieve` on the
  offline echo model, every ingest format, `serve` started on a free port
  and stopped by SIGTERM / Ctrl+Break (it now shuts down cleanly). What is
  left needs a real provider (token usage block) or host-dependent memory
  readings.
- **`storage`** — the codec error branches (`codec.rs`, 79 %) and the
  platform-specific budget detection: cgroup reading only runs on Linux and
  `GlobalMemoryStatusEx` only on Windows, so each platform reports the
  other half as uncovered.
- **Web** — 687 vitest tests: the pure logic (`extractText`, `ingestApi`,
  `logBuffer`, `mergeProposals`, `providerConfig`) and, since 2026-09-22,
  every React page and component rendered with Testing Library in jsdom
  (`*.render.test.tsx` next to each file; `api.ts` is mocked per test,
  `fetch` is stubbed to reject so nothing can reach the network), the
  JavaScript auth modules included (`Login`, `Signup`, `OAuthCallback`,
  `ProtectedRoute`, `msBE`, at 97 to 100 %). The eleven
  uncovered lines are unreachable through the UI (a button disabled while
  the guard would fire, a dead branch behind a constant state). Branches
  are at 93 %; what is left is SSR guards, `instanceof Error` fallbacks and
  transient busy labels. `vitest.config.ts` enforces 90 % on lines,
  statements, functions and branches: `npm run test:coverage`, which the CI
  `web` job runs, fails below that bar.

Reproduce locally (Rust needs `rustup component add llvm-tools-preview`
and `cargo install cargo-llvm-cov` once):

```bash
cargo llvm-cov --workspace --summary-only          # per-file table, TOTAL at the bottom
cargo llvm-cov --workspace --html                  # browsable report in target/llvm-cov/html/
mkdir -p target/llvm-cov && cargo llvm-cov --workspace --json --output-path target/llvm-cov/cov.json
cd web && npm run test:coverage                    # vitest + V8, table in the terminal
cd .. && python3 scripts/coverage_badges.py --rust target/llvm-cov/cov.json \
    --web web/coverage/coverage-summary.json --out badges   # same badges/table as the CI
```

```powershell
cargo llvm-cov --workspace --summary-only
New-Item -ItemType Directory -Force target/llvm-cov | Out-Null
cargo llvm-cov --workspace --json --output-path target/llvm-cov/cov.json
Set-Location web; npm run test:coverage; Set-Location ..
python scripts/coverage_badges.py --rust target/llvm-cov/cov.json --web web/coverage/coverage-summary.json --out badges
```

How the badges work: the `coverage` job of [ci.yml](.github/workflows/ci.yml)
runs both measurements, `scripts/coverage_badges.py` writes one
[shields.io endpoint](https://shields.io/badges/endpoint) document per figure
(`rust.json`, `web.json`, `crate-<name>.json`) and the job force-pushes them
to the orphan branch `coverage-badges`. The README embeds
`img.shields.io/endpoint?url=<raw file>`, so no third-party coverage service
and no secret is involved. The figures on Windows differ by a few tenths
(platform-specific code), which is why the published measurement is the
Linux one.

## Testing

```bash
cargo test --workspace          # 576 Rust tests (2026-09-22): unit, integration, end-to-end (binary spawned)
cd web && npm run test          # 687 vitest tests: logic + every page rendered (Testing Library, jsdom)
cd web && npm run test:coverage # same, with the 90 % coverage thresholds enforced
```

Coverage of that suite is in [Test coverage](#test-coverage) and on the
badges at the top of this file.

## License

Copyright © 2026 **Winven AI Sarl**, Route de Crassier 7, 1262 Eysins,
VD, Switzerland.

This software is **dual-licensed**:

1. **AGPL-3.0-or-later** — open-source track. See [`LICENSE`](./LICENSE)
   for the full text. The Affero clause means that if you operate this
   software (or a modified version) as a network/SaaS service, you must
   make the corresponding source code available to every user of that
   service.
2. **Winven Commercial License** — proprietary track. See
   [`LICENSE-COMMERCIAL.md`](./LICENSE-COMMERCIAL.md). Removes the
   AGPL's copyleft and SaaS-source-disclosure obligations; bundles
   support and indemnification. Negotiated case by case with Winven AI
   Sarl at the address above.

You may pick whichever track fits your use, but you must comply with
the chosen one in full. Every source file carries an SPDX dual
expression in its header:

```
SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
```

Use without an AGPL-compliant deployment **and** without a signed
commercial agreement is a license violation.
