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
| Dependencies | Dependabot on cargo, both npm packages and the actions; `cargo deny` checks licences, advisories, sources and duplicate versions against `deny.toml`. | `.github/dependabot.yml`, `deny.toml` |
| Release | The image is signed with cosign (keyless, the workflow's OIDC identity) and a CycloneDX SBOM is attached to every release. Verify with `cosign verify --certificate-identity-regexp 'github.com/masspe/ai-ontology.com' --certificate-oidc-issuer https://token.actions.githubusercontent.com ghcr.io/masspe/ai-ontology:<tag>`. | `.github/workflows/release.yml` |
| npm | `ignore-scripts=true`: no dependency runs code at install time. | `web/.npmrc`, `auth-server/.npmrc` |
| Git | Line endings fixed to LF, binaries declared: a whitespace-only rewrite of the tree is impossible, so a real change cannot hide in noise. | `.gitattributes` |
| Git, local | Pre-commit and pre-push hooks run the guard. Enable once per clone: `git config core.hooksPath .githooks`. | `.githooks/` |
| Git, local | Commits signed with the developer's SSH key (`gpg.format ssh`, `commit.gpgsign true`), so an unsigned commit shows as unverified on GitHub. | per-clone git config |
| Review | Code owners on everything; the CI and policy files need both owners. | `.github/CODEOWNERS` |
| Runtime | Security headers on every response (`nosniff`, no framing, strict referrer, `frame-ancestors 'none'`, HSTS behind HTTPS), CORS restricted to the public URL once set, body limits per route, a CSRF header on the cookie routes; built-in login (bcrypt, 15-minute HS256 access tokens, server-side sessions with a rotating refresh cookie — HttpOnly, SameSite=Lax — ended at logout, "sign out everywhere" or account deletion, per-IP rate limit on `/auth/*`), sign-in through OpenID Connect (authorization code with PKCE, `state`/`nonce` in a signed HttpOnly cookie, `id_token` verified RS256 against the provider's JWKS, accounts created only for allowed e-mail domains), security labels on sheets and clearances per account enforced on every read and write path (a hidden sheet is a 404, never listed, counted, traversed, retrieved or cited), named API keys hashed at rest and scoped (clearances, read-only, expiry, rotation), an audit log with a hash chain (`ontology audit-verify`) that also records sign-ins, refusals, access changes and the domains every question touched, a per-domain on-premises policy routing such questions to a local model or refusing them, provider API keys sealed at rest with a tenant master key (AES-256-GCM, `master.key` or `ONTOLOGY_MASTER_KEY`, `ontology rotate-key`), audit log of every write, `--memory-mode strict` refusing a store that does not fit its budget, distroless image running as a non-root user. | `crates/server`, `Dockerfile` |

## What the repository owner must keep enabled on GitHub

These cannot be expressed in files; they are settings of the repository.

- Branch protection or a ruleset on `main`: no force push, no deletion,
  pull request required with one approving review from a code owner,
  required status checks (exact job names) `repo guard`, `rustfmt`, `clippy`, `deny (cargo-deny)`, `test (ubuntu-latest)`, `test (windows-latest)`, `web (vitest, tsc, build)`, `audit (cargo, npm)`, `auth-server (node --test)`,
  signed commits required, linear history.
- Secret scanning with push protection, Dependabot alerts, private
  vulnerability reporting.
- Actions: only actions from GitHub and verified creators, default workflow
  permissions read-only, approval required for workflows from forks.
- Two-factor authentication on every account with write access; personal
  access tokens with an expiry and the narrowest scope.

## Threat model

What is protected, against whom, and what is left to the operator:
[docs/THREAT-MODEL.md](docs/THREAT-MODEL.md).
