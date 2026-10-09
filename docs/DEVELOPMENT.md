# Development

How to run the three development servers, the tests and the coverage
measurement. For installing the product, see [DEPLOY.md](DEPLOY.md).

## Running the full stack locally

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

The Node `auth-server` is **optional**: the binary's built-in login (`serve --login`)
covers sign-up, sessions, accounts and API keys; keep the Node server only for
Google / Microsoft OAuth sign-in. See [auth-server/README.md](../auth-server/README.md)
for that setup. The ontology API needs no environment variable at all — the LLM
provider is configured in the UI (**Settings → Configuration**) and stored in
`data/settings.json`.

### Things that will cost you an hour if you do not know them

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

### Running the services separately

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
(see [`web/vite.config.ts`](../web/vite.config.ts)), so the SPA needs no URL
configuration in development. `VITE_API_BASE` and `VITE_AUTH_API_BASE` only
matter when serving the built bundle from a different host. Overriding the
proxy targets is possible with `ONTOLOGY_API_URL` and `AUTH_API_URL`.

### Seeding the demo graph

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


## Generating an ontology from the UI


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

## Tests

```bash
cargo test --workspace          # unit, integration and end-to-end tests (the binary is spawned)
cd web && npm run test          # vitest: logic + every page rendered (Testing Library, jsdom)
cd web && npm run test:coverage # same, with the 90 % coverage thresholds enforced
cargo llvm-cov --workspace --summary-only   # line coverage of the Rust workspace
```

The CI enforces 90 % line coverage per Rust crate and on the web suite and
publishes the figures as the badges of the README. How it does so is in
[ARCHITECTURE.md](ARCHITECTURE.md#tests-and-coverage).

On a 16 GB laptop with 14 threads: the `release` build of the CLI takes
about 13 minutes, the full web suite about 3 minutes, the instrumented
Rust suite about 15 minutes.
