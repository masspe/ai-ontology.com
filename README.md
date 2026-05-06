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
./target/release/ontology --data $DATA serve --bind 127.0.0.1:8080 &
curl -s localhost:8080/stats | jq
curl -s -XPOST localhost:8080/retrieve -H 'content-type: application/json' \
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
$server = Start-Process -FilePath .\target\release\ontology.exe -ArgumentList @('--data', $env:DATA, 'serve', '--bind', '127.0.0.1:8080') -PassThru
Invoke-RestMethod http://127.0.0.1:8080/stats
Invoke-RestMethod -Method Post -Uri http://127.0.0.1:8080/retrieve -ContentType 'application/json' -Body '{"query":"retrieval augmented generation","top_k":4,"lexical_weight":0.5,"expansion":{"max_depth":2}}'
Stop-Process -Id $server.Id
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
