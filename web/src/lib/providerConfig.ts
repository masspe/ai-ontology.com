// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Load/save helpers for the client-side connection store. Mirrors the ontology
// API URL + bearer token to the legacy `ontology.apiBase` / `ontology.apiToken`
// keys so existing `apiBase()` / `apiToken()` callers keep working without any
// change in precedence.
//
// No provider credential passes through here any more: keys, base URLs and
// model names live in the server settings store. See `types/providerConfig.ts`.
//
// IMPORTANT: do NOT import from `../api` here — `api.ts` falls back to this
// module and a circular import would break the bundler.

import {
  DEFAULT_PROVIDER_CONFIG,
  LEGACY_PROVIDER_FIELDS,
  PROVIDER_CONFIG_STORAGE_KEY,
  type LegacyProviderSecrets,
  type ProviderConfig,
} from "../types/providerConfig";

/** Read the persisted config, merged over defaults. SSR-safe. */
export function loadProviderConfig(): ProviderConfig {
  if (typeof window === "undefined") return { ...DEFAULT_PROVIDER_CONFIG };
  try {
    const raw = window.localStorage.getItem(PROVIDER_CONFIG_STORAGE_KEY);
    if (!raw) return { ...DEFAULT_PROVIDER_CONFIG };
    const parsed = JSON.parse(raw) as Partial<ProviderConfig>;
    return {
      ontologyApiUrl: parsed.ontologyApiUrl ?? DEFAULT_PROVIDER_CONFIG.ontologyApiUrl,
      ontologyBearerToken:
        parsed.ontologyBearerToken ?? DEFAULT_PROVIDER_CONFIG.ontologyBearerToken,
      authApiUrl: parsed.authApiUrl ?? DEFAULT_PROVIDER_CONFIG.authApiUrl,
    };
  } catch {
    return { ...DEFAULT_PROVIDER_CONFIG };
  }
}

/**
 * Persist the config and mirror the ontology URL + token to legacy keys so
 * `apiBase()` / `apiToken()` consumers continue working unchanged.
 *
 * Only the three transport fields are written: any legacy provider block left
 * in the stored object is dropped, which is also how it finally disappears
 * from browsers that had one.
 */
export function saveProviderConfig(cfg: ProviderConfig): void {
  if (typeof window === "undefined") return;
  try {
    const clean: ProviderConfig = {
      ontologyApiUrl: cfg.ontologyApiUrl,
      ontologyBearerToken: cfg.ontologyBearerToken,
      authApiUrl: cfg.authApiUrl,
    };
    window.localStorage.setItem(PROVIDER_CONFIG_STORAGE_KEY, JSON.stringify(clean));

    // Mirror to legacy keys (backward compat).
    const url = clean.ontologyApiUrl.trim().replace(/\/$/, "");
    if (url) window.localStorage.setItem("ontology.apiBase", url);
    else window.localStorage.removeItem("ontology.apiBase");

    const tok = clean.ontologyBearerToken.trim();
    if (tok) window.localStorage.setItem("ontology.apiToken", tok);
    else window.localStorage.removeItem("ontology.apiToken");
  } catch {
    /* quota / privacy mode — ignore */
  }
}

/**
 * Provider credentials an older build left in this browser, or `null` when
 * there are none. Returned so the UI can offer to move them to the server
 * before deleting them, rather than silently discarding a key the user may
 * not have written down anywhere else.
 */
export function readLegacyProviderSecrets(): LegacyProviderSecrets | null {
  if (typeof window === "undefined") return null;
  try {
    const raw = window.localStorage.getItem(PROVIDER_CONFIG_STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    const found: LegacyProviderSecrets = {};
    for (const field of LEGACY_PROVIDER_FIELDS) {
      const value = parsed[field];
      if (typeof value === "string" && value.trim()) found[field] = value.trim();
    }
    return Object.keys(found).length > 0 ? found : null;
  } catch {
    return null;
  }
}

/** Delete the legacy provider block, keeping the transport fields intact. */
export function purgeLegacyProviderSecrets(): void {
  if (typeof window === "undefined") return;
  saveProviderConfig(loadProviderConfig());
}
