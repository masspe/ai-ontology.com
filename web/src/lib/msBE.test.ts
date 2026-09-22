// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Unit tests of the auth-server client: token / user persistence, the
// low-level `request()` (URL, method, headers, body, credentials, error
// mapping), the 401 handler (default redirect and override) and every
// `msBE.auth.*` wrapper against a stubbed `fetch`.

import { beforeEach, describe, expect, it, vi } from "vitest";

interface AuthUser {
  id?: number;
  email?: string;
  name?: string;
}
interface AuthApi {
  signup(args: { email: string; password: string; name: string }): Promise<unknown>;
  login(args: { email: string; password: string }): Promise<unknown>;
  me(): Promise<AuthUser | null>;
  logout(): Promise<void>;
  isAuthenticated(): boolean;
  currentUser(): AuthUser | null;
  googleLoginUrl(next?: string): string;
  microsoftLoginUrl(next?: string): string;
  consumeOAuthToken(token: string | null | undefined): boolean;
}
interface MsBEModule {
  tokenStore: { get(): string | null; set(t: string): void; clear(): void };
  userStore: { get(): AuthUser | null; set(u: AuthUser): void };
  setUnauthorizedHandler(fn: () => void): void;
  msBE: { auth: AuthApi; request(method: string, path: string, body?: unknown): Promise<unknown>; API_BASE: string };
  default: MsBEModule["msBE"];
}
interface RequestError extends Error {
  status?: number;
  body?: unknown;
  cause?: unknown;
}

// `API_BASE` is read from `import.meta.env` at module load and the 401
// handler is module state: every test gets a fresh copy of the module.
async function loadMsBE(): Promise<MsBEModule> {
  vi.resetModules();
  // @ts-expect-error JS module
  return (await import("./msBE")) as MsBEModule;
}

const ls = () => window.localStorage;

function jsonResponse(body: unknown, status = 200, statusText = "OK"): Response {
  return new Response(JSON.stringify(body), {
    status,
    statusText,
    headers: { "content-type": "application/json" },
  });
}

function stubFetch(res: Response | (() => Promise<Response>)) {
  const impl = typeof res === "function" ? res : () => Promise.resolve(res);
  const fn = vi.fn<typeof fetch>(() => impl());
  vi.stubGlobal("fetch", fn);
  return fn;
}

interface CapturedCall {
  url: string;
  method: string | undefined;
  headers: Record<string, string>;
  body: unknown;
  init: RequestInit;
}

function call(fetchMock: ReturnType<typeof stubFetch>, i = 0): CapturedCall {
  const [url, init] = fetchMock.mock.calls[i] as [string, RequestInit];
  const raw = init.body;
  return {
    url,
    method: init.method,
    headers: (init.headers ?? {}) as Record<string, string>,
    body: typeof raw === "string" ? JSON.parse(raw) : undefined,
    init,
  };
}

let mod: MsBEModule;
beforeEach(async () => {
  mod = await loadMsBE();
  // Never let the default 401 handler touch window.location unless a test
  // installs its own location stub.
  mod.setUnauthorizedHandler(() => {});
});

describe("tokenStore", () => {
  it("reads, writes and clears the token together with the cached user", () => {
    expect(mod.tokenStore.get()).toBeNull();
    mod.tokenStore.set("jwt-1");
    expect(ls().getItem("msBE.token")).toBe("jwt-1");
    expect(mod.tokenStore.get()).toBe("jwt-1");
    ls().setItem("msBE.user", JSON.stringify({ id: 1 }));
    mod.tokenStore.clear();
    expect(ls().getItem("msBE.token")).toBeNull();
    expect(ls().getItem("msBE.user")).toBeNull();
  });

  it("swallows storage failures: get() returns null, set()/clear() do not throw", () => {
    const boom = () => {
      throw new Error("quota");
    };
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(boom);
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(boom);
    vi.spyOn(Storage.prototype, "removeItem").mockImplementation(boom);
    expect(mod.tokenStore.get()).toBeNull();
    expect(() => mod.tokenStore.set("x")).not.toThrow();
    expect(() => mod.tokenStore.clear()).not.toThrow();
  });
});

