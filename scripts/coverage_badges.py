#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
# Copyright (C) 2026 Winven AI Sarl
# Route de Crassier 7, 1262 Eysins, VD, CH
#
# This file is part of ai-ontology.com.
# Dual-licensed: AGPL-3.0-or-later OR a commercial license
# from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.
"""Turn coverage reports into shields.io endpoint badges and a Markdown table.

Inputs (both optional, a missing one is simply skipped):
  --rust  <cargo llvm-cov --json output>          line coverage per crate (src/ only)
  --web   <vitest coverage/coverage-summary.json>  line coverage of web/src

Outputs, in --out (default `badges/`):
  rust.json, web.json, crate-<name>.json   shields.io endpoint documents
  COVERAGE.md                              per-crate table for the README
  summary.json                             the raw figures

The README embeds the badges through
`https://img.shields.io/endpoint?url=<raw url of the json>`; the CI job
publishes `badges/` on the `coverage-badges` branch after every push to
`main`, so the numbers on the README are the last measurement on Linux.

Line coverage is the figure shown everywhere: it is the one both tools
agree on (llvm-cov "lines", V8 "lines") and the easiest to reason about.
"""

from __future__ import annotations

import argparse
import collections
import json
import pathlib
import sys
from datetime import date


def color(pct: float) -> str:
    if pct >= 90:
        return "brightgreen"
    if pct >= 80:
        return "green"
    if pct >= 70:
        return "yellowgreen"
    if pct >= 50:
        return "yellow"
    if pct >= 30:
        return "orange"
    return "red"


def badge(label: str, pct: float) -> dict:
    return {
        "schemaVersion": 1,
        "label": label,
        "message": f"{pct:.1f}%",
        "color": color(pct),
    }


def rust_by_crate(report: dict) -> dict[str, tuple[int, int]]:
    """{crate: (covered_lines, total_lines)} over `crates/<crate>/src/**`."""
    agg: dict[str, list[int]] = collections.defaultdict(lambda: [0, 0])
    for f in report["data"][0]["files"]:
        name = f["filename"].replace("\\", "/")
        if "/crates/" not in name:
            continue
        rel = name.split("/crates/", 1)[1]
        parts = rel.split("/")
        if len(parts) < 3 or parts[1] != "src":
            continue  # integration tests, benches, build scripts
        s = f["summary"]["lines"]
        agg[parts[0]][0] += s["covered"]
        agg[parts[0]][1] += s["count"]
    return {k: (v[0], v[1]) for k, v in sorted(agg.items())}


def pct(covered: int, total: int) -> float:
    return 100.0 * covered / total if total else 0.0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--rust", type=pathlib.Path)
    ap.add_argument("--web", type=pathlib.Path)
    ap.add_argument("--out", type=pathlib.Path, default=pathlib.Path("badges"))
    ap.add_argument(
        "--fail-under",
        type=float,
        default=None,
        help="exit 1 when any Rust crate or the web is below this line percentage",
    )
    args = ap.parse_args()
    if not args.rust and not args.web:
        ap.error("give at least one of --rust / --web")

    args.out.mkdir(parents=True, exist_ok=True)
    summary: dict = {"date": date.today().isoformat(), "crates": {}}
    lines: list[str] = []

    if args.rust:
        report = json.loads(args.rust.read_text(encoding="utf-8"))
        crates = rust_by_crate(report)
        cov = sum(c for c, _ in crates.values())
        tot = sum(t for _, t in crates.values())
        rust_pct = pct(cov, tot)
        summary["rust"] = {"covered": cov, "total": tot, "pct": round(rust_pct, 1)}
        (args.out / "rust.json").write_text(
            json.dumps(badge("coverage · rust", rust_pct)) + "\n", encoding="utf-8"
        )
        lines.append("| Crate | Line coverage | Lines |")
        lines.append("|---|---|---|")
        for crate, (c, t) in crates.items():
            p = pct(c, t)
            summary["crates"][crate] = {"covered": c, "total": t, "pct": round(p, 1)}
            (args.out / f"crate-{crate}.json").write_text(
                json.dumps(badge(crate, p)) + "\n", encoding="utf-8"
            )
            lines.append(f"| `{crate}` | {p:.1f} % | {c} / {t} |")
        lines.append(f"| **Rust total** | **{rust_pct:.1f} %** | {cov} / {tot} |")

    if args.web:
        web = json.loads(args.web.read_text(encoding="utf-8"))["total"]["lines"]
        web_pct = pct(web["covered"], web["total"])
        summary["web"] = {"covered": web["covered"], "total": web["total"], "pct": round(web_pct, 1)}
        (args.out / "web.json").write_text(
            json.dumps(badge("coverage · web", web_pct)) + "\n", encoding="utf-8"
        )
        if not lines:
            lines.append("| Component | Line coverage | Lines |")
            lines.append("|---|---|---|")
        lines.append(f"| **Web (`web/src`)** | **{web_pct:.1f} %** | {web['covered']} / {web['total']} |")

    (args.out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    table = "\n".join(lines) + "\n"
    (args.out / "COVERAGE.md").write_text(
        f"Line coverage measured on {summary['date']} (Linux, full test suite).\n\n{table}",
        encoding="utf-8",
    )
    sys.stdout.write(table)
    if args.fail_under is not None:
        low = [(k, v["pct"]) for k, v in summary["crates"].items() if v["pct"] < args.fail_under]
        if "web" in summary and summary["web"]["pct"] < args.fail_under:
            low.append(("web", summary["web"]["pct"]))
        if low:
            names = ", ".join(f"{k} {p:.1f} %" for k, p in low)
            sys.stderr.write(f"coverage below {args.fail_under:.0f} %: {names}\n")
            return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
