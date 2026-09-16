// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "../api";
import { analyzeIngest, applyIngest } from "./ingestApi";
import type { OntologyProposal } from "./proposalTypes";

const emptyProposal: OntologyProposal = {
  concept_types: [],
  relation_types: [],
  concepts: [],
  relations: [],
  rules: [],
  actions: [],
};

function stubFetch(res: Response | (() => Promise<Response>)) {
  const impl = typeof res === "function" ? res : () => Promise.resolve(res);
  const fn = vi.fn<typeof fetch>(() => impl());
  vi.stubGlobal("fetch", fn);
  return fn;
}

function lastInit(fn: ReturnType<typeof stubFetch>): RequestInit {
  return fn.mock.calls[0][1] as RequestInit;
}

beforeEach(() => {
  window.localStorage.setItem("ontology.apiBase", "http://api.test");
});

describe("analyzeIngest", () => {
  const file = new File(["hello"], "notes.txt", { type: "text/plain" });

  it("POSTs multipart form data to `${apiBase()}/ingest/analyze`", async () => {
    const fetchMock = stubFetch(new Response(JSON.stringify(emptyProposal), { status: 200 }));
    await analyzeIngest({ file });
    expect(fetchMock).toHaveBeenCalledOnce();
    expect(fetchMock.mock.calls[0][0]).toBe("http://api.test/ingest/analyze");
    const init = lastInit(fetchMock);
    expect(init.method).toBe("POST");
    expect(init.body).toBeInstanceOf(FormData);
  });

  it("puts the file under `file` with its original name and content", async () => {
    const fetchMock = stubFetch(new Response(JSON.stringify(emptyProposal), { status: 200 }));
    await analyzeIngest({ file });
    const form = lastInit(fetchMock).body as FormData;
    const sent = form.get("file");
    expect(sent).toBeInstanceOf(File);
    expect((sent as File).name).toBe("notes.txt");
    await expect((sent as File).text()).resolves.toBe("hello");
  });

  it("adds provider, model and language_hint fields when given", async () => {
    const fetchMock = stubFetch(new Response(JSON.stringify(emptyProposal), { status: 200 }));
    await analyzeIngest({ file, provider: "anthropic", model: "claude-x", languageHint: "fr" });
    const form = lastInit(fetchMock).body as FormData;
    expect(form.get("provider")).toBe("anthropic");
    expect(form.get("model")).toBe("claude-x");
    expect(form.get("language_hint")).toBe("fr");
    expect([...form.keys()].sort()).toEqual(["file", "language_hint", "model", "provider"]);
  });

  it("omits provider, model and language_hint when not given or empty", async () => {
    const fetchMock = stubFetch(new Response(JSON.stringify(emptyProposal), { status: 200 }));
    await analyzeIngest({ file, model: "", languageHint: "" });
    const form = lastInit(fetchMock).body as FormData;
    expect([...form.keys()]).toEqual(["file"]);
  });

  it("sends the bearer token when one is stored, and no content-type (the browser sets the multipart boundary)", async () => {
    window.localStorage.setItem("msBE.token", "jwt");
    const fetchMock = stubFetch(new Response(JSON.stringify(emptyProposal), { status: 200 }));
    await analyzeIngest({ file });
    const headers = lastInit(fetchMock).headers as Record<string, string>;
    expect(headers).toEqual({ authorization: "Bearer jwt" });
  });

  it("sends no Authorization header without a token", async () => {
    const fetchMock = stubFetch(new Response(JSON.stringify(emptyProposal), { status: 200 }));
    await analyzeIngest({ file });
    expect(lastInit(fetchMock).headers).toEqual({});
  });

  it("resolves with the parsed proposal", async () => {
    const proposal = { ...emptyProposal, concepts: [{ client_ref: "c1", concept_type: "T", name: "n" }] };
    stubFetch(new Response(JSON.stringify(proposal), { status: 200 }));
    await expect(analyzeIngest({ file })).resolves.toEqual(proposal);
  });

  it("throws an ApiError whose message includes status, statusText and the plain-text body", async () => {
    stubFetch(new Response("file too large", { status: 413, statusText: "Payload Too Large" }));
    const err = (await analyzeIngest({ file }).catch((e: unknown) => e)) as ApiError;
    expect(err).toBeInstanceOf(ApiError);
    expect(err.message).toBe("analyze failed: 413 Payload Too Large — file too large");
    expect(err.status).toBe(413);
    expect(err.body).toBe("file too large");
  });

  it("parses a JSON error body into `body` while keeping the raw JSON text in the message", async () => {
    const raw = JSON.stringify({ error: "unsupported provider" });
    stubFetch(new Response(raw, { status: 400, statusText: "Bad Request" }));
    const err = (await analyzeIngest({ file }).catch((e: unknown) => e)) as ApiError;
    expect(err.body).toEqual({ error: "unsupported provider" });
    expect(err.message).toBe(`analyze failed: 400 Bad Request — ${raw}`);
  });

  it("truncates the body detail in the message to 400 characters", async () => {
    stubFetch(new Response("x".repeat(1000), { status: 500, statusText: "Internal Server Error" }));
    const err = (await analyzeIngest({ file }).catch((e: unknown) => e)) as ApiError;
    expect(err.message).toBe(`analyze failed: 500 Internal Server Error — ${"x".repeat(400)}`);
  });

  it("has no detail suffix and a null body when the error body is empty", async () => {
    stubFetch(new Response(null, { status: 500, statusText: "Internal Server Error" }));
    const err = (await analyzeIngest({ file }).catch((e: unknown) => e)) as ApiError;
    expect(err.message).toBe("analyze failed: 500 Internal Server Error");
    expect(err.body).toBeNull();
  });

  it("surfaces a network failure as an ApiError naming the origin", async () => {
    stubFetch(() => Promise.reject(new TypeError("Failed to fetch")));
    const err = (await analyzeIngest({ file }).catch((e: unknown) => e)) as ApiError;
    expect(err).toBeInstanceOf(ApiError);
    expect(err.status).toBe(0);
    expect(err.message).toContain("http://api.test");
  });
});