describe("userStore", () => {
  it("returns null when nothing is stored", () => {
    expect(mod.userStore.get()).toBeNull();
  });

  it("round-trips a user through JSON", () => {
    mod.userStore.set({ id: 7, email: "a@b.co" });
    expect(ls().getItem("msBE.user")).toBe('{"id":7,"email":"a@b.co"}');
    expect(mod.userStore.get()).toEqual({ id: 7, email: "a@b.co" });
  });

  it("returns null for corrupt JSON and ignores write failures", () => {
    ls().setItem("msBE.user", "{not json");
    expect(mod.userStore.get()).toBeNull();
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("quota");
    });
    expect(() => mod.userStore.set({ id: 1 })).not.toThrow();
  });
});

describe("request()", () => {
  it("GET: same-origin URL by default, accept header only, credentials included, no body", async () => {
    const f = stubFetch(jsonResponse({ ok: true }));
    const data = await mod.msBE.request("GET", "/auth/me");
    expect(data).toEqual({ ok: true });
    expect(mod.msBE.API_BASE).toBe("");
    const c = call(f);
    expect(c.url).toBe("/auth/me");
    expect(c.method).toBe("GET");
    expect(c.headers).toEqual({ accept: "application/json" });
    expect(c.init.credentials).toBe("include");
    expect(c.init.body).toBeUndefined();
  });

  it("POST: JSON content-type and serialised body", async () => {
    const f = stubFetch(jsonResponse({}));
    await mod.msBE.request("POST", "/auth/x", { a: 1 });
    const c = call(f);
    expect(c.method).toBe("POST");
    expect(c.headers["content-type"]).toBe("application/json");
    expect(c.init.body).toBe('{"a":1}');
  });

  it("adds the bearer token when one is stored", async () => {
    ls().setItem("msBE.token", "jwt-abc");
    const f = stubFetch(jsonResponse({}));
    await mod.msBE.request("GET", "/auth/me");
    expect(call(f).headers.authorization).toBe("Bearer jwt-abc");
  });

  it("prefixes VITE_AUTH_API_BASE, stripping one trailing slash", async () => {
    vi.stubEnv("VITE_AUTH_API_BASE", "http://auth.test/");
    mod = await loadMsBE();
    mod.setUnauthorizedHandler(() => {});
    expect(mod.msBE.API_BASE).toBe("http://auth.test");
    const f = stubFetch(jsonResponse({}));
    await mod.msBE.request("GET", "/auth/me");
    expect(call(f).url).toBe("http://auth.test/auth/me");
    expect(mod.msBE.auth.googleLoginUrl("/x")).toBe("http://auth.test/auth/oauth/google/start?next=%2Fx");
  });

  it("returns null on an empty 2xx body and on a non-JSON body", async () => {
    stubFetch(new Response(null, { status: 204 }));
    expect(await mod.msBE.request("GET", "/auth/empty")).toBeNull();
    stubFetch(new Response("<html>", { status: 200 }));
    expect(await mod.msBE.request("GET", "/auth/html")).toBeNull();
  });

  it("maps a fetch rejection to a status-0 'Network error' carrying the cause", async () => {
    const cause = new TypeError("Failed to fetch");
    stubFetch(() => Promise.reject(cause));
    const err = (await mod.msBE.request("GET", "/auth/me").catch((e: unknown) => e)) as RequestError;
    expect(err).toBeInstanceOf(Error);
    expect(err.message).toBe("Network error");
    expect(err.status).toBe(0);
    expect(err.cause).toBe(cause);
  });

  it("non-2xx with a JSON `error` field: message is that field, status and body are attached", async () => {
    stubFetch(jsonResponse({ error: "bad credentials" }, 400, "Bad Request"));
    const err = (await mod.msBE.request("POST", "/auth/login", {}).catch((e: unknown) => e)) as RequestError;
    expect(err.message).toBe("bad credentials");
    expect(err.status).toBe(400);
    expect(err.body).toEqual({ error: "bad credentials" });
  });

  it("non-2xx without a usable body: message falls back to '<status> <statusText>'", async () => {
    stubFetch(new Response("oops", { status: 500, statusText: "Server Error" }));
    const err = (await mod.msBE.request("GET", "/auth/me").catch((e: unknown) => e)) as RequestError;
    expect(err.message).toBe("500 Server Error");
    expect(err.status).toBe(500);
    expect(err.body).toBeNull();
  });
});

