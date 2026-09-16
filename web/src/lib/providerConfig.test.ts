// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { describe, expect, it } from "vitest";
import {
  loadProviderConfig,
  purgeLegacyProviderSecrets,
  readLegacyProviderSecrets,
  saveProviderConfig,
} from "./providerConfig";
import {
  DEFAULT_PROVIDER_CONFIG,
  LEGACY_PROVIDER_FIELDS,
  PROVIDER_CONFIG_STORAGE_KEY,
} from "../types/providerConfig";

const ls = () => window.localStorage;
const stored = () => JSON.parse(ls().getItem(PROVIDER_CONFIG_STORAGE_KEY) ?? "null");

describe("types/providerConfig — constants", () => {
  it("defaults to the local dev servers with no bearer token", () => {
    expect(DEFAULT_PROVIDER_CONFIG).toEqual({
      ontologyApiUrl: "http://localhost:5000",
      ontologyBearerToken: "",
      authApiUrl: "http://localhost:4000",
    });
  });

  it("stores under the 'ontology.providerConfig' key", () => {
    expect(PROVIDER_CONFIG_STORAGE_KEY).toBe("ontology.providerConfig");
  });

  it("lists the six legacy provider fields in UI order", () => {
    expect(LEGACY_PROVIDER_FIELDS).toEqual([
      "activeLLMProvider",
      "openaiKey",
      "anthropicKey",
      "infomaniakKey",
      "infomaniakBaseUrl",
      "infomaniakModel",
    ]);
  });
});

describe("loadProviderConfig", () => {
  it("returns the defaults when nothing is stored", () => {
    expect(loadProviderConfig()).toEqual(DEFAULT_PROVIDER_CONFIG);
  });

  it("returns a fresh copy, never the shared DEFAULT_PROVIDER_CONFIG object", () => {
    const cfg = loadProviderConfig();
    expect(cfg).not.toBe(DEFAULT_PROVIDER_CONFIG);
    cfg.ontologyApiUrl = "mutated";
    expect(DEFAULT_PROVIDER_CONFIG.ontologyApiUrl).toBe("http://localhost:5000");
  });

  it("returns the defaults when the stored value is corrupt JSON", () => {
    ls().setItem(PROVIDER_CONFIG_STORAGE_KEY, "{not json");
    expect(loadProviderConfig()).toEqual(DEFAULT_PROVIDER_CONFIG);
  });

  it("returns the defaults when the stored JSON is the literal null", () => {
    ls().setItem(PROVIDER_CONFIG_STORAGE_KEY, "null");
    expect(loadProviderConfig()).toEqual(DEFAULT_PROVIDER_CONFIG);
  });

  it("merges a partial stored object over the defaults", () => {
    ls().setItem(PROVIDER_CONFIG_STORAGE_KEY, JSON.stringify({ ontologyApiUrl: "https://api.example" }));
    expect(loadProviderConfig()).toEqual({
      ontologyApiUrl: "https://api.example",
      ontologyBearerToken: "",
      authApiUrl: "http://localhost:4000",
    });
  });

  it("keeps an explicitly empty string (only null/undefined fall back to defaults)", () => {
    ls().setItem(PROVIDER_CONFIG_STORAGE_KEY, JSON.stringify({ ontologyApiUrl: "", authApiUrl: null }));
    const cfg = loadProviderConfig();
    expect(cfg.ontologyApiUrl).toBe("");
    expect(cfg.authApiUrl).toBe("http://localhost:4000");
  });

  it("drops legacy provider fields from the returned object", () => {
    ls().setItem(
      PROVIDER_CONFIG_STORAGE_KEY,
      JSON.stringify({ ontologyApiUrl: "http://x", openaiKey: "sk-old", activeLLMProvider: "openai" }),
    );
    expect(Object.keys(loadProviderConfig()).sort()).toEqual(["authApiUrl", "ontologyApiUrl", "ontologyBearerToken"]);
  });
});

