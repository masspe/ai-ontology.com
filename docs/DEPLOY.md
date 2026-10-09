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
  `/data/users.json` and the token secret in `/data/jwt.secret`.

One process per client (STORAGE.md §10.8): each client has its own container,
its own data volume, its own accounts, its own memory limit.

## 2. Starting a client

```sh
docker compose up -d --build          # two example clients: acme, globex
open http://localhost:5001 in a browser (acme; globex is on 5002)
```

`compose.yaml` shows the model: one service per client, one volume per
service, `mem_limit`, a health probe (`ontology healthcheck`), a clean stop
(`stop_grace_period`). Login attempts are rate-limited per IP address: behind
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

Google and Microsoft sign-in (OAuth) are not served by the binary: the buttons
send the user back to the login page with `error=…_not_configured`. The Node
`auth-server/` is optional and only needed for Google/Microsoft OAuth.

### API keys and audit log

For a machine caller (an ERP, a script), the administrator creates a **named
key** rather than lending their own account: `POST /auth/keys {"name":"ERP"}`
returns the secret once (`ok_…`); `GET /auth/keys` lists the keys (name,
prefix, creation, by whom), `DELETE /auth/keys/<id>` revokes immediately. The
key is presented like a token (`Authorization: Bearer ok_…`); it opens the
API, but not account management, reset or backup, which need a signed-in
user. The `users.json` file keeps only a fingerprint.

Every successful write (read-only POSTs, such as search, question and
analysis, excluded) is recorded in `/data/audit.jsonl` (one JSON line: time,
caller, method, path, status, request identifier). `GET /audit?limit=100`
reads back the last lines, administrators only. The file is part of the
`/data` volume; it is not in the store backup (§5), so copy it along with the
volume. Options: `--audit-log <path>`, `--no-audit`.

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
docker run --rm -v acme-data:/data -v acme-backups:/backups --entrypoint sh ontology:local \
  -c 'mv /data/store /data/store.old && /usr/local/bin/ontology --data /data restore /backups'
docker start acme
```

The restored store has its own identity: its next backup goes to an empty
folder (or to an emptied `/backups`), never on top of the original backup.
The small files next to the store (`users.json`, `jwt.secret`, `settings.json`)
are not part of the backup: copy them along with the volume, or supply the
secret through `--jwt-secret-env`. The CI runs backup through the API, restore
through the command and a graph comparison on every change
(`scripts/e2e_image.sh`), and the store's tests check the round trip, the
incremental behavior and the refusals (`crates/storage/tests/backup.rs`).

## 6. Upgrading

A new image opens the previous one's store as is: the format is versioned and
migrated automatically on open. The CI checks this on every change (job
`upgrade`, `scripts/upgrade_check.sh`: the previous version of `main` writes a
store, the current version opens it, writes to it, compacts, backs up and
restores). Procedure: back up (§5), `docker compose pull` or `build`,
`docker compose up -d`; the container's log says what was migrated. Rollback:
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
