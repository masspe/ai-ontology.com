// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Typed REST client for the ontology-server HTTP API.

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

/** Resolve the bearer token from localStorage, if any. */
export function apiToken(): string | null {
  if (typeof window === "undefined") return null;
  const t = window.localStorage.getItem("ontology.apiToken");
  return t && t.trim() ? t : null;
}

function headers(json = false): HeadersInit {
  const h: Record<string, string> = {};
  if (json) h["content-type"] = "application/json";
  const t = apiToken();
  if (t) h["authorization"] = `Bearer ${t}`;
  return h;
}

async function http<T>(path: string, init: RequestInit = {}): Promise<T> {
  const url = `${apiBase()}${path}`;
  const res = await fetch(url, init);
  if (!res.ok) {
    let msg = `${res.status} ${res.statusText}`;
    try {
      const body = await res.json();
      if (body && typeof body === "object" && "error" in body) msg = body.error as string;
    } catch {
      try {
        msg = await res.text();
      } catch {
        /* ignore */
      }
    }
    throw new Error(msg);
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

export interface Ontology {
  concept_types: Record<string, ConceptTypeDef>;
  relation_types: Record<string, RelationTypeDef>;
  rule_types?: Record<string, any>;
  action_types?: Record<string, any>;
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
  properties?: Record<string, any>;
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
  if (!res.ok || !res.body) {
    throw new Error(`stream failed: ${res.status} ${res.statusText}`);
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
  if (!res.ok) throw new Error(`upload failed: ${res.status} ${res.statusText}`);
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
