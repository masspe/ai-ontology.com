#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
# Copyright (C) 2026 Winven AI Sarl
# Route de Crassier 7, 1262 Eysins, VD, CH
#
# This file is part of ai-ontology.com.
# Dual-licensed: AGPL-3.0-or-later OR a commercial license
# from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.
"""Refuse the file shapes used by the 2026-09-24 supply-chain injection.

Runs in CI on every push and PR (`python3 scripts/repo_guard.py`) over the
tracked files, and refuses:

  fake-binary   a font / image / archive whose bytes are text, or a font
                without its format magic (a script disguised as `.woff2`)
  auto-task     a VS Code task with `runOn: folderOpen`, or a settings
                file enabling `task.allowAutomaticTasks`
  vscode-dir    any tracked file under `.vscode/` (the .gitignore excludes
                it on purpose; the attack un-ignored it)
  npm-hook      a package.json with a lifecycle hook (`preinstall`,
                `postinstall`, `prepare`) - none is needed in this repo
  obfuscated    a source file with javascript-obfuscator identifiers
                (`_0x1a2b`), a line over 2 000 characters, or code hidden
                after a run of 80+ spaces on the same line

Exit 1 with one line per finding. `--self-test` checks every rule fires on
a synthetic bad tree and stays silent on this repository.

See docs/INCIDENT-2026-09-25.md for what these shapes looked like.
"""

from __future__ import annotations

import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile

FONT_MAGIC = {
    ".woff": (b"wOFF",),
    ".woff2": (b"wOF2",),
    ".ttf": (b"\x00\x01\x00\x00", b"true"),
    ".otf": (b"OTTO",),
}
# .eot has its magic at offset 34; keep it in the "must not be text" set only.
BINARY_EXT = set(FONT_MAGIC) | {
    ".eot", ".png", ".jpg", ".jpeg", ".gif", ".ico", ".webp", ".pdf",
    ".zip", ".gz", ".xlsx", ".docx", ".wasm",
}
SOURCE_EXT = {".js", ".mjs", ".cjs", ".jsx", ".ts", ".tsx", ".py", ".rs", ".sh", ".ps1"}

OBF_IDENT = re.compile(rb"_0x[0-9a-f]{4,}")
HIDDEN_CODE = re.compile(rb"\S {80,}\S")
MAX_LINE = 2000
OBF_IDENT_MIN = 10


def is_text(head: bytes) -> bool:
    """True when the first bytes look like text: no NUL, mostly printable."""
    if not head or b"\x00" in head:
        return False
    printable = sum(32 <= b < 127 or b in (9, 10, 13) for b in head)
    return printable / len(head) > 0.95


def check_file(root: pathlib.Path, rel: str) -> list[str]:
    path = root / rel
    ext = path.suffix.lower()
    name = path.name
    top = rel.split("/", 1)[0]
    out: list[str] = []
    try:
        data = path.read_bytes()
    except OSError as e:  # pragma: no cover - CI checkouts are readable
        return [f"{rel}: unreadable ({e})"]

    if top == ".vscode":
        out.append(f"{rel}: vscode-dir - .vscode/ must stay untracked")

    if ext in BINARY_EXT:
        head = data[:512]
        if is_text(head):
            out.append(f"{rel}: fake-binary - {ext} file is text")
        elif ext in FONT_MAGIC and not data.startswith(FONT_MAGIC[ext]):
            out.append(f"{rel}: fake-binary - {ext} without its format magic")

    if name in ("tasks.json", "settings.json") or ext == ".code-workspace":
        text = data.decode("utf-8", "replace")
        if "folderOpen" in text:
            out.append(f"{rel}: auto-task - runOn folderOpen")
        if "allowAutomaticTasks" in text:
            out.append(f"{rel}: auto-task - task.allowAutomaticTasks")

    if name == "package.json":
        try:
            scripts = json.loads(data.decode("utf-8")).get("scripts", {}) or {}
        except (ValueError, AttributeError):
            scripts = {}
        for hook in ("preinstall", "postinstall", "prepare", "preprepare", "postprepare"):
            if hook in scripts:
                out.append(f"{rel}: npm-hook - scripts.{hook}")

    if ext in SOURCE_EXT:
        n = len(OBF_IDENT.findall(data))
        if n >= OBF_IDENT_MIN:
            out.append(f"{rel}: obfuscated - {n} javascript-obfuscator identifiers")
        for i, line in enumerate(data.split(b"\n"), 1):
            if len(line) > MAX_LINE:
                out.append(f"{rel}:{i}: obfuscated - line of {len(line)} characters")
                break
            if HIDDEN_CODE.search(line):
                out.append(f"{rel}:{i}: obfuscated - code hidden after 80+ spaces")
                break
    return out


