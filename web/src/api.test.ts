// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { beforeEach, describe, expect, it, vi } from "vitest";

// `ENV_BASE` is read from `import.meta.env` at module load, and the 401
// handler is module state, so every test gets a fresh copy of the module.
async function loadApi() {
  vi.resetModules();
  return await import("./api");
}

type Api = Awaited<ReturnType<typeof loadApi>>;

const ls = () => window.localStorage;

function jsonResponse(body: unknown, status = 200, statusText = "OK"): Response {
  return new Response(JSON.stringify(body), {
    status,
    statusText,
    headers: { "content-type": "application/json" },
  });
}

/** Replace global fetch with a mock resolving to `res`; returns the mock. */
function stubFetch(res: Response | (() => Promise<Response>)) {
  const impl = typeof res === "function" ? res : () => Promise.resolve(res);
  const fn = vi.fn<typeof fetch>(() => impl());
  vi.stubGlobal("fetch", fn);
  return fn;
}

describe("apiBase — precedence and normalisation", () => {
  let api: Api;
  beforeEach(async () => {
    vi.stubEnv("VITE_API_BASE", "http://env.example:5000/");
    api = await loadApi();
  });

  it("uses the legacy 'ontology.apiBase' override first, stripping one trailing slash", () => {
    ls().setItem("ontology.apiBase", "http://override.example:1/");
    ls().setItem("ontology.providerConfig", JSON.stringify({ ontologyApiUrl: "http://pc.example:2" }));
    expect(api.apiBase()).toBe("http://override.example:1");
  });

  it("ignores a blank override and falls back to the provider config, trimmed and without trailing slash", () => {
    ls().setItem("ontology.apiBase", "   ");
    ls().setItem("ontology.providerConfig", JSON.stringify({ ontologyApiUrl: "  http://pc.example:2/ " }));
    expect(api.apiBase()).toBe("http://pc.example:2");
  });

  it("ignores a blank provider config URL and falls back to VITE_API_BASE without trailing slash", () => {
    ls().setItem("ontology.providerConfig", JSON.stringify({ ontologyApiUrl: " " }));
    expect(api.apiBase()).toBe("http://env.example:5000");
  });

  it("ignores a corrupt provider config and falls back to VITE_API_BASE", () => {
    ls().setItem("ontology.providerConfig", "{broken");
    expect(api.apiBase()).toBe("http://env.example:5000");
  });

  it("returns the empty string (same-origin) when nothing is configured", async () => {
    vi.stubEnv("VITE_API_BASE", "");
    api = await loadApi();
    expect(api.apiBase()).toBe("");
  });

  it("returns the empty string when VITE_API_BASE is undefined", async () => {
    vi.stubEnv("VITE_API_BASE", undefined);
    api = await loadApi();
    expect(api.apiBase()).toBe("");
  });
});

describe("apiToken — precedence", () => {
  let api: Api;
  beforeEach(async () => {
    api = await loadApi();
  });

  it("prefers the auth-server JWT 'msBE.token' over everything else", () => {
    ls().setItem("msBE.token", "jwt");
    ls().setItem("ontology.apiToken", "legacy");
    ls().setItem("ontology.providerConfig", JSON.stringify({ ontologyBearerToken: "pc" }));
    expect(api.apiToken()).toBe("jwt");
  });

  it("falls back to the legacy 'ontology.apiToken' when the JWT is blank", () => {
    ls().setItem("msBE.token", "  ");
    ls().setItem("ontology.apiToken", "legacy");
    expect(api.apiToken()).toBe("legacy");
  });

  it("falls back to the provider config token, trimmed", () => {
    ls().setItem("ontology.providerConfig", JSON.stringify({ ontologyBearerToken: "  pc  " }));
    expect(api.apiToken()).toBe("pc");
  });

  it("returns null when no token is stored anywhere", () => {
    expect(api.apiToken()).toBeNull();
  });
});

