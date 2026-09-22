// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Endpoint-level tests of the REST client: every exported wrapper is called
// once against a stubbed `fetch`, and the URL, method, headers and JSON body
// it produced are asserted, together with the parsed result. `askStream`
// (SSE parsing over a ReadableStream) and `upload` (multipart) have their
// own sections. `api.test.ts` covers base/token resolution and `http()`'s
// generic error handling.

import { beforeEach, describe, expect, it, vi } from "vitest";

async function loadApi() {
  vi.resetModules();
  return await import("./api");
}
type Api = Awaited<ReturnType<typeof loadApi>>;

const BASE = "http://api.test";
const TOKEN = "jwt-abc";

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
  method: string;
  headers: Headers;
  body: unknown;
  rawBody: BodyInit | null | undefined;
  init: RequestInit | undefined;
}

/** The i-th `fetch` call, with its JSON body parsed. */
function call(fetchMock: ReturnType<typeof stubFetch>, i = 0): CapturedCall {
  const [url, init] = fetchMock.mock.calls[i] as [string, RequestInit | undefined];
  const rawBody = init?.body;
  const body = typeof rawBody === "string" ? JSON.parse(rawBody) : undefined;
  return { url, method: init?.method ?? "GET", headers: new Headers(init?.headers), body, rawBody, init };
}

let api: Api;
beforeEach(async () => {
  window.localStorage.setItem("ontology.apiBase", `${BASE}/`);
  window.localStorage.setItem("msBE.token", TOKEN);
  api = await loadApi();
  api.setUnauthorizedHandler(() => {});
});

// ---------------------------------------------------------------------------
// Plain JSON endpoints
// ---------------------------------------------------------------------------

interface Case {
  name: string;
  run: (a: Api) => Promise<unknown>;
  url: string;
  method: string;
  body?: unknown;
  /** Server reply; defaults to `{ ok: true }` (or 204 when `noContent`). */
  reply?: unknown;
  noContent?: boolean;
}

const RULE = { rule_type: "Obligation", name: "Pay", applies_to: [1, 2], strict: true };
const ACTION = { action_type: "Notify", name: "Ping", subject: 3, object: null };
const ONTOLOGY = { concept_types: { Person: { name: "Person" } }, relation_types: {} };

