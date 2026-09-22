// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Rendering tests of the Settings page's "Diagnostique" tab (the seven
// self-checks and their summary banner) and "Feedback" tab (list, detail
// pane, delete).

import { beforeEach, describe, expect, it, vi } from "vitest";
import Settings from "./Settings";
import { renderPage, screen, waitFor, within } from "../test/render";
import type { Feedback, Settings as ServerSettings, Stats } from "../api";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return {
    ...actual,
    getSettings: vi.fn(),
    patchSettings: vi.fn(),
    getOcrStatus: vi.fn(),
    resetAll: vi.fn(),
    getStats: vi.fn(),
    listFeedbacks: vi.fn(),
    deleteFeedback: vi.fn(),
    testLlm: vi.fn(),
    listLlmModels: vi.fn(),
    listInfomaniakProducts: vi.fn(),
  };
});

import * as api from "../api";

const mocked = api as unknown as {
  getSettings: ReturnType<typeof vi.fn>;
  getOcrStatus: ReturnType<typeof vi.fn>;
  getStats: ReturnType<typeof vi.fn>;
  listFeedbacks: ReturnType<typeof vi.fn>;
  deleteFeedback: ReturnType<typeof vi.fn>;
};

function makeSettings(active_provider = "default"): ServerSettings {
  return {
    retrieval: { top_k: 8, lexical_weight: 0.5, expansion_depth: 1 },
    ui: { theme: "light", graph_layout: "dagre" },
    llm: {
      active_provider,
      openai_api_key_hint: "",
      openai_base_url: "",
      openai_model: "",
      anthropic_api_key_hint: "",
      anthropic_base_url: "",
      anthropic_model: "",
      infomaniak_api_key_hint: "",
      infomaniak_product_id: "",
      infomaniak_base_url: "",
      infomaniak_model: "",
      temperature: 0.7,
      max_tokens: 2048,
    },
    ocr: {
      provider: "tesseract",
      auto_fallback: false,
      min_text_threshold: 200,
      tesseract_languages: "fra+eng",
      google_api_key_hint: "",
    },
  };
}

const stats = (concepts: number, relations: number): Stats => ({
  concepts,
  relations,
  rules: 3,
  actions: 4,
  concept_types: 5,
  relation_types: 6,
  rule_types: 7,
  action_types: 8,
  deltas: { concepts_pct: 0, relations_pct: 0, concept_types_pct: 0, relation_types_pct: 0 },
});

/** A `fetch` that answers `/healthz` and `/settings/llm/test` like a healthy server. */
function healthyFetch(llmBody: unknown = { ok: true, model: "gpt-4o" }, llmOk = true, healthStatus = 200) {
  return vi.fn((url: string) => {
    if (url.endsWith("/healthz")) {
      return Promise.resolve({ ok: healthStatus < 400, status: healthStatus, text: () => Promise.resolve("ok\n") });
    }
    return Promise.resolve({
      ok: llmOk,
      status: llmOk ? 200 : 500,
      json: () => (llmBody instanceof Error ? Promise.reject(llmBody) : Promise.resolve(llmBody)),
    });
  });
}

async function openDiagnostics() {
  const page = renderPage(<Settings />);
  await page.user.click(screen.getByRole("button", { name: /Diagnostique/ }));
  return page;
}

/** The DiagRow element for a named check. */
function row(name: string): HTMLElement {
  return screen.getByText(name).closest("div[style*='cursor']")!.parentElement!;
}

beforeEach(() => {
  mocked.getSettings.mockResolvedValue(makeSettings());
  mocked.getOcrStatus.mockResolvedValue({
    tesseract: { available: false, ghostscript_available: false },
    google_vision: { configured: false, auth: "api_key" },
  });
  mocked.getStats.mockResolvedValue(stats(12, 34));
  mocked.listFeedbacks.mockResolvedValue([]);
  mocked.deleteFeedback.mockResolvedValue(undefined);
});

