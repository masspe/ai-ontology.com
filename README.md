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
| `ontology-rag`     | Prompt builder + `LanguageModel` trait (echo + Anthropic clients with prompt caching). |
| `ontology-server`  | axum HTTP server exposing `/concepts`, `/relations`, `/retrieve`, `/ask`. |
| `ontology-cli`     | `ontology` binary tying it all together. |

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
                              │      (Anthropic / Echo)        │
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
./target/release/ontology --data $DATA snapshot
./target/release/ontology --data $DATA compact          # snapshot + truncate WAL
./target/release/ontology --data $DATA path \
    --from-type Person --from-name Alice \
    --to-type   Person --to-name   Bob
./target/release/ontology --data $DATA export out.jsonl  # round-trips through `ingest`

# HTTP API
./target/release/ontology --data $DATA serve --bind 127.0.0.1:8080 &
curl -s localhost:8080/stats | jq
curl -s -XPOST localhost:8080/retrieve -H 'content-type: application/json' \
  -d '{"query":"retrieval augmented generation","top_k":4,"lexical_weight":0.5,"expansion":{"max_depth":2}}'
```

## Observability

`GET /metrics` returns Prometheus-format gauges (`ontology_concepts`,
`ontology_relations`, `ontology_concept_types`, `ontology_relation_types`).
Wire it into your Prometheus scrape config alongside the bearer token.

## Ingest formats

* `*.jsonl` / `*.ndjson` — one tagged `Record` per line.
* `*.triples` / `*.txt`  — `Type:Name predicate Type:Name`, `#` comments.
* `*.csv` — header row with a `name` column; `--csv-type <Type>` required.
* `-` (literal hyphen) — read JSONL from stdin: `cat data.jsonl | ontology ingest -`.

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

The Anthropic client retries 408 / 409 / 429 / 5xx with full-jitter
exponential backoff (default 3 retries; configurable via
`AnthropicModel::with_max_retries`). When the server sends a `retry-after`
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

Licensed under the **GNU Lesser General Public License v3.0 or any
later version** (LGPL-3.0-or-later). See `LICENSE` for the LGPL-3.0
text and `LICENSE.GPL` for the referenced GPL-3.0 text. Each source
file carries an SPDX short-form header
(`SPDX-License-Identifier: LGPL-3.0-or-later`).

In short: you can use, modify and redistribute this software,
including in proprietary applications that link against it as a
library. Modifications to the LGPL'd files themselves must be
released under the same license. See the LICENSE files for the full,
binding terms.
