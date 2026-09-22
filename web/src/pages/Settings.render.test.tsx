// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Rendering tests of the Settings page, "Général" tab: tab switching, the
// retrieval / UI preference rows, the browser-side server connection card
// (save + `/healthz` probe), the legacy-keys migration banner and the danger
// zone (reset behind the confirm dialog). The LLM and OCR cards have their own
// file (`Settings.llm.render.test.tsx`); the Diagnostics and Feedback tabs too
// (`Settings.diagnostics.render.test.tsx`).

import { beforeEach, describe, expect, it, vi } from "vitest";
import Settings from "./Settings";
import { fireEvent, renderPage, screen, waitFor } from "../test/render";
import type { LlmSettings, OcrSettings, Settings as ServerSettings, SettingsPatch } from "../api";

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
  patchSettings: ReturnType<typeof vi.fn>;
  getOcrStatus: ReturnType<typeof vi.fn>;
  resetAll: ReturnType<typeof vi.fn>;
  getStats: ReturnType<typeof vi.fn>;
  listFeedbacks: ReturnType<typeof vi.fn>;
};

function makeSettings(over: Partial<ServerSettings> = {}): ServerSettings {
  return {
    retrieval: { top_k: 8, lexical_weight: 0.5, expansion_depth: 1 },
    ui: { theme: "light", graph_layout: "dagre" },
    llm: {
      active_provider: "default",
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
    ...over,
  };
}

/** Shallow-merge a patch the way the server would echo it back. */
function mergeSettings(base: ServerSettings, patch: SettingsPatch): ServerSettings {
  return {
    retrieval: { ...base.retrieval, ...patch.retrieval },
    ui: { ...base.ui, ...patch.ui },
    llm: { ...base.llm, ...(patch.llm as Partial<LlmSettings>) },
    ocr: { ...base.ocr, ...(patch.ocr as Partial<OcrSettings>) },
  };
}

/** A minimal `Response` for the `/healthz` probe. */
function healthz(
  body: unknown,
  init: { ok?: boolean; status?: number; statusText?: string; jsonFails?: boolean } = {},
) {
  return {
    ok: init.ok ?? true,
    status: init.status ?? 200,
    statusText: init.statusText ?? "OK",
    json: () => (init.jsonFails ? Promise.reject(new Error("not json")) : Promise.resolve(body)),
    text: () => Promise.resolve(typeof body === "string" ? body : JSON.stringify(body)),
  };
}

beforeEach(() => {
  const base = makeSettings();
  mocked.getSettings.mockResolvedValue(base);
  mocked.patchSettings.mockImplementation((patch: SettingsPatch) =>
    Promise.resolve(mergeSettings(base, patch)),
  );
  mocked.getOcrStatus.mockResolvedValue({
    tesseract: { available: true, ocrmypdf_version: "16.1", ghostscript_available: true },
    google_vision: { configured: false, auth: "api_key" },
  });
  mocked.resetAll.mockResolvedValue(undefined);
  // Only reached by the tab-switching test; the panels have their own file.
  mocked.getStats.mockRejectedValue(new Error("stats offline"));
  mocked.listFeedbacks.mockResolvedValue([]);
});

describe("Settings page — shell and tabs", () => {
  it("shows the header, loads the server settings and fills the retrieval rows", async () => {
    renderPage(<Settings />);
    expect(screen.getByRole("heading", { name: "Settings" })).toBeInTheDocument();
    // Both cards show a loading placeholder until `getSettings` resolves.
    expect(screen.getAllByText("Loading…")).toHaveLength(2);
    expect(await screen.findByDisplayValue("8")).toBeInTheDocument();
    expect(screen.getByDisplayValue("0.5")).toBeInTheDocument();
    expect(screen.getByDisplayValue("1")).toBeInTheDocument();
    expect(screen.getByDisplayValue("Light")).toBeInTheDocument();
    expect(screen.getByDisplayValue("Dagre (hierarchical)")).toBeInTheDocument();
    expect(mocked.getSettings).toHaveBeenCalledTimes(1);
    expect(screen.queryByText("Loading…")).not.toBeInTheDocument();
  });

  it("shows the load error in the banner and keeps the placeholders", async () => {
    mocked.getSettings.mockRejectedValueOnce(new Error("settings unavailable"));
    renderPage(<Settings />);
    expect(await screen.findByText("settings unavailable")).toBeInTheDocument();
    expect(screen.getAllByText("Loading…")).toHaveLength(2);
    // Without settings neither the LLM nor the OCR card is mounted.
    expect(screen.queryByText("Moteur OCR")).not.toBeInTheDocument();
    expect(mocked.getOcrStatus).not.toHaveBeenCalled();
  });

  it("stringifies a non-Error rejection", async () => {
    mocked.getSettings.mockRejectedValueOnce("boom");
    renderPage(<Settings />);
    expect(await screen.findByText("boom")).toBeInTheDocument();
  });

  it("switches between the three tabs", async () => {
    const { user } = renderPage(<Settings />);
    await screen.findByDisplayValue("8");
    const general = screen.getByRole("button", { name: /Général/ });
    const diag = screen.getByRole("button", { name: /Diagnostique/ });
    const feedback = screen.getByRole("button", { name: /Feedback/ });
    expect(general).toHaveClass("btn-primary");
    expect(diag).toHaveClass("btn-ghost");

    await user.click(diag);
    expect(diag).toHaveClass("btn-primary");
    expect(screen.getByText("Auto-diagnostic de l'application")).toBeInTheDocument();
    expect(screen.queryByText("Retrieval defaults")).not.toBeInTheDocument();

    await user.click(feedback);
    expect(await screen.findByText("Feedback reçus")).toBeInTheDocument();
    expect(screen.queryByText("Auto-diagnostic de l'application")).not.toBeInTheDocument();

    await user.click(general);
    expect(await screen.findByText("Retrieval defaults")).toBeInTheDocument();
  });
});

describe("Settings page — retrieval defaults and UI preferences", () => {
  it("patches each retrieval field and confirms with the success banner", async () => {
    renderPage(<Settings />);
    const [topK, lexical, depth] = await screen.findAllByRole("spinbutton");
    fireEvent.change(topK, { target: { value: "12" } });
    await waitFor(() =>
      expect(mocked.patchSettings).toHaveBeenCalledWith({ retrieval: { top_k: 12 } }),
    );
    expect(await screen.findByText("Settings saved.")).toBeInTheDocument();
    expect(screen.getByDisplayValue("12")).toBeInTheDocument();

    fireEvent.change(lexical, { target: { value: "0.9" } });
    await waitFor(() =>
      expect(mocked.patchSettings).toHaveBeenCalledWith({ retrieval: { lexical_weight: 0.9 } }),
    );
    fireEvent.change(depth, { target: { value: "3" } });
    await waitFor(() =>
      expect(mocked.patchSettings).toHaveBeenCalledWith({ retrieval: { expansion_depth: 3 } }),
    );
  });

  it("patches the theme and the graph layout", async () => {
    const { user } = renderPage(<Settings />);
    const theme = await screen.findByDisplayValue("Light");
    await user.selectOptions(theme, "dark");
    await waitFor(() => expect(mocked.patchSettings).toHaveBeenCalledWith({ ui: { theme: "dark" } }));
    expect(await screen.findByDisplayValue("Dark")).toBeInTheDocument();

    await user.selectOptions(screen.getByDisplayValue("Dagre (hierarchical)"), "force");
    await waitFor(() =>
      expect(mocked.patchSettings).toHaveBeenCalledWith({ ui: { graph_layout: "force" } }),
    );
    expect(await screen.findByDisplayValue("Force-directed")).toBeInTheDocument();
  });

  it("shows a patch failure in the error banner and clears the previous success", async () => {
    renderPage(<Settings />);
    const [topK] = await screen.findAllByRole("spinbutton");
    fireEvent.change(topK, { target: { value: "9" } });
    expect(await screen.findByText("Settings saved.")).toBeInTheDocument();

    mocked.patchSettings.mockRejectedValueOnce(new Error("write denied"));
    fireEvent.change(topK, { target: { value: "10" } });
    expect(await screen.findByText("write denied")).toBeInTheDocument();
    expect(screen.queryByText("Settings saved.")).not.toBeInTheDocument();

    mocked.patchSettings.mockRejectedValueOnce(42);
    fireEvent.change(topK, { target: { value: "11" } });
    expect(await screen.findByText("42")).toBeInTheDocument();
  });
});

describe("Settings page — server connection card", () => {
  it("starts from the defaults and persists the edited config to localStorage", async () => {
    const { user } = renderPage(<Settings />);
    const url = screen.getByLabelText("Ontology API base URL");
    const auth = screen.getByLabelText("Auth server URL");
    expect(url).toHaveValue("http://localhost:5000");
    expect(auth).toHaveValue("http://localhost:4000");

    await user.clear(url);
    await user.type(url, "https://api.example.test/");
    await user.clear(auth);
    await user.type(auth, "https://auth.example.test");
    await user.type(screen.getByPlaceholderText("(leave blank if server is open)"), "tok-123");
    await user.click(screen.getByRole("button", { name: /Save all/ }));

    expect(screen.getByText(/Connexion serveur enregistrée/)).toBeInTheDocument();
    const stored = JSON.parse(window.localStorage.getItem("ontology.providerConfig") ?? "{}");
    expect(stored).toEqual({
      ontologyApiUrl: "https://api.example.test/",
      ontologyBearerToken: "tok-123",
      authApiUrl: "https://auth.example.test",
    });
    // Mirrored to the legacy keys `apiBase()` reads (trailing slash dropped).
    expect(window.localStorage.getItem("ontology.apiBase")).toBe("https://api.example.test");
    expect(window.localStorage.getItem("ontology.apiToken")).toBe("tok-123");
    expect(screen.getByText("https://api.example.test", { selector: "code" })).toBeInTheDocument();
  });

  it("loads a previously stored config", () => {
    window.localStorage.setItem(
      "ontology.providerConfig",
      JSON.stringify({ ontologyApiUrl: "http://stored:1", ontologyBearerToken: "abc", authApiUrl: "http://stored:2" }),
    );
    renderPage(<Settings />);
    expect(screen.getByLabelText("Ontology API base URL")).toHaveValue("http://stored:1");
    expect(screen.getByLabelText("Auth server URL")).toHaveValue("http://stored:2");
    expect(screen.getByPlaceholderText("(leave blank if server is open)")).toHaveValue("abc");
  });

  it("toggles the bearer token visibility", async () => {
    const { user } = renderPage(<Settings />);
    const token = screen.getByPlaceholderText("(leave blank if server is open)");
    expect(token).toHaveAttribute("type", "password");
    await user.click(screen.getByRole("button", { name: "Show key" }));
    expect(token).toHaveAttribute("type", "text");
    await user.click(screen.getByRole("button", { name: "Hide key" }));
    expect(token).toHaveAttribute("type", "password");
  });

  it("reports an unreachable server (fetch rejects)", async () => {
    const { user } = renderPage(<Settings />);
    await user.click(screen.getByRole("button", { name: /Test connection/ }));
    expect(await screen.findByText("❌ network access is disabled in unit tests")).toBeInTheDocument();
    expect(fetch).toHaveBeenCalledWith("http://localhost:5000/healthz", { headers: {} });
    expect(screen.getByRole("button", { name: /Test connection/ })).toBeEnabled();
  });

  it("reports the server version on success and sends the bearer token", async () => {
    const fetchMock = vi.fn().mockResolvedValue(healthz({ version: "1.2.3" }));
    vi.stubGlobal("fetch", fetchMock);
    const { user } = renderPage(<Settings />);
    await user.type(screen.getByPlaceholderText("(leave blank if server is open)"), " tok ");
    await user.click(screen.getByRole("button", { name: /Test connection/ }));
    expect(await screen.findByText("✅ Server reachable — version 1.2.3")).toBeInTheDocument();
    expect(fetchMock).toHaveBeenCalledWith("http://localhost:5000/healthz", {
      headers: { authorization: "Bearer tok" },
    });
  });

  it("falls back to an unknown version when /healthz is not JSON", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(healthz("ok", { jsonFails: true })));
    const { user } = renderPage(<Settings />);
    await user.click(screen.getByRole("button", { name: /Test connection/ }));
    expect(await screen.findByText("✅ Server reachable — version unknown")).toBeInTheDocument();
  });

  it("reports an HTTP error status", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(healthz("", { ok: false, status: 503, statusText: "Service Unavailable" })),
    );
    const { user } = renderPage(<Settings />);
    await user.click(screen.getByRole("button", { name: /Test connection/ }));
    expect(await screen.findByText("❌ HTTP 503 Service Unavailable")).toBeInTheDocument();
  });

  it("refuses to probe an empty base URL without touching the network", async () => {
    const { user } = renderPage(<Settings />);
    await user.clear(screen.getByLabelText("Ontology API base URL"));
    await user.click(screen.getByRole("button", { name: /Test connection/ }));
    expect(await screen.findByText("❌ Ontology API base URL is empty")).toBeInTheDocument();
    expect(fetch).not.toHaveBeenCalled();
  });

  it("disables the test button while the probe is pending", async () => {
    let release!: (value: unknown) => void;
    vi.stubGlobal("fetch", vi.fn(() => new Promise((resolve) => { release = resolve; })));
    const { user } = renderPage(<Settings />);
    await user.click(screen.getByRole("button", { name: /Test connection/ }));
    const pending = screen.getByRole("button", { name: "Testing…" });
    expect(pending).toBeDisabled();
    release(healthz({ version: "9" }));
    expect(await screen.findByText("✅ Server reachable — version 9")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Test connection/ })).toBeEnabled();
  });
});

