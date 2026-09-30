#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
# Copyright (C) 2026 Mediasoft & Cie S.A.
#
# Upgrade drill (ROADMAP §3.8.5): a store written by the PREVIOUS version
# of the binary is opened by the CURRENT one. Hydration must give the same
# graph (stats and export identical), the current version must still write
# to it, compact it (rewriting it in its own format), reopen it, and back
# it up and restore it. The CI job `upgrade` runs this with the binary of
# the previous `main` against the one of the commit under test.
#
#   scripts/upgrade_check.sh <previous ontology binary> <current ontology binary>
set -euo pipefail

OLD=$(realpath "$1")
NEW=$(realpath "$2")
HERE=$(cd "$(dirname "$0")/.." && pwd)
F="$HERE/examples/finance"
D=$(mktemp -d)
trap 'rm -rf "$D"' EXIT
export RUST_LOG=${RUST_LOG:-warn}

echo "== previous version writes two stores (finance example, generated 20k)"
"$OLD" --data "$D/data" ingest --ontology "$F/ontology.json" "$F/seed.jsonl"
"$OLD" --data "$D/data" ingest --text-type Contract "$F/contracts/"
"$OLD" --data "$D/data" ingest --xlsx-type Invoice "$F/invoices.xlsx"
"$OLD" --data "$D/data" ingest --xlsx-type LineItem "$F/line_items.xlsx"
"$OLD" --data "$D/data" ingest "$F/relations.jsonl"
"$OLD" --data "$D/data" stats | tee "$D/old.stats"
"$OLD" --data "$D/data" export "$D/old.jsonl" >/dev/null
"$OLD" --data "$D/big" bench gen --concepts 20000 --relations 40000 --json >/dev/null
"$OLD" --data "$D/big" stats | tee "$D/old-big.stats"

echo "== current version opens them: same graph"
"$NEW" --data "$D/data" stats > "$D/new.stats"
diff "$D/old.stats" "$D/new.stats"
"$NEW" --data "$D/data" export "$D/new.jsonl" >/dev/null
# Property maps have no fixed order: compare canonical JSON, line-sorted.
canon() { python3 -c 'import json,sys; [print(x) for x in sorted(json.dumps(json.loads(l), sort_keys=True) for l in sys.stdin if l.strip())]' < "$1"; }
diff <(canon "$D/old.jsonl") <(canon "$D/new.jsonl") >/dev/null || { echo "export differs after upgrade"; exit 1; }
"$NEW" --data "$D/big" stats > "$D/new-big.stats"
diff "$D/old-big.stats" "$D/new-big.stats"

echo "== current version writes, compacts, reopens"
printf '{"kind":"concept","id":0,"concept_type":"Company","name":"Upgrade Corp","description":"added by the current version"}\n' > "$D/add.jsonl"
"$NEW" --data "$D/data" ingest "$D/add.jsonl" | grep -q "1 concepts"
"$NEW" --data "$D/data" compact >/dev/null
"$NEW" --data "$D/data" stats | grep -qx "concepts: $(( $(sed -n 's/^concepts: //p' "$D/old.stats") + 1 ))"

echo "== current version backs up and restores the upgraded store"
"$NEW" --data "$D/data" backup "$D/bak" >/dev/null
"$NEW" --data "$D/restored" restore "$D/bak" >/dev/null
diff <("$NEW" --data "$D/data" stats) <("$NEW" --data "$D/restored" stats)

echo "upgrade check: ok ($(basename "$OLD") -> $(basename "$NEW"))"
