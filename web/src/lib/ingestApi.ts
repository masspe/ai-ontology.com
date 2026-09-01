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
import type {
  ApplyDecision,
  ApplyReport,
  DecisionAction,
  OntologyProposal,
} from "./proposalTypes";

export interface AnalyzeOptions {
  file: File;
  /**
   * Provider to route this analysis to, or `"default"` / omitted to use the
   * one selected in Settings. Credentials are resolved server-side from the
   * settings store — the browser never holds or forwards an API key.
   */
  provider?: "default" | "openai" | "anthropic" | "infomaniak";
  /** Model override. Omitted: the configured model for that provider. */
  model?: string;
  /** ISO-639-1 hint that bypasses automatic detection. */
  languageHint?: string;
}

export async function analyzeIngest(opts: AnalyzeOptions): Promise<OntologyProposal> {
  const form = new FormData();
  form.append("file", opts.file, opts.file.name);
  if (opts.provider) form.append("provider", opts.provider);
  if (opts.model) form.append("model", opts.model);
  if (opts.languageHint) form.append("language_hint", opts.languageHint);

  const headers: Record<string, string> = {};
  const tok = apiToken();
  if (tok) headers["authorization"] = `Bearer ${tok}`;

  const res = await fetch(`${apiBase()}/ingest/analyze`, {
    method: "POST",
    body: form,
    headers,
  });
  if (!res.ok) {
    const raw = await res.text().catch(() => "");
    let body: unknown = raw;
    try {
      body = raw ? JSON.parse(raw) : null;
    } catch {
      /* keep raw text */
    }
    const detail = typeof body === "string" ? body : raw;
    const suffix = detail ? ` — ${detail.slice(0, 400)}` : "";
    throw new ApiError(
      `analyze failed: ${res.status} ${res.statusText}${suffix}`,
      res.status,
      body,
    );
  }
  return (await res.json()) as OntologyProposal;
}

export interface ApplyOptions {
  proposal: OntologyProposal;
  decisions: ApplyDecision[];
  strict?: boolean;
  defaultAction?: DecisionAction;
}

export async function applyIngest(opts: ApplyOptions): Promise<ApplyReport> {
  const body = {
    proposal: opts.proposal,
    decisions: opts.decisions,
    strict: opts.strict ?? false,
    default_action: opts.defaultAction ?? "skip",
  };

  const headers: Record<string, string> = { "content-type": "application/json" };
  const tok = apiToken();
  if (tok) headers["authorization"] = `Bearer ${tok}`;

  const res = await fetch(`${apiBase()}/ingest/apply`, {
    method: "POST",
    headers,
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    let errBody: unknown = null;
    try {
      errBody = await res.json();
    } catch {
      /* ignore */
    }
    throw new ApiError(
      `apply failed: ${res.status} ${res.statusText}`,
      res.status,
      errBody,
    );
  }
  return (await res.json()) as ApplyReport;
}