const CASES: Case[] = [
  // ---- stats / ontology
  { name: "getStats", run: (a) => a.getStats(), url: "/stats", method: "GET", reply: { concepts: 1 } },
  { name: "getStatsHistory", run: (a) => a.getStatsHistory(), url: "/stats/history", method: "GET", reply: { samples: [] } },
  { name: "getOntology", run: (a) => a.getOntology(), url: "/ontology", method: "GET", reply: ONTOLOGY },
  { name: "replaceOntology", run: (a) => a.replaceOntology(ONTOLOGY), url: "/ontology", method: "PUT", body: ONTOLOGY, reply: ONTOLOGY },
  {
    name: "generateOntology",
    run: (a) => a.generateOntology("contracts and parties"),
    url: "/ontology/generate",
    method: "POST",
    body: { description: "contracts and parties" },
    reply: { ontology: ONTOLOGY, model: "gpt" },
  },
  {
    name: "generateRule",
    run: (a) => a.generateRule("pay on time", "Obligation", [4, 5]),
    url: "/rules/generate",
    method: "POST",
    body: { description: "pay on time", rule_type: "Obligation", applies_to: [4, 5] },
    reply: { name: "Pay", when: "due", then: "pay", description: "", strict: false },
  },
  // ---- concepts
  { name: "listConcepts (no params)", run: (a) => a.listConcepts(), url: "/concepts", method: "GET", reply: { total: 0, concepts: [] } },
  {
    name: "listConcepts (all params, offset 0 kept)",
    run: (a) => a.listConcepts({ type: "Person", q: "al ice", limit: 10, offset: 0 }),
    url: "/concepts?type=Person&q=al+ice&limit=10&offset=0",
    method: "GET",
    reply: { total: 1, concepts: [{ id: 1 }] },
  },
  {
    name: "listConcepts (empty strings dropped)",
    run: (a) => a.listConcepts({ type: "", q: "", limit: 5 }),
    url: "/concepts?limit=5",
    method: "GET",
  },
  { name: "getConcept", run: (a) => a.getConcept(9), url: "/concepts/9", method: "GET", reply: { id: 9, name: "x" } },
  {
    name: "createConcept",
    run: (a) => a.createConcept({ concept_type: "Person", name: "Alice", description: "d", properties: { age: 3 } }),
    url: "/concepts",
    method: "POST",
    body: { id: 0, concept_type: "Person", name: "Alice", description: "d", properties: { age: 3 } },
    reply: { id: 12 },
  },
  {
    name: "updateConcept",
    run: (a) => a.updateConcept(12, { name: "Bob" }),
    url: "/concepts/12",
    method: "PATCH",
    body: { name: "Bob" },
    reply: { id: 12, name: "Bob" },
  },
  { name: "deleteConcept", run: (a) => a.deleteConcept(12), url: "/concepts/12", method: "DELETE", noContent: true },
  {
    name: "deleteConcepts",
    run: (a) => a.deleteConcepts([1, 2, 3]),
    url: "/concepts/delete",
    method: "POST",
    body: { ids: [1, 2, 3] },
    reply: { deleted: 2, relations: 4, missing: [3] },
  },
  { name: "resetAll", run: (a) => a.resetAll(), url: "/reset", method: "POST", noContent: true },
  // ---- rules
  { name: "listRules", run: (a) => a.listRules(), url: "/rules", method: "GET", reply: [] },
  { name: "getRule", run: (a) => a.getRule(3), url: "/rules/3", method: "GET", reply: { id: 3 } },
  { name: "createRule", run: (a) => a.createRule(RULE), url: "/rules", method: "POST", body: { id: 0, ...RULE }, reply: { id: 3 } },
  {
    name: "createRule (caller id wins over the default 0)",
    run: (a) => a.createRule({ ...RULE, id: 77 }),
    url: "/rules",
    method: "POST",
    body: { ...RULE, id: 77 },
    reply: { id: 77 },
  },
  { name: "updateRule", run: (a) => a.updateRule(3, { strict: false }), url: "/rules/3", method: "PATCH", body: { strict: false }, reply: { id: 3 } },
  { name: "deleteRule", run: (a) => a.deleteRule(3), url: "/rules/3", method: "DELETE", noContent: true },
  // ---- actions
  { name: "listActions", run: (a) => a.listActions(), url: "/actions", method: "GET", reply: [] },
  { name: "getAction", run: (a) => a.getAction(4), url: "/actions/4", method: "GET", reply: { id: 4 } },
  { name: "createAction", run: (a) => a.createAction(ACTION), url: "/actions", method: "POST", body: { id: 0, ...ACTION }, reply: { id: 4 } },
  { name: "updateAction", run: (a) => a.updateAction(4, { effect: "sent" }), url: "/actions/4", method: "PATCH", body: { effect: "sent" }, reply: { id: 4 } },
  { name: "deleteAction", run: (a) => a.deleteAction(4), url: "/actions/4", method: "DELETE", noContent: true },
  // ---- relations
  { name: "listRelations (no params)", run: (a) => a.listRelations(), url: "/relations", method: "GET", reply: { total: 0, relations: [] } },
  {
    name: "listRelations (all params)",
    run: (a) => a.listRelations({ source: 0, target: 2, type: "knows", limit: 3, offset: 6 }),
    url: "/relations?source=0&target=2&type=knows&limit=3&offset=6",
    method: "GET",
  },
  { name: "getRelation", run: (a) => a.getRelation(5), url: "/relations/5", method: "GET", reply: { id: 5 } },
  {
    name: "createRelation (weight defaults to 1)",
    run: (a) => a.createRelation({ relation_type: "knows", source: 1, target: 2 }),
    url: "/relations",
    method: "POST",
    body: { id: 0, weight: 1, relation_type: "knows", source: 1, target: 2 },
    reply: { id: 5 },
  },
  {
    name: "createRelation (explicit weight and properties kept)",
    run: (a) => a.createRelation({ relation_type: "knows", source: 1, target: 2, weight: 0.25, properties: { since: 2020 } }),
    url: "/relations",
    method: "POST",
    body: { id: 0, weight: 0.25, relation_type: "knows", source: 1, target: 2, properties: { since: 2020 } },
    reply: { id: 6 },
  },
  { name: "updateRelation", run: (a) => a.updateRelation(5, { weight: 0.5 }), url: "/relations/5", method: "PATCH", body: { weight: 0.5 }, reply: { id: 5 } },
  { name: "deleteRelation", run: (a) => a.deleteRelation(5), url: "/relations/5", method: "DELETE", noContent: true },
  // ---- ask / subgraph
  {
    name: "ask (defaults)",
    run: (a) => a.ask({ query: "who?" }),
    url: "/ask",
    method: "POST",
    body: { query: "who?", top_k: 8, lexical_weight: 0.5, concept_types: [], expansion: { max_depth: 2, max_nodes: 64 } },
    reply: { answer: "them" },
  },
  {
    name: "ask (explicit values, including zeros)",
    run: (a) => a.ask({ query: "q", top_k: 0, lexical_weight: 0, concept_types: ["A"], expansion: { max_depth: 0, max_nodes: 5 } }),
    url: "/ask",
    method: "POST",
    body: { query: "q", top_k: 0, lexical_weight: 0, concept_types: ["A"], expansion: { max_depth: 0, max_nodes: 5 } },
    reply: { answer: "a" },
  },
  {
    name: "ask (partial expansion falls back per field)",
    run: (a) => a.ask({ query: "q", expansion: { max_nodes: 9 } }),
    url: "/ask",
    method: "POST",
    body: { query: "q", top_k: 8, lexical_weight: 0.5, concept_types: [], expansion: { max_depth: 2, max_nodes: 9 } },
  },
  {
    name: "getSubgraph",
    run: (a) => a.getSubgraph({ seed_query: "acme", limit: 20, expansion_depth: 1 }),
    url: "/subgraph",
    method: "POST",
    body: { seed_query: "acme", limit: 20, expansion_depth: 1 },
    reply: { subgraph: { concepts: [], relations: [] } },
  },
  // ---- files
  { name: "getFiles", run: (a) => a.getFiles(), url: "/files", method: "GET", reply: { files: [] } },
  { name: "deleteFile", run: (a) => a.deleteFile(8), url: "/files/8", method: "DELETE", noContent: true },
  // ---- saved queries
  { name: "getQueries", run: (a) => a.getQueries(), url: "/queries", method: "GET", reply: { queries: [] } },
  {
    name: "createQuery",
    run: (a) => a.createQuery({ name: "n", query: "q", top_k: 3 }),
    url: "/queries",
    method: "POST",
    body: { name: "n", query: "q", top_k: 3 },
    reply: { id: 1, name: "n" },
  },
  { name: "updateQuery", run: (a) => a.updateQuery(1, { name: "m" }), url: "/queries/1", method: "PATCH", body: { name: "m" }, reply: { id: 1, name: "m" } },
  { name: "deleteQuery", run: (a) => a.deleteQuery(1), url: "/queries/1", method: "DELETE", noContent: true },
  { name: "runQuery", run: (a) => a.runQuery(1), url: "/queries/1/run", method: "POST", reply: { answer: "42" } },
  // ---- feedback
  { name: "listFeedbacks", run: (a) => a.listFeedbacks(), url: "/feedbacks", method: "GET", reply: [] },
  {
    name: "createFeedback",
    run: (a) => a.createFeedback({ kind: "bug", title: "t", description: "d", screenshot: null }),
    url: "/feedbacks",
    method: "POST",
    body: { kind: "bug", title: "t", description: "d", screenshot: null },
    reply: { id: 2, kind: "bug", title: "t" },
  },
  { name: "deleteFeedback", run: (a) => a.deleteFeedback(2), url: "/feedbacks/2", method: "DELETE", noContent: true },
  // ---- logs
  { name: "tailServerLogs (default limit)", run: (a) => a.tailServerLogs(), url: "/logs/tail?limit=200", method: "GET", reply: { lines: ["a"] } },
  { name: "tailServerLogs (explicit limit)", run: (a) => a.tailServerLogs(50), url: "/logs/tail?limit=50", method: "GET", reply: { lines: [] } },
  // ---- settings / LLM
  { name: "testLlm (no overrides)", run: (a) => a.testLlm(), url: "/settings/llm/test", method: "POST", body: {}, reply: { ok: true, provider: "openai" } },
  {
    name: "testLlm (overrides)",
    run: (a) => a.testLlm({ provider: "anthropic", api_key: "k", model: "m" }),
    url: "/settings/llm/test",
    method: "POST",
    body: { provider: "anthropic", api_key: "k", model: "m" },
    reply: { ok: false, provider: "anthropic", error: "bad key" },
  },
  { name: "listLlmModels (no overrides)", run: (a) => a.listLlmModels(), url: "/settings/llm/models", method: "POST", body: {}, reply: { models: [] } },
  {
    name: "listLlmModels (overrides)",
    run: (a) => a.listLlmModels({ provider: "infomaniak", product_id: "42" }),
    url: "/settings/llm/models",
    method: "POST",
    body: { provider: "infomaniak", product_id: "42" },
    reply: { models: [{ id: "m", label: "M" }], endpoint: "https://x" },
  },
  {
    name: "listInfomaniakProducts (no key → empty body)",
    run: (a) => a.listInfomaniakProducts(),
    url: "/settings/llm/infomaniak/products",
    method: "POST",
    body: {},
    reply: { products: [] },
  },
  {
    name: "listInfomaniakProducts (key)",
    run: (a) => a.listInfomaniakProducts("secret"),
    url: "/settings/llm/infomaniak/products",
    method: "POST",
    body: { api_key: "secret" },
    reply: { products: [{ product_id: "1", name: "AI Tools" }] },
  },
  { name: "getSettings", run: (a) => a.getSettings(), url: "/settings", method: "GET", reply: { ui: { theme: "dark" } } },
  { name: "getOcrStatus", run: (a) => a.getOcrStatus(), url: "/settings/ocr/status", method: "GET", reply: { tesseract: { available: true } } },
  {
    name: "patchSettings",
    run: (a) => a.patchSettings({ llm: { openai_api_key: "sk" }, retrieval: { top_k: 4 } }),
    url: "/settings",
    method: "PATCH",
    body: { llm: { openai_api_key: "sk" }, retrieval: { top_k: 4 } },
    reply: { retrieval: { top_k: 4 } },
  },
];

