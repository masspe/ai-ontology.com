# Changelog

All notable changes to this project are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Fixed

- A sync with a watermark compared the kept value and the new one as
  text: after 9 came 10, which sorts before it, and the watermark stayed
  at 9 — every later run read the same rows again. Values that read as
  numbers now compare as numbers.
- SQL Server dates and times were kept in the driver's debug form, which
  the engine cannot read back: a date column could not be a watermark.
  They are ISO 8601 now (`2026-02-01T10:00:00`, a `datetimeoffset` as
  UTC with `Z`).
- The ODBC connector returned every value as text; integers, floats and
  bits are now the JSON values their column type says.
- A type declared by several sources kept only the last source's
  property list: the source scanner's `Screen` declaration erased the
  `help` property the help-page import had added, and the next import
  refused every screen. A declaration now adds its properties to the
  registered type's; nothing is taken away.
- A cross-reference `--sync` after the schema dump refused (or, forced,
  would have deleted) the tables and fields no program touches and their
  `has_field` links — the dump's, not the cross-reference's.
  `IngestOptions::prune_keep` names the types an import declares but
  shares; the xref sync keeps `Database`, `Table`, `Field`, `has_field`,
  `in_database`, `fires_on`.

### Changed

- The container image is built with the SQL Server and PostgreSQL
  drivers (`--build-arg FEATURES=…`); ODBC stays a build by hand.
- The SQL Server connector asks the engine for no more rows than the cap
  (`TOP`), instead of cutting the result after it has travelled.
- `url` 2.5.9, `quinn-proto` 0.11.19 and `quinn-udp` 0.5.16, past
  RUSTSEC-2026-0592, -0523, -0524 and -0538.
- Retrieval ranks by grain. The lexical index scores with BM25 (repeats
  saturate, a screen with a long help page is no longer ranked under a
  five-word label). A sheet the question names exactly — a screen code,
  a table — comes first, the boost weighted by how identifying the word
  is. A concept type can be declared `detail` (the `Field` type is):
  its sheets are never seeds unless the question gives their full name
  (`sdba.nogf`) or asks for the type, so tables and screens are not
  crowded out by their twenty thousand fields. On the draft question set
  the pass rate went from 7/12 to 11/12.
- `ontology eval --lexical-weight <0..1>` measures one signal alone.

### Added

- PostgreSQL as a data source (kind `postgres`, build feature
  `postgres`, pure Rust with rustls): tables, columns, a query with a
  watermark, named queries. Every session is read-only on the server
  whatever the account may do, a statement ends at the source's timeout,
  one statement per call, TLS on by default with the certificate checked
  against the public authorities and the host's own.
- The connectors are tested against real engines: the CI job
  `connectors` starts a PostgreSQL and a SQL Server, seeds them and
  reads them through each driver (ODBC through the PostgreSQL ODBC
  driver), then syncs a source into the graph and asks it through
  `/ask`. The same tests run against any engine named in
  `ONTOLOGY_TEST_POSTGRES`, `ONTOLOGY_TEST_SQLSERVER` or
  `ONTOLOGY_TEST_ODBC` (`crates/io/tests/live_engines.rs`).
- A property test on named-query parameters: whatever a question
  contains, the statement sent is the administrator's with one quoted
  literal in it.
- Data sources fail well. Every call to a source ends within its
  timeout (the ODBC driver gets the same limit for the statement). Five
  calls in a row without an answer open the source's circuit for a
  minute: syncs and lookups fail at once instead of queueing behind a
  dead engine; *Test connection* goes through and closes it. `/metrics`
  carries one series per source (`ontology_source_calls_total`,
  `_failures_total`, `_rows_total`, `_lookups_total`, `_syncs_total`,
  `_sync_failures_total`, `ontology_source_breaker_open`).
