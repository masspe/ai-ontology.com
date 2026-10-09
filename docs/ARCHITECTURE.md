# Architecture

## Overview

A Rust workspace implementing an ontology-structured graph database with a
hybrid retrieval layer and a RAG pipeline that grounds language-model answers
in retrieved subgraphs. One `ontology` binary serves the HTTP API, the web UI
and the user login; in production there is one container, and one store, per
client.

The line-coverage badges below are live: after every push to `main` the CI
measures line coverage of the whole Rust suite and of the web UI and publishes
the figures. The bar is 90 % minimum: per crate for Rust, enforced by the `coverage`
job on pushes to `main`, and on the web suite by the `vitest.config.ts`
thresholds, checked on every push and pull request.

```
┌──────────────┐    ┌──────────────┐    ┌──────────────────┐
│  Source       │───▶│  ingest_*    │───▶│ OntologyGraph    │
│  (jsonl, …)   │    │  (validate)  │    │ + segmented store│
└──────────────┘    └──────────────┘    └─────┬────────────┘
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
                              │   DeepSeek / Infomaniak / Echo)│
                              └────────────────────────────────┘
```

## Crates

| Crate              | Lines covered | Role |
| ------------------ | ------------- | ---- |
| `ontology-graph`   | ![graph line coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fmasspe%2Fai-ontology.com%2Fcoverage-badges%2Fcrate-graph.json&label=lines&style=flat-square) | Concepts, typed relations, schema validation, traversals. |
| `ontology-storage` | ![storage line coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fmasspe%2Fai-ontology.com%2Fcoverage-badges%2Fcrate-storage.json&label=lines&style=flat-square) | Binary segmented store (`<data>/store/`): framed records with CRC, positional index, per-batch fsync, torn-tail recovery, memory-mapped sealed segments; automatic migration from the legacy `graph.log`. Pluggable `Store` trait. Format in [docs/STORAGE.md](STORAGE.md). |
| `ontology-index`   | ![index line coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fmasspe%2Fai-ontology.com%2Fcoverage-badges%2Fcrate-index.json&label=lines&style=flat-square) | Lexical (TF-IDF) + vector (cosine) + graph-expansion retrieval. |
| `ontology-io`      | ![io line coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fmasspe%2Fai-ontology.com%2Fcoverage-badges%2Fcrate-io.json&label=lines&style=flat-square) | `Source` / `Sink` traits with JSONL and triples adapters; CSV, XLSX, DOCX and plain-text readers; chunking. |
| `ontology-rag`     | ![rag line coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fmasspe%2Fai-ontology.com%2Fcoverage-badges%2Fcrate-rag.json&label=lines&style=flat-square) | Prompt builder + `LanguageModel` trait (echo, Anthropic, OpenAI, DeepSeek, Infomaniak; with prompt caching). |
| `ontology-server`  | ![server line coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fmasspe%2Fai-ontology.com%2Fcoverage-badges%2Fcrate-server.json&label=lines&style=flat-square) | axum HTTP server exposing the whole REST API: `/concepts`, `/relations`, `/retrieve`, `/ask`, `/ontology`, `/upload` and `/ingest/*`, `/rules`, `/actions`, `/queries`, `/files`, `/settings`, `/auth/*`, `/audit`, `/backup`, `/compact`, `/metrics`, `/healthz`, `/stats`. |
| `ontology-cli`     | ![cli line coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fmasspe%2Fai-ontology.com%2Fcoverage-badges%2Fcrate-cli.json&label=lines&style=flat-square) | `ontology` binary tying it all together. |
| `web/`             | ![web line coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fmasspe%2Fai-ontology.com%2Fcoverage-badges%2Fweb.json&label=lines&style=flat-square) | Vite + React UI (home, files, import review, data model, sheets, graph, questions, rules, actions, settings). |

## Request path

1. **Ingest.** A `Source` (JSONL, triples; see `ontology-io`) yields concepts
   and relations. The `ingest_*` functions validate each of them against the
   ontology schema (concept types, typed relations) before it reaches the
   graph.