describe("saveProviderConfig", () => {
  it("persists exactly the three transport fields", () => {
    saveProviderConfig({
      ontologyApiUrl: "http://a:1",
      ontologyBearerToken: "tok",
      authApiUrl: "http://b:2",
      // legacy junk that must not survive a save
      ...({ openaiKey: "sk-old" } as object),
    });
    expect(stored()).toEqual({ ontologyApiUrl: "http://a:1", ontologyBearerToken: "tok", authApiUrl: "http://b:2" });
  });

  it("mirrors the ontology URL to legacy 'ontology.apiBase' trimmed and without trailing slash", () => {
    saveProviderConfig({ ...DEFAULT_PROVIDER_CONFIG, ontologyApiUrl: "  https://api.example/  " });
    expect(ls().getItem("ontology.apiBase")).toBe("https://api.example");
    // the stored config keeps the value exactly as entered
    expect(stored().ontologyApiUrl).toBe("  https://api.example/  ");
  });

  it("removes the legacy 'ontology.apiBase' key when the URL is blank", () => {
    ls().setItem("ontology.apiBase", "http://stale");
    saveProviderConfig({ ...DEFAULT_PROVIDER_CONFIG, ontologyApiUrl: "   " });
    expect(ls().getItem("ontology.apiBase")).toBeNull();
  });

  it("mirrors the trimmed bearer token to legacy 'ontology.apiToken'", () => {
    saveProviderConfig({ ...DEFAULT_PROVIDER_CONFIG, ontologyBearerToken: "  secret " });
    expect(ls().getItem("ontology.apiToken")).toBe("secret");
  });

  it("removes the legacy 'ontology.apiToken' key when the token is blank", () => {
    ls().setItem("ontology.apiToken", "stale");
    saveProviderConfig({ ...DEFAULT_PROVIDER_CONFIG, ontologyBearerToken: "" });
    expect(ls().getItem("ontology.apiToken")).toBeNull();
  });

  it("round-trips through loadProviderConfig", () => {
    const cfg = { ontologyApiUrl: "http://a:1", ontologyBearerToken: "t", authApiUrl: "http://b:2" };
    saveProviderConfig(cfg);
    expect(loadProviderConfig()).toEqual(cfg);
  });
});

describe("readLegacyProviderSecrets", () => {
  it("returns null when nothing is stored or the store is corrupt", () => {
    expect(readLegacyProviderSecrets()).toBeNull();
    ls().setItem(PROVIDER_CONFIG_STORAGE_KEY, "{oops");
    expect(readLegacyProviderSecrets()).toBeNull();
  });

  it("returns null when only transport fields are stored", () => {
    saveProviderConfig(DEFAULT_PROVIDER_CONFIG);
    expect(readLegacyProviderSecrets()).toBeNull();
  });

  it("returns only the non-blank string legacy fields, trimmed", () => {
    ls().setItem(
      PROVIDER_CONFIG_STORAGE_KEY,
      JSON.stringify({
        ontologyApiUrl: "http://x",
        openaiKey: "  sk-a  ",
        anthropicKey: "   ",
        infomaniakKey: 42,
        infomaniakModel: "llama",
        unrelated: "ignored",
      }),
    );
    expect(readLegacyProviderSecrets()).toEqual({ openaiKey: "sk-a", infomaniakModel: "llama" });
  });
});

describe("purgeLegacyProviderSecrets", () => {
  it("deletes the legacy block while preserving the transport fields", () => {
    ls().setItem(
      PROVIDER_CONFIG_STORAGE_KEY,
      JSON.stringify({
        ontologyApiUrl: "http://keep:5000",
        ontologyBearerToken: "keep-tok",
        authApiUrl: "http://keep:4000",
        openaiKey: "sk-old",
        activeLLMProvider: "openai",
      }),
    );
    purgeLegacyProviderSecrets();
    expect(stored()).toEqual({
      ontologyApiUrl: "http://keep:5000",
      ontologyBearerToken: "keep-tok",
      authApiUrl: "http://keep:4000",
    });
    expect(readLegacyProviderSecrets()).toBeNull();
    expect(ls().getItem("ontology.apiBase")).toBe("http://keep:5000");
    expect(ls().getItem("ontology.apiToken")).toBe("keep-tok");
  });

  it("is a no-op that writes defaults when nothing was stored", () => {
    purgeLegacyProviderSecrets();
    expect(stored()).toEqual(DEFAULT_PROVIDER_CONFIG);
  });
});