- Named queries: a data source can answer a question live — a trigger
  (a regular expression with named groups) and an administrator's SELECT
  with `{group}` blanks; a matching question binds the groups as safe
  literals, runs the query (200 rows at most) and the rows join the
  prompt as a delimited data block. The model never writes SQL.
- Data sources: a database or a file the graph is kept in step with
  (`/sources`, the *Sources de données* page, administrators). A source
  carries mappings — a table or a SELECT becomes sheets of a type, a
  foreign-key column a link — previewed before they run, synced by hand
  or every N minutes as background jobs through the regular ingester
  (batches of 2 000 rows under the writer lock; sheets the source no
  longer has are marked stale; a watermark column makes a sync
  incremental). Connectors: CSV (path or HTTPS URL) in every build, SQL
  Server and ODBC (OpenEdge and anything with a driver) behind the
  `sqlserver` and `odbc` build features. Credentials sealed at rest and
  never served; private and local hosts refused unless allowed per source.
- Background jobs: a list of the work that outlives a request (a sync, a
  re-index) with status, progress and the tail of each job's log —
  `GET /jobs`, `GET /jobs/<id>`, `DELETE /jobs/<id>` to stop one between
  batches — kept in `jobs.json` across restarts (a job running at the
  stop shows as interrupted). The top bar lights up while a job runs or
  after a recent failure.
- Nothing accumulates: the audit log rotates past 64 MB (the last ten
  files kept, the hash chain running across them; `--audit-rotate-mb`,
  `--audit-keep`); the embedding cache of a previous model is removed when
  another is configured; the release workflow deletes untagged image
  layers and image versions beyond the newest ten on GHCR; debug builds
  carry line tables only (`Cargo.toml` profiles) and
  `scripts/clean-build` sweeps stale build artifacts, test stores in the
  temp directory and the npm cache.
- Platform hardening: security headers on every response (`nosniff`,
  `X-Frame-Options: DENY`, a strict referrer policy, a content security
  policy forbidding framing, base and object injection, HSTS once the
  public URL is HTTPS); CORS restricted to the public URL once it is set;
  request bodies capped at 8 MB except uploads (512 MB) and the import
  assistant (64 MB); the cookie routes (`/auth/refresh`, `/auth/logout`)
  require `X-Requested-With: ontology`, which the UI sends. `cargo deny`
  in CI (`deny.toml`); releases carry a CycloneDX SBOM and the image is
  signed with cosign (keyless). `docs/THREAT-MODEL.md`.
- Scoped API keys: `POST /auth/keys` takes `clearances` (the sheets the
  key may see; absent: everything), `read_only` (every write refused;
  retrievals and questions are reads), `expires_in_days`;
  `POST /auth/keys/<id>/rotate` issues a fresh secret with the same
  scope; the listing shows the scope and the time of last use.
- The audit log is a hash chain (`prev`, `hash` on every line;
  `ontology audit-verify` walks it) and records, beyond writes: sign-ins
  and their means, refused sign-ins and why, role and clearance changes,
  keys created or rotated, and every question or retrieval with the
  storage domains it touched — the question's text only with
  `serve --audit-questions`, a short hash otherwise. `GET /audit` filters
  by `who`, `path` prefix and `since`.
- Questions that must not leave the premises: `llm.on_premises_domains`
  names the storage domains whose sheets stay inside; a question whose
  context holds one is answered by `llm.on_premises_provider` only (a
  model on your network behind an OpenAI-compatible URL) or refused with
  the reason. The strictest domain in the context decides.
