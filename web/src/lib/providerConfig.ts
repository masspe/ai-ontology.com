// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Load/save helpers for the provider config store. Mirrors the ontology API
// URL + bearer token to the legacy `ontology.apiBase` / `ontology.apiToken`
// keys so existing `apiBase()` / `apiToken()` callers keep working without
// any change in precedence.
//
// IMPORTANT: do NOT import from `../api` here — `api.ts` falls back to this
// module and a circular import would break the bundler.

import {
  DEFAULT_PROVIDER_CONFIG,
  PROVIDER_CONFIG_STORAGE_KEY,
  type LLMProvider,
  type ProviderConfig,
} from "../types/providerConfig";

/** Read the persisted config, merged over defaults. SSR-safe. */
export function loadProviderConfig(): ProviderConfig {
  if (typeof window === "undefined") return { ...DEFAULT_PROVIDER_CONFIG };
  try {
    const raw = window.localStorage.getItem(PROVIDER_CONFIG_STORAGE_KEY);
    if (!raw) return { ...DEFAULT_PROVIDER_CONFIG };
    const parsed = JSON.parse(raw) as Partial<ProviderConfig>;
    return { ...DEFAULT_PROVIDER_CONFIG, ...parsed };
  } catch {
    return { ...DEFAULT_PROVIDER_CONFIG };
  }
}

/**
 * Persist the config and mirror the ontology URL + token to legacy keys so
 * `apiBase()` / `apiToken()` consumers continue working unchanged.
 */
export function saveProviderConfig(cfg: ProviderConfig): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(PROVIDER_CONFIG_STORAGE_KEY, JSON.stringify(cfg));

    // Mirror to legacy keys (backward compat).
    const url = cfg.ontologyApiUrl.trim().replace(/\/$/, "");
    if (url) window.localStorage.setItem("ontology.apiBase", url);
    else window.localStorage.removeItem("ontology.apiBase");

    const tok = cfg.ontologyBearerToken.trim();
    if (tok) window.localStorage.setItem("ontology.apiToken", tok);
    else window.localStorage.removeItem("ontology.apiToken");
  } catch {
    /* quota / privacy mode — ignore */
  }
}

/** Return the API key that corresponds to a given provider, or `""`. */
export function apiKeyForProvider(cfg: ProviderConfig, provider: LLMProvider): string {
  switch (provider) {
    case "openai":
      return cfg.openaiKey;
    case "anthropic":
      return cfg.anthropicKey;
    case "infomaniak":
      return cfg.infomaniakKey;
    default:
      return "";
  }
}

/**
 * Extra fields to forward to `/ingest/analyze` (and similar endpoints) so the
 * backend can route to a user-configured provider. Returns only fields with
 * non-empty values.
 */
export function getActiveProviderRequestFields(
  cfg: ProviderConfig = loadProviderConfig(),
  override?: LLMProvider,
): { provider?: LLMProvider; model?: string; base_url?: string; api_key?: string } {
  const provider = override ?? cfg.activeLLMProvider;
  const out: { provider?: LLMProvider; model?: string; base_url?: string; api_key?: string } = {};
  if (provider && provider !== "default") out.provider = provider;
  const key = apiKeyForProvider(cfg, provider);
  if (key) out.api_key = key;
  if (provider === "infomaniak") {
    if (cfg.infomaniakBaseUrl) out.base_url = cfg.infomaniakBaseUrl;
    if (cfg.infomaniakModel) out.model = cfg.infomaniakModel;
  }
  return out;
}
