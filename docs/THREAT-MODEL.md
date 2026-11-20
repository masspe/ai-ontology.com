# Threat model

What the product protects, against whom, how, and what is left to the
operator. One process per client (DEPLOY.md §2): everything below is per
tenant.

## 1. Assets

| Asset | Where | Why it matters |
|---|---|---|
| The graph: sheets, links, documents attached to them | `/data/store` | The client's software model and business knowledge; some sheets carry security labels (payroll, a site, a function) |
| The questions asked and the answers given | in flight; the audit log keeps the domains touched (the text only with `--audit-questions`) | They reveal what people work on |
| Provider credentials (LLM, embeddings), OpenID client secrets | `/data/settings.json`, sealed with `/data/master.key` | A leaked key is money and data exfiltration through a provider |
| Accounts, password hashes, sessions, API keys | `/data/users.json` (hashes and fingerprints only) | Identity of every reader and writer |
| The audit log | `/data/audit.jsonl`, hash-chained | Proof of who did what |

## 2. Trust boundaries

```
 browser ──HTTPS (reverse proxy)──► ontology serve ──► store on the volume
    │                                   │
    │ OpenID Connect                    │ provider API (HTTPS), or a model on the premises
    ▼                                   ▼
 identity provider               language model
```

- **The browser is untrusted.** Every request carries a short-lived access
  token (15 min) or an API key; the refresh secret is an HttpOnly cookie
  scoped to `/auth` with `SameSite=Lax`, and refreshing or logging out
  needs the `X-Requested-With` header a cross-site form cannot send. The
  UI's answers never carry a sheet the caller may not see.
- **The identity provider is trusted for identity only.** Its `id_token`
  is verified (signature against its published keys, issuer, audience,
  expiry, nonce); accounts are created only for allowed e-mail domains;
  group claims may grant the administrator role or become clearances, as
  the administrator configured.
- **The language model is untrusted with secrets and trusted with what it
  is sent.** It receives only sheets the caller may see, rendered as data
  in delimited blocks, never as instructions; a domain declared
  on-premises is only ever sent to the on-premises provider, or the
  question is refused. Provider keys never leave the process except to
  the provider.
- **The data directory is trusted as the operator's.** Whoever reads the
  volume reads the graph; the settings' secrets are sealed with a key
  kept beside them (or in the environment), so a copy of the file alone
  reveals nothing.
- **Another tenant is a stranger.** One process, one volume, one set of
  accounts per client; nothing is shared.

## 3. Threats and the measure that answers each

| Threat | Measure | Where |
|---|---|---|
| A reader sees a sheet beyond their clearance (direct, through a listing, a link, a path, the graph page, a retrieval, an answer, an export) | Labels on sheets, clearances per account, enforced on every read path; a hidden sheet is a 404; export for administrators | `crates/server/src/lib.rs` (`visibility_for`, `ensure_visible`), tests `access.rs` |
| A writer labels a sheet beyond their clearance, or edits a hidden one | 403 / 404 on the write paths | same |
| A stolen access token | 15-minute life; refused at once after logout, "sign out everywhere" or account deletion | `auth.rs` sessions |
| A stolen refresh cookie | Rotates on every use, bound to `/auth`, HttpOnly, `SameSite=Lax`; the custom header defeats cross-site use | `auth.rs`, CSRF check |
| A stolen API key | Scoped (clearances, read-only, expiry), rotatable, revocable at once, last use logged | `auth.rs` keys |
| Credential guessing | Per-IP rate limit on `/auth/*`; bcrypt; refusals audited | `rate_limit_layer`, audit |
| A forged or replayed OpenID response | `state` and `nonce` in a signed cookie, PKCE, issuer/audience/expiry/nonce checks, keys fetched from the discovery document | `oidc.rs`, tests `oidc.rs` |
| Prompt injection through imported content | Retrieved content rendered as data in delimited blocks; the model has no tools | `crates/rag/src/prompt.rs` |
| Data leaving the premises through a provider | Per-domain on-premises policy: such questions go to the local provider or are refused | `SettingsRoutedModel` |
| A copy of the data directory | Provider keys sealed (AES-256-GCM, master key); password hashes; key fingerprints | `secrets.rs` |
| Tampering with the audit log | Hash chain; `ontology audit-verify` | `audit.rs` |
| Clickjacking, MIME sniffing, referrer leaks | `X-Frame-Options`, `Content-Security-Policy: frame-ancestors 'none'; base-uri 'self'; object-src 'none'`, `nosniff`, `Referrer-Policy`, HSTS when the public URL is HTTPS | `security_headers_layer` |
| Oversized requests | Body limits: 8 MB by default, more only on upload and ingest routes | `build_router` |
| A compromised dependency | `cargo audit` and `cargo deny` (advisories, licences, sources) in CI; `npm audit`; lockfiles; pinned actions; `ignore-scripts` | `.github/workflows/ci.yml`, `deny.toml` |
| A tampered release | Images signed with cosign (keyless, GitHub OIDC); SBOM (CycloneDX) attached to every release | `.github/workflows/release.yml` |
| A corrupt store after a crash | Checksummed segments, recovery truncates the torn tail only, write-ahead order | STORAGE.md |

## 4. Out of scope, and what the operator must do

- **TLS and the public name** are the reverse proxy's (Caddy, Traefik):
  terminate HTTPS there, forward `X-Forwarded-Proto`, set the public URL in
  Settings → Sign-in so cookies are `Secure` and HSTS is sent.
- **Network isolation of the on-premises model**: the product routes to
  it; the operator makes sure that endpoint does not forward elsewhere.
- **Backups of `/data`** (store, `users.json`, `settings.json`,
  `master.key`, `audit.jsonl`): the product backs the store up on request;
  the operator keeps the copies somewhere safe, and the master key with them.
- **Imports write labels unchecked**: an import is an administrator's act
  (keep the `import` routes to administrators and scoped keys without
  write).
- **Denial of service** beyond the rate limit on sign-in and the body
  limits: a reverse proxy's rate limiting and the container's memory limit
  (`--memory-mode strict`).
- **Physical and host security**, the identity provider's own security,
  and the language-model provider's handling of what it is sent are the
  operator's and the providers'.
