// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// API client for the LLM-assisted ingest review workflow.
//
// `analyzeIngest` POSTs the file as multipart to `/ingest/analyze` and
// resolves with the full LLM-generated proposal. `applyIngest` POSTs the
// (possibly edited) proposal plus user decisions to `/ingest/apply` and
// returns the per-item outcome report.
import { apiBase, apiToken, ApiError } from "../api";
export async function analyzeIngest(opts) {
    const form = new FormData();
    form.append("file", opts.file, opts.file.name);
    if (opts.provider)
        form.append("provider", opts.provider);
    if (opts.model)
        form.append("model", opts.model);
    if (opts.languageHint)
        form.append("language_hint", opts.languageHint);
    const headers = {};
    const tok = apiToken();
    if (tok)
        headers["authorization"] = `Bearer ${tok}`;
    const res = await fetch(`${apiBase()}/ingest/analyze`, {
        method: "POST",
        body: form,
        headers,
    });
    if (!res.ok) {
        let body = null;
        try {
            body = await res.json();
        }
        catch {
            /* ignore */
        }
        throw new ApiError(`analyze failed: ${res.status} ${res.statusText}`, res.status, body);
    }
    return (await res.json());
}
export async function applyIngest(opts) {
    const body = {
        proposal: opts.proposal,
        decisions: opts.decisions,
        strict: opts.strict ?? false,
        default_action: opts.defaultAction ?? "skip",
    };
    const headers = { "content-type": "application/json" };
    const tok = apiToken();
    if (tok)
        headers["authorization"] = `Bearer ${tok}`;
    const res = await fetch(`${apiBase()}/ingest/apply`, {
        method: "POST",
        headers,
        body: JSON.stringify(body),
    });
    if (!res.ok) {
        let errBody = null;
        try {
            errBody = await res.json();
        }
        catch {
            /* ignore */
        }
        throw new ApiError(`apply failed: ${res.status} ${res.statusText}`, res.status, errBody);
    }
    return (await res.json());
}
