// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Typed REST client for the ontology-server HTTP API.
/* eslint-disable @typescript-eslint/no-explicit-any */
const ENV_BASE = import.meta.env.VITE_API_BASE ?? "";
/** Resolve the API base URL: localStorage override → vite env → same-origin. */
export function apiBase() {
    if (typeof window !== "undefined") {
        const override = window.localStorage.getItem("ontology.apiBase");
        if (override && override.trim())
            return override.replace(/\/$/, "");
    }
    return ENV_BASE.replace(/\/$/, "");
}
/** Resolve the bearer token from localStorage, if any. */
export function apiToken() {
    if (typeof window === "undefined")
        return null;
    const t = window.localStorage.getItem("ontology.apiToken");
    return t && t.trim() ? t : null;
}
function headers(json = false) {
    const h = {};
    if (json)
        h["content-type"] = "application/json";
    const t = apiToken();
    if (t)
        h["authorization"] = `Bearer ${t}`;
    return h;
}
async function http(path, init = {}) {
    const url = `${apiBase()}${path}`;
    const res = await fetch(url, init);
    if (!res.ok) {
        let msg = `${res.status} ${res.statusText}`;
        try {
            const body = await res.json();
            if (body && typeof body === "object" && "error" in body)
                msg = body.error;
        }
        catch {
            try {
                msg = await res.text();
            }
            catch {
                /* ignore */
            }
        }
        throw new Error(msg);
    }
    if (res.status === 204)
        return undefined;
    return (await res.json());
}
// ---------------------------------------------------------------------------
// Endpoints
// ---------------------------------------------------------------------------
export const getStats = () => http("/stats");
export const getStatsHistory = () => http("/stats/history");
export const getOntology = () => http("/ontology");
export const replaceOntology = (o) => http("/ontology", {
    method: "PUT",
    headers: headers(true),
    body: JSON.stringify(o),
});
export const generateOntology = (description) => http("/ontology/generate", {
    method: "POST",
    headers: headers(true),
    body: JSON.stringify({ description }),
});
export const listConcepts = (params = {}) => {
    const qs = new URLSearchParams();
    if (params.type)
        qs.set("type", params.type);
    if (params.q)
        qs.set("q", params.q);
    if (params.limit != null)
        qs.set("limit", String(params.limit));
    if (params.offset != null)
        qs.set("offset", String(params.offset));
    const s = qs.toString();
    return http(`/concepts${s ? `?${s}` : ""}`);
};
export const deleteConcept = (id) => http(`/concepts/${id}`, { method: "DELETE", headers: headers() });
export const ask = (req) => http("/ask", {
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
export async function askStream(req, handlers, signal) {
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
        if (done)
            break;
        buffer += decoder.decode(value, { stream: true });
        const events = buffer.split("\n\n");
        buffer = events.pop() ?? "";
        for (const raw of events) {
            const lines = raw.split("\n");
            let event = "message";
            let data = "";
            for (const line of lines) {
                if (line.startsWith("event:"))
                    event = line.slice(6).trim();
                else if (line.startsWith("data:"))
                    data += line.slice(5).trim();
            }
            if (!data)
                continue;
            try {
                const parsed = JSON.parse(data);
                if (event === "retrieved" && parsed.subgraph)
                    handlers.onRetrieved?.(parsed.subgraph);
                else if (event === "token")
                    handlers.onToken?.(parsed.text ?? parsed.delta ?? "");
                else if (event === "end")
                    handlers.onEnd?.(parsed);
                else if (event === "error")
                    handlers.onError?.(parsed.message ?? "stream error");
            }
            catch {
                /* ignore */
            }
        }
    }
}
export async function upload(file, opts) {
    const form = new FormData();
    form.append("file", file, file.name);
    form.append("kind", opts.kind);
    if (opts.conceptType)
        form.append("concept_type", opts.conceptType);
    if (opts.name)
        form.append("name", opts.name);
    const url = `${apiBase()}/upload`;
    const h = {};
    const t = apiToken();
    if (t)
        h["authorization"] = `Bearer ${t}`;
    const res = await fetch(url, { method: "POST", body: form, headers: h });
    if (!res.ok)
        throw new Error(`upload failed: ${res.status} ${res.statusText}`);
    return (await res.json());
}
export const getSubgraph = (req) => http("/subgraph", {
    method: "POST",
    headers: headers(true),
    body: JSON.stringify(req),
});
export const exportGraphUrl = (format = "jsonl") => `${apiBase()}/export?format=${format}`;
export const getFiles = () => http("/files");
export const deleteFile = (id) => http(`/files/${id}`, { method: "DELETE", headers: headers() });
export const getQueries = () => http("/queries");
export const createQuery = (q) => http("/queries", {
    method: "POST",
    headers: headers(true),
    body: JSON.stringify(q),
});
export const updateQuery = (id, patch) => http(`/queries/${id}`, {
    method: "PATCH",
    headers: headers(true),
    body: JSON.stringify(patch),
});
export const deleteQuery = (id) => http(`/queries/${id}`, { method: "DELETE", headers: headers() });
export const runQuery = (id) => http(`/queries/${id}/run`, { method: "POST", headers: headers() });
export const getSettings = () => http("/settings");
export const patchSettings = (patch) => http("/settings", {
    method: "PATCH",
    headers: headers(true),
    body: JSON.stringify(patch),
});
