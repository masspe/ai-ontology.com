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
| `ontology-rag`     | Prompt builder + `LanguageModel` trait (echo + Anthropic clients). |
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

# HTTP API
./target/release/ontology --data $DATA serve --bind 127.0.0.1:8080 &
curl -s localhost:8080/stats | jq
curl -s -XPOST localhost:8080/retrieve -H 'content-type: application/json' \
  -d '{"query":"retrieval augmented generation","top_k":4,"lexical_weight":0.5,"expansion":{"max_depth":2}}'
```

## Ingest formats

* `*.jsonl` / `*.ndjson` — one tagged `Record` per line.
* `*.triples` / `*.txt`  — `Type:Name predicate Type:Name`, `#` comments.
* `*.csv` — header row with a `name` column; `--csv-type <Type>` required.

## Testing

```bash
cargo test --workspace
```