describe("JSON endpoints — URL, method, headers, body and parsed result", () => {
  it.each(CASES)("$name", async (c) => {
    const reply = c.reply ?? { ok: true };
    const fetchMock = stubFetch(c.noContent ? new Response(null, { status: 204 }) : jsonResponse(reply));
    const result = await c.run(api);
    expect(fetchMock).toHaveBeenCalledOnce();
    const got = call(fetchMock);
    expect(got.url).toBe(`${BASE}${c.url}`);
    expect(got.method).toBe(c.method);
    expect(got.headers.get("authorization")).toBe(`Bearer ${TOKEN}`);
    if (c.body !== undefined) {
      expect(got.headers.get("content-type")).toBe("application/json");
      expect(got.body).toEqual(c.body);
    } else {
      expect(got.headers.has("content-type")).toBe(false);
      expect(got.rawBody).toBeUndefined();
    }
    if (c.noContent) expect(result).toBeUndefined();
    else expect(result).toEqual(reply);
  });

  it("omits Authorization on JSON writes when no token is stored", async () => {
    window.localStorage.removeItem("msBE.token");
    const fetchMock = stubFetch(jsonResponse({ id: 1 }));
    await api.createRule(RULE);
    const got = call(fetchMock);
    expect(got.headers.has("authorization")).toBe(false);
    expect(got.headers.get("content-type")).toBe("application/json");
  });

  it("omits Authorization on DELETE when no token is stored", async () => {
    window.localStorage.removeItem("msBE.token");
    const fetchMock = stubFetch(new Response(null, { status: 204 }));
    await api.deleteRule(1);
    expect(call(fetchMock).headers.has("authorization")).toBe(false);
  });

  it("targets the same origin (relative URL) when no base is configured", async () => {
    window.localStorage.removeItem("ontology.apiBase");
    vi.stubEnv("VITE_API_BASE", "");
    api = await loadApi();
    const fetchMock = stubFetch(jsonResponse({}));
    await api.getSettings();
    expect(call(fetchMock).url).toBe("/settings");
  });
});

