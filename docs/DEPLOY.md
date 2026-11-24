# Deployment: one image, one container per client

This document is the product's installation page: authentication inside the
binary, a server you run, backup by copying files. It assumes Docker on a Linux server that you administer.

## 1. What the image contains

A single image, `Dockerfile` at the repository root, five stages (web build, dependency recipe, dependency cache, compilation,
runtime): build of the web interface (Node), the Rust dependency recipe and
cache, compilation of the `ontology` binary, and a minimal final image
(distroless) with the binary and the interface. At startup the binary does everything on port 5000:

- the API (`/concepts`, `/retrieve`, `/metrics`, …);
- the web interface, served for every address that is not an API route
  (`/`, `/graph`, `/concepts/…` return `index.html`, the browser does the
  rest);
- user login (`/auth/login`, `/auth/signup`, `/auth/me`, `/auth/logout`,
  account management by the administrator), with the accounts in
  `/data/users.json` and the token secret in `/data/jwt.secret`;
- the settings (`/data/settings.json`, written from the UI), whose secrets
  — provider API keys — are sealed with the master key in
  `/data/master.key` (generated on first start) or in the environment
  variable `ONTOLOGY_MASTER_KEY` (64 hex characters) when set. A copy of
  the settings file without the key reveals no credential.

One process per client (STORAGE.md §10.8): each client has its own container,
its own data volume, its own accounts, its own memory limit.

## 2. Starting a client

```sh
docker compose up -d                  # two example clients: acme, globex (pulls the published image)
docker compose up -d --build          # same, building the image from this checkout
open http://localhost:5001 in a browser (acme; globex is on 5002)
```

