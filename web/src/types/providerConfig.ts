// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Centralized provider/keys configuration stored under
// `localStorage["ontology.providerConfig"]`.
//
// NOTE: Field shape inferred from PART 2 of the implementation spec (Server
// URLs sub-section, Provider & Keys section, Infomaniak wiring). If Part 1
// of the spec adds further fields, extend this interface — `loadProviderConfig`
// merges with `DEFAULT_PROVIDER_CONFIG` so missing fields are safe.

export type LLMProvider = "default" | "openai" | "anthropic" | "infomaniak";

export interface ProviderConfig {
  /** Ontology API base URL. Mirrored to legacy `ontology.apiBase`. */
  ontologyApiUrl: string;
  /** Optional bearer token for the ontology API. Mirrored to legacy `ontology.apiToken`. */
  ontologyBearerToken: string;
  /** Auth server URL (msBE). */
  authApiUrl: string;

  /** Active LLM provider — drives IngestWizard / OntologyBuilder defaults. */
  activeLLMProvider: LLMProvider;

  /** API keys (plaintext in localStorage — only enter on trusted devices). */
  openaiKey: string;
  anthropicKey: string;
  infomaniakKey: string;

  /** Infomaniak custom base URL + optional default model. */
  infomaniakBaseUrl: string;
  infomaniakModel: string;
}

export const DEFAULT_PROVIDER_CONFIG: ProviderConfig = {
  ontologyApiUrl: "http://localhost:5000",
  ontologyBearerToken: "",
  authApiUrl: "http://localhost:4000",
  activeLLMProvider: "default",
  openaiKey: "",
  anthropicKey: "",
  infomaniakKey: "",
  infomaniakBaseUrl: "https://api.infomaniak.com/1/ai",
  infomaniakModel: "",
};

export const PROVIDER_CONFIG_STORAGE_KEY = "ontology.providerConfig";
