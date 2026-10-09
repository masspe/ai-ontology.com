# Changelog

All notable changes to this project are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0] - 2026-10-09

First public release.

### Added

- Typed knowledge graph with a schema (sheet types, fields, link types),
  cursor-paginated REST API with an OpenAPI description served at `/docs`.
- Import of Word, Excel, CSV, JSONL, triples and text; PDF and scanned images
  read in the browser (pdf.js, tesseract.js); an import assistant that
  proposes sheets and links for review before anything is written.
- Hybrid retrieval (lexical + vector, fused per request) with bounded graph
  expansion; questions answered with citations checked against the
  retrieved sheets, streamed over Server-Sent Events.
- Language models: Anthropic, OpenAI, DeepSeek, Infomaniak, offline echo;
  keys stored in the data directory; prompt caching where supported.
- Web UI in French and English: home with a three-step start, files, import
  review, data model (generated from a description or chosen from three
  ready-made models, with an impact check before a change), sheets, graph,
  questions, rules, actions, settings, an interactive guide.
- Built-in sign-up and login (the first account is the administrator), named
  API keys, audit log of every write, per-IP rate limit on `/auth/*`.
- Segmented store: every write on disk before it changes the graph, CRC
  checked records, one `fsync` per batch, torn-tail recovery, memory-mapped
  sealed segments, partitioning by storage domain, automatic migration of
  the legacy log.
- Memory budget read from the container limit, `strict` and `adaptive`
  modes, payloads kept on disk (`--tier p1`); 2 million sheets and
  10 million links measured on a 16 GB runner in P1.
- Incremental backup (`POST /backup`, `ontology backup`) and verified
  restore (`ontology restore`).
- Prometheus metrics (`/metrics`), per storage domain.
- One container image (distroless, non-root) serving the API and the UI on
  one port; `compose.yaml` with one container per client, health probe,
  clean stop; the CI builds the image, exercises it end to end and opens
  stores written by the previous `main` with the current build.

### Known issues

- Questions about amounts on the finance example ("total invoiced to
  Initech?") may answer "I don't know": numeric fields do not always reach
  the prompt.
- The graph canvas blanks for a couple of frames when a sheet is selected on
  slow machines.
- Google / Microsoft sign-in is not served by the binary; the optional Node
  `auth-server/` covers it.

[Unreleased]: https://github.com/masspe/ai-ontology.com/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/masspe/ai-ontology.com/releases/tag/v0.1.0