describe("applyIngest", () => {
  const report = {
    concept_types: [],
    relation_types: [],
    concepts: [],
    relations: [],
    rules: [],
    actions: [],
    created: 0,
    merged: 0,
    skipped: 0,
    failed: 0,
  };

  it("POSTs JSON to `${apiBase()}/ingest/apply` with strict=false and default_action='skip' by default", async () => {
    const fetchMock = stubFetch(new Response(JSON.stringify(report), { status: 200 }));
    const decisions = [{ client_ref: "c1", action: "merge" as const }];
    await applyIngest({ proposal: emptyProposal, decisions });
    expect(fetchMock.mock.calls[0][0]).toBe("http://api.test/ingest/apply");
    const init = lastInit(fetchMock);
    expect(init.method).toBe("POST");
    expect((init.headers as Record<string, string>)["content-type"]).toBe("application/json");
    expect(JSON.parse(init.body as string)).toEqual({
      proposal: emptyProposal,
      decisions,
      strict: false,
      default_action: "skip",
    });
  });

  it("forwards explicit strict and defaultAction values", async () => {
    const fetchMock = stubFetch(new Response(JSON.stringify(report), { status: 200 }));
    await applyIngest({ proposal: emptyProposal, decisions: [], strict: true, defaultAction: "create_new" });
    const body = JSON.parse(lastInit(fetchMock).body as string);
    expect(body.strict).toBe(true);
    expect(body.default_action).toBe("create_new");
  });

  it("adds the bearer token next to content-type when one is stored", async () => {
    window.localStorage.setItem("msBE.token", "jwt");
    const fetchMock = stubFetch(new Response(JSON.stringify(report), { status: 200 }));
    await applyIngest({ proposal: emptyProposal, decisions: [] });
    expect(lastInit(fetchMock).headers).toEqual({
      "content-type": "application/json",
      authorization: "Bearer jwt",
    });
  });

  it("resolves with the parsed report", async () => {
    stubFetch(new Response(JSON.stringify({ ...report, created: 2 }), { status: 200 }));
    await expect(applyIngest({ proposal: emptyProposal, decisions: [] })).resolves.toEqual({ ...report, created: 2 });
  });

  it("throws ApiError('apply failed: <status> <statusText>') with the parsed JSON body", async () => {
    stubFetch(new Response(JSON.stringify({ error: "dangling" }), { status: 422, statusText: "Unprocessable" }));
    const err = (await applyIngest({ proposal: emptyProposal, decisions: [] }).catch((e: unknown) => e)) as ApiError;
    expect(err).toBeInstanceOf(ApiError);
    expect(err.message).toBe("apply failed: 422 Unprocessable");
    expect(err.status).toBe(422);
    expect(err.body).toEqual({ error: "dangling" });
  });

  it("keeps a null body when the error response is not JSON", async () => {
    stubFetch(new Response("<html>oops</html>", { status: 502, statusText: "Bad Gateway" }));
    const err = (await applyIngest({ proposal: emptyProposal, decisions: [] }).catch((e: unknown) => e)) as ApiError;
    expect(err.message).toBe("apply failed: 502 Bad Gateway");
    expect(err.body).toBeNull();
  });
});