- Access control on the API: a reader's clearances (`PATCH /auth/users/<id>
  {"clearances": [...], "role": ...}`, administrators only; or the OpenID
  provider's group claims with `groups_as_clearances`) decide which
  labelled sheets exist for them — a 404 on the sheet, neither listed nor
  counted, links hidden, no path through, never in a subgraph, a retrieval
  or an answer, no write to it, no label beyond one's clearances (403),
  the export for administrators. Administrators, the service token and
  API keys see everything; without a login, so does everyone. Visibility
  is cached per clearance set and generation. The **Accès** page (account
  menu) edits roles and clearances; the sheet page shows and edits labels.
- Security labels on sheets (`Concept.labels`, `ConceptPatch.labels`):
  a reader sees a sheet when their clearances include every label of it.
  The graph takes a reader's `Visibility` on traversals (`TraversalSpec`),
  paths, closures and listings — a hidden sheet is never crossed, listed
  or counted — and the index takes it on `RetrievalRequest`, so a hidden
  sheet is never a candidate and the expansion inherits the reader. In
  the store, a labelled concept is written as an appended record shape
  (`ConceptV2`, `UpdateConceptV2`); a concept without labels keeps the
  shape every build reads.
- Sessions: an access token lives 15 minutes; a rotating refresh secret
  in an HttpOnly cookie (`/auth` only) renews it for 30 days after the
  last use (`POST /auth/refresh`; the web UI renews on a 401 by itself).
  Logout ends the session, so its tokens are refused at once;
  `DELETE /auth/sessions` — "sign out everywhere" in the account menu —
  ends every session of the account; deleting an account ends its
  sessions. Sessions live in `users.json`, secret hashed.
- Sign-in through OpenID Connect, in the binary: Microsoft Entra ID,
  Google or any provider with a discovery document, configured by the
  administrator in Settings → Sign-in (public URL, issuer, client id and
  secret, allowed e-mail domains, administrator groups). Authorization
  code with PKCE, `state` and `nonce` in a signed HttpOnly cookie, the
  `id_token` verified RS256 against the provider's keys; an account is
  found by provider subject, attached by verified e-mail, or created when
  the e-mail's domain is allowed; group claims grant the administrator
  role (and take it from provider-owned accounts). The login and sign-up
  pages show one button per configured provider (`GET /auth/providers`).
  The Node `auth-server/` is deprecated.
- Secrets at rest: every secret field of the settings file (the provider
  API keys) is sealed with a tenant master key, AES-256-GCM, before it is
  written — `master.key` beside the settings, generated owner-only on
  first use, or `ONTOLOGY_MASTER_KEY` (64 hex characters). A settings
  file written before this is read as it is and sealed on its next save;
  a secret the key does not open is cleared and logged, never a reason to
  refuse the file. `ontology rotate-key` re-seals with a fresh key.
- `ontology ingest --df <dump> --df-db <name>` reads a Progress
  data-definition dump (`.df`): every table and field of the database —
  including the ones no program touches — with the labels, descriptions,
  help texts, types and formats the Data Dictionary holds, the primary and
  unique keys as `primary_key` / `unique_keys` on the table, and a
  `fires_on` link from each database trigger program to its table. Labels
  become the sheets' descriptions, so a question asked in the users'
  language (« salariés ») finds the table (`emp`).
- Answers name their sources: every `#id` the model cites is rendered as
  the sheet it points to (`Table · appdb.emp`, linked), and the `Cited:`
  line becomes the row of sheets the answer rests on, cited ones first —
  on the home page as the answer streams and on the Questions page.
- The interface, continued: the home page opens on the question (the
  pinned ones as chips under it, the counts as a quiet strip); a wordmark
  in the top bar; the sheet page as a card-index entry (type, name in
  mono, links grouped by type with counts, properties as a list); empty
  states as an invitation; graph node colours from a categorical palette
  validated for colour-vision deficiency on light and dark; and a dark
  theme — the toggle in the top bar, "system" by default.
