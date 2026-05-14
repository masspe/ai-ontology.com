// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Typed REST client for the ontology-server HTTP API.
// Shares JWT storage with the auth-server SPA client (`msBE`): the same token
// authenticates both backends. Any 401 funnels through the unauthorized
// handler so the user is logged out and bounced to /login.

/* eslint-disable @typescript-eslint/no-explicit-any */

const ENV_BASE = (import.meta.env.VITE_API_BASE as string | undefined) ?? "";

/** Resolve the API base URL: localStorage override → vite env → same-origin. */
export function apiBase(): string {
  if (typeof window !== "undefined") {
    const override = window.localStorage.getItem("ontology.apiBase");
    if (override && override.trim()) return override.replace(/\/$/, "");
  }
  return ENV_BASE.replace(/\/$/, "");
}

/**
 * Resolve the bearer token. Prefer the JWT minted by the auth-server
 * (`msBE.token`) so the Rust API enforces the same identity. Falls back to
 * the legacy `ontology.apiToken` service token for back-compat.
 */
export function apiToken(): string | null {
  if (typeof window === "undefined") return null;
  const jwt = window.localStorage.getItem("msBE.token");
  if (jwt && jwt.trim()) return jwt;
  const legacy = window.localStorage.getItem("ontology.apiToken");
  return legacy && legacy.trim() ? legacy : null;
}

// ---- 401 handler -----------------------------------------------------------
// Mirror the contract from msBE: on 401 we clear auth state and redirect to
// /login?next=<current>. The handler is overridable so non-browser callers
// (tests, SSR) can plug in their own behavior.
let onUnauthorized: () => void = () => {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.removeItem("msBE.token");
    window.localStorage.removeItem("msBE.user");
  } catch {
    /* ignore */
  }
  const path = window.location.pathname;
  if (path !== "/login" && path !== "/signup") {
    const next = encodeURIComponent(window.location.pathname + window.location.search);
    window.location.assign(`/login?next=${next}`);
  }
};
export function setUnauthorizedHandler(fn: () => void): void {
  onUnauthorized = fn;
}

function headers(json = false): HeadersInit {
  const h: Record<string, string> = {};
  if (json) h["content-type"] = "application/json";
  const t = apiToken();
  if (t) h["authorization"] = `Bearer ${t}`;
  return h;
}

export class ApiError extends Error {
  status: number;
  body: unknown;
  constructor(message: string, status: number, body: unknown) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.body = body;
  }
}