2. **Graph and store.** Accepted records go into `OntologyGraph` (in memory)
   and, in the same step, into the `Store` (the segmented store under
   `<data>/store/`, see [Storage](#storage)). The store is the source of truth;
   the graph and the indexes are rebuilt from it at startup.
3. **Index.** `HybridIndex` indexes each concept twice: lexical (inverted
   TF-IDF / BM25-style lists) and vector (L2-normalized embeddings).
4. **Retrieve.** A query is scored by both indexes, fused (see
   [Retrieval and ranking](#retrieval-and-ranking)), then the top hits are
   expanded into a `Subgraph` by a bounded, typed traversal. The result is a
   `ScoredConcept[]` plus the `Subgraph`. Exposed as `POST /retrieve`.
5. **Prompt.** `PromptBuilder` renders the ontology, the retrieved concepts
   and the subgraph into a prompt. The ontology is the stable part (per
   knowledge base) and is what prompt caching targets (see
   [Prompt caching](#prompt-caching)).
6. **Model.** The `LanguageModel` trait sends the prompt to Anthropic, OpenAI,
   DeepSeek, Infomaniak, or the `Echo` model (no network, for tests and demos). The
   answer and its usage come back as a `RagAnswer`. Exposed as `POST /ask`.

## Storage

The `ontology-storage` crate persists the graph as a binary segmented store
under `<data>/store/`. The full format (file layout, record header, index
entries, MANIFEST, hydration, recovery, measurements) is specified in
[STORAGE.md](STORAGE.md); that document is written in French. In short:

* **Segments.** Records are appended to segment files as *framed records*,
  each carrying its length, codec and a CRC. A segment has a data file
  (`.data`) and a positional index (`.idx`). The data file is the source of
  truth; the index is derived from it.
* **Durability.** One `fsync` per write batch, not per record. After a crash
  the store recovers by truncating a torn tail (a partially written last
  record) when it is reopened.
* **Sealed segments.** When a segment is full it is sealed: it becomes
  immutable and is memory-mapped for reads. The active segment is never
  memory-mapped.
* **Partitioning by domain.** Records are partitioned by their `ns` domain,
  each domain having its own stream of segments. A domain is for locality,
  retention and access rights inside one ontology.
* **Memory tiers.** In P0 the whole graph is in memory. In P1, concept
  payloads stay on disk and are read back on demand (see
  [Memory budget and tiers](#memory-budget-and-tiers)).
* **Migration.** A data directory written by the legacy single-file
  `graph.log` is migrated automatically to the segmented store on open.
* **Backup.** Because sealed segments are immutable, a backup is a copy of
  the store's files: the `MANIFEST.json` and the segments, taken under the
  write lock. See [DEPLOY.md](DEPLOY.md).
* **Codecs.** Each record header carries its codec, so JSON and postcard
  records coexist in one store until a whole-store compaction switches it
  (`ontology --data D compact --codec postcard`).

The `Store` trait is pluggable; the segment store is the default.

## Retrieval and ranking

Implemented in `crates/index`.

* **Inverted-index lexical search** (`lexical.rs`): a TF-IDF / BM25-style
  inverted index. Per-term IDF is `ln((n − df + 0.5) / (df + 0.5) + 1.0)`; the
  per-document weight is length-normalized as `sqrt(tf / doc_len) · idf`. Only
  documents that actually contain a query term are scored (no full scan).
* **Flat cosine vector search** (`vector.rs`, `embed.rs`): vectors are
  **L2-normalized at insertion**, so similarity collapses to a plain dot
  product `Σ q[i]·doc[i]`, with no per-query normalization in the hot loop.
* **Hybrid fusion with min-max normalization** (`hybrid.rs::rank`): lexical and
  vector scores are each normalized to `[0,1]` against their own max, then
  blended:

  ```text
  score = lexical_weight · (lex / lex_max) + (1 − lexical_weight) · (vec / vec_max)
  ```

  `lexical_weight` is a per-request knob (default `0.5`).
* **Relevance-ratio noise floor**: candidates scoring below `0.4 ×` the top
  hit are dropped, so weak tail matches never reach the LLM prompt.
* **Adaptive candidate pools**: when a request adds type filters the candidate
  pool is widened (`4×` → `16×` `top_k`) so enough survivors remain after
  post-filtering, without paying that cost on unfiltered queries.
* **Trigram substring index** (`graph.rs`): concept-name substring search uses
  a `[char; 3]` trigram inverted index; matching intersects the candidate sets
  for the query's trigrams instead of scanning every name (it falls back to a
  linear scan only for queries shorter than 3 chars).

## Graph algorithms

The graph layer keeps traversal **bounded, typed, and read-lock-free** so that
retrieval stays predictable even on dense ontologies. Everything below is
implemented in `ontology-graph` and `ontology-index`.

### Traversal and pathfinding (`crates/graph/src/traversal.rs`)

| Algorithm | Function | What it does |
| --------- | -------- | ------------ |
| **Bounded BFS shortest path** | `shortest_path` | Undirected breadth-first search between two concepts with unit edge weights, capped by `max_depth`. Tracks `parent` + incident `RelationId` to reconstruct the full edge-labelled path. Exposed as `POST /path`. |
| **N-hop subgraph expansion** | `expand` | Level-by-level BFS from a set of seeds, hard-bounded by both `max_depth` **and** `max_nodes`. Optional filters on relation type, concept type, and direction (`outgoing` / `incoming` / `both`). Returns the `Subgraph` fed to the prompt builder. |
| **Typed relation closure** | `closure` | BFS along a single relation type (in *and* out edges) to compute transitive chains such as `partOf` / `locatedIn`, used by the RAG layer to pull in implied context. |

All three are **depth- and node-capped on entry**, so a pathological "expand the
whole graph" request degrades gracefully instead of blowing up: the cost of a
retrieval is a function of `top_k` and `TraversalSpec`, not of total graph size.

**Typed adjacency.** Edges are bucketed by relation type, so a relation-filtered
expansion walks only the relevant bucket instead of every incident edge.

**Out of scope by design.** There is no weighted shortest path (Dijkstra/A\*),
centrality/PageRank, or approximate-nearest-neighbor (ANN) vector index. Edges
are unit-weight and the vector index is exact/flat. These are natural extension
points rather than current behavior.

## Concurrency

* The graph stores nodes/edges in `DashMap`s (sharded, lock-free reads);
  concepts, relations, and typed in/out adjacency lists all live in them, and
  reads never take a global lock.
* Schema mutations take a short `parking_lot::RwLock` write.
* `IdAllocator` is lock-free: it hands out ids with a single
  `AtomicU64::fetch_add`, and restore reconciles the watermark with
  `compare_exchange_weak`.
* `HybridIndex` uses `RwLock`-protected inverted lists; reads are concurrent.
* **Generation-counter cache invalidation**: `AtomicU64` generation counters
  (`concepts_gen` / `relations_gen`) bump on any mutation and invalidate the
  bounded (capacity 256) pagination caches in one cheap compare, avoiding
  per-entry eviction bookkeeping.
* The pipeline is `Send + Sync + Clone`, so a single `RagPipeline` value can
  serve many concurrent answers.

## Memory budget and tiers

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
[STORAGE.md §8.1](STORAGE.md#81-le-budget-pas-la-ram-totale).

**Sizing** (measured coefficients, 1.3 KB payloads, 5 relations per concept;
extrapolated, see STORAGE.md §8.1 for the assumptions):

| Node | Today (P0) | With P1 (payloads on disk, `--tier p1`) |
|---|---|---|
| 16 GB, default fraction 0.6 | ~1.8 M concepts / 9 M relations (2 M / 10 M measured on a 16 GB runner: 9.9 GB private heap, over budget, and the retrieval index no longer fits) | **2 M / 10 M measured: 6.4 GB** private heap after hydration, the retrieval index then fits (÷1.55; ÷1.57 at 500 k) |
| 64 GB, fraction 0.6 | ~7 M / 35 M | ~9.5 M / 47 M |
| 64 GB, `--heap-fraction 0.8` (dedicated node) | ~9.6 M / 48 M | **~13 M / 63 M** |

The design target of 10 M concepts / 50 M relations per store needs a
dedicated 64 GB node (`--heap-fraction 0.8`) with P1; that figure is
extrapolated and has not yet been measured on a 64 GB machine. On 16 GB the
guarantee is 2 M / 10 M **in P1** (`--tier p1`, measured on a GitHub runner,
STORAGE.md §7.8). Use `--memory-mode strict` on a sized
deployment so an oversized store fails at startup with both figures instead of
being killed later.

## Prompt caching

The Anthropic client routes the ontology (stable per knowledge base) into a
separately-cached `system` block via `cache_control: {"type": "ephemeral"}`,
so repeated queries against the same KB pay roughly 10% of the input price
for the cached prefix on subsequent requests within the TTL (5 min default).
Verify hits via `RagAnswer.usage.cache_read_input_tokens`. The minimum
cacheable prefix on Claude Opus 4.7 is 4096 tokens; below that the
breakpoint is silently ignored, with no error.

`temperature` is automatically omitted on Claude Opus 4.7 (the API rejects
it with a 400). Older models still receive it.

OpenAI and DeepSeek both perform **automatic** server-side prefix caching
on identical leading content: the `OpenAiModel` folds `cached_context`
into the leading `system` message so byte-stable prefixes pay the cached
rate without any client opt-in. Cache hits are exposed via
`usage.cache_read_input_tokens` (mapped from OpenAI's
`prompt_tokens_details.cached_tokens` and DeepSeek's
`prompt_cache_hit_tokens`).

All HTTP clients retry 408 / 409 / 429 / 5xx with full-jitter
exponential backoff (default 3 retries; configurable via
`with_max_retries`). When the server sends a `retry-after`
header it's honored verbatim.

## Observability

`GET /metrics` returns Prometheus-format gauges (exposition 0.0.4). Wire it
into your Prometheus scrape config alongside the bearer token.

| Family | What it says |
|---|---|
| `ontology_concepts`, `ontology_relations`, `ontology_rules`, `ontology_actions`, `ontology_*_types` | Sizes of the graph and of the ontology. |
| `ontology_process_rss_bytes` | Resident set size of the process. |
| `ontology_memory_*`, `ontology_domains_loaded` / `_skipped` / `_p1`, `ontology_resident_payloads` | The memory plan of the startup (budget, estimate, what was loaded, P1) — see "Memory budget". |
| `ontology_store_next_seq`, `ontology_store_syncs` | Next global sequence number; `fdatasync` calls since the store was opened. |
| `ontology_store_last_compaction_seconds`, `_timestamp_seconds`, `_partitions_removed` | The last compaction since open (absent until one ran). |
| `ontology_stream_sealed_segments{ns}`, `ontology_stream_data_bytes{ns}`, `ontology_stream_records{ns}`, `ontology_stream_last_seq{ns}`, `ontology_stream_syncs{ns}` | One series per storage domain (`ns="meta"` is the schema stream): sealed segments, bytes of `.data`, records, last sequence number, syncs since open. |
| `ontology_domain_tier{ns,tier}` | 1 on the current memory tier of each domain (`p0` everything in memory, `p1` payloads on disk). |

The per-stream families exist only on the segment store (the default).

## Deployment model

A storage domain (`ns`) is for locality, retention and access rights inside
one tenant's ontology, never for spreading tenants over one store. Multi-tenancy
is **one store per tenant, served by one process per tenant**
(`ontology serve --data <tenant>`; see
[STORAGE.md §10.8](STORAGE.md#108-un-store-par-tenant--décision-t5--un-processus-par-tenant)):
the write lock, the memory budget and failure isolation are naturally per
process, and a multi-store process would multiply open files and the
incompressible memory floor by the number of tenants for nothing. Routing a
tenant to its process is the reverse proxy's job. In production this is one
container per client, the container's memory limit being the budget the
memory plan reads (see [Memory budget and tiers](#memory-budget-and-tiers)).

The image (`Dockerfile`) ships the binary and the built UI; `compose.yaml`
shows two tenants, each with its volume, memory limit, health probe
(`ontology healthcheck`) and clean stop. `scripts/e2e_image.sh` is what the
CI runs against every build: seed, sign-up, login, API and UI on one port,
`docker stop`, restart with data and account intact. Installation guide:
[DEPLOY.md](DEPLOY.md).

## Benchmarks

The capacity measurements of [STORAGE.md](STORAGE.md) §7.8 are reproducible
with `ontology bench`, on a dedicated data directory (never point it at real
data):

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

The same sequence runs on a GitHub runner (16 GB, no laptop needed) through
the manual **bench** workflow: Actions → bench → *Run workflow*, with the
store size as inputs (defaults: 2 M concepts / 10 M relations, the 16 GB
guarantee). It publishes the JSON results as an artifact and a P0/P1 table
in the job summary.

`--json` prints one object per run. `ontology --data D compact --codec postcard`
switches an existing store to the binary codec (whole-store compaction; every
record header carries its codec, so JSON and postcard segments coexist until
then).

## Tests and coverage

```bash
cargo test --workspace            # unit, integration and end-to-end tests (the binary is spawned)
cd web && npm run test:coverage   # vitest with the 90 % thresholds of vitest.config.ts
cargo llvm-cov --workspace --summary-only
```

The CI runs the Rust suite on Ubuntu and Windows, the web suite with its
thresholds, `clippy -D warnings`, `rustfmt`, `cargo audit` and `npm audit`,
builds the container image and exercises it end to end
(`scripts/e2e_image.sh`), and opens stores written by the previous `main`
with the current build (`scripts/upgrade_check.sh`). On every push to `main`
it measures line coverage of the whole Rust workspace (cargo-llvm-cov) and of
the web UI (V8), fails below 90 % on any crate or on the web, and publishes
the figures as the badges of the README (branch `coverage-badges`).
