// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
// 
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

// Tiny API client. The base URL comes from VITE_API_BASE if set at build
// time, otherwise it falls back to http://localhost:8080 — the default
// `ontology serve --bind 127.0.0.1:8080`.

const BASE: string =
  (import.meta.env.VITE_API_BASE as string | undefined) ?? "http://localhost:8080";

export interface Stats {
  concepts: number;
  relations: number;
  concept_types: number;
  relation_types: number;
}

export async function getStats(): Promise<Stats> {
  const r = await fetch(`${BASE}/stats`);
  if (!r.ok) throw new Error(`stats: ${r.status}`);
  return r.json();
}

export interface Concept {
  id: number;
  concept_type: string;
  name: string;
  description?: string;
  properties?: Record<string, unknown>;
}

export interface ListConceptsResponse {
  total: number;
  concepts: Concept[];
}

export async function listConcepts(opts: {
  type?: string;
  q?: string;
  limit?: number;
  offset?: number;
} = {}): Promise<ListConceptsResponse> {
  const p = new URLSearchParams();
  if (opts.type) p.set("type", opts.type);
  if (opts.q) p.set("q", opts.q);
  if (opts.limit != null) p.set("limit", String(opts.limit));
  if (opts.offset != null) p.set("offset", String(opts.offset));
  const qs = p.toString();
  const r = await fetch(`${BASE}/concepts${qs ? `?${qs}` : ""}`);
  if (!r.ok) throw new Error(`concepts: ${r.status}`);
  return r.json();
}

export interface ConceptType {
  name: string;
  parent?: string | null;
  description?: string;
  properties?: string[] | null;
}

export interface RelationType {
  name: string;
  domain: string;
  range: string;
  cardinality?: string;
  symmetric?: boolean;
  description?: string;
}

export interface OntologySchema {
  concept_types: Record<string, ConceptType>;
  relation_types: Record<string, RelationType>;
}

export async function getOntology(): Promise<OntologySchema> {
  const r = await fetch(`${BASE}/ontology`);
  if (!r.ok) throw new Error(`ontology: ${r.status}`);
  return r.json();
}

export interface UploadResponse {
  ingested: { concepts: number; relations: number; ontology_updates: number };
}

/**
 * Upload a single file via multipart/form-data. The server inspects the
 * `kind` field and dispatches to the matching ingester.
 */
export async function upload(
  kind: "ontology" | "jsonl" | "triples" | "csv" | "xlsx" | "text",
  file: File,
  opts?: { concept_type?: string; name?: string },
): Promise<UploadResponse> {
  const fd = new FormData();
  fd.append("kind", kind);
  if (opts?.concept_type) fd.append("concept_type", opts.concept_type);
  if (opts?.name) fd.append("name", opts.name);
  fd.append("file", file);
  const r = await fetch(`${BASE}/upload`, { method: "POST", body: fd });
  if (!r.ok) throw new Error(`upload: ${r.status} ${await r.text()}`);
  return r.json();
}

export interface ScoredConcept {
  id: number;
  score: number;
  lexical: number;
  vector: number;
}

export interface RagStreamHandlers {
  onRetrieved?: (
    payload: { query: string; scored: ScoredConcept[]; subgraph: unknown },
  ) => void;
  onToken?: (text: string) => void;
  onEnd?: (payload: {
    usage: {
      input_tokens?: number;
      output_tokens?: number;
      cache_creation_input_tokens?: number;
      cache_read_input_tokens?: number;
    };
    model?: string;
    stop_reason?: string | null;
  }) => void;
  onError?: (msg: string) => void;
}

/**
 * Drive POST /ask/stream and parse the SSE frames into typed callbacks.
 *
 * Why fetch + manual parsing instead of EventSource? EventSource only
 * supports GET; the server's /ask/stream is POST so it can carry the
 * RetrievalRequest body. Manual ReadableStream parsing is cheap.
 */
export async function askStream(
  query: string,
  handlers: RagStreamHandlers,
  opts: {
    top_k?: number;
    depth?: number;
    concept_types?: string[];
    signal?: AbortSignal;
  } = {},
): Promise<void> {
  const body = JSON.stringify({
    query,
    top_k: opts.top_k ?? 8,
    lexical_weight: 0.5,
    concept_types: opts.concept_types ?? [],
    expansion: { max_depth: opts.depth ?? 2 },
  });
  const r = await fetch(`${BASE}/ask/stream`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body,
    signal: opts.signal,
  });
  if (!r.ok || !r.body) {
    throw new Error(`ask/stream: ${r.status} ${await r.text()}`);
  }

  const reader = r.body.getReader();
  const dec = new TextDecoder();
  let buf = "";
  while (true) {
    const { value, done } = await reader.read();
    if (done) break;
    buf += dec.decode(value, { stream: true });

    // SSE event boundary is a blank line.
    let idx;
    while ((idx = buf.indexOf("\n\n")) !== -1) {
      const raw = buf.slice(0, idx);
      buf = buf.slice(idx + 2);
      let event = "message";
      const dataLines: string[] = [];
      for (const line of raw.split("\n")) {
        if (line.startsWith("event:")) {
          event = line.slice(6).trim();
        } else if (line.startsWith("data:")) {
          dataLines.push(line.slice(5).trimStart());
        }
      }
      const data = dataLines.join("\n");
      if (!data) continue;
      try {
        const payload = JSON.parse(data);
        switch (event) {
          case "retrieved":
            handlers.onRetrieved?.(payload);
            break;
          case "token":
            // Wire shape: { "type": "token", "text": "<delta>" }
            if (typeof payload.text === "string") handlers.onToken?.(payload.text);
            break;
          case "end":
            handlers.onEnd?.(payload);
            return;
          case "error":
            handlers.onError?.(payload.message ?? data);
            return;
        }
      } catch (e) {
        // Malformed frame — skip rather than crash the whole stream.
        console.warn("bad SSE frame", data, e);
      }
    }
  }
}