describe("exportGraphUrl", () => {
  it("defaults to jsonl and prefixes the configured base", () => {
    expect(api.exportGraphUrl()).toBe(`${BASE}/export?format=jsonl`);
    expect(api.exportGraphUrl("json")).toBe(`${BASE}/export?format=json`);
  });

  it("is relative when no base is configured", async () => {
    window.localStorage.removeItem("ontology.apiBase");
    vi.stubEnv("VITE_API_BASE", "");
    api = await loadApi();
    expect(api.exportGraphUrl()).toBe("/export?format=jsonl");
  });
});

// ---------------------------------------------------------------------------
// Error paths through the wrappers
// ---------------------------------------------------------------------------

describe("error responses", () => {
  it("404 with a JSON {error} body → ApiError carrying that message", async () => {
    stubFetch(jsonResponse({ error: "concept 99 not found" }, 404, "Not Found"));
    const err = (await api.getConcept(99).catch((e: unknown) => e)) as InstanceType<typeof api.ApiError>;
    expect(err).toBeInstanceOf(api.ApiError);
    expect(err.status).toBe(404);
    expect(err.message).toBe("concept 99 not found");
    expect(err.body).toEqual({ error: "concept 99 not found" });
  });

  it("404 with a plain-text body → ApiError carrying the text when json() fails but text() works", async () => {
    const fake = {
      ok: false,
      status: 404,
      statusText: "Not Found",
      json: () => Promise.reject(new SyntaxError("not json")),
      text: () => Promise.resolve("no such rule"),
    } as unknown as Response;
    stubFetch(fake);
    const err = (await api.getRule(1).catch((e: unknown) => e)) as InstanceType<typeof api.ApiError>;
    expect(err.status).toBe(404);
    expect(err.message).toBe("no such rule");
    expect(err.body).toBeNull();
  });

  it("keeps '<status> <statusText>' when both json() and text() fail", async () => {
    const fake = {
      ok: false,
      status: 500,
      statusText: "Server Error",
      json: () => Promise.reject(new Error("a")),
      text: () => Promise.reject(new Error("b")),
    } as unknown as Response;
    stubFetch(fake);
    await expect(api.listRules()).rejects.toMatchObject({ status: 500, message: "500 Server Error" });
  });

  it("a 401 on a write calls the unauthorized handler and rejects", async () => {
    const handler = vi.fn();
    api.setUnauthorizedHandler(handler);
    stubFetch(new Response(null, { status: 401 }));
    await expect(api.patchSettings({ ui: { theme: "x" } })).rejects.toMatchObject({ status: 401, message: "Unauthorized" });
    expect(handler).toHaveBeenCalledOnce();
  });

  it("a network failure on a write explains the origin", async () => {
    stubFetch(() => Promise.reject(new TypeError("NetworkError when attempting to fetch resource.")));
    const err = (await api.deleteConcepts([1]).catch((e: unknown) => e)) as InstanceType<typeof api.ApiError>;
    expect(err.status).toBe(0);
    expect(err.message).toContain(`API injoignable sur ${BASE}`);
    expect(err.message).toContain("NetworkError when attempting to fetch resource.");
  });
});

