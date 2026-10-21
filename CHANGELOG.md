# Changelog

All notable changes to this project are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.2.0] - 2026-10-21

### Added

- Import of a Progress OpenEdge code base as a software model: `ontology
  ingest --xref` reads the compiler's `*.xref.xml` cross-reference (6 880
  files, 8 GB, in half a minute) into Module, Program, Include, Class,
  Database, Table and Field sheets with the links between them (runs,
  includes, invokes, reads, updates, creates, deletes, field-level
  access); `ontology ingest --abl` reads the `.p`/`.w`/`.i` sources for
  what the cross-reference lacks — the joins between tables, with their
  keys, and the screens with their code (`ca100`), title, tabs and menu.
  `ontology xref-stats` is the dry run: the import counted, not written.
- Re-import in place: `--xref` and `--abl` update what changed and
  journal nothing for what did not; `--sync` deletes what the files no
  longer say (a program whose file is gone, a link a recompile dropped),
  refused above a quarter of a type's sheets unless `--sync-force`.
- `PropertyValue` constructors (`From<&str>`, `From<String>`, `list()`);
  `NamedRelation` records carry properties; `RelationType::limits()`.

### Changed

- Relations are journaled in batches like concepts (one barrier per 256
  instead of one per relation). Journal order stays arrival order; a
  constrained type settles the batch only when a pending sibling shares
  the limited endpoint, including through a symmetric type's inverse.
- Subgraph expansion admits the strongest links first under the node
  cap, round-robin across relation types, and clones an edge only once
  admitted.
- The prompt caps property values (240 characters, 16 list items), shows
  a link's properties and weight in its facts, and keeps the facts a
  share of the budget — sheets give way first, by whole lines.
- Symmetric inverse edges carry the original's properties and follow
  its updates.

### Fixed

- Updating a symmetric relation left its inverse untouched.
- Graph errors from the concept delete handler were reported as 400.

### Security

- `zip` 4.6 → 9.0 and `calamine` 0.36 → 0.37 (which brings `zip` 9 of its
  own): RUSTSEC-2026-0367, -0368 and -0369, all three in
  `ZipArchive::extract`, which this code base never calls — it reads named
  entries out of `.docx` and spreadsheet archives. One `zip` in the tree
  now instead of two, and one `quick-xml`.
- `rustls` 0.23.45 → 0.23.46, `rustls-webpki` 0.103.15 → 0.103.16,
  `webpki-roots` 1.0.7 → 1.0.10: RUSTSEC-2026-0346 to -0349, -0381, -0382.

## [0.1.1] - 2026-10-09

### Fixed

- The properties of the retrieved sheets (an invoice's `amount_eur`,
  `issue_date`, …) now reach the prompt, so questions about amounts are
  answered from the data instead of "I don't know".

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

- The graph canvas blanks for a couple of frames when a sheet is selected on
  slow machines.
- Google / Microsoft sign-in is not served by the binary; the optional Node
  `auth-server/` covers it.

[Unreleased]: https://github.com/masspe/ai-ontology.com/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/masspe/ai-ontology.com/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/masspe/ai-ontology.com/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/masspe/ai-ontology.com/releases/tag/v0.1.0
