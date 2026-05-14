// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Typed REST client for the ontology-server HTTP API.
// Shares JWT storage with the auth-server SPA client (`msBE`): the same token
// authenticates both backends. Any 401 funnels through the unauthorized
// handler so the user is logged out and bounced to /login.
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
/**
 * Resolve the bearer token. Prefer the JWT minted by the auth-server
 * (`msBE.token`) so the Rust API enforces the same identity. Falls back to
 * the legacy `ontology.apiToken` service token for back-compat.
 */
export function apiToken() {
    if (typeof window === "undefined")
        return null;
    const jwt = window.localStorage.getItem("msBE.token");
    if (jwt && jwt.trim())
        return jwt;
    const legacy = window.localStorage.getItem("ontology.apiToken");
    return legacy && legacy.trim() ? legacy : null;
}
// ---- 401 handler -----------------------------------------------------------
// Mirror the contract from msBE: on 401 we clear auth state and redirect to
// /login?next=<current>. The handler is overridable so non-browser callers
// (tests, SSR) can plug in their own behavior.
let onUnauthorized = () => {
    if (typeof window === "undefined")
        return;
    try {
        window.localStorage.removeItem("msBE.token");
        window.localStorage.removeItem("msBE.user");
    }
    catch {
        /* ignore */
    }
    const path = window.location.pathname;
    if (path !== "/login" && path !== "/signup") {
        const next = encodeURIComponent(window.location.pathname + window.location.search);
        window.location.assign(`/login?next=${next}`);
    }
};
export function setUnauthorizedHandler(fn) {
    onUnauthorized = fn;
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
export class ApiError extends Error {
    status;
    body;
    constructor(message, status, body) {
        super(message);
        this.name = "ApiError";
        this.status = status;
        this.body = body;
    }
}
async function http(path, init = {}) {
    const url = `${apiBase()}${path}`;
    // Merge in Authorization automatically when the caller didn't already set
    // one — guarantees the JWT is forwarded on every request without sprinkling
    // `headers()` at every call site.
    const merged = { ...init };
    const hasAuth = merged.headers
        ? new Headers(merged.headers).has("authorization")
        : false;
    if (!hasAuth) {
        const auto = new Headers(merged.headers ?? {});
        const tok = apiToken();
        if (tok)
            auto.set("authorization", `Bearer ${tok}`);
        merged.headers = auto;
    }
    const res = await fetch(url, merged);
    if (res.status === 401) {
        onUnauthorized();
        throw new ApiError("Unauthorized", 401, null);
    }
    if (!res.ok) {
        let msg = `${res.status} ${res.statusText}`;
        let body = null;
        try {
            body = await res.json();
            if (body && typeof body === "object" && "error" in body) {
                msg = body.error;
            }
        }
        catch {
            try {
                msg = await res.text();
            }
            catch {
                /* ignore */
            }
        }
        throw new ApiError(msg, res.status, body);
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
export const createConcept = (c) => http("/concepts", {
    method: "POST",
    headers: headers(true),
    body: JSON.stringify({ id: 0, ...c }),
});
export const updateConcept = (id, patch) => http(`/concepts/${id}`, {
    method: "PATCH",
    headers: headers(true),
    body: JSON.stringify(patch),
});
export const getConcept = (id) => http(`/concepts/${id}`);
// ---- Rules ---------------------------------------------------------------
export const listRules = () => http("/rules");
export const getRule = (id) => http(`/rules/${id}`);
export const createRule = (r) => http("/rules", {
    method: "POST",
    headers: headers(true),
    body: JSON.stringify({ id: 0, ...r }),
});
export const deleteRule = (id) => http(`/rules/${id}`, { method: "DELETE", headers: headers() });
export const updateRule = (id, patch) => http(`/rules/${id}`, {
    method: "PATCH",
    headers: headers(true),
    body: JSON.stringify(patch),
});
// ---- Actions -------------------------------------------------------------
export const listActions = () => http("/actions");
export const getAction = (id) => http(`/actions/${id}`);
export const createAction = (a) => http("/actions", {
    method: "POST",
    headers: headers(true),
    body: JSON.stringify({ id: 0, ...a }),
});
export const deleteAction = (id) => http(`/actions/${id}`, { method: "DELETE", headers: headers() });
export const updateAction = (id, patch) => http(`/actions/${id}`, {
    method: "PATCH",
    headers: headers(true),
    body: JSON.stringify(patch),
});
export const listRelations = (params = {}) => {
    const qs = new URLSearchParams();
    if (params.source != null)
        qs.set("source", String(params.source));
    if (params.target != null)
        qs.set("target", String(params.target));
    if (params.type)
        qs.set("type", params.type);
    if (params.limit != null)
        qs.set("limit", String(params.limit));
    if (params.offset != null)
        qs.set("offset", String(params.offset));
    const s = qs.toString();
    return http(`/relations${s ? `?${s}` : ""}`);
};
export const getRelation = (id) => http(`/relations/${id}`);
export const createRelation = (r) => http("/relations", {
    method: "POST",
    headers: headers(true),
    body: JSON.stringify({ id: 0, weight: r.weight ?? 1.0, ...r }),
});
export const updateRelation = (id, patch) => http(`/relations/${id}`, {
    method: "PATCH",
    headers: headers(true),
    body: JSON.stringify(patch),
});
export const deleteRelation = (id) => http(`/relations/${id}`, { method: "DELETE", headers: headers() });
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
    if (res.status === 401) {
        onUnauthorized();
        throw new ApiError("Unauthorized", 401, null);
    }
    if (!res.ok) {
        throw new ApiError(`upload failed: ${res.status} ${res.statusText}`, res.status, null);
    }
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
