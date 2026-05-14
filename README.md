# ai-ontology.com

A Rust workspace implementing an ontology-structured graph database with a
hybrid retrieval layer and a RAG pipeline that grounds language-model
answers in retrieved subgraphs.

## Crates

| Crate              | Role |
| ------------------ | ---- |
| `ontology-graph`   | Concepts, typed relations, schema validation, traversals. |
| `ontology-storage` | Append-only WAL + bincode snapshots; pluggable `Store` trait. |
| `ontology-index`   | Lexical (TF-IDF) + vector (cosine) + graph-expansion retrieval. |
| `ontology-io`      | `Source` / `Sink` traits with JSONL and triples adapters. |
| `ontology-rag`     | Prompt builder + `LanguageModel` trait (echo, Anthropic, OpenAI, DeepSeek; with prompt caching). |
| `ontology-server`  | axum HTTP server exposing `/concepts`, `/relations`, `/retrieve`, `/ask`, `/ontology`. |
| `ontology-cli`     | `ontology` binary tying it all together. |
| `web/`             | Vite + React UI (Ask · Browse · Upload tabs).                          |

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

## Quickstart

### POSIX shell

```bash
cargo build --release
DATA=./data
./target/release/ontology --data $DATA ingest \
    --ontology examples/sample-ontology.json examples/sample.triples
./target/release/ontology --data $DATA stats
./target/release/ontology --data $DATA retrieve "retrieval augmented generation"
./target/release/ontology --data $DATA ask "Who wrote about RAG?"            # echo
ANTHROPIC_API_KEY=... ./target/release/ontology --data $DATA \
    ask --anthropic "Who wrote about RAG?"
OPENAI_API_KEY=... ./target/release/ontology --data $DATA \
    ask --openai --model gpt-4o-mini "Who wrote about RAG?"
DEEPSEEK_API_KEY=... ./target/release/ontology --data $DATA \
    ask --deepseek "Who wrote about RAG?"
./target/release/ontology --data $DATA snapshot
./target/release/ontology --data $DATA compact          # snapshot + truncate WAL
./target/release/ontology --data $DATA path \
    --from-type Person --from-name Alice \
    --to-type   Person --to-name   Bob
./target/release/ontology --data $DATA export out.jsonl  # round-trips through `ingest`

# HTTP API
./target/release/ontology --data $DATA serve --bind 127.0.0.1:5000 &
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
.\target\release\ontology.exe --data $env:DATA ask "Who wrote about RAG?"            # echo
$env:ANTHROPIC_API_KEY = "..."
.\target\release\ontology.exe --data $env:DATA ask --anthropic "Who wrote about RAG?"
$env:OPENAI_API_KEY = "..."
.\target\release\ontology.exe --data $env:DATA ask --openai --model gpt-4o-mini "Who wrote about RAG?"
$env:DEEPSEEK_API_KEY = "..."
.\target\release\ontology.exe --data $env:DATA ask --deepseek "Who wrote about RAG?"
.\target\release\ontology.exe --data $env:DATA snapshot
.\target\release\ontology.exe --data $env:DATA compact          # snapshot + truncate WAL
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
| GET    | `/concepts`         | paginated list; query: `type`, `q`, `limit`, `offset`    |
| POST   | `/concepts`         | create a `Concept`, returns `{id}`                       |
| GET    | `/concepts/:id`     | fetch one concept                                        |
| PATCH  | `/concepts/:id`     | partial update (`ConceptPatch`)                          |
| DELETE | `/concepts/:id`     | remove concept and cascade incident edges                |
| POST   | `/relations`        | create a `Relation`, returns `{id}`                      |
| POST   | `/retrieve`         | `RetrievalRequest` → ranked seeds + subgraph             |
| POST   | `/ask`              | `RetrievalRequest` → full `RagAnswer`                    |
| POST   | `/ask/stream`       | same, streamed as Server-Sent Events                     |
| POST   | `/path`             | shortest path between two named concepts                 |
| POST   | `/upload`           | multipart ingest (`kind`, `file`, optional `concept_type`) |
| POST   | `/compact`          | snapshot + truncate WAL                                  |

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

Three services cooperate. Start them in three terminals (or use the
combined script in step 3 to run the ontology API + web together).

#### 1. Auth server (`http://localhost:4000`)