def tracked_files(root: pathlib.Path) -> list[str]:
    res = subprocess.run(
        ["git", "-C", str(root), "ls-files", "-z"], check=True, capture_output=True
    )
    return [f for f in res.stdout.decode("utf-8").split("\0") if f]


def run(root: pathlib.Path, files: list[str] | None = None) -> list[str]:
    findings: list[str] = []
    for rel in files if files is not None else tracked_files(root):
        if (root / rel).is_file():
            findings.extend(check_file(root, rel))
    return findings


def self_test() -> int:
    """Every rule fires once on a synthetic tree; the real repo is clean."""
    with tempfile.TemporaryDirectory() as d:
        root = pathlib.Path(d)
        bad = {
            "fonts/fa.woff2": b"global['!']='9-9993-2';var _0x5b5d=function(){};",
            "fonts/real.woff2": b"wOF2" + b"\x00" * 60,
            "fonts/odd.ttf": b"NOPE" + b"\x00" * 60,
            ".vscode/tasks.json": b'{"tasks":[{"runOptions":{"runOn":"folderOpen"}}]}',
            ".vscode/settings.json": b'{"task.allowAutomaticTasks": true}',
            "package.json": b'{"scripts":{"postinstall":"node x.js"}}',
            "a.js": b"var " + b",".join(b"_0x%04x=1" % i for i in range(12)) + b";\n",
            "b.js": b"export default r;" + b" " * 149 + b"evil();\n",
            "c.js": b"x=1;" + b"y" * 2100 + b"\n",
            "ok.rs": b"fn main() {\n    println!(\"hi\");\n}\n",
        }
        for rel, content in bad.items():
            p = root / rel
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_bytes(content)
        found = run(root, list(bad))
        expect = [
            ("fonts/fa.woff2", "fake-binary"),
            ("fonts/odd.ttf", "fake-binary"),
            (".vscode/tasks.json", "vscode-dir"),
            (".vscode/tasks.json", "auto-task"),
            (".vscode/settings.json", "auto-task"),
            ("package.json", "npm-hook"),
            ("a.js", "obfuscated"),
            ("b.js", "obfuscated"),
            ("c.js", "obfuscated"),
        ]
        for rel, rule in expect:
            assert any(f.startswith(rel) and rule in f for f in found), (rel, rule, found)
        assert not [f for f in found if f.startswith(("fonts/real.woff2", "ok.rs"))], found
    here = pathlib.Path(__file__).resolve().parent.parent
    clean = run(here)
    assert not clean, clean
    print("repo_guard self-test: ok")
    return 0


def main(argv: list[str]) -> int:
    if "--self-test" in argv:
        return self_test()
    root = pathlib.Path(argv[1]) if len(argv) > 1 else pathlib.Path(os.getcwd())
    findings = run(root)
    for f in findings:
        print(f)
    if findings:
        print(f"repo_guard: {len(findings)} finding(s) - see docs/INCIDENT-2026-09-25.md", file=sys.stderr)
        return 1
    print("repo_guard: clean")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