- `ontology eval <questions.jsonl> [--top-k 5] [--min 0.8]`: retrieval
  measured against real questions — each line `{"question", "expect":
  [{"type", "name"}]}`, a pass when an expected sheet is in the top-k;
  rank or miss per question, the pass share, a non-zero exit below
  `--min` so CI can gate on it. The question file stays outside the
  repository (it holds the customer's vocabulary).
- Entity linking: `ontology link-mentions --from <types> --to <types>`
  links the sheets of some types to the sheets their text names — a help
  page or a ticket that says `gl100` or `gl/w-gl100.w` gets a
  `mentions_<from>_<to>` link to that screen or program, weighted by how
  often; program paths match by their file name too, an alias that could
  mean two sheets means none. Runs again without rewriting.
- Conversations: the Questions page keeps each answer as a turn and sends
  the turns before with the next question (`history` on `/ask` and
  `/ask/stream`: question, answer, cited ids), so a follow-up means what
  the last answer named — the sheets cited earlier join the subgraph at
  depth 0 and the prompt carries the last four exchanges. Threads are
  kept in the browser; an earlier question that cited the same sheets is
  offered as « déjà demandé » and can be resumed.
- Retrieval vectors from a provider's embedding model (OpenAI,
  Infomaniak, any OpenAI-compatible `/v1/embeddings`): Settings → LLM →
  *Vecteurs de recherche*, or `llm.embedding_provider` /
  `llm.embedding_model`. Multilingual meaning in place of the hashing
  embedder; vectors cached on disk under `<data>/embeddings/` so a
  restart re-embeds nothing; a provider that does not answer is reported
  at start and the built-in embedder keeps serving. `Embedder` gains
  `name`, `embed_batch`, `flush`; the reindex embeds in groups of 64 on
  every core.
- Documents attached to sheets by key: `ontology ingest --doc-for <Type>
  [--doc-key <regex>] [--doc-fallback-type <Type>] [--doc-property <name>]
  <folder>` reads HTML, text and Markdown files; each file's text becomes
  a property of the sheet its title or file name names, its title and
  first lines the sheet's description; a file about no known sheet
  becomes a document sheet (fragmented like any text document) or is
  counted. First use: an ERP's user help pages attached to its screens,
  so a question in the users' words finds the screen.
- The graph page on a large model: `POST /subgraph` caps the links it
  returns (`max_edges`, strongest first, round-robin across types — a
  250-sheet neighbourhood of a hub no longer ships thousands of links),
  takes `relation_types` / `exclude_relation_types`, and relation types
  can be declared `detail` (below the grain people ask about: the
  software model's field-level access) — hidden from the graph page
  unless its new toggle asks. Edge labels switch off past 300 links;
  only the visible elements are rendered.
- The interface reads as a card index: IBM Plex Sans (self-hosted; Plex
  Mono for identifiers), one petrol accent kept for what is interactive
  or cited, hairline cards without drop shadows, sentence-case table
  headers, a visible keyboard focus ring, reduced motion respected.
- `docs/SOFTWARE-MODEL.md`: how a code base becomes a graph and why
  answers are what they are.
- `IngestOptions::merge_properties`: an upsert that keeps the properties
  and description the new record does not set, so the cross-reference,
  the sources and the schema dump can each write their part of one Table
  or Field sheet. On for the three `erp` importers.

### Security

- The host policy of data sources judged an IPv4 address written as IPv6
  (`::ffff:169.254.169.254`, the NAT64 prefix) as public; it is now
  judged as the IPv4 address it is, and the carrier-NAT range, `0.0.0.0/8`,
  broadcast and multicast are refused too. The address that passed the
  check is the one the connector connects to (no second DNS lookup), and
  a CSV URL that redirects is refused instead of followed.

## [0.2.0] - 2026-10-21

### Added

- Import of a Progress OpenEdge code base as a software model: `ontology
  ingest --xref` reads the compiler's `*.xref.xml` cross-reference (6 880
  files, 8 GB, in half a minute) into Module, Program, Include, Class,
  Database, Table and Field sheets with the links between them (runs,
  includes, invokes, reads, updates, creates, deletes, field-level
  access); `ontology ingest --abl` reads the `.p`/`.w`/`.i` sources for
  what the cross-reference lacks — the joins between tables, with their
  keys, and the screens with their code (`gl100`), title, tabs and menu.
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