describe("Diagnostics tab", () => {
  it("runs every check on mount and reports the failures", async () => {
    // Default harness: fetch rejects, no JWT, no provider config, stats fail.
    mocked.getStats.mockRejectedValue(new Error("stats offline"));
    await openDiagnostics();
    expect(await screen.findByText("Erreurs détectées")).toBeInTheDocument();
    expect(screen.getByText(/2 OK · 3 avertissements · 2 erreurs · \d+ ms/)).toBeInTheDocument();

    expect(row("Serveur ontologie")).toHaveTextContent("Injoignable");
    expect(row("Serveur ontologie")).toHaveTextContent("✕");
    expect(row("Paramètres serveur")).toHaveTextContent("Settings chargés");
    expect(row("Données")).toHaveTextContent("Impossible de charger les statistiques");
    expect(row("LLM (openai)")).toHaveTextContent("Non configuré ou injoignable");
    expect(row("LLM (openai)")).toHaveTextContent("⚠");
    expect(row("Authentification")).toHaveTextContent("Aucun JWT — accès anonyme");
    expect(row("Configuration locale")).toHaveTextContent("providerConfig vide");
    expect(row("Stockage navigateur")).toHaveTextContent("localStorage accessible");

    // Environment tiles.
    expect(screen.getByText("Environnement").nextElementSibling).toHaveTextContent("test");
    expect(screen.getByText("Dernière vérification").nextElementSibling).not.toHaveTextContent("—");
    expect(screen.getByText(/User-Agent:/)).toBeInTheDocument();
  });

  it("passes every check against a healthy server with a JWT and a stored config", async () => {
    const fetchMock = healthyFetch();
    vi.stubGlobal("fetch", fetchMock);
    window.localStorage.setItem("msBE.token", "jwt-abc");
    window.localStorage.setItem(
      "ontology.providerConfig",
      JSON.stringify({ ontologyApiUrl: "http://api.test:5000", ontologyBearerToken: "", authApiUrl: "" }),
    );
    mocked.getSettings.mockResolvedValue(makeSettings("anthropic"));
    const { user } = await openDiagnostics();
    expect(await screen.findByText("Tous les contrôles sont passés")).toBeInTheDocument();
    expect(screen.getByText(/7 OK · 0 avertissement · 0 erreur/)).toBeInTheDocument();
    expect(screen.getByText("Tous les contrôles sont passés").parentElement).toHaveClass("success-banner");

    expect(row("Serveur ontologie")).toHaveTextContent("Connexion active (ok)");
    expect(row("Données")).toHaveTextContent("12 concepts, 34 relations, 3 règles, 4 actions");
    expect(row("LLM (anthropic)")).toHaveTextContent("Connexion OK (gpt-4o)");
    expect(row("Authentification")).toHaveTextContent("JWT présent dans le navigateur");
    expect(row("Configuration locale")).toHaveTextContent("providerConfig chargé");
    expect(screen.getByText("API base").nextElementSibling).toHaveTextContent("http://api.test:5000");

    expect(fetchMock).toHaveBeenCalledWith("http://api.test:5000/healthz");
    expect(fetchMock).toHaveBeenCalledWith("http://api.test:5000/settings/llm/test", {
      method: "POST",
      headers: { "content-type": "application/json", authorization: "Bearer jwt-abc" },
      body: JSON.stringify({ provider: "anthropic" }),
    });

    // Expanding a row reveals its details; collapsing hides them again.
    await user.click(screen.getByText("Paramètres serveur"));
    expect(screen.getByText("provider actif: anthropic")).toBeInTheDocument();
    expect(row("Paramètres serveur")).toHaveTextContent("▾");
    await user.click(screen.getByText("Authentification"));
    expect(screen.getByText("Longueur du token: 7 caractères")).toBeInTheDocument();
    await user.click(screen.getByText("Serveur ontologie"));
    expect(screen.getByText("GET http://api.test:5000/healthz → 200")).toBeInTheDocument();
    await user.click(screen.getByText("Données"));
    expect(screen.getByText("Types — concepts:5, relations:6, rules:7, actions:8")).toBeInTheDocument();
    await user.click(screen.getByText("Paramètres serveur"));
    expect(screen.queryByText("provider actif: anthropic")).not.toBeInTheDocument();
    expect(row("Paramètres serveur")).toHaveTextContent("▸");
    // A row without details opens to nothing.
    await user.click(screen.getByText("Stockage navigateur"));
    expect(row("Stockage navigateur").querySelector("pre")).toBeNull();
  });

  it("downgrades to warnings for an empty store, no JWT and a rejected LLM probe", async () => {
    vi.stubGlobal("fetch", healthyFetch({ ok: false, error: "no key configured" }, false));
    mocked.getStats.mockResolvedValue(stats(0, 0));
    const { user } = await openDiagnostics();
    expect(await screen.findByText("Avertissements détectés")).toBeInTheDocument();
    expect(screen.getByText(/3 OK · 4 avertissements · 0 erreur · \d+ ms/)).toBeInTheDocument();
    expect(screen.getByText("Avertissements détectés").parentElement).toHaveClass("warn-banner");
    expect(row("Données")).toHaveTextContent("0 concepts, 0 relations");
    await user.click(screen.getByText("LLM (openai)"));
    expect(screen.getByText("no key configured")).toBeInTheDocument();
    await user.click(screen.getByText("Authentification"));
    expect(screen.getByText("Connectez-vous pour activer les routes protégées.")).toBeInTheDocument();
  });

  it("falls back to the HTTP status when the LLM probe answers non-JSON, and to openai when settings fail", async () => {
    vi.stubGlobal("fetch", healthyFetch(new Error("not json"), false, 502));
    mocked.getSettings.mockRejectedValue(new Error("settings 500"));
    const { user } = await openDiagnostics();
    expect(await screen.findByText("Erreurs détectées")).toBeInTheDocument();
    // A reachable server answering non-2xx is still "Injoignable", with the status as detail.
    expect(row("Serveur ontologie")).toHaveTextContent("Injoignable");
    await user.click(screen.getByText("Serveur ontologie"));
    expect(screen.getByText("HTTP 502")).toBeInTheDocument();
    expect(row("Paramètres serveur")).toHaveTextContent("Échec du chargement");
    await user.click(screen.getByText("Paramètres serveur"));
    expect(screen.getByText("settings 500")).toBeInTheDocument();
    await user.click(screen.getByText("LLM (openai)"));
    expect(screen.getByText("HTTP 500")).toBeInTheDocument();
  });

  it("reports an unusable localStorage and ignores a corrupt provider config", async () => {
    window.localStorage.setItem("ontology.providerConfig", "{not json");
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("quota exceeded");
    });
    await openDiagnostics();
    expect(await screen.findByText("Erreurs détectées")).toBeInTheDocument();
    expect(row("Stockage navigateur")).toHaveTextContent("localStorage indisponible");
    expect(row("Configuration locale")).toHaveTextContent("providerConfig vide");
  });

  it("re-runs on demand and shows the in-progress state meanwhile", async () => {
    const { user } = await openDiagnostics();
    expect(await screen.findByText("Erreurs détectées")).toBeInTheDocument();
    expect(mocked.getSettings).toHaveBeenCalledTimes(2); // page + first diagnostic run

    // Only the first probe (`/healthz`) is held back; the LLM probe fails fast.
    let release!: (v: unknown) => void;
    const held = new Promise((resolve) => { release = resolve; });
    vi.stubGlobal(
      "fetch",
      vi.fn((url: string) => (url.endsWith("/healthz") ? held : Promise.reject(new Error("offline")))),
    );
    await user.click(screen.getByRole("button", { name: /Relancer/ }));
    expect(screen.getByRole("button", { name: "Analyse…" })).toBeDisabled();
    expect(screen.getByText("Analyse en cours…")).toBeInTheDocument();
    expect(screen.queryByText("Erreurs détectées")).not.toBeInTheDocument();
    release({ ok: true, status: 200, text: () => Promise.resolve("fine") });
    expect(await screen.findByText("Connexion active (fine)")).toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole("button", { name: /Relancer/ })).toBeEnabled());
    expect(mocked.getSettings).toHaveBeenCalledTimes(3);
  });
});

