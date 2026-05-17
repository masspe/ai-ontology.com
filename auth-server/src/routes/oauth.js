// SPDX-License-Identifier: AGPL-3.0-or-later
// OAuth 2.0 Authorization Code flow for Google and Microsoft (Entra ID).
// State (CSRF) + next path are carried in a short-lived signed cookie.

import { Router } from "express";
import crypto from "node:crypto";
import { config } from "../config.js";
import { users } from "../store.js";
import { signJwt } from "../jwt.js";

const router = Router();
const STATE_COOKIE = "oauth_state";
const STATE_TTL_MS = 10 * 60 * 1000;

function b64url(buf) {
  return Buffer.from(buf).toString("base64").replace(/=+$/, "").replace(/\+/g, "-").replace(/\//g, "_");
}
function signState(payload) {
  const body = b64url(JSON.stringify(payload));
  const mac = crypto.createHmac("sha256", config.oauthStateSecret).update(body).digest();
  return `${body}.${b64url(mac)}`;
}
function verifyState(token) {
  if (typeof token !== "string" || !token.includes(".")) return null;
  const [body, sig] = token.split(".");
  const expected = b64url(crypto.createHmac("sha256", config.oauthStateSecret).update(body).digest());
  if (sig !== expected) return null;
  try {
    const decoded = JSON.parse(Buffer.from(body.replace(/-/g, "+").replace(/_/g, "/"), "base64").toString("utf8"));
    if (!decoded.exp || decoded.exp < Date.now()) return null;
    return decoded;
  } catch { return null; }
}
function safeNext(raw) {
  if (typeof raw !== "string" || !raw.startsWith("/") || raw.startsWith("//")) return "/";
  return raw;
}

function setStateCookie(res, token) {
  res.cookie(STATE_COOKIE, token, {
    httpOnly: true,
    sameSite: "lax",
    secure: config.nodeEnv === "production",
    maxAge: STATE_TTL_MS,
    path: "/auth/oauth",
  });
}

function redirectToFrontend(res, params) {
  const url = new URL(config.frontendOAuthCallback);
  // Errors go in the query string so the SPA can route on them and so they
  // appear in logs for debugging. Successful tokens go in the URL fragment
  // so they never reach `Referer` headers or proxy access logs.
  const hashParts = [];
  for (const [k, v] of Object.entries(params)) {
    if (v == null) continue;
    if (k === "error") {
      url.searchParams.set(k, String(v));
    } else {
      hashParts.push(`${encodeURIComponent(k)}=${encodeURIComponent(String(v))}`);
    }
  }
  let target = url.toString();
  if (hashParts.length > 0) target += `#${hashParts.join("&")}`;
  res.redirect(target);
}

// ---------------------------------------------------------------------------
// Provider definitions
// ---------------------------------------------------------------------------
const providers = {
  google: {
    name: "google",
    enabled: () => Boolean(config.google.clientId && config.google.clientSecret),
    authUrl: "https://accounts.google.com/o/oauth2/v2/auth",
    tokenUrl: "https://oauth2.googleapis.com/token",
    userInfoUrl: "https://openidconnect.googleapis.com/v1/userinfo",
    scope: "openid email profile",
    // Google-specific: request a refresh token on first consent.
    extraAuthParams: { access_type: "offline", prompt: "select_account" },
    clientId: () => config.google.clientId,
    clientSecret: () => config.google.clientSecret,
    redirectUri: () => config.google.redirectUri,
    extractProfile: (info) => ({
      sub: info.sub,
      email: info.email,
      emailVerified: info.email_verified === true || info.email_verified === "true",
      name: info.name || info.given_name || null,
      picture: info.picture || null,
    }),
  },
  microsoft: {
    name: "microsoft",
    enabled: () => Boolean(config.microsoft.clientId && config.microsoft.clientSecret),
    authUrl: () => `https://login.microsoftonline.com/${config.microsoft.tenant}/oauth2/v2.0/authorize`,
    tokenUrl: () => `https://login.microsoftonline.com/${config.microsoft.tenant}/oauth2/v2.0/token`,
    userInfoUrl: "https://graph.microsoft.com/oidc/userinfo",
    // Refresh tokens come from the `offline_access` scope on the v2.0 endpoint;
    // `access_type=offline` is Google-only and must NOT be sent here.
    scope: "openid email profile offline_access",
    extraAuthParams: { prompt: "select_account", response_mode: "query" },
    clientId: () => config.microsoft.clientId,
    clientSecret: () => config.microsoft.clientSecret,
    redirectUri: () => config.microsoft.redirectUri,
    extractProfile: (info) => ({
      sub: info.sub,
      email: info.email || info.preferred_username,
      emailVerified: true, // Entra-managed accounts
      name: info.name || null,
      picture: info.picture || null,
    }),
  },
};

// ---------------------------------------------------------------------------
// Start flow
// ---------------------------------------------------------------------------
function startHandler(providerKey) {
  return (req, res) => {
    const p = providers[providerKey];
    if (!p.enabled()) {
      // Bounce back to the SPA with a structured error so the UI can show a
      // friendly message instead of leaving the user staring at raw JSON.
      console.warn(`[oauth/${providerKey}] start blocked: provider not configured`);
      return redirectToFrontend(res, { error: `${providerKey}_not_configured` });
    }

    const next = safeNext(req.query.next);
    const nonce = crypto.randomBytes(16).toString("hex");
    const stateToken = signState({ n: nonce, next, exp: Date.now() + STATE_TTL_MS });
    setStateCookie(res, stateToken);

    const authUrl = typeof p.authUrl === "function" ? p.authUrl() : p.authUrl;
    const url = new URL(authUrl);
    url.searchParams.set("client_id", p.clientId());
    url.searchParams.set("redirect_uri", p.redirectUri());
    url.searchParams.set("response_type", "code");
    url.searchParams.set("scope", p.scope);
    url.searchParams.set("state", nonce);
    for (const [k, v] of Object.entries(p.extraAuthParams || {})) {
      url.searchParams.set(k, v);
    }
    res.redirect(url.toString());
  };
}

// ---------------------------------------------------------------------------
// Callback
// ---------------------------------------------------------------------------
function callbackHandler(providerKey) {
  return async (req, res) => {
    const p = providers[providerKey];
    try {
      if (!p.enabled()) throw new Error("provider_not_configured");

      const code = req.query.code;
      const state = req.query.state;
      const stateCookie = req.cookies?.[STATE_COOKIE];
      res.clearCookie(STATE_COOKIE, { path: "/auth/oauth" });

      const decoded = verifyState(stateCookie);
      if (!decoded || decoded.n !== state) throw new Error("invalid_state");
      if (!code || typeof code !== "string") throw new Error("missing_code");

      // 1) Exchange code -> access token
      const tokenUrl = typeof p.tokenUrl === "function" ? p.tokenUrl() : p.tokenUrl;
      const form = new URLSearchParams({
        client_id: p.clientId(),
        client_secret: p.clientSecret(),
        code,
        grant_type: "authorization_code",
        redirect_uri: p.redirectUri(),
      });
      const tokRes = await fetch(tokenUrl, {
        method: "POST",
        headers: { "content-type": "application/x-www-form-urlencoded", accept: "application/json" },
        body: form,
      });
      if (!tokRes.ok) {
        const txt = await tokRes.text();
        console.error(`[oauth/${providerKey}] token exchange failed:`, tokRes.status, txt);
        throw new Error("token_exchange_failed");
      }
      const tok = await tokRes.json();
      if (!tok.access_token) throw new Error("no_access_token");

      // 2) Fetch userinfo
      const uiRes = await fetch(p.userInfoUrl, {
        headers: { authorization: `Bearer ${tok.access_token}`, accept: "application/json" },
      });
      if (!uiRes.ok) throw new Error("userinfo_failed");
      const info = await uiRes.json();
      const profile = p.extractProfile(info);
      if (!profile.sub || !profile.email) throw new Error("missing_profile");

      // 3) Find-or-attach-or-create user
      let user = await users.findByProvider(p.name, profile.sub);
      if (!user) {
        const existing = await users.findByEmail(profile.email);
        if (existing) {
          // Only auto-link if email is verified by the provider.
          if (!profile.emailVerified) throw new Error("email_not_verified");
          user = await users.linkProvider(existing.id, p.name, profile.sub);
        } else {
          user = await users.create({
            email: profile.email,
            name: profile.name,
            picture: profile.picture,
            provider: p.name,
            sub: profile.sub,
          });
        }
      }

      // 4) Issue JWT and bounce back to the SPA
      const token = signJwt(user);
      return redirectToFrontend(res, { token, next: safeNext(decoded.next) });
    } catch (e) {
      console.error(`[oauth/${providerKey}] error:`, e.message);
      return redirectToFrontend(res, { error: e.message || "oauth_error" });
    }
  };
}

router.get("/google/start", startHandler("google"));
router.get("/google/callback", callbackHandler("google"));
router.get("/microsoft/start", startHandler("microsoft"));
router.get("/microsoft/callback", callbackHandler("microsoft"));

export default router;