async function http<T>(path: string, init: RequestInit = {}): Promise<T> {
  const url = `${apiBase()}${path}`;
  // Merge in Authorization automatically when the caller didn't already set
  // one — guarantees the JWT is forwarded on every request without sprinkling
  // `headers()` at every call site.
  const merged: RequestInit = { ...init };
  const hasAuth = merged.headers
    ? new Headers(merged.headers).has("authorization")
    : false;
  if (!hasAuth) {
    const auto = new Headers(merged.headers ?? {});
    const tok = apiToken();
    if (tok) auto.set("authorization", `Bearer ${tok}`);
    merged.headers = auto;
  }

  const res = await fetch(url, merged);
  if (res.status === 401) {
    onUnauthorized();
    throw new ApiError("Unauthorized", 401, null);
  }
  if (!res.ok) {
    let msg = `${res.status} ${res.statusText}`;
    let body: unknown = null;
    try {
      body = await res.json();
      if (body && typeof body === "object" && "error" in body) {
        msg = (body as { error: string }).error;
      }
    } catch {
      try {
        msg = await res.text();
      } catch {
        /* ignore */
      }
    }
    throw new ApiError(msg, res.status, body);
  }
  if (res.status === 204) return undefined as unknown as T;
  return (await res.json()) as T;
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface Stats {
  concepts: number;
  relations: number;
  rules: number;
  actions: number;
  concept_types: number;
  relation_types: number;
  rule_types: number;
  action_types: number;
  deltas: {
    concepts_pct: number;
    relations_pct: number;
    concept_types_pct: number;
    relation_types_pct: number;
  };
}

export interface StatsSample {
  ts: number;
  concepts: number;
  relations: number;
  concept_types: number;
  relation_types: number;
}

export interface StatsHistory {
  samples: StatsSample[];
}

export interface ConceptTypeDef {
  name: string;
  description?: string | null;
  parent?: string | null;
  properties?: Record<string, any> | null;
}

export interface RelationTypeDef {
  name: string;
  domain: string;
  range: string;
  cardinality: string;
  symmetric: boolean;
  description?: string;
}

export interface RuleTypeDef {
  name: string;
  when?: string;
  then?: string;
  applies_to?: string[];
  strict?: boolean;
  description?: string;
}

export interface ActionTypeDef {
  name: string;
  subject: string;
  object?: string | null;
  parameters?: string[];
  effect?: string;
  description?: string;
}

export interface Ontology {
  concept_types: Record<string, ConceptTypeDef>;
  relation_types: Record<string, RelationTypeDef>;
  rule_types?: Record<string, RuleTypeDef>;
  action_types?: Record<string, ActionTypeDef>;
}

export interface Concept {
  id: number;
  concept_type: string;
  name: string;
  description?: string;
  properties?: Record<string, any>;
}

export interface Relation {
  id: number;
  relation_type: string;
  source: number;
  target: number;
  weight?: number;
  properties?: Record<string, any>;
}

export interface Rule {
  id: number;
  rule_type: string;
  name: string;
  when?: string;
  then?: string;
  applies_to: number[];
  strict: boolean;
  description?: string;
  properties?: Record<string, any>;
}

export interface Action {
  id: number;
  action_type: string;
  name: string;
  subject: number;
  object?: number | null;
  parameters?: Record<string, any>;
  effect?: string;
  description?: string;
}

export interface Subgraph {
  concepts: Concept[];
  relations: Relation[];
}

export interface ListConceptsResponse {
  total: number;
  concepts: Concept[];
}

export interface FileRecord {
  id: number;
  name: string;
  size: number;
  kind: string;
  status: string;
  uploaded_at: number;
  concepts: number;
  relations: number;
  ontology_updates: number;
  concept_type?: string | null;
}

export interface SavedQuery {
  id: number;
  name: string;
  query: string;
  top_k: number;
  lexical_weight: number;
  concept_types: string[];
  expansion_depth: number;
  created_at: number;
  last_run_at?: number | null;
}

export interface Settings {
  retrieval: {
    top_k: number;
    lexical_weight: number;
    expansion_depth: number;
  };
  ui: {
    theme: string;
    graph_layout: string;
  };
}

export interface SettingsPatch {
  retrieval?: Partial<Settings["retrieval"]>;
  ui?: Partial<Settings["ui"]>;
}

export interface RagAnswer {
  answer: string;
  citations?: Array<{ id: number; name: string; concept_type: string }>;
  usage?: Record<string, any>;
  subgraph?: Subgraph;
}

export interface UploadResponse {
  file_id: number;
  ingested: {
    concepts: number;
    relations: number;
    ontology_updates: number;
  };
}

// ---------------------------------------------------------------------------
// Endpoints
// ---------------------------------------------------------------------------

export const getStats = () => http<Stats>("/stats");
export const getStatsHistory = () => http<StatsHistory>("/stats/history");
export const getOntology = () => http<Ontology>("/ontology");
export const replaceOntology = (o: Ontology) =>
  http<Ontology>("/ontology", {
    method: "PUT",
    headers: headers(true),
    body: JSON.stringify(o),
  });
export const generateOntology = (description: string) =>
  http<{ ontology: Ontology; model: string }>("/ontology/generate", {
    method: "POST",
    headers: headers(true),
    body: JSON.stringify({ description }),
  });

export const listConcepts = (params: {
  type?: string;
  q?: string;
  limit?: number;
  offset?: number;
} = {}) => {
  const qs = new URLSearchParams();
  if (params.type) qs.set("type", params.type);
  if (params.q) qs.set("q", params.q);
  if (params.limit != null) qs.set("limit", String(params.limit));
  if (params.offset != null) qs.set("offset", String(params.offset));
  const s = qs.toString();
  return http<ListConceptsResponse>(`/concepts${s ? `?${s}` : ""}`);
};

export const deleteConcept = (id: number) =>
  http<void>(`/concepts/${id}`, { method: "DELETE", headers: headers() });

export const createConcept = (c: {
  concept_type: string;
  name: string;
  description?: string;
  properties?: Record<string, any>;
}) =>
  http<{ id: number }>("/concepts", {
    method: "POST",
    headers: headers(true),
    body: JSON.stringify({ id: 0, ...c }),
  });

export const updateConcept = (
  id: number,
  patch: { name?: string; description?: string; properties?: Record<string, any> },
) =>
  http<Concept>(`/concepts/${id}`, {
    method: "PATCH",
    headers: headers(true),
    body: JSON.stringify(patch),
  });

export const getConcept = (id: number) => http<Concept>(`/concepts/${id}`);

// ---- Rules ---------------------------------------------------------------

export const listRules = () => http<Rule[]>("/rules");
export const getRule = (id: number) => http<Rule>(`/rules/${id}`);
export const createRule = (r: Omit<Rule, "id"> & { id?: number }) =>
  http<{ id: number }>("/rules", {
    method: "POST",
    headers: headers(true),
    body: JSON.stringify({ id: 0, ...r }),
  });
export const deleteRule = (id: number) =>
  http<void>(`/rules/${id}`, { method: "DELETE", headers: headers() });
export const updateRule = (
  id: number,
  patch: Partial<Omit<Rule, "id" | "rule_type">>,
) =>
  http<Rule>(`/rules/${id}`, {
    method: "PATCH",
    headers: headers(true),
    body: JSON.stringify(patch),
  });

// ---- Actions -------------------------------------------------------------

export const listActions = () => http<Action[]>("/actions");
export const getAction = (id: number) => http<Action>(`/actions/${id}`);
export const createAction = (a: Omit<Action, "id"> & { id?: number }) =>
  http<{ id: number }>("/actions", {
    method: "POST",
    headers: headers(true),
    body: JSON.stringify({ id: 0, ...a }),
  });
export const deleteAction = (id: number) =>
  http<void>(`/actions/${id}`, { method: "DELETE", headers: headers() });
export const updateAction = (
  id: number,
  patch: Partial<Omit<Action, "id" | "action_type">>,
) =>
  http<Action>(`/actions/${id}`, {
    method: "PATCH",
    headers: headers(true),
    body: JSON.stringify(patch),
  });

// ---- Relations -----------------------------------------------------------

export interface ListRelationsResponse {
  total: number;
  relations: Relation[];
}

export const listRelations = (
  params: {
    source?: number;
    target?: number;
    type?: string;
    limit?: number;
    offset?: number;
  } = {},
) => {
  const qs = new URLSearchParams();
  if (params.source != null) qs.set("source", String(params.source));
  if (params.target != null) qs.set("target", String(params.target));
  if (params.type) qs.set("type", params.type);
  if (params.limit != null) qs.set("limit", String(params.limit));
  if (params.offset != null) qs.set("offset", String(params.offset));
  const s = qs.toString();
  return http<ListRelationsResponse>(`/relations${s ? `?${s}` : ""}`);
};

export const getRelation = (id: number) =>
  http<Relation>(`/relations/${id}`);

export const createRelation = (r: {
  relation_type: string;
  source: number;
  target: number;
  weight?: number;
  properties?: Record<string, any>;
}) =>
  http<{ id: number }>("/relations", {
    method: "POST",
    headers: headers(true),
    body: JSON.stringify({ id: 0, weight: r.weight ?? 1.0, ...r }),
  });

export const updateRelation = (
  id: number,
  patch: { weight?: number; properties?: Record<string, any> },
) =>
  http<Relation>(`/relations/${id}`, {
    method: "PATCH",
    headers: headers(true),
    body: JSON.stringify(patch),
  });

export const deleteRelation = (id: number) =>
  http<void>(`/relations/${id}`, { method: "DELETE", headers: headers() });

export const ask = (req: {
  query: string;
  top_k?: number;
  lexical_weight?: number;
  concept_types?: string[];
  expansion?: { max_depth?: number; max_nodes?: number };
}) =>
  http<RagAnswer>("/ask", {
    method: "POST",
    headers: headers(true),
    body: JSON.stringify({
      query: req.query,
      top_k: req.top_k ?? 8,
      lexical_weight: req.lexical_weight ?? 0.5,
      concept_types: req.concept_types ?? [],
      expansion: { max_depth: req.expansion?.max_depth ?? 2, max_nodes: req.expansion?.max_nodes ?? 64 },
    }),
  });

export interface AskStreamHandlers {
  onRetrieved?: (subgraph: Subgraph) => void;
  onToken?: (text: string) => void;
  onEnd?: (info: Record<string, any>) => void;
  onError?: (err: string) => void;
}

export async function askStream(
  req: { query: string; top_k?: number; lexical_weight?: number; concept_types?: string[] },
  handlers: AskStreamHandlers,
  signal?: AbortSignal,
): Promise<void> {
  const url = `${apiBase()}/ask/stream`;
  const res = await fetch(url, {
    method: "POST",
    headers: { ...headers(true), accept: "text/event-stream" },
    body: JSON.stringify({
      query: req.query,
      top_k: req.top_k ?? 8,
      lexical_weight: req.lexical_weight ?? 0.5,
      concept_types: req.concept_types ?? [],
      expansion: { max_depth: 2, max_nodes: 64 },
    }),
    signal,
  });
  if (res.status === 401) {
    onUnauthorized();
    throw new ApiError("Unauthorized", 401, null);
  }
  if (!res.ok || !res.body) {
    throw new ApiError(`stream failed: ${res.status} ${res.statusText}`, res.status, null);
  }
  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });
    const events = buffer.split("\n\n");
    buffer = events.pop() ?? "";
    for (const raw of events) {
      const lines = raw.split("\n");
      let event = "message";
      let data = "";
      for (const line of lines) {
        if (line.startsWith("event:")) event = line.slice(6).trim();
        else if (line.startsWith("data:")) data += line.slice(5).trim();
      }
      if (!data) continue;
      try {
        const parsed = JSON.parse(data);
        if (event === "retrieved" && parsed.subgraph) handlers.onRetrieved?.(parsed.subgraph);
        else if (event === "token") handlers.onToken?.(parsed.text ?? parsed.delta ?? "");
        else if (event === "end") handlers.onEnd?.(parsed);
        else if (event === "error") handlers.onError?.(parsed.message ?? "stream error");
      } catch {
        /* ignore */
      }
    }
  }
}