describe("unreachableMessage", () => {
  it("names the origin only (no path) of an absolute URL, and includes the error detail", async () => {
    const { unreachableMessage } = await loadApi();
    const msg = unreachableMessage("http://127.0.0.1:5000/stats?x=1", new TypeError("Failed to fetch"));
    expect(msg).toContain("API injoignable sur http://127.0.0.1:5000 (Failed to fetch)");
    expect(msg).not.toContain("/stats");
  });

  it("points at the Vite proxy for a relative URL", async () => {
    const { unreachableMessage } = await loadApi();
    expect(unreachableMessage("/stats", new Error("boom"))).toContain("API injoignable sur le proxy Vite (boom)");
  });

  it("mentions the `npm run dev` compile delay and where to change the address", async () => {
    const { unreachableMessage } = await loadApi();
    const msg = unreachableMessage("/x", new Error("e"));
    expect(msg).toContain("npm run dev");
    expect(msg).toContain("Réglages → Connexion serveur");
  });

  it("stringifies a non-Error rejection reason", async () => {
    const { unreachableMessage } = await loadApi();
    expect(unreachableMessage("/x", "plain string")).toContain("(plain string)");
  });
});

describe("fetchOrExplain", () => {
  it("wraps a rejected fetch in an ApiError with status 0, null body and the origin in the message", async () => {
    const { fetchOrExplain, ApiError } = await loadApi();
    stubFetch(() => Promise.reject(new TypeError("Failed to fetch")));
    const err = await fetchOrExplain("http://localhost:5000/stats").catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiError);
    const apiErr = err as InstanceType<typeof ApiError>;
    expect(apiErr.name).toBe("ApiError");
    expect(apiErr.status).toBe(0);
    expect(apiErr.body).toBeNull();
    expect(apiErr.message).toContain("http://localhost:5000");
    expect(apiErr.message).toContain("Failed to fetch");
    expect(apiErr.message).toContain("npm run dev");
  });

  it("passes url and init through and returns the Response untouched on success", async () => {
    const { fetchOrExplain } = await loadApi();
    const res = new Response("ok");
    const fetchMock = stubFetch(res);
    const init = { method: "POST" };
    await expect(fetchOrExplain("http://h/x", init)).resolves.toBe(res);
    expect(fetchMock).toHaveBeenCalledExactlyOnceWith("http://h/x", init);
  });
});

