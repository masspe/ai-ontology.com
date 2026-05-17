// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Centralized API client. Exposes msBE.auth.* and handles 401 -> logout/redirect.

// Default to same-origin so the Vite dev server (and any reverse proxy in
// production) can forward /auth/* to the auth backend. Override with
// VITE_AUTH_API_BASE only when calling the auth server directly.
const API_BASE = (import.meta.env.VITE_AUTH_API_BASE || "").replace(/\/$/, "");
const TOKEN_KEY = "msBE.token";
const USER_KEY = "msBE.user";

// ---- token storage ---------------------------------------------------------
export const tokenStore = {
  get: () => {
    try { return localStorage.getItem(TOKEN_KEY); } catch { return null; }
  },
  set: (t) => { try { localStorage.setItem(TOKEN_KEY, t); } catch { /* ignore */ } },
  clear: () => {
    try { localStorage.removeItem(TOKEN_KEY); localStorage.removeItem(USER_KEY); } catch { /* ignore */ }
  },
};

export const userStore = {
  get: () => {
    try { const raw = localStorage.getItem(USER_KEY); return raw ? JSON.parse(raw) : null; }
    catch { return null; }
  },
  set: (u) => { try { localStorage.setItem(USER_KEY, JSON.stringify(u)); } catch { /* ignore */ } },
};

// ---- 401 handler -----------------------------------------------------------
let onUnauthorized = () => {
  tokenStore.clear();
  if (typeof window !== "undefined" && window.location.pathname !== "/login") {
    const next = encodeURIComponent(window.location.pathname + window.location.search);
    window.location.assign(`/login?next=${next}`);
  }
};
export function setUnauthorizedHandler(fn) { onUnauthorized = fn; }

// ---- low-level fetch -------------------------------------------------------
async function request(method, path, body) {
  const headers = { "accept": "application/json" };
  if (body !== undefined) headers["content-type"] = "application/json";
  const token = tokenStore.get();
  if (token) headers["authorization"] = `Bearer ${token}`;

  let res;
  try {
    res = await fetch(`${API_BASE}${path}`, {
      method,
      headers,
      credentials: "include",
      body: body !== undefined ? JSON.stringify(body) : undefined,
    });
  } catch (e) {
    const err = new Error("Network error");
    err.cause = e;
    err.status = 0;
    throw err;
  }

  if (res.status === 401) {
    onUnauthorized();
    const err = new Error("Unauthorized");
    err.status = 401;
    throw err;
  }

  const text = await res.text();
  const data = text ? safeJson(text) : null;

  if (!res.ok) {
    const err = new Error((data && data.error) || `${res.status} ${res.statusText}`);
    err.status = res.status;
    err.body = data;
    throw err;
  }
  return data;
}

function safeJson(t) { try { return JSON.parse(t); } catch { return null; } }

// ---- public API ------------------------------------------------------------
const auth = {
  async signup({ email, password, name }) {
    const data = await request("POST", "/auth/signup", { email, password, name });
    if (data?.token) { tokenStore.set(data.token); userStore.set(data.user); }
    return data;
  },
  async login({ email, password }) {
    const data = await request("POST", "/auth/login", { email, password });
    if (data?.token) { tokenStore.set(data.token); userStore.set(data.user); }
    return data;
  },
  async me() {
    const data = await request("GET", "/auth/me");
    if (data?.user) userStore.set(data.user);
    return data?.user ?? null;
  },
  async logout() {
    try { await request("POST", "/auth/logout"); } catch { /* ignore */ }
    tokenStore.clear();
  },
  isAuthenticated: () => Boolean(tokenStore.get()),
  currentUser: () => userStore.get(),

  // OAuth: redirect to backend which initiates the provider flow.
  googleLoginUrl: (next = "/") =>
    `${API_BASE}/auth/oauth/google/start?next=${encodeURIComponent(next)}`,
  microsoftLoginUrl: (next = "/") =>
    `${API_BASE}/auth/oauth/microsoft/start?next=${encodeURIComponent(next)}`,

  // Used by the OAuth callback page to consume `?token=...`.
  consumeOAuthToken(token) {
    if (!token) return false;
    tokenStore.set(token);
    return true;
  },
};

export const msBE = { auth, request, API_BASE };
export default msBE;
