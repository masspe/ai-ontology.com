# auth-server — Login / Signup / OAuth (Google + Microsoft)

Express auth backend for the AI-Ontology web app. Issues JWTs consumed by the
SPA via the centralized `msBE.auth` client.

## Run

```powershell
cd auth-server
npm install
Copy-Item .env.example .env
# Edit .env: set JWT_SECRET, OAUTH_STATE_SECRET, and provider client IDs/secrets.
npm run dev
```

Server listens on `http://localhost:4000`.

## Test

```powershell
npm test
```

## Endpoints

| Method | Path                                  | Auth   | Notes                                  |
| ------ | ------------------------------------- | ------ | -------------------------------------- |
| POST   | `/auth/signup`                        | —      | `{ email, password, name }` → JWT      |
| POST   | `/auth/login`                         | —      | `{ email, password }` → JWT            |
| GET    | `/auth/me`                            | Bearer | Returns the current user               |
| POST   | `/auth/logout`                        | —      | Stateless ack (client clears storage)  |
| GET    | `/auth/oauth/google/start?next=/`     | —      | Begin Google OAuth flow                |
| GET    | `/auth/oauth/google/callback`         | —      | Google redirect target                 |
| GET    | `/auth/oauth/microsoft/start?next=/`  | —      | Begin Microsoft OAuth flow             |
| GET    | `/auth/oauth/microsoft/callback`      | —      | Microsoft redirect target              |

## OAuth provider configuration

### Google Cloud Console
1. APIs & Services → Credentials → Create OAuth client ID → Web application.
2. Authorized JavaScript origin: `http://localhost:5173`
3. Authorized redirect URI: `http://localhost:4000/auth/oauth/google/callback`
4. Copy Client ID → `GOOGLE_CLIENT_ID`, Secret → `GOOGLE_CLIENT_SECRET`.

### Azure portal (Entra ID)
1. App registrations → New registration.
2. Redirect URI (Web): `http://localhost:4000/auth/oauth/microsoft/callback`
3. Supported account types: choose to fit your tenancy (use `common` for both
   work and personal accounts in `MICROSOFT_TENANT`).
4. Certificates & secrets → New client secret → copy to `MICROSOFT_CLIENT_SECRET`.
5. Copy Application (client) ID → `MICROSOFT_CLIENT_ID`.

## User linking strategy

- New OAuth identity, unknown email → create user, attach provider identity.
- New OAuth identity, known email **and** `email_verified === true` → attach
  provider to the existing account (auto-link).
- New OAuth identity, known email but **not** verified → refuse (`email_not_verified`)
  to avoid account takeover.

## Production checklist

- Replace the JSON store (`src/store.js`) with a real database.
- Set `NODE_ENV=production`, strong `JWT_SECRET` / `OAUTH_STATE_SECRET`.
- Serve only over HTTPS; the OAuth `state` cookie becomes `Secure` automatically.
- Tighten `WEB_ORIGIN` to the actual frontend origin(s).
- Consider a JWT denylist or refresh tokens for revocation.
