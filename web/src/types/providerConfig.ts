// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Client-side connection settings stored under
// `localStorage["ontology.providerConfig"]`.
//
// This store holds **transport configuration only** — which backend to talk to
// and with what bearer token. Provider credentials (OpenAI / Anthropic /
// Infomaniak keys, base URLs, model names) used to live here too, which meant
// two competing sources of truth and API keys sitting in plaintext in every
// browser that had ever opened the app. They now live exclusively in the
// server's settings store, reachable through `GET`/`PATCH /settings`, and the
// server never returns a raw key.
//
// Keys written by an older build are detected and cleaned up by
// `readLegacyProviderSecrets` / `purgeLegacyProviderSecrets`.

/** Providers the backend can route a request to. */
export type LLMProvider = "default" | "openai" | "anthropic" | "infomaniak";

export interface ProviderConfig {
  /** Ontology API base URL. Mirrored to legacy `ontology.apiBase`. */
  ontologyApiUrl: string;
  /** Optional bearer token for the ontology API. Mirrored to legacy `ontology.apiToken`. */
  ontologyBearerToken: string;
  /** Auth server URL (msBE). */
  authApiUrl: string;
}

export const DEFAULT_PROVIDER_CONFIG: ProviderConfig = {
  ontologyApiUrl: "http://localhost:5000",
  ontologyBearerToken: "",
  authApiUrl: "http://localhost:4000",
};

export const PROVIDER_CONFIG_STORAGE_KEY = "ontology.providerConfig";

/**
 * Provider fields an older build kept in localStorage. Read once so the user
 * can move them to the server, then deleted — see `readLegacyProviderSecrets`.
 */
export interface LegacyProviderSecrets {
  activeLLMProvider?: string;
  openaiKey?: string;
  anthropicKey?: string;
  infomaniakKey?: string;
  infomaniakBaseUrl?: string;
  infomaniakModel?: string;
}

/** Field names of the legacy provider block, in the order the UI reports them. */
export const LEGACY_PROVIDER_FIELDS: Array<keyof LegacyProviderSecrets> = [
  "activeLLMProvider",
  "openaiKey",
  "anthropicKey",
  "infomaniakKey",
  "infomaniakBaseUrl",
  "infomaniakModel",
];