export async function upload(
  file: File,
  opts: { kind: string; conceptType?: string; name?: string },
): Promise<UploadResponse> {
  const form = new FormData();
  form.append("file", file, file.name);
  form.append("kind", opts.kind);
  if (opts.conceptType) form.append("concept_type", opts.conceptType);
  if (opts.name) form.append("name", opts.name);
  const url = `${apiBase()}/upload`;
  const h: Record<string, string> = {};
  const t = apiToken();
  if (t) h["authorization"] = `Bearer ${t}`;
  const res = await fetch(url, { method: "POST", body: form, headers: h });
  if (res.status === 401) {
    onUnauthorized();
    throw new ApiError("Unauthorized", 401, null);
  }
  if (!res.ok) {
    throw new ApiError(`upload failed: ${res.status} ${res.statusText}`, res.status, null);
  }
  return (await res.json()) as UploadResponse;
}

export const getSubgraph = (req: {
  seed_concept_ids?: number[];
  seed_query?: string;
  seed_concept_types?: string[];
  limit?: number;
  expansion_depth?: number;
}) =>
  http<{ subgraph: Subgraph }>("/subgraph", {
    method: "POST",
    headers: headers(true),
    body: JSON.stringify(req),
  });

export const exportGraphUrl = (format: "jsonl" | "json" = "jsonl") =>
  `${apiBase()}/export?format=${format}`;