describe("Settings page — danger zone", () => {
  it("does nothing when the confirm dialog is cancelled", async () => {
    const { user } = renderPage(<Settings />);
    await user.click(screen.getByRole("button", { name: /Réinitialiser toutes les données/ }));
    expect(screen.getByRole("dialog")).toHaveTextContent("Réinitialiser toutes les données");
    await user.click(screen.getByRole("button", { name: "Annuler" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(mocked.resetAll).not.toHaveBeenCalled();
  });

  it("resets everything once confirmed", async () => {
    const { user } = renderPage(<Settings />);
    await user.click(screen.getByRole("button", { name: /Réinitialiser toutes les données/ }));
    await user.click(screen.getByRole("button", { name: "Tout supprimer" }));
    await waitFor(() => expect(mocked.resetAll).toHaveBeenCalledTimes(1));
  });

  it("shows the reset error and re-enables the button", async () => {
    mocked.resetAll.mockRejectedValueOnce(new Error("reset refused"));
    const { user } = renderPage(<Settings />);
    await user.click(screen.getByRole("button", { name: /Réinitialiser toutes les données/ }));
    await user.click(screen.getByRole("button", { name: "Tout supprimer" }));
    expect(await screen.findByText("reset refused")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Réinitialiser toutes les données/ })).toBeEnabled();

    mocked.resetAll.mockRejectedValueOnce("plain failure");
    await user.click(screen.getByRole("button", { name: /Réinitialiser toutes les données/ }));
    await user.click(screen.getByRole("button", { name: "Tout supprimer" }));
    expect(await screen.findByText("plain failure")).toBeInTheDocument();
  });
});

describe("Settings page — legacy keys banner", () => {
  const legacyStore = (extra: Record<string, unknown>) =>
    window.localStorage.setItem(
      "ontology.providerConfig",
      JSON.stringify({ ontologyApiUrl: "http://localhost:5000", ontologyBearerToken: "", authApiUrl: "http://localhost:4000", ...extra }),
    );

  it("is absent when the browser holds no provider secret", () => {
    renderPage(<Settings />);
    expect(screen.queryByText(/Clés stockées dans ce navigateur/)).not.toBeInTheDocument();
  });

  it("lists the legacy fields and imports them to the server, then purges the browser", async () => {
    legacyStore({
      activeLLMProvider: "infomaniak",
      openaiKey: " sk-old ",
      anthropicKey: "sk-ant-old",
      infomaniakKey: "ik-old",
      infomaniakBaseUrl: "https://api.infomaniak.com/1/ai",
      infomaniakModel: "mixtral",
    });
    const { user } = renderPage(<Settings />);
    expect(
      screen.getByText(/activeLLMProvider, openaiKey, anthropicKey, infomaniakKey, infomaniakBaseUrl, infomaniakModel/),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /Importer vers le serveur/ }));
    await waitFor(() =>
      expect(mocked.patchSettings).toHaveBeenCalledWith({
        llm: {
          openai_api_key: "sk-old",
          anthropic_api_key: "sk-ant-old",
          infomaniak_api_key: "ik-old",
          infomaniak_model: "mixtral",
          active_provider: "infomaniak",
        },
      }),
    );
    await waitFor(() =>
      expect(screen.queryByText(/Clés stockées dans ce navigateur/)).not.toBeInTheDocument(),
    );
    // The base URL is deliberately not imported, and the store keeps only transport fields.
    const stored = JSON.parse(window.localStorage.getItem("ontology.providerConfig") ?? "{}");
    expect(stored).toEqual({ ontologyApiUrl: "http://localhost:5000", ontologyBearerToken: "", authApiUrl: "http://localhost:4000" });
    expect(await screen.findByText("Settings saved.")).toBeInTheDocument();
  });

  it("skips the server write when only the default provider was stored", async () => {
    legacyStore({ activeLLMProvider: "default" });
    const { user } = renderPage(<Settings />);
    await user.click(screen.getByRole("button", { name: /Importer vers le serveur/ }));
    await waitFor(() =>
      expect(screen.queryByText(/Clés stockées dans ce navigateur/)).not.toBeInTheDocument(),
    );
    expect(mocked.patchSettings).not.toHaveBeenCalled();
  });

  it("discards the legacy block without calling the server", async () => {
    legacyStore({ openaiKey: "sk-old" });
    const { user } = renderPage(<Settings />);
    await user.click(screen.getByRole("button", { name: /Supprimer du navigateur/ }));
    expect(screen.queryByText(/Clés stockées dans ce navigateur/)).not.toBeInTheDocument();
    expect(mocked.patchSettings).not.toHaveBeenCalled();
    expect(window.localStorage.getItem("ontology.providerConfig")).not.toContain("openaiKey");
  });
});