Issues JWTs consumed by the SPA. Required for login / signup / OAuth.

```powershell
cd auth-server
npm install
Copy-Item .env.example .env       # then edit: JWT_SECRET, OAUTH_STATE_SECRET, provider keys
npm run dev
```

See [auth-server/README.md](./auth-server/README.md) for OAuth provider
setup (Google, Microsoft).

#### 2. Ontology API (`http://127.0.0.1:5000`)

The Rust HTTP server. To enable **Generate Ontology** from the UI, export
an LLM key before starting it (Anthropic, OpenAI or DeepSeek):

```powershell
cargo build --release
$env:ANTHROPIC_API_KEY = "sk-ant-..."     # or OPENAI_API_KEY / DEEPSEEK_API_KEY
.\target\release\ontology.exe --data .\data serve --bind 127.0.0.1:5000 --anthropic
```

POSIX equivalent:

```bash
cargo build --release
ANTHROPIC_API_KEY=sk-ant-... \
  ./target/release/ontology --data ./data serve --bind 127.0.0.1:5000 --anthropic
```

#### 3. Frontend (`http://localhost:5173`)

```powershell
cd web
npm install
$env:VITE_API_BASE = "http://127.0.0.1:5000"
$env:VITE_AUTH_BASE = "http://localhost:4000"
npm run dev:web
```

Shortcut: `npm run dev` inside `web/` launches the ontology API
(`cargo run -p ontology-cli -- ... serve`) **and** the Vite dev server in
parallel via `concurrently`. Use this when you do not need a release build
of the backend. The auth server still needs to be started separately.

The `dev:server` script also passes `--seed ../examples/finance`, so on a
fresh `data/` directory the API starts with the bundled finance demo
already loaded (Companies, People, Contracts, Invoices, LineItems and
their relations — visible at `GET /stats`). Seeding is skipped when the
persistent store already contains concepts, so restarts are idempotent;
delete the `data/` folder to force a re-seed.

To seed from another example, pass `--seed` to `ontology serve` directly:

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

> If **Generate Ontology** returns `503 no language model configured`,
> restart the ontology API with one of `--anthropic` / `--openai` /
> `--deepseek` and the matching `*_API_KEY` env var.

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

Four `LanguageModel` implementations ship in `ontology-rag`:

| Backend            | Constructor                                | Env var              | Default model    |
| ------------------ | ------------------------------------------ | -------------------- | ---------------- |
| Echo (offline fake)| `EchoModel`                                | —                    | `echo`           |
| Anthropic Messages | `AnthropicModel::new(key)`                 | `ANTHROPIC_API_KEY`  | `claude-opus-4-7`|
| OpenAI Chat        | `OpenAiModel::new(key)`                    | `OPENAI_API_KEY`     | `gpt-4o-mini`    |
| DeepSeek Chat      | `OpenAiModel::deepseek(key)`               | `DEEPSEEK_API_KEY`   | `deepseek-chat`  |

DeepSeek's API is OpenAI-compatible byte-for-byte (including streaming
SSE and the `usage` block) so it shares `OpenAiModel` with a different
base URL. All three HTTP clients support streaming, retry 408 / 409 /
429 / 5xx with full-jitter exponential backoff (default 3 retries),
and honor server-sent `retry-after`.

Select a backend on the CLI with mutually-exclusive flags on `ask` and
`serve`:

```bash
ontology --data $DATA ask --anthropic               "Who wrote about RAG?"
ontology --data $DATA ask --openai --model gpt-4o   "Who wrote about RAG?"
ontology --data $DATA ask --deepseek                "Who wrote about RAG?"
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

## Testing

```bash
cargo test --workspace
```

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
