#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
# Copyright (C) 2026 Mediasoft & Cie S.A.
#
# Runs a command; when it fails, repeats the lines that say why as a GitHub
# annotation, so the reason is on the run's summary page and not only in a
# log that needs a login to read.
#
#   scripts/ci-annotate.sh cargo test ...
set -uo pipefail

out="$(mktemp)"
"$@" 2>&1 | tee "$out"
code=${PIPESTATUS[0]}
if [ "$code" -ne 0 ]; then
  why="$(grep -E -A8 'panicked at|^error' "$out" | head -60)"
  # An annotation is one line: the workflow command's escapes for the rest.
  why="${why//'%'/%25}"
  why="${why//$'\r'/%0D}"
  why="${why//$'\n'/%0A}"
  echo "::error::${why:-exit code $code}"
fi
rm -f "$out"
exit "$code"