describe("http() — via the exported wrappers", () => {
  let api: Api;
  beforeEach(async () => {
    ls().setItem("ontology.apiBase", "http://api.test");
    api = await loadApi();
    // Never let the default 401 handler touch window.location in tests.
    api.setUnauthorizedHandler(() => {});
  });

  it("GETs `${apiBase()}${path}` and returns the parsed JSON", async () => {
    const fetchMock = stubFetch(jsonResponse({ concepts: 3 }));
    await expect(api.getStats()).resolves.toEqual({ concepts: 3 });
    expect(fetchMock).toHaveBeenCalledOnce();
    expect(fetchMock.mock.calls[0][0]).toBe("http://api.test/stats");
  });

  it("adds an Authorization: Bearer header from apiToken() when the caller set none", async () => {
    ls().setItem("msBE.token", "jwt-123");
    const fetchMock = stubFetch(jsonResponse({}));
    await api.getStats();
    const init = fetchMock.mock.calls[0][1] as RequestInit;
    expect(new Headers(init.headers).get("authorization")).toBe("Bearer jwt-123");
  });

  it("sends no Authorization header when no token is stored", async () => {
    const fetchMock = stubFetch(jsonResponse({}));
    await api.getStats();
    const init = fetchMock.mock.calls[0][1] as RequestInit;
    expect(new Headers(init.headers).has("authorization")).toBe(false);
  });

  it("keeps the caller's content-type while adding Authorization", async () => {
    ls().setItem("msBE.token", "jwt");
    const fetchMock = stubFetch(jsonResponse({ id: 1 }));
    await api.createConcept({ concept_type: "Person", name: "Alice" });
    const init = fetchMock.mock.calls[0][1] as RequestInit;
    const h = new Headers(init.headers);
    expect(h.get("content-type")).toBe("application/json");
    expect(h.get("authorization")).toBe("Bearer jwt");
    expect(init.method).toBe("POST");
    expect(JSON.parse(init.body as string)).toEqual({ id: 0, concept_type: "Person", name: "Alice" });
  });

  it("resolves to undefined on a 204 No Content", async () => {
    stubFetch(new Response(null, { status: 204 }));
    await expect(api.deleteConcept(7)).resolves.toBeUndefined();
  });

  it("throws an ApiError carrying the server's {error} message, status and body on a non-2xx JSON response", async () => {
    stubFetch(jsonResponse({ error: "name already taken" }, 409, "Conflict"));
    const err = (await api.getStats().catch((e: unknown) => e)) as InstanceType<typeof api.ApiError>;
    expect(err).toBeInstanceOf(api.ApiError);
    expect(err.message).toBe("name already taken");
    expect(err.status).toBe(409);
    expect(err.body).toEqual({ error: "name already taken" });
  });

  it("falls back to '<status> <statusText>' when the JSON error body has no `error` field", async () => {
    stubFetch(jsonResponse({ detail: "x" }, 500, "Internal Server Error"));
    const err = (await api.getStats().catch((e: unknown) => e)) as InstanceType<typeof api.ApiError>;
    expect(err.message).toBe("500 Internal Server Error");
    expect(err.body).toEqual({ detail: "x" });
  });

  it("uses '<status> <statusText>' for a non-JSON error body (the body stream is consumed by the JSON attempt)", async () => {
    stubFetch(new Response("plain failure", { status: 502, statusText: "Bad Gateway" }));
    const err = (await api.getStats().catch((e: unknown) => e)) as InstanceType<typeof api.ApiError>;
    expect(err).toBeInstanceOf(api.ApiError);
    expect(err.status).toBe(502);
    expect(err.body).toBeNull();
    // res.json() fails, then res.text() also fails on the already-consumed
    // body, so the message stays at the status line.
    expect(err.message).toBe("502 Bad Gateway");
  });

  it("calls the unauthorized handler once and throws ApiError('Unauthorized', 401) on a 401", async () => {
    const handler = vi.fn();
    api.setUnauthorizedHandler(handler);
    stubFetch(new Response(null, { status: 401, statusText: "Unauthorized" }));
    const err = (await api.getStats().catch((e: unknown) => e)) as InstanceType<typeof api.ApiError>;
    expect(handler).toHaveBeenCalledOnce();
    expect(err).toBeInstanceOf(api.ApiError);
    expect(err.message).toBe("Unauthorized");
    expect(err.status).toBe(401);
    expect(err.body).toBeNull();
  });

  it("does not call the unauthorized handler for other error statuses", async () => {
    const handler = vi.fn();
    api.setUnauthorizedHandler(handler);
    stubFetch(jsonResponse({ error: "nope" }, 403, "Forbidden"));
    await expect(api.getStats()).rejects.toMatchObject({ status: 403 });
    expect(handler).not.toHaveBeenCalled();
  });

  it("turns a network failure into an ApiError(status 0) that names the configured origin", async () => {
    stubFetch(() => Promise.reject(new TypeError("Failed to fetch")));
    const err = (await api.getStats().catch((e: unknown) => e)) as InstanceType<typeof api.ApiError>;
    expect(err).toBeInstanceOf(api.ApiError);
    expect(err.status).toBe(0);
    expect(err.message).toContain("http://api.test");
  });
});

describe("default unauthorized handler", () => {
  it("clears the auth-server session keys and redirects to /login?next=<current path>", async () => {
    ls().setItem("msBE.token", "jwt");
    ls().setItem("msBE.user", "{}");
    ls().setItem("ontology.apiBase", "http://api.test");
    const api = await loadApi();
    const assign = vi.fn();
    vi.stubGlobal("location", {
      ...window.location,
      pathname: "/concepts",
      search: "?page=2",
      assign,
    });
    stubFetch(new Response(null, { status: 401 }));
    await expect(api.getStats()).rejects.toMatchObject({ status: 401 });
    expect(ls().getItem("msBE.token")).toBeNull();
    expect(ls().getItem("msBE.user")).toBeNull();
    expect(assign).toHaveBeenCalledExactlyOnceWith("/login?next=%2Fconcepts%3Fpage%3D2");
  });

  it("does not redirect when already on /login", async () => {
    ls().setItem("ontology.apiBase", "http://api.test");
    const api = await loadApi();
    const assign = vi.fn();
    vi.stubGlobal("location", { ...window.location, pathname: "/login", search: "", assign });
    stubFetch(new Response(null, { status: 401 }));
    await expect(api.getStats()).rejects.toMatchObject({ status: 401 });
    expect(assign).not.toHaveBeenCalled();
  });
});