describe("401 handling", () => {
  it("calls the installed handler and rejects with status 401 without reading the body", async () => {
    const handler = vi.fn();
    mod.setUnauthorizedHandler(handler);
    const res = new Response(JSON.stringify({ error: "expired" }), { status: 401 });
    const text = vi.spyOn(res, "text");
    stubFetch(res);
    await expect(mod.msBE.request("GET", "/auth/me")).rejects.toMatchObject({ message: "Unauthorized", status: 401 });
    expect(handler).toHaveBeenCalledTimes(1);
    expect(text).not.toHaveBeenCalled();
  });

  it("default handler: clears the session and redirects to /login?next=<path+search>", async () => {
    mod = await loadMsBE();
    ls().setItem("msBE.token", "jwt");
    ls().setItem("msBE.user", "{}");
    const assign = vi.fn();
    vi.stubGlobal("location", { ...window.location, pathname: "/concepts", search: "?page=2", assign });
    stubFetch(new Response(null, { status: 401 }));
    await expect(mod.msBE.request("GET", "/auth/me")).rejects.toMatchObject({ status: 401 });
    expect(ls().getItem("msBE.token")).toBeNull();
    expect(ls().getItem("msBE.user")).toBeNull();
    expect(assign).toHaveBeenCalledExactlyOnceWith("/login?next=%2Fconcepts%3Fpage%3D2");
  });

  it("default handler: does not redirect when already on /login", async () => {
    mod = await loadMsBE();
    ls().setItem("msBE.token", "jwt");
    const assign = vi.fn();
    vi.stubGlobal("location", { ...window.location, pathname: "/login", search: "", assign });
    stubFetch(new Response(null, { status: 401 }));
    await expect(mod.msBE.request("GET", "/auth/me")).rejects.toMatchObject({ status: 401 });
    expect(ls().getItem("msBE.token")).toBeNull();
    expect(assign).not.toHaveBeenCalled();
  });
});