// ---------------------------------------------------------------------------
// askStream — SSE over a ReadableStream
// ---------------------------------------------------------------------------

function sseResponse(chunks: string[], init: ResponseInit = {}): Response {
  const enc = new TextEncoder();
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const c of chunks) controller.enqueue(enc.encode(c));
      controller.close();
    },
  });
  return new Response(stream, { status: 200, headers: { "content-type": "text/event-stream" }, ...init });
}

describe("askStream", () => {
  it("POSTs the request with defaults, accept: text/event-stream and the abort signal", async () => {
    const fetchMock = stubFetch(sseResponse([]));
    const ctrl = new AbortController();
    await api.askStream({ query: "hello" }, {}, ctrl.signal);
    const got = call(fetchMock);
    expect(got.url).toBe(`${BASE}/ask/stream`);
    expect(got.method).toBe("POST");
    expect(got.headers.get("accept")).toBe("text/event-stream");
    expect(got.headers.get("content-type")).toBe("application/json");
    expect(got.headers.get("authorization")).toBe(`Bearer ${TOKEN}`);
    expect(got.body).toEqual({
      query: "hello",
      top_k: 8,
      lexical_weight: 0.5,
      concept_types: [],
      expansion: { max_depth: 2, max_nodes: 64 },
    });
    expect(got.init?.signal).toBe(ctrl.signal);
  });

  it("forwards explicit top_k, lexical_weight and concept_types", async () => {
    const fetchMock = stubFetch(sseResponse([]));
    await api.askStream({ query: "q", top_k: 3, lexical_weight: 0.9, concept_types: ["Contract"] }, {});
    expect(call(fetchMock).body).toMatchObject({ top_k: 3, lexical_weight: 0.9, concept_types: ["Contract"] });
  });

  it("dispatches retrieved / token / end / error events to the handlers", async () => {
    const subgraph = { concepts: [{ id: 1, concept_type: "A", name: "a" }], relations: [] };
    stubFetch(
      sseResponse([
        `event: retrieved\ndata: ${JSON.stringify({ subgraph })}\n\n`,
        `event: token\ndata: {"text":"Hel"}\n\n`,
        `event: token\ndata: {"delta":"lo"}\n\n`,
        `event: token\ndata: {}\n\n`,
        `event: error\ndata: {"message":"llm quota"}\n\n`,
        `event: end\ndata: {"usage":{"tokens":5}}\n\n`,
      ]),
    );
    const onRetrieved = vi.fn();
    const onToken = vi.fn();
    const onEnd = vi.fn();
    const onError = vi.fn();
    await api.askStream({ query: "q" }, { onRetrieved, onToken, onEnd, onError });
    expect(onRetrieved).toHaveBeenCalledExactlyOnceWith(subgraph);
    expect(onToken.mock.calls.map((c) => c[0])).toEqual(["Hel", "lo", ""]);
    expect(onError).toHaveBeenCalledExactlyOnceWith("llm quota");
    expect(onEnd).toHaveBeenCalledExactlyOnceWith({ usage: { tokens: 5 } });
  });

  it("reassembles events split across chunks and concatenates multi-line data", async () => {
    stubFetch(
      sseResponse([
        "event: tok",
        "en\ndata: {\"te",
        "xt\":\"A\"}\n\nevent: token\n",
        "data: {\"text\":\ndata: \"B\"}\n\n",
        "event: token\ndata: {\"text\":\"tail-without-terminator\"}",
      ]),
    );
    const onToken = vi.fn();
    await api.askStream({ query: "q" }, { onToken });
    // The trailing event never gets its blank-line terminator: it stays buffered.
    expect(onToken.mock.calls.map((c) => c[0])).toEqual(["A", "B"]);
  });

  it("ignores events without data, unknown event names, malformed JSON and 'retrieved' without a subgraph", async () => {
    stubFetch(
      sseResponse([
        "event: token\n\n",
        ": comment only\n\n",
        "event: heartbeat\ndata: {\"x\":1}\n\n",
        "event: token\ndata: {not json\n\n",
        "event: retrieved\ndata: {\"nothing\":true}\n\n",
        "data: {\"text\":\"default event is message\"}\n\n",
        "event: error\ndata: {}\n\n",
      ]),
    );
    const onToken = vi.fn();
    const onRetrieved = vi.fn();
    const onError = vi.fn();
    await api.askStream({ query: "q" }, { onToken, onRetrieved, onError });
    expect(onToken).not.toHaveBeenCalled();
    expect(onRetrieved).not.toHaveBeenCalled();
    expect(onError).toHaveBeenCalledExactlyOnceWith("stream error");
  });

  it("works when no handler is provided for an event", async () => {
    stubFetch(sseResponse(["event: token\ndata: {\"text\":\"x\"}\n\nevent: end\ndata: {}\n\n"]));
    await expect(api.askStream({ query: "q" }, {})).resolves.toBeUndefined();
  });

  it("calls the unauthorized handler and throws on 401", async () => {
    const handler = vi.fn();
    api.setUnauthorizedHandler(handler);
    stubFetch(new Response(null, { status: 401 }));
    await expect(api.askStream({ query: "q" }, {})).rejects.toMatchObject({ status: 401, message: "Unauthorized" });
    expect(handler).toHaveBeenCalledOnce();
  });

  it("throws 'stream failed' on a non-2xx status", async () => {
    stubFetch(new Response("nope", { status: 503, statusText: "Service Unavailable" }));
    const err = (await api.askStream({ query: "q" }, {}).catch((e: unknown) => e)) as InstanceType<typeof api.ApiError>;
    expect(err).toBeInstanceOf(api.ApiError);
    expect(err.status).toBe(503);
    expect(err.message).toBe("stream failed: 503 Service Unavailable");
  });

  it("throws 'stream failed' when a 200 has no body", async () => {
    stubFetch({ ok: true, status: 200, statusText: "OK", body: null } as unknown as Response);
    await expect(api.askStream({ query: "q" }, {})).rejects.toMatchObject({ status: 200, message: "stream failed: 200 OK" });
  });

  it("explains an unreachable server", async () => {
    stubFetch(() => Promise.reject(new TypeError("Failed to fetch")));
    const err = (await api.askStream({ query: "q" }, {}).catch((e: unknown) => e)) as InstanceType<typeof api.ApiError>;
    expect(err.status).toBe(0);
    expect(err.message).toContain(`API injoignable sur ${BASE} (Failed to fetch)`);
  });
});

