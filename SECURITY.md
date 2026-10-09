# Security policy

## Reporting a vulnerability

Use GitHub's private vulnerability reporting on this repository
("Security" tab, "Report a vulnerability"). Do not open a public issue.
You will get an acknowledgement within 5 working days.

## What the repository enforces

| Layer | Measure | Where |
|---|---|---|
| CI, first job | `scripts/repo_guard.py` refuses file shapes that hide executable code in a repository: fake binaries (text where a font or image is expected), VS Code `folderOpen` tasks, any tracked `.vscode/` file, npm lifecycle hooks, obfuscated sources, and internal documents that do not belong in the public tree. Every other job `needs` it. | `.github/workflows/ci.yml` |
| CI | GitHub Actions pinned to commit SHAs; workflow token read-only by default; `cargo audit` and `npm audit --audit-level=high`; `--locked` on every cargo command; the optional auth-server has its own tests. | `.github/workflows/ci.yml` |
| Dependencies | Dependabot on cargo, both npm packages and the actions. | `.github/dependabot.yml` |
| npm | `ignore-scripts=true`: no dependency runs code at install time. | `web/.npmrc`, `auth-server/.npmrc` |
| Git | Line endings fixed to LF, binaries declared: a whitespace-only rewrite of the tree is impossible, so a real change cannot hide in noise. | `.gitattributes` |
| Git, local | Pre-commit and pre-push hooks run the guard. Enable once per clone: `git config core.hooksPath .githooks`. | `.githooks/` |
| Git, local | Commits signed with the developer's SSH key (`gpg.format ssh`, `commit.gpgsign true`), so an unsigned commit shows as unverified on GitHub. | per-clone git config |
| Review | Code owners on everything; the CI and policy files need both owners. | `.github/CODEOWNERS` |
| Runtime | Built-in login (bcrypt, HS256 sessions, per-IP rate limit on `/auth/*`), named API keys hashed at rest, audit log of every write, `--memory-mode strict` refusing a store that does not fit its budget, distroless image running as a non-root user. | `crates/server`, `Dockerfile` |

## What the repository owner must keep enabled on GitHub

These cannot be expressed in files; they are settings of the repository.

- Branch protection or a ruleset on `main`: no force push, no deletion,
  pull request required with one approving review from a code owner,
  required status checks (exact job names) `repo guard`, `rustfmt`, `clippy`, `test (ubuntu-latest)`, `test (windows-latest)`, `web (vitest, tsc, build)`, `audit (cargo, npm)`, `auth-server (node --test)`,
  signed commits required, linear history.
- Secret scanning with push protection, Dependabot alerts, private
  vulnerability reporting.
- Actions: only actions from GitHub and verified creators, default workflow
  permissions read-only, approval required for workflows from forks.
- Two-factor authentication on every account with write access; personal
  access tokens with an expiry and the narrowest scope.