export const getFiles = () => http<{ files: FileRecord[] }>("/files");
export const deleteFile = (id: number) =>
  http<void>(`/files/${id}`, { method: "DELETE", headers: headers() });

export const getQueries = () => http<{ queries: SavedQuery[] }>("/queries");
export const createQuery = (q: {
  name: string;
  query: string;
  top_k?: number;
  lexical_weight?: number;
  concept_types?: string[];
  expansion_depth?: number;
}) =>
  http<SavedQuery>("/queries", {
    method: "POST",
    headers: headers(true),
    body: JSON.stringify(q),
  });
export const updateQuery = (id: number, patch: Partial<SavedQuery>) =>
  http<SavedQuery>(`/queries/${id}`, {
    method: "PATCH",
    headers: headers(true),
    body: JSON.stringify(patch),
  });
export const deleteQuery = (id: number) =>
  http<void>(`/queries/${id}`, { method: "DELETE", headers: headers() });
export const runQuery = (id: number) =>
  http<RagAnswer>(`/queries/${id}/run`, { method: "POST", headers: headers() });

export const getSettings = () => http<Settings>("/settings");
export const patchSettings = (patch: SettingsPatch) =>
  http<Settings>("/settings", {
    method: "PATCH",
    headers: headers(true),
    body: JSON.stringify(patch),
  });
