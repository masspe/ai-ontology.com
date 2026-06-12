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
* **Durable + crash-safe storage.** An append-only WAL plus bincode snapshots
  (with `compact` to truncate) gives fast restarts and a clean recovery story
  behind a pluggable `Store` trait.
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
.\target\release\ontology.exe --data $env:DATA snapshotin
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