`compose.yaml` shows the model: one service per client, one volume per
service, `mem_limit`, a health probe (`ontology healthcheck`), a clean stop
(`stop_grace_period`). Two probes, neither asks for a token: `/healthz`
says the process is alive, `/readyz` says it answers. `serve` opens its
port before it replays the store: until the replay is done `/healthz` is
200, `/readyz` and every other route are 503 with `Retry-After`. Point a
liveness probe at `/healthz` and a readiness probe (or the load
balancer's check) at `/readyz`, as `compose.yaml` does. Login attempts are rate-limited per IP address: behind
a reverse proxy, the address the binary sees is the proxy's, unless the proxy
carries that limit itself. For one more client, copy a service block and its
volume; routing by host name (TLS included) is the job of the reverse proxy in
front of the containers (Caddy or Traefik do this in a few lines).

Useful binary options (in `command:`):

| Option | Purpose |
|---|---|
| `--memory-mode strict` (image default) | a store that does not fit in the container's memory limit refuses to start, giving both figures, instead of being killed later |
| `--tier p1` | payloads on disk: the setting for a 16 GB node beyond one million concepts (STORAGE.md §7.8) |
| `--login` | built-in login (enabled by the image); `--allow-signup` leaves sign-up open after the first account |
| `--jwt-secret-env NAME` | take the token secret from the environment variable `NAME` rather than from `/data/jwt.secret` |
| `ONTOLOGY_MASTER_KEY` (environment) | the master key sealing the settings' secrets, instead of `/data/master.key`; `ontology rotate-key` re-seals them with a fresh key file (back the volume up first) |
| `--seed /path` | load an example (schema + JSONL) into an empty store, once |
| `--web /srv/web` | folder of the built interface (the one in the image) |
| `--backup-dir /backups` | folder where `POST /backup` copies the store (image default; §5) |

## 3. First account and following accounts

The **first account created** on a client becomes its administrator; from then
on open sign-up is closed (response 403 "Sign-up is closed"). The
administrator creates the following accounts:

```sh
TOKEN=$(curl -s -X POST http://localhost:5001/auth/login \
  -H 'content-type: application/json' \
  -d '{"email":"admin@acme.ch","password":"…"}' | python3 -c 'import json,sys;print(json.load(sys.stdin)["token"])')
curl -s -X POST http://localhost:5001/auth/users -H "authorization: Bearer $TOKEN" \
  -H 'content-type: application/json' \
  -d '{"email":"jean@acme.ch","password":"Passw0rd-example!","name":"Jean"}'
```

Rules: valid address, password of 8 characters and three classes (uppercase,
lowercase, digit, other), non-empty name. Passwords are hashed (bcrypt); a
`users.json` written by the former Node server remains valid as is. Such a file
has no administrator and sign-up is closed on it: to designate one, stop the
container, add `"role": "admin"` to an account in the file, restart. Deleting
an account revokes its tokens immediately. Accounts are created by the
administrator through the API, as above.

### Sessions

A sign-in (password or OpenID Connect) opens a **session**: the access
token in the response lives 15 minutes; a refresh secret, in an HttpOnly
cookie scoped to `/auth`, renews it (`POST /auth/refresh`, the secret
rotates each time) for 30 days after the last renewal. The web UI renews
by itself when a call comes back 401. `POST /auth/logout` ends the
session of the cookie — its access tokens are refused from that moment,
not at their expiry; `DELETE /auth/sessions` (the account menu's "sign out
everywhere") ends every session of the account; deleting an account ends
its sessions. Sessions are kept in `users.json` (secret hashed). A token
signed with the shared secret by something else (`--jwt-secret-env`) has
no session and is accepted until it expires, as before.

### Sign-in with a company identity (OpenID Connect)

The binary signs users in through any OpenID Connect provider — Microsoft
Entra ID and Google first — so identity, MFA and leavers stay with the
client's IT. The administrator configures it in **Settings → Sign-in** (or
`PATCH /settings {"signin": …}`): the public URL of the server, then one
provider per button with its issuer (`https://login.microsoftonline.com/<tenant-id>/v2.0`,
`https://accounts.google.com`, …), client id and secret (sealed at rest with
the master key, §1), the **e-mail domains** whose users get an account on
their first sign-in (empty: only existing accounts may sign in), and the
**groups** whose members are administrators (empty: the provider does not
decide roles; a local administrator is never demoted by the groups). At the
provider, register the redirect URI the page shows:
`<public URL>/auth/oauth/<id>/callback`, and ask for the `groups` claim when
you use it (Entra: *Token configuration → Add groups claim*).

The flow is the authorization code with PKCE; `state` and `nonce` live in a
signed, HttpOnly cookie; the `id_token` is verified RS256 against the
provider's published keys (issuer, audience, expiry, nonce). A user is
found by provider subject, else attached to the account with the same
verified e-mail, else created. The login page shows one button per
configured provider (`GET /auth/providers`); a refused sign-in comes back
to the UI with `?error=<code>` (`domain_not_allowed`, `id_token_invalid`,
`invalid_state`, …) and the detail in the server log. The Node
`auth-server/` is deprecated: nothing needs it any more.

### Who sees what: labels and clearances

A sheet may carry **security labels** (`labels` on a concept, e.g.
`["payroll", "ch"]`); an account has **clearances**. A reader sees a sheet
when their clearances include every label of it — a sheet without labels
is visible to every account. Administrators, the service token and API
keys see everything. The rule holds on every read path: a hidden sheet is
a 404 (like an id that never existed), is neither listed nor counted, its
links are not listed, no path runs through it, the graph page, retrieval
and the question never carry it, and it cannot be written to; a label
beyond one's clearances cannot be attached (403), and the export of the
whole graph is for administrators. Visibility is computed once per
clearance set and graph generation (a hidden-id set, cached), so a read
costs one lookup per sheet.

The administrator sets clearances in **Accès** (the account menu) or with
`PATCH /auth/users/<id> {"clearances": ["payroll"], "role": "user"}`;
labels are edited on the sheet (« Corriger ») or with
`PATCH /concepts/<id> {"labels": [...]}`. A label or clearance is
`[a-z0-9][a-z0-9_.-]{0,63}`. With OpenID Connect, a provider can turn the
user's group claims into their clearances at every sign-in
(`groups_as_clearances`). A change takes effect on the next request, the
tokens already issued included.

### API keys and audit log

For a machine caller (an ERP, a script), the administrator creates a **named
key** rather than lending their own account — on the **Access** page, or
through the API: `POST /auth/keys {"name":"ERP"}`
returns the secret once (`ok_…`); `GET /auth/keys` lists the keys (name,
prefix, creation, by whom, scope, last use), `DELETE /auth/keys/<id>` revokes
immediately, `POST /auth/keys/<id>/rotate` issues a fresh secret with the same
scope (the old one dies at once). A key can be **scoped**: `"clearances":
["erp"]` restricts it to the sheets those labels allow (absent: it sees
everything), `"read_only": true` refuses every write (a retrieval or a
question is a read), `"expires_in_days": 90` ends it. The key is presented
like a token (`Authorization: Bearer ok_…`); it opens the API, but not
account management, reset or backup, which need a signed-in user. The
`users.json` file keeps only a fingerprint and the time of last use.

Every successful write is recorded in `/data/audit.jsonl` (one JSON line:
time, caller, method, path, status, request identifier), and so are the
events that matter without being writes: a sign-in and its means (password,
the provider), a refused sign-in and why, a role or clearance change, a key
created or rotated, and every question or retrieval with the **storage
domains it touched** — the text of the question itself only with
`--audit-questions`, otherwise a short hash of it. Each line carries the
hash of the line before and its own (`prev`, `hash`): a line removed,
altered or inserted breaks the chain, which `ontology audit-verify` checks
(exit 1 naming the first bad line). `GET /audit?limit=100&who=<email>&path=/auth&since=<RFC 3339>`
reads back the last lines, administrators only. The file rotates past
64 MB (`audit.<unix seconds>.jsonl` beside it, the last ten kept, the chain
running across files; `--audit-rotate-mb`, `--audit-keep`). It is part of
the `/data` volume; it is not in the store backup (§5), so copy it along
with the volume. Options: `--audit-log <path>`, `--no-audit`,
`--audit-questions`.

### Data sources

**Data sources** (`/sources`, administrators) keep the graph in step with
a database or a file: a source says where and how to connect (the
password or the ODBC connection string sealed at rest with the master key,
never served; `sources.json` beside the store), and carries **mappings**:
a table (or a `SELECT`) becomes sheets of a type — a key column is the
identity, a name column the name, the other columns the properties, and a
foreign-key column becomes a link to a sheet of another type (created as a
bare sheet when it is not there yet). *Test connection*, browse the tables
and columns, *Preview* (the first 50 rows and the sheets they would be,
nothing written), *Sync* now, or every N minutes. A sync is a background
job (`/jobs`, the light in the top bar): rows go through the regular
ingester by batches of 2 000 (the writer lock per batch, never per sync),
so a running sync never blocks a question. A full sync marks the sheets
the source no longer has as `stale: true` (nothing is deleted); with a
**watermark column** (an updated-at or an increasing id) a sync reads only
the rows past the last value seen. Each sheet carries `source =
"<source>/<mapping>"`. At most 200 000 rows per run: past that, use a
watermark column.

Kinds in every build: `csv` (a path on the server, or an HTTPS URL
fetched on every run; `;` or `,`). Behind a build feature: `sqlserver`
(pure Rust, TLS by default, `ApplicationIntent=ReadOnly`), `postgres`
(pure Rust, TLS by default against the public authorities and the ones
the host trusts; every session is read-only — but for the one transaction
of an approved write, below — and its statements end at the source's
timeout on the server) and `odbc` (the driver manager and the
engine's driver on the host). The container image is built with
`sqlserver` and `postgres` (`--build-arg FEATURES=…` to change that); a
binary built by hand takes `cargo build -p ontology-cli --features
sqlserver,postgres,odbc`, and ODBC in a container needs an image with
`unixODBC` and the driver. *Accept a self-signed certificate* on a source
keeps the encryption and stops checking who answers: for a server inside
the network only.

Values arrive typed where the engine says what they are (integers,
floats, booleans); exact decimals keep their digits as text, dates are
text, year first. A watermark compares as a number when both values read
as one, as text otherwise — such a date sorts as written.

A host that resolves to a loopback, link-local or private
address is refused unless the source says *the server is on the private
network* (an administrator typing a host is the classic way to reach a
cloud's metadata service). The address that passed the check is the one
connected to (a second DNS answer cannot lead elsewhere), and a file URL
that redirects is refused: give the final address.

**When the engine does not answer.** Every call to a source ends within
its timeout (60 s unless the source says otherwise; the ODBC driver is
given the same limit for the statement). After five calls in a row that
got no answer, the source's circuit opens: for a minute, syncs and
lookups fail at once with *circuit open* instead of queueing behind a
dead engine, then one call is let through. *Test connection* always goes
through, and closes the circuit when the engine answers. A statement the
engine refuses is an answer: it never opens the circuit.

**OpenEdge (Progress) through ODBC.** The SQL engine, not the ABL broker:
the DBA confirms the SQL broker's port and that it is started with SQL
servers. A read-only SQL account with `GRANT SELECT` on the listed tables
(tables created from ABL belong to `PUB`: `PUB.prog`). Rows wider than a
field's SQL width make the engine refuse the query (error 7864); the
connector says so and names the fix (`dbtool` option 2, or
`-SQLWidthUpdate ON` on the broker). The Progress driver is licensed and
not redistributable: install it where the binary runs.

The form asks what the ODBC administrator of the server's machine shows:
a **data source name** (a DSN set up there, listed by `GET /sources/odbc`
with the installed drivers), or a **driver** with the host, port and
database; then the account. The connection string is built from them —
`DSN=…` or `Driver={…};HOST=…;PORT=…;DB=…`, then `UID=…;PWD=…`, a value
with `;` or braces put in braces. `HOST`, `PORT` and `DB` are the short
keywords of the DataDirect drivers (Progress among them); a driver that
reads other names gets a DSN, or the whole connection string in the
advanced field, which wins over the fields when given.

**A CSV from the administrator's computer.** *Browse…* on a CSV source
sends the file to `POST /sources/files` (administrators, the upload cap):
it is kept in `source-files/` beside `sources.json` under a cleaned name
(`.csv`, `.tsv` or `.txt` only), the same name again replacing it, and
the source gets that path. Send it again before a sync to read a newer
copy.

### Live lookups at question time (named queries)

A source can carry **named queries**: a trigger (a regular expression
with named groups) and a `SELECT` written by the administrator with
`{group}` blanks. A question that matches the trigger — « J'ai le ticket
n° 1234, comment résoudre ? » against `(?i)ticket\s*(?:n°|#)?\s*(?P<id>\d{1,9})`
— runs the SELECT with the groups bound as quoted literals (letters,
digits and a few separators only; a quote never binds) and the rows join
the prompt as a delimited data block, after the retrieved sheets. The
model never writes SQL. At most 200 rows; `admin_only` keeps a query for
callers who see everything; *Try* on the page shows what a question would
get. The audit line of the question names the query (`lookup:<name>`).

### Writing back to a source (approved actions)

Off by default: a source is read only until an administrator says
`allow_writes` on it. Such a source carries **named statements**: one
`UPDATE`, `INSERT` or `DELETE` written by the administrator, bound to an
action type of the data model, with `{blank}`s and a row limit.

```
PATCH /sources/1               {"allow_writes": true}
POST  /sources/1/statements    {"name": "close an order", "action_type": "close",
                                "sql": "update orders set state = 'done' where code = {subject.code}",
                                "max_rows": 1}
POST  /actions/42/run          -> 201, an approval, "pending"
GET   /approvals
POST  /approvals/7/approve     (another administrator)
POST  /approvals/7/reject
```

A blank reads the action: `{subject}` and `{object}` are the names of its
sheets, `{subject.code}` / `{object.code}` a property of one, any other
name a parameter of the action. A value is written as a literal only — a
short text of letters, digits and a few separators (a quote never
passes), a number, or yes/no; anything else and the run is refused,
nothing is asked.

**Run** on an action (the *Ask to run* button of the Actions page) writes
nothing: it files an approval that shows the statement exactly as it
would run. **Another** signed-in administrator approves it (the Actions
page lists what waits) — never the one who asked, never an API key or
the service token, and not at all on a server without sign-in. The
statement then runs in one transaction; if it would change more rows
than `max_rows` (1 unless said otherwise, 10 000 at most) it is rolled
back and the approval is `failed`. Changing the statement or closing the
source voids what was asked before: reject it and run the action again.
Asking, approving (with the rows changed, or the failure) and rejecting
are in the audit log with the statement, as is every change of
`allow_writes`.

What to know before opening a source:

- Give a writing source **its own database account**, with `UPDATE` /
  `INSERT` / `DELETE` on the tables its statements name and nothing
  more; keep the read-only account for the source that syncs. On a SQL
  Server availability group, point it at the primary.
- Four eyes are two **accounts**: an administrator who can create
  accounts can make a second one. The audit log names both.
- A write that ends on the source's timeout is reported `failed`, yet
  the engine may have committed it: check the database before running
  the action again. A server stopped in the middle of a write leaves the
  approval `running`; the audit log and the database say what happened.
- The audit line carries the statement with its values.
- The last 200 decided approvals are listed; the audit log keeps them
  all. Statements are administered through the API (no page yet), by
  administrators — and by an unscoped API key or the service token,
  which can prepare a statement but neither ask nor approve.

### Background jobs

A sync, a re-index, anything that outlives a request runs as a **job**:
`GET /jobs` lists them (newest first; `?active=true` for the running
ones), `GET /jobs/<id>` shows status, progress and the tail of its log,
`DELETE /jobs/<id>` asks a running one to stop between two batches. The
top bar shows a light while a job runs or when one failed in the last
hour. The list lives in `/data/jobs.json` (the last 200 jobs); a job that
was running when the process stopped shows as *interrupted* after the
restart.

### What never accumulates

A backup is a mirror of the store in `/backups`, brought up to date on
each `POST /backup` (only the changed files are copied), not a new folder
every time. The embedding cache keeps one file per model and deletes the
others' when the model changes. The audit log rotates. Container images:
every release pushes tags to GHCR and the release workflow deletes the
untagged layers and the tagged versions beyond the newest ten.

### Questions that must not leave the premises

A storage domain (the `ns` of its concept types) can be declared
on-premises — a tick on the data-model page, or
`PATCH /settings {"llm": {"on_premises_domains": ["payroll"],
"on_premises_provider": "openai"}}`. A question whose context holds a sheet
of such a domain is answered only by the **on-premises provider** — a model
served on your network behind an OpenAI-compatible URL (`openai_base_url`),
whatever the active provider is — and refused with the reason when none is
configured. A question that touches no such domain goes to the active
provider as usual. The strictest domain in the context decides. The audit
line of every question names the domains it touched.

## 4. Memory

The container's limit (`mem_limit`) is what the binary reads as its budget
(cgroup v2), fraction 0.6 by default (STORAGE.md §8.1). Measured reference
points (§7.8): 2×10⁶ concepts and 10⁷ relations fit in 16 GB in P1 (6.4 GB of
heap after hydration); 10⁷ / 5×10⁷ need 64 GB.

## 5. Backup and restore

The backup is a **copy of the
store's files** (`MANIFEST.json` and the `.data` / `.idx` segments, in the
store's layout), taken under the write lock, hence consistent. Sealed segments
are immutable: a repeated backup to the same place only copies the MANIFEST,
the active segments and whatever was sealed since, and removes the segments a
compaction has replaced. The MANIFEST is removed at the start and rewritten
last: an interrupted backup is not restorable ("not a backup") and is simply
run again; it is never restored truncated. A backup folder serves a single
store (identity in the MANIFEST): a second client, or a restored copy, is
refused there.

**Backing up a running container**: `POST /backup` (authenticated call) copies
the store into the folder set at startup by `--backup-dir` (the image sets it
to `/backups`, `compose.yaml` mounts one volume per client there). The
destination is never chosen by the caller. The response says what was copied:

```sh
curl -X POST -H "authorization: Bearer $TOKEN" http://localhost:5001/backup
# {"files":13,"copied":3,"bytes":41290,"records":1284}
```

A scheduled task (cron on the host server) that calls this route and then
synchronizes the `backups` volume to external storage (a mounted share,
`rclone` to an S3-compatible object store, a second server) gives a daily
incremental backup. Retention: keep the external copies for 30 days; the
`backups` volume itself only holds the latest one.

**Backing up a stopped store**: `ontology --data /data backup /backups` does
the same copy from the command line (refused while a server holds the store).

**Restoring**: `ontology --data <folder> restore <backup>` rebuilds
`<folder>/store`: copy into a working folder, opening, full re-read (a backup
that cannot be re-read is rejected before anything is touched), then putting
it in place. The command refuses to overwrite an existing store: move it away
first. The full re-read loads the graph into memory: run it in a separate
container, not next to the server that is running under its memory limit. For
one client:

```sh
docker stop acme
docker run --rm -v acme-data:/data -v acme-backups:/backups --entrypoint sh ghcr.io/masspe/ai-ontology:0.1.1 \
  -c 'mv /data/store /data/store.old && /usr/local/bin/ontology --data /data restore /backups'
docker start acme
```

The restored store has its own identity: its next backup goes to an empty
folder (or to an emptied `/backups`), never on top of the original backup.
The small files next to the store (`users.json`, `jwt.secret`, `settings.json`,
`master.key`, `jobs.json`, `sources.json`) are not part of the backup: copy them along with the volume, or
supply the secrets through `--jwt-secret-env` and `ONTOLOGY_MASTER_KEY`. A
`settings.json` restored without its `master.key` keeps everything but the
provider keys, which the administrator enters again. The CI runs backup through the API, restore
through the command and a graph comparison on every change
(`scripts/e2e_image.sh`), and the store's tests check the round trip, the
incremental behavior and the refusals (`crates/storage/tests/backup.rs`).

## 6. Upgrading

A new image opens the previous one's store as is: the format is versioned and
migrated automatically on open. The CI checks this on every change (job
`upgrade`, `scripts/upgrade_check.sh`: the previous version of `main` writes a
store, the current version opens it, writes to it, compacts, backs up and
restores). Procedure: back up (§5), set the new version tag of the image in
`compose.yaml` (`ghcr.io/masspe/ai-ontology:<version>`, or `--build` from a
checkout), `docker compose up -d`; the container's log says what was migrated. Rollback:
stop, restore the backup into a fresh volume, restart on the previous image.

## 7. End-to-end check

`scripts/e2e_image.sh` is what the CI runs on every change (job `image`):
image build, start on a fresh volume with the finance example, creation of the
administrator, login, reading the graph through the API and the interface on
the same port, a write, clean stop (`docker stop`), restart on the same
volume, data and account found again, health probe from inside the container,
backup through the API and restore through the command. The same script can be
run by hand against any image:

```sh
docker build -t ontology:local .
scripts/e2e_image.sh ontology:local
```