// ---------------------------------------------------------------------------
// upload — multipart form
// ---------------------------------------------------------------------------

describe("upload", () => {
  const file = new File(["hello"], "notes.txt", { type: "text/plain" });

  it("POSTs a FormData with file, kind and the optional fields, bearer only (no content-type)", async () => {
    const reply = { file_id: 3, ingested: { concepts: 1, relations: 0, ontology_updates: 0 } };
    const fetchMock = stubFetch(jsonResponse(reply));
    await expect(api.upload(file, { kind: "document", conceptType: "Contract", name: "Notes" })).resolves.toEqual(reply);
    const got = call(fetchMock);
    expect(got.url).toBe(`${BASE}/upload`);
    expect(got.method).toBe("POST");
    expect(got.headers.get("authorization")).toBe(`Bearer ${TOKEN}`);
    expect(got.headers.has("content-type")).toBe(false);
    const form = got.rawBody as FormData;
    expect(form).toBeInstanceOf(FormData);
    const sent = form.get("file") as File;
    expect(sent.name).toBe("notes.txt");
    expect(form.get("kind")).toBe("document");
    expect(form.get("concept_type")).toBe("Contract");
    expect(form.get("name")).toBe("Notes");
  });

  it("omits concept_type / name when not given and sends no Authorization without a token", async () => {
    window.localStorage.removeItem("msBE.token");
    const fetchMock = stubFetch(jsonResponse({ file_id: 1, ingested: { concepts: 0, relations: 0, ontology_updates: 0 } }));
    await api.upload(file, { kind: "csv" });
    const got = call(fetchMock);
    expect(got.headers.has("authorization")).toBe(false);
    const form = got.rawBody as FormData;
    expect(form.get("kind")).toBe("csv");
    expect(form.has("concept_type")).toBe(false);
    expect(form.has("name")).toBe(false);
  });

  it("calls the unauthorized handler and throws on 401", async () => {
    const handler = vi.fn();
    api.setUnauthorizedHandler(handler);
    stubFetch(new Response(null, { status: 401 }));
    await expect(api.upload(file, { kind: "document" })).rejects.toMatchObject({ status: 401, message: "Unauthorized" });
    expect(handler).toHaveBeenCalledOnce();
  });

  it("throws 'upload failed: <status> <statusText>' on other errors", async () => {
    stubFetch(jsonResponse({ error: "too big" }, 413, "Payload Too Large"));
    const err = (await api.upload(file, { kind: "document" }).catch((e: unknown) => e)) as InstanceType<typeof api.ApiError>;
    expect(err).toBeInstanceOf(api.ApiError);
    expect(err.status).toBe(413);
    expect(err.message).toBe("upload failed: 413 Payload Too Large");
    expect(err.body).toBeNull();
  });

  it("explains an unreachable server", async () => {
    stubFetch(() => Promise.reject(new TypeError("Failed to fetch")));
    await expect(api.upload(file, { kind: "document" })).rejects.toMatchObject({ status: 0 });
  });
});

// ---------------------------------------------------------------------------
// Header merging in http()
// ---------------------------------------------------------------------------

describe("http() header merging", () => {
  it("keeps an Authorization header the wrapper already set (JSON writes carry the token themselves)", async () => {
    const fetchMock = stubFetch(jsonResponse({ id: 1 }));
    await api.createQuery({ name: "n", query: "q" });
    const got = call(fetchMock);
    // Only one authorization value, from `headers(true)`, not a duplicate.
    expect(got.headers.get("authorization")).toBe(`Bearer ${TOKEN}`);
    expect(got.headers.get("authorization")?.split(",")).toHaveLength(1);
  });

  it("prefers the legacy service token when no JWT is stored", async () => {
    window.localStorage.removeItem("msBE.token");
    window.localStorage.setItem("ontology.apiToken", "svc");
    const fetchMock = stubFetch(jsonResponse({}));
    await api.getOntology();
    expect(call(fetchMock).headers.get("authorization")).toBe("Bearer svc");
  });
});