describe("Feedback tab", () => {
  const items: Feedback[] = [
    {
      id: 1,
      created_at: 1_700_000_000,
      kind: "bug",
      title: "Crash on upload",
      description: "Steps:\n1. upload\n2. boom",
      screenshot: "data:image/png;base64,AAAA",
      frontend_logs: "l1\nl2\nl3",
      backend_logs: "server line",
      user_agent: "TestBrowser/1.0",
      url: "http://app.test/ingest",
    },
    { id: 2, created_at: 1_700_000_100, kind: "error", title: "500 on save", description: "", frontend_logs: "", backend_logs: "" },
    { id: 3, created_at: 1_700_000_200, kind: "evolution", title: "Dark mode", description: "", frontend_logs: "", backend_logs: "" },
    { id: 4, created_at: 1_700_000_300, kind: "improvement", title: "Faster graph", description: "", frontend_logs: "", backend_logs: "" },
    { id: 5, created_at: 1_700_000_400, kind: "question", title: "How to export?", description: "", frontend_logs: "", backend_logs: "" },
  ];

  async function openFeedback() {
    const page = renderPage(<Settings />);
    await page.user.click(screen.getByRole("button", { name: /Feedback/ }));
    return page;
  }

  it("shows the loading state, then the empty state", async () => {
    let release!: (v: Feedback[]) => void;
    mocked.listFeedbacks.mockImplementationOnce(() => new Promise((resolve) => { release = resolve; }));
    await openFeedback();
    expect(screen.getByText("Chargement…")).toBeInTheDocument();
    release([]);
    expect(await screen.findByText("Aucun feedback pour l'instant.")).toBeInTheDocument();
  });

  it("shows a load error", async () => {
    mocked.listFeedbacks.mockRejectedValueOnce(new Error("feedback store down"));
    await openFeedback();
    expect(await screen.findByText("feedback store down")).toBeInTheDocument();
  });

  it("lists every feedback with a kind badge and a formatted date", async () => {
    mocked.listFeedbacks.mockResolvedValue(items);
    await openFeedback();
    expect(await screen.findByText("Crash on upload")).toBeInTheDocument();
    expect(screen.getByText("🐞 Bug")).toBeInTheDocument();
    expect(screen.getByText("⚠ Erreur")).toBeInTheDocument();
    expect(screen.getByText("✦ Évolution")).toBeInTheDocument();
    expect(screen.getByText("💡 Amélioration")).toBeInTheDocument();
    // Unknown kinds fall back to the raw value.
    expect(screen.getByText("question")).toBeInTheDocument();
    expect(screen.getAllByRole("row")).toHaveLength(6); // header + 5
    expect(screen.getByText(new Date(1_700_000_000 * 1000).toLocaleString())).toBeInTheDocument();
  });

  it("opens the detail pane with every optional section, then closes it", async () => {
    mocked.listFeedbacks.mockResolvedValue(items);
    const { user } = await openFeedback();
    await user.click(await screen.findByText("Crash on upload"));
    expect(screen.getByRole("heading", { level: 3, name: "Crash on upload" })).toBeInTheDocument();
    expect(screen.getByText(/Steps:/)).toHaveTextContent("Steps: 1. upload 2. boom");
    expect(screen.getByText("http://app.test/ingest", { selector: "code" })).toBeInTheDocument();
    expect(screen.getByText("TestBrowser/1.0", { selector: "code" })).toBeInTheDocument();
    expect(screen.getByRole("img", { name: "screenshot" })).toHaveAttribute("src", "data:image/png;base64,AAAA");
    expect(screen.getByText("Logs frontend (3 lignes)")).toBeInTheDocument();
    expect(screen.getByText("Logs backend (1 lignes)")).toBeInTheDocument();
    // Two badges now: the row and the pane.
    expect(screen.getAllByText("🐞 Bug")).toHaveLength(2);

    // Switching to a bare item hides the optional sections.
    await user.click(screen.getByText("500 on save"));
    expect(screen.getByRole("heading", { level: 3, name: "500 on save" })).toBeInTheDocument();
    expect(screen.queryByRole("img")).not.toBeInTheDocument();
    expect(screen.queryByText(/Logs frontend/)).not.toBeInTheDocument();
    expect(screen.queryByText(/User-Agent :/)).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "×" }));
    expect(screen.queryByRole("heading", { level: 3 })).not.toBeInTheDocument();
  });

  it("deletes a feedback without opening it, clears the selection and reloads", async () => {
    let remaining = items;
    mocked.listFeedbacks.mockImplementation(() => Promise.resolve(remaining));
    mocked.deleteFeedback.mockImplementation((id: number) => {
      remaining = remaining.filter((f) => f.id !== id);
      return Promise.resolve(undefined);
    });
    const { user } = await openFeedback();
    await user.click(await screen.findByText("Crash on upload"));
    expect(screen.getByRole("heading", { level: 3, name: "Crash on upload" })).toBeInTheDocument();

    const firstRow = screen.getByRole("cell", { name: "Crash on upload" }).closest("tr")!;
    await user.click(within(firstRow).getByTitle("Supprimer"));
    await waitFor(() => expect(mocked.deleteFeedback).toHaveBeenCalledWith(1));
    await waitFor(() =>
      expect(screen.queryByRole("cell", { name: "Crash on upload" })).not.toBeInTheDocument(),
    );
    expect(screen.queryByRole("heading", { level: 3 })).not.toBeInTheDocument();
    expect(mocked.listFeedbacks).toHaveBeenCalledTimes(2);
    expect(screen.getByText("500 on save")).toBeInTheDocument();

    // Deleting a non-selected row keeps the current selection.
    await user.click(screen.getByText("Dark mode"));
    await user.click(within(screen.getByText("500 on save").closest("tr")!).getByTitle("Supprimer"));
    await waitFor(() => expect(mocked.deleteFeedback).toHaveBeenCalledWith(2));
    await waitFor(() => expect(screen.queryByText("500 on save")).not.toBeInTheDocument());
    expect(screen.getByRole("heading", { level: 3, name: "Dark mode" })).toBeInTheDocument();
    expect(mocked.listFeedbacks).toHaveBeenCalledTimes(3);
  });

  it("shows a delete failure in the banner", async () => {
    mocked.listFeedbacks.mockResolvedValue(items);
    mocked.deleteFeedback.mockRejectedValueOnce(new Error("forbidden"));
    const { user } = await openFeedback();
    await screen.findByText("Crash on upload");
    await user.click(screen.getAllByTitle("Supprimer")[4]);
    expect(await screen.findByText("forbidden")).toBeInTheDocument();
    // The row click was not propagated by the delete button.
    expect(screen.queryByRole("heading", { level: 3 })).not.toBeInTheDocument();
    mocked.deleteFeedback.mockRejectedValueOnce("nope");
    await user.click(screen.getAllByTitle("Supprimer")[3]);
    expect(await screen.findByText("nope")).toBeInTheDocument();
  });
});