describe("msBE.auth", () => {
  const user = { id: 3, email: "ann@example.com", name: "Ann" };

  it("signup: POST /auth/signup with the credentials, persists token and user", async () => {
    const f = stubFetch(jsonResponse({ token: "jwt-s", user }));
    const data = await mod.msBE.auth.signup({ email: "ann@example.com", password: "Pa55word!", name: "Ann" });
    const c = call(f);
    expect(c.url).toBe("/auth/signup");
    expect(c.method).toBe("POST");
    expect(c.body).toEqual({ email: "ann@example.com", password: "Pa55word!", name: "Ann" });
    expect(data).toEqual({ token: "jwt-s", user });
    expect(ls().getItem("msBE.token")).toBe("jwt-s");
    expect(mod.userStore.get()).toEqual(user);
  });

  it("signup: without a token in the reply nothing is persisted", async () => {
    stubFetch(jsonResponse({ pending: true }));
    await mod.msBE.auth.signup({ email: "a@b.co", password: "x", name: "n" });
    expect(ls().getItem("msBE.token")).toBeNull();
    expect(ls().getItem("msBE.user")).toBeNull();
  });

  it("login: POST /auth/login, persists token and user, reports authenticated", async () => {
    const f = stubFetch(jsonResponse({ token: "jwt-l", user }));
    expect(mod.msBE.auth.isAuthenticated()).toBe(false);
    expect(mod.msBE.auth.currentUser()).toBeNull();
    await mod.msBE.auth.login({ email: "ann@example.com", password: "secret" });
    const c = call(f);
    expect(c.url).toBe("/auth/login");
    expect(c.method).toBe("POST");
    expect(c.body).toEqual({ email: "ann@example.com", password: "secret" });
    expect(mod.msBE.auth.isAuthenticated()).toBe(true);
    expect(mod.msBE.auth.currentUser()).toEqual(user);
  });

  it("login: an empty reply body persists nothing", async () => {
    stubFetch(new Response(null, { status: 200 }));
    expect(await mod.msBE.auth.login({ email: "a@b.co", password: "x" })).toBeNull();
    expect(mod.msBE.auth.isAuthenticated()).toBe(false);
  });

  it("me: GET /auth/me with the bearer token, caches and returns the user", async () => {
    ls().setItem("msBE.token", "jwt-m");
    const f = stubFetch(jsonResponse({ user }));
    expect(await mod.msBE.auth.me()).toEqual(user);
    const c = call(f);
    expect(c.url).toBe("/auth/me");
    expect(c.method).toBe("GET");
    expect(c.headers.authorization).toBe("Bearer jwt-m");
    expect(mod.userStore.get()).toEqual(user);
  });

  it("me: returns null when the reply carries no user", async () => {
    stubFetch(jsonResponse({}));
    expect(await mod.msBE.auth.me()).toBeNull();
    stubFetch(new Response(null, { status: 200 }));
    expect(await mod.msBE.auth.me()).toBeNull();
    expect(ls().getItem("msBE.user")).toBeNull();
  });

  it("logout: POST /auth/logout then clears the session", async () => {
    ls().setItem("msBE.token", "jwt");
    ls().setItem("msBE.user", "{}");
    const f = stubFetch(new Response(null, { status: 204 }));
    await mod.msBE.auth.logout();
    const c = call(f);
    expect(c.url).toBe("/auth/logout");
    expect(c.method).toBe("POST");
    expect(c.headers.authorization).toBe("Bearer jwt");
    expect(ls().getItem("msBE.token")).toBeNull();
    expect(ls().getItem("msBE.user")).toBeNull();
  });

  it("logout: clears the session even when the server call fails", async () => {
    ls().setItem("msBE.token", "jwt");
    stubFetch(() => Promise.reject(new Error("offline")));
    await expect(mod.msBE.auth.logout()).resolves.toBeUndefined();
    expect(ls().getItem("msBE.token")).toBeNull();
  });

  it("OAuth start URLs default to next=/ and encode a custom next", () => {
    expect(mod.msBE.auth.googleLoginUrl()).toBe("/auth/oauth/google/start?next=%2F");
    expect(mod.msBE.auth.microsoftLoginUrl()).toBe("/auth/oauth/microsoft/start?next=%2F");
    expect(mod.msBE.auth.googleLoginUrl("/files?tab=2")).toBe("/auth/oauth/google/start?next=%2Ffiles%3Ftab%3D2");
    expect(mod.msBE.auth.microsoftLoginUrl("/a b")).toBe("/auth/oauth/microsoft/start?next=%2Fa%20b");
  });

  it("consumeOAuthToken stores a token and rejects an empty one", () => {
    expect(mod.msBE.auth.consumeOAuthToken("")).toBe(false);
    expect(mod.msBE.auth.consumeOAuthToken(null)).toBe(false);
    expect(ls().getItem("msBE.token")).toBeNull();
    expect(mod.msBE.auth.consumeOAuthToken("jwt-o")).toBe(true);
    expect(ls().getItem("msBE.token")).toBe("jwt-o");
  });

  it("the default export is the same object as the named `msBE`", () => {
    expect(mod.default).toBe(mod.msBE);
  });
});
