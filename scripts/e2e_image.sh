#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
# Copyright (C) 2026 Mediasoft & Cie S.A.
#
# End-to-end check of the container image (ROADMAP §3.8.1): start it on a
# fresh volume seeded with the finance example, create the administrator,
# log in, read the graph through the API and the UI through the same port,
# stop it the way `docker stop` does, start it again on the same volume and
# find the data and the account still there.
#
#   scripts/e2e_image.sh [image]      (default: ontology:e2e)
set -euo pipefail

IMAGE="${1:-ontology:e2e}"
NAME="ontology-e2e-$$"
VOL="ontology-e2e-vol-$$"
PORT="${E2E_PORT:-5077}"
BASE="http://127.0.0.1:${PORT}"
HERE="$(cd "$(dirname "$0")/.." && pwd)"

cleanup() {
  docker rm -f "$NAME" >/dev/null 2>&1 || true
  docker volume rm "$VOL" >/dev/null 2>&1 || true
}
trap cleanup EXIT

start() {
  docker run -d --name "$NAME" -p "${PORT}:5000" \
    -v "$VOL:/data" -v "$HERE/examples/finance:/seed:ro" \
    --memory=2g -e RUST_LOG=info "$IMAGE" \
    --memory-mode strict serve --bind 0.0.0.0:5000 --web /srv/web --login --seed /seed \
    --backup-dir /data/backups >/dev/null
  for _ in $(seq 1 60); do
    if curl -fsS "$BASE/healthz" >/dev/null 2>&1; then return 0; fi
    sleep 1
  done
  echo "server not healthy after 60 s"; docker logs "$NAME"; exit 1
}

json() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }

echo "== start on a fresh volume"
docker volume create "$VOL" >/dev/null
start

echo "== the UI is served on the same port"
curl -fsS "$BASE/" | grep "<div id=\"root\"" >/dev/null || { echo "no UI at /"; exit 1; }
curl -fsS "$BASE/concepts/edit/1" | grep "<div id=\"root\"" >/dev/null || { echo "client-side route not served"; exit 1; }

echo "== the API needs a login"
code=$(curl -s -o /dev/null -w '%{http_code}' "$BASE/stats")
[ "$code" = "401" ] || { echo "expected 401 on /stats, got $code"; exit 1; }

echo "== first account = administrator"
resp=$(curl -fsS -X POST "$BASE/auth/signup" -H 'content-type: application/json' \
  -d '{"email":"admin@example.com","password":"Passw0rd!","name":"Admin"}')
echo "$resp" | json "d['user']['role']" | grep -x admin >/dev/null
TOKEN=$(echo "$resp" | json "d['token']")
code=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE/auth/signup" -H 'content-type: application/json' \
  -d '{"email":"other@example.com","password":"Passw0rd!","name":"Other"}')
[ "$code" = "403" ] || { echo "sign-up should be closed, got $code"; exit 1; }

echo "== the seeded graph is there"
stats=$(curl -fsS -H "authorization: Bearer $TOKEN" "$BASE/stats")
echo "$stats" | json "d['concepts']" | grep -x 22 >/dev/null
echo "$stats" | json "d['relations']" | grep -x 38 >/dev/null
echo "$stats" | json "d['memory']['budget_source']" | grep -x cgroup-v2 >/dev/null || { echo "budget not read from the cgroup: $stats"; exit 1; }
curl -fsS -H "authorization: Bearer $TOKEN" "$BASE/metrics" | grep '^ontology_stream_records{ns="meta"}' >/dev/null

echo "== a write survives a restart"
curl -fsS -X POST -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  "$BASE/concepts" -d '{"id":0,"concept_type":"Company","name":"E2E Corp","description":"created by the e2e","properties":{}}' >/dev/null
docker stop -t 30 "$NAME" >/dev/null
docker logs "$NAME" 2>&1 | grep "server stopped" >/dev/null || { echo "no clean stop in the log"; docker logs "$NAME"; exit 1; }
docker rm "$NAME" >/dev/null
start
resp=$(curl -fsS -X POST "$BASE/auth/login" -H 'content-type: application/json' \
  -d '{"email":"admin@example.com","password":"Passw0rd!"}')
TOKEN=$(echo "$resp" | json "d['token']")
stats=$(curl -fsS -H "authorization: Bearer $TOKEN" "$BASE/stats")
echo "$stats" | json "d['concepts']" | grep -x 23 >/dev/null
docker logs "$NAME" 2>&1 | grep "generated a new JWT secret" >/dev/null && { echo "secret regenerated on restart"; exit 1; }

echo "== healthcheck subcommand inside the container"
docker exec "$NAME" /usr/local/bin/ontology --data /data healthcheck http://127.0.0.1:5000/healthz

echo "== backup through the API, restore with the CLI, same graph"
report=$(curl -fsS -X POST -H "authorization: Bearer $TOKEN" "$BASE/backup")
echo "$report" | json "d['records']" | grep -E '^[1-9][0-9]*$' >/dev/null || { echo "bad backup report: $report"; exit 1; }
docker exec "$NAME" /usr/local/bin/ontology --data /data/restored restore /data/backups | grep "23 concepts, 38 relations" >/dev/null
docker exec "$NAME" /usr/local/bin/ontology --data /data/restored stats | grep -x "concepts: 23" >/dev/null

echo "e2e: ok"
