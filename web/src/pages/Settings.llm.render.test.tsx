// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Rendering tests of the two server-stored provider cards on the Settings
// page: the LLM configuration (provider switch, probes that never save,
// model catalogue, Infomaniak product detection, "Appliquer") and the OCR
// engine card (status probe, toggles, Google key).

import { beforeEach, describe, expect, it, vi } from "vitest";
import Settings from "./Settings";
import { fireEvent, renderPage, screen, waitFor, within } from "../test/render";
import type {
  LlmSettings,
  OcrSettings,
  OcrStatus,
  Settings as ServerSettings,
  SettingsPatch,
} from "../api";

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
  testLlm: ReturnType<typeof vi.fn>;
  listLlmModels: ReturnType<typeof vi.fn>;
  listInfomaniakProducts: ReturnType<typeof vi.fn>;
};

function makeSettings(llm: Partial<LlmSettings> = {}, ocr: Partial<OcrSettings> = {}): ServerSettings {
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
      ...llm,
    },
    ocr: {
      provider: "tesseract",
      auto_fallback: false,
      min_text_threshold: 200,
      tesseract_languages: "fra+eng",
      google_api_key_hint: "",
      ...ocr,
    },
  };
}

function mergeSettings(base: ServerSettings, patch: SettingsPatch): ServerSettings {
  return {
    retrieval: { ...base.retrieval, ...patch.retrieval },
    ui: { ...base.ui, ...patch.ui },
    llm: { ...base.llm, ...(patch.llm as Partial<LlmSettings>) },
    ocr: { ...base.ocr, ...(patch.ocr as Partial<OcrSettings>) },
  };
}

const okStatus: OcrStatus = {
  tesseract: { available: true, ocrmypdf_version: "16.1", ghostscript_available: false },
  google_vision: { configured: false, auth: "api_key" },
};

/** Serve `settings` from the mocks and echo patches back merged. */
function serve(settings: ServerSettings) {
  mocked.getSettings.mockResolvedValue(settings);
  mocked.patchSettings.mockImplementation((patch: SettingsPatch) =>
    Promise.resolve(mergeSettings(settings, patch)),
  );
}

/** Scope queries to the LLM card (the OCR card has look-alike controls). */
async function llmCard() {
  const title = await screen.findByText(/^Configuration /);
  return within(title.closest("section")!);
}

/**
 * The "Charger les modèles" button is the first labelable element of the
 * "Modèle" label, so its accessible name is the label's text, not its own:
 * reach it through its title.
 */
function loadModelsButton(card: ReturnType<typeof within>) {
  return card.getByTitle("Interroger le catalogue du fournisseur");
}

async function ocrCard() {
  const title = await screen.findByText("Moteur OCR");
  return within(title.closest("section")!);
}

beforeEach(() => {
  serve(makeSettings());
  mocked.getOcrStatus.mockResolvedValue(okStatus);
  mocked.testLlm.mockResolvedValue({ ok: true, provider: "openai" });
  mocked.listLlmModels.mockResolvedValue({ models: [] });
  mocked.listInfomaniakProducts.mockResolvedValue({ products: [] });
});

describe("LLM card — initial state", () => {
  it("defaults to OpenAI with the offline model list when no provider is active", async () => {
    renderPage(<Settings />);
    const card = await llmCard();
    expect(card.getByText("Configuration OpenAI")).toBeInTheDocument();
    expect(card.getByText("Non configuré")).toBeInTheDocument();
    expect(card.getByText(/Aucun fournisseur actif/)).toBeInTheDocument();
    expect(card.getByLabelText("Fournisseur")).toHaveValue("openai");
    expect(card.getByPlaceholderText("sk-…")).toHaveValue("");
    expect(card.getByRole("link", { name: "platform.openai.com" })).toBeInTheDocument();
    expect(card.getByRole("option", { name: "gpt-4o-mini" })).toBeInTheDocument();
    expect(card.getByDisplayValue("— aucun —")).toBeInTheDocument();
    expect(card.getByText("Température (0.70)")).toBeInTheDocument();
    expect(card.getByLabelText("Max tokens")).toHaveValue(2048);
    // Probing the catalogue needs a key (typed or saved).
    expect(loadModelsButton(card)).toBeDisabled();
    expect(card.getByRole("button", { name: /Tester OpenAI/ })).toBeEnabled();
    expect(card.getByRole("button", { name: /Appliquer/ })).toBeEnabled();
  });

  it("shows the active provider, its model and the saved key hint", async () => {
    serve(makeSettings({ active_provider: "openai", openai_model: "gpt-4o", openai_api_key_hint: "sk-…abcd" }));
    renderPage(<Settings />);
    const card = await llmCard();
    expect(card.getByText("⊘ Configuré")).toBeInTheDocument();
    const active = card.getByText(/^Actif :/).closest("p")!;
    expect(active).toHaveTextContent("Actif : OpenAI · gpt-4o");
    expect(card.getByText("sk-…abcd", { selector: "code" })).toBeInTheDocument();
    expect(card.getByPlaceholderText("•••••••••• (laisser vide pour conserver)")).toBeInTheDocument();
    expect(card.getByDisplayValue("gpt-4o")).toBeInTheDocument();
    expect(loadModelsButton(card)).toBeEnabled();
  });

  it("says when the active provider has no model", async () => {
    serve(makeSettings({ active_provider: "anthropic" }));
    renderPage(<Settings />);
    const card = await llmCard();
    expect(card.getByText(/^Actif :/).closest("p")).toHaveTextContent("Actif : Anthropic — aucun modèle sélectionné");
    // The form opens on the active provider.
    expect(card.getByText("Configuration Anthropic")).toBeInTheDocument();
    expect(card.getByRole("link", { name: "console.anthropic.com" })).toBeInTheDocument();
    expect(card.getByRole("option", { name: "claude-sonnet-4-6" })).toBeInTheDocument();
  });

  it("exposes a model unknown to the catalogue as a manual entry", async () => {
    serve(makeSettings({ active_provider: "openai", openai_model: "ft:my-tune" }));
    const { user } = renderPage(<Settings />);
    const card = await llmCard();
    expect(card.getByDisplayValue("Saisir manuellement…")).toBeInTheDocument();
    const manual = card.getByPlaceholderText("nom-du-modèle");
    expect(manual).toHaveValue("ft:my-tune");
    await user.type(manual, "-v2");
    expect(manual).toHaveValue("ft:my-tune-v2");
    // Picking "— aucun —" leaves manual mode…
    await user.selectOptions(card.getByDisplayValue("Saisir manuellement…"), "");
    expect(card.queryByPlaceholderText("nom-du-modèle")).not.toBeInTheDocument();
    // …and picking a listed model selects it.
    await user.selectOptions(card.getByDisplayValue("— aucun —"), "gpt-4o-mini");
    expect(card.getByDisplayValue("gpt-4o-mini")).toBeInTheDocument();
    // "Saisir manuellement…" itself clears the model (the input appears once a
    // value outside the list exists).
    await user.selectOptions(card.getByDisplayValue("gpt-4o-mini"), "__custom__");
    expect(card.getByDisplayValue("— aucun —")).toBeInTheDocument();
  });

  it("toggles the key visibility and the advanced base-URL field", async () => {
    const { user } = renderPage(<Settings />);
    const card = await llmCard();
    const key = card.getByPlaceholderText("sk-…");
    expect(key).toHaveAttribute("type", "password");
    await user.click(card.getByTitle("Afficher"));
    expect(key).toHaveAttribute("type", "text");
    await user.click(card.getByTitle("Masquer"));
    expect(key).toHaveAttribute("type", "password");

    expect(card.queryByPlaceholderText("(URL par défaut du fournisseur)")).not.toBeInTheDocument();
    await user.click(card.getByRole("button", { name: /▸ Avancé/ }));
    expect(card.getByPlaceholderText("(URL par défaut du fournisseur)")).toBeInTheDocument();
    await user.click(card.getByRole("button", { name: /▾ Avancé/ }));
    expect(card.queryByPlaceholderText("(URL par défaut du fournisseur)")).not.toBeInTheDocument();
  });
});

describe("LLM card — provider switch", () => {
  it("re-syncs the form for Anthropic and Infomaniak", async () => {
    serve(makeSettings({ anthropic_model: "claude-haiku-4-5", infomaniak_product_id: "777", infomaniak_api_key_hint: "ik…9" }));
    const { user } = renderPage(<Settings />);
    let card = await llmCard();
    await user.selectOptions(card.getByLabelText("Fournisseur"), "anthropic");
    card = await llmCard();
    expect(card.getByText("Configuration Anthropic")).toBeInTheDocument();
    expect(card.getByText(/Format sk-ant-…/)).toBeInTheDocument();
    expect(card.getByDisplayValue("claude-haiku-4-5")).toBeInTheDocument();
    expect(card.getByRole("button", { name: /Tester Anthropic/ })).toBeInTheDocument();
    expect(card.queryByText("Product ID AI Tools")).not.toBeInTheDocument();

    await user.selectOptions(card.getByLabelText("Fournisseur"), "infomaniak");
    card = await llmCard();
    expect(card.getByText("Configuration Infomaniak AI (Suisse)")).toBeInTheDocument();
    expect(card.getByText("⊘ Configuré")).toBeInTheDocument();
    expect(card.getByText(/Jeton créé dans le Manager Infomaniak/)).toBeInTheDocument();
    expect(card.getByPlaceholderText("101112")).toHaveValue("777");
    expect(card.getByText("api.infomaniak.com/2/ai/777/openai/v1")).toBeInTheDocument();
    // No offline catalogue for Infomaniak.
    expect(card.getByText(/Le catalogue Infomaniak dépend de votre compte/)).toBeInTheDocument();
    expect(card.queryByRole("option", { name: "gpt-4o" })).not.toBeInTheDocument();
    await user.click(card.getByRole("button", { name: /▸ Avancé/ }));
    expect(card.getByPlaceholderText("(déduite du product ID)")).toBeInTheDocument();
  });

  it("derives the Infomaniak URL from the typed product id and gates detection on a key", async () => {
    const { user } = renderPage(<Settings />);
    let card = await llmCard();
    await user.selectOptions(card.getByLabelText("Fournisseur"), "infomaniak");
    card = await llmCard();
    expect(card.getByText("api.infomaniak.com/2/ai/{product_id}/openai/v1")).toBeInTheDocument();
    const detect = card.getByRole("button", { name: /Détecter/ });
    expect(detect).toBeDisabled();
    expect(detect).toHaveAttribute("title", "Saisissez d'abord la clé API");
    await user.type(card.getByPlaceholderText("101112"), "42");
    expect(card.getByText("api.infomaniak.com/2/ai/42/openai/v1")).toBeInTheDocument();
    await user.type(card.getByPlaceholderText("sk-…"), "ik-1");
    expect(detect).toBeEnabled();
    expect(detect).toHaveAttribute("title", "Lire le product ID depuis /1/ai");
    expect(card.getByText(/Aucun modèle chargé|Le catalogue Infomaniak/)).toBeInTheDocument();
  });
});

describe("LLM card — Infomaniak product detection", () => {
  async function openInfomaniak(user: ReturnType<typeof renderPage>["user"], key = "ik-1") {
    let card = await llmCard();
    await user.selectOptions(card.getByLabelText("Fournisseur"), "infomaniak");
    card = await llmCard();
    if (key) await user.type(card.getByPlaceholderText("sk-…"), key);
    return card;
  }

  it("fills the product id when exactly one product is found", async () => {
    mocked.listInfomaniakProducts.mockResolvedValue({ products: [{ product_id: "42", name: "Prod A" }] });
    const { user } = renderPage(<Settings />);
    const card = await openInfomaniak(user);
    await user.click(card.getByRole("button", { name: /Détecter/ }));
    expect(await card.findByText("Product ID détecté : 42 (Prod A)")).toBeInTheDocument();
    expect(mocked.listInfomaniakProducts).toHaveBeenCalledWith("ik-1");
    expect(card.getByPlaceholderText("101112")).toHaveValue("42");
    expect(card.queryByDisplayValue("— choisir un produit —")).not.toBeInTheDocument();
  });

  it("offers a picker when several products are found", async () => {
    mocked.listInfomaniakProducts.mockResolvedValue({
      products: [{ product_id: "1", name: "Prod A" }, { product_id: "2" }],
    });
    const { user } = renderPage(<Settings />);
    const card = await openInfomaniak(user);
    await user.click(card.getByRole("button", { name: /Détecter/ }));
    expect(await card.findByText("2 produits AI Tools — choisissez-en un.")).toBeInTheDocument();
    const picker = card.getByDisplayValue("— choisir un produit —");
    expect(card.getByRole("option", { name: "Prod A (1)" })).toBeInTheDocument();
    expect(card.getByRole("option", { name: "2" })).toBeInTheDocument();
    await user.selectOptions(picker, "2");
    expect(card.getByPlaceholderText("101112")).toHaveValue("2");
    expect(card.getByText("api.infomaniak.com/2/ai/2/openai/v1")).toBeInTheDocument();
  });

  it("reports an empty catalogue, with the server error when there is one", async () => {
    mocked.listInfomaniakProducts.mockResolvedValueOnce({ products: [], error: "token rejected" });
    const { user } = renderPage(<Settings />);
    const card = await openInfomaniak(user);
    await user.click(card.getByRole("button", { name: /Détecter/ }));
    expect(await card.findByText("token rejected")).toBeInTheDocument();
    await user.click(card.getByRole("button", { name: /Détecter/ }));
    expect(await card.findByText("Aucun produit AI Tools trouvé.")).toBeInTheDocument();
  });

  it("uses the saved key (no override) when the provider is already configured", async () => {
    serve(makeSettings({ infomaniak_api_key_hint: "ik…9" }));
    mocked.listInfomaniakProducts.mockResolvedValue({ products: [{ product_id: "5" }] });
    const { user } = renderPage(<Settings />);
    const card = await openInfomaniak(user, "");
    expect(card.getByRole("button", { name: /Détecter/ })).toBeEnabled();
    await user.click(card.getByRole("button", { name: /Détecter/ }));
    expect(await card.findByText("Product ID détecté : 5")).toBeInTheDocument();
    expect(mocked.listInfomaniakProducts).toHaveBeenCalledWith(undefined);
  });

  it("shows a thrown error in the banner", async () => {
    mocked.listInfomaniakProducts.mockRejectedValueOnce(new Error("network down"));
    const { user } = renderPage(<Settings />);
    const card = await openInfomaniak(user);
    await user.click(card.getByRole("button", { name: /Détecter/ }));
    expect(await card.findByText("network down")).toBeInTheDocument();
    expect(card.getByText("network down")).toHaveClass("error-banner");
  });
});

describe("LLM card — model catalogue", () => {
  it("loads the models with the typed key as an override and renders their metadata", async () => {
    mocked.listLlmModels.mockResolvedValue({
      models: [
        { id: "gpt-4o", label: "GPT-4o", max_input_tokens: 128_000, beta: true },
        { id: "dall-e-3", label: "DALL·E 3", kind: "image" },
        { id: "gpt-4o-mini", label: "gpt-4o-mini", kind: "llm" },
      ],
    });
    const { user } = renderPage(<Settings />);
    const card = await llmCard();
    await user.type(card.getByPlaceholderText("sk-…"), "sk-new");
    await user.click(loadModelsButton(card));
    expect(await card.findByText("3 modèle(s) chargé(s) depuis OpenAI.")).toBeInTheDocument();
    expect(mocked.listLlmModels).toHaveBeenCalledWith({ provider: "openai", api_key: "sk-new" });
    expect(card.getByRole("option", { name: "GPT-4o · 128k ctx · beta" })).toBeInTheDocument();
    expect(card.getByRole("option", { name: "DALL·E 3 · image" })).toBeInTheDocument();
    expect(card.getByRole("option", { name: "gpt-4o-mini" })).toBeInTheDocument();
    expect(card.queryByRole("option", { name: "gpt-4.1" })).not.toBeInTheDocument();
    expect(card.getByText(/modèles non conversationnels/)).toBeInTheDocument();
    // The typed key is not saved by a probe.
    expect(mocked.patchSettings).not.toHaveBeenCalled();
  });

  it("warns when the selected model is missing from the fresh catalogue", async () => {
    serve(makeSettings({ active_provider: "openai", openai_model: "gpt-4o", openai_api_key_hint: "sk-…x" }));
    mocked.listLlmModels.mockResolvedValue({ models: [{ id: "o3", label: "o3" }] });
    const { user } = renderPage(<Settings />);
    const card = await llmCard();
    await user.click(loadModelsButton(card));
    expect(await card.findByText("1 modèle(s) chargé(s), mais « gpt-4o » n'y figure pas.")).toBeInTheDocument();
    expect(card.getByText(/n'y figure pas/)).toHaveClass("warn-banner");
    expect(mocked.listLlmModels).toHaveBeenCalledWith({ provider: "openai", model: "gpt-4o" });
    // Selection kept (now as a manual entry) rather than silently switched.
    expect(card.getByPlaceholderText("nom-du-modèle")).toHaveValue("gpt-4o");
  });

  it("shows the provider error when the catalogue comes back empty", async () => {
    mocked.listLlmModels.mockResolvedValueOnce({ models: [], error: "catalogue 404", endpoint: "https://x/models" });
    const { user } = renderPage(<Settings />);
    const card = await llmCard();
    await user.type(card.getByPlaceholderText("sk-…"), "k");
    await user.click(loadModelsButton(card));
    expect(await card.findByText("catalogue 404 — appel : https://x/models")).toBeInTheDocument();
    // The offline fallback stays in place.
    expect(card.getByRole("option", { name: "gpt-4o" })).toBeInTheDocument();

    mocked.listLlmModels.mockResolvedValueOnce({ models: [], error: "nope" });
    await user.click(loadModelsButton(card));
    expect(await card.findByText("nope")).toBeInTheDocument();

    mocked.listLlmModels.mockRejectedValueOnce("exploded");
    await user.click(loadModelsButton(card));
    expect(await card.findByText("exploded")).toBeInTheDocument();
  });

  it("shows the busy label and disables the actions while loading", async () => {
    let release!: (v: unknown) => void;
    mocked.listLlmModels.mockImplementationOnce(() => new Promise((resolve) => { release = resolve; }));
    const { user } = renderPage(<Settings />);
    const card = await llmCard();
    await user.type(card.getByPlaceholderText("sk-…"), "k");
    await user.click(loadModelsButton(card));
    expect(loadModelsButton(card)).toHaveTextContent("…");
    expect(loadModelsButton(card)).toBeDisabled();
    expect(card.getByRole("button", { name: /Appliquer/ })).toBeDisabled();
    expect(card.getByRole("button", { name: /Tester OpenAI/ })).toBeDisabled();
    release({ models: [] });
    expect(await card.findByText("0 modèle(s) chargé(s) depuis OpenAI.")).toBeInTheDocument();
    expect(card.getByRole("button", { name: /Appliquer/ })).toBeEnabled();
  });
});

describe("LLM card — test connection", () => {
  it("reports a successful probe with model and endpoint", async () => {
    mocked.testLlm.mockResolvedValue({
      ok: true, provider: "openai", model: "gpt-4o", endpoint: "https://api.openai.com/v1/chat/completions",
    });
    const { user } = renderPage(<Settings />);
    const card = await llmCard();
    await user.click(card.getByRole("button", { name: /Tester OpenAI/ }));
    expect(
      await card.findByText("Connexion OpenAI OK (modèle gpt-4o) — https://api.openai.com/v1/chat/completions"),
    ).toBeInTheDocument();
    expect(mocked.testLlm).toHaveBeenCalledWith({ provider: "openai" });
  });

  it("reports a bare success and a provider-side failure", async () => {
    mocked.testLlm.mockResolvedValueOnce({ ok: true, provider: "openai" });
    const { user } = renderPage(<Settings />);
    const card = await llmCard();
    await user.click(card.getByRole("button", { name: /Tester OpenAI/ }));
    expect(await card.findByText("Connexion OpenAI OK")).toBeInTheDocument();

    mocked.testLlm.mockResolvedValueOnce({ ok: false, provider: "openai", error: "401 bad key", endpoint: "https://x/models" });
    await user.click(card.getByRole("button", { name: /Tester OpenAI/ }));
    expect(await card.findByText("401 bad key — appel : https://x/models")).toBeInTheDocument();

    mocked.testLlm.mockResolvedValueOnce({ ok: false, provider: "openai" });
    await user.click(card.getByRole("button", { name: /Tester OpenAI/ }));
    expect(await card.findByText("échec")).toBeInTheDocument();

    mocked.testLlm.mockRejectedValueOnce(new Error("timeout"));
    await user.click(card.getByRole("button", { name: /Tester OpenAI/ }));
    expect(await card.findByText("timeout")).toBeInTheDocument();
  });

  it("sends every typed field as an override, trimmed, without saving", async () => {
    const { user } = renderPage(<Settings />);
    let card = await llmCard();
    await user.selectOptions(card.getByLabelText("Fournisseur"), "infomaniak");
    card = await llmCard();
    await user.type(card.getByPlaceholderText("sk-…"), "  ik-2  ");
    await user.type(card.getByPlaceholderText("101112"), "42");
    await user.click(card.getByRole("button", { name: /▸ Avancé/ }));
    await user.type(card.getByPlaceholderText("(déduite du product ID)"), "https://proxy.test ");
    // No catalogue: pick a manual model through the picker + Charger path is
    // covered elsewhere; here the model stays empty and must be absent.
    await user.click(card.getByRole("button", { name: /Tester Infomaniak/ }));
    await waitFor(() =>
      expect(mocked.testLlm).toHaveBeenCalledWith({
        provider: "infomaniak",
        api_key: "ik-2",
        base_url: "https://proxy.test",
        product_id: "42",
      }),
    );
    expect(mocked.patchSettings).not.toHaveBeenCalled();
  });
});

describe("LLM card — apply", () => {
  it("writes the OpenAI configuration, clears the key and confirms", async () => {
    mocked.listLlmModels.mockResolvedValue({ models: [{ id: "gpt-4o", label: "gpt-4o" }] });
    const { user } = renderPage(<Settings />);
    const card = await llmCard();
    await user.type(card.getByPlaceholderText("sk-…"), " sk-new ");
    await user.selectOptions(card.getByDisplayValue("— aucun —"), "gpt-4o");
    fireEvent.change(card.getByRole("slider"), { target: { value: "0.3" } });
    expect(card.getByText("Température (0.30)")).toBeInTheDocument();
    fireEvent.change(card.getByLabelText("Max tokens"), { target: { value: "4096" } });
    await user.click(card.getByRole("button", { name: /Appliquer/ }));
    expect(await card.findByText("OpenAI appliqué avec le modèle gpt-4o. Effectif immédiatement.")).toBeInTheDocument();
    expect(mocked.patchSettings).toHaveBeenCalledWith({
      llm: {
        active_provider: "openai",
        temperature: 0.3,
        max_tokens: 4096,
        openai_api_key: "sk-new",
        openai_base_url: "",
        openai_model: "gpt-4o",
      },
    });
    expect(card.getByPlaceholderText("sk-…")).toHaveValue("");
    expect(screen.getByText("Settings saved.")).toBeInTheDocument();
    // The echoed settings re-sync the "Actif" line.
    expect(card.getByText(/^Actif :/).closest("p")).toHaveTextContent("Actif : OpenAI · gpt-4o");
  });

  it("writes the Anthropic key and asks for a model when none is chosen", async () => {
    let release!: (v: unknown) => void;
    mocked.patchSettings.mockImplementationOnce(() => new Promise((resolve) => { release = resolve; }));
    const { user } = renderPage(<Settings />);
    let card = await llmCard();
    await user.selectOptions(card.getByLabelText("Fournisseur"), "anthropic");
    card = await llmCard();
    await user.type(card.getByPlaceholderText("sk-…"), "sk-ant-new");
    await user.click(card.getByRole("button", { name: /Appliquer/ }));
    expect(card.getByRole("button", { name: "Application…" })).toBeDisabled();
    expect(card.getByRole("button", { name: /Tester Anthropic/ })).toBeDisabled();
    expect(mocked.patchSettings).toHaveBeenCalledWith({
      llm: {
        active_provider: "anthropic",
        temperature: 0.7,
        max_tokens: 2048,
        anthropic_api_key: "sk-ant-new",
        anthropic_base_url: "",
        anthropic_model: "",
      },
    });
    release(makeSettings({ active_provider: "anthropic" }));
    expect(await card.findByText("Anthropic enregistré. Choisissez un modèle pour pouvoir générer.")).toBeInTheDocument();
    expect(card.getByRole("button", { name: /Appliquer/ })).toBeEnabled();
  });

  it("writes the Infomaniak product id and base URL", async () => {
    serve(makeSettings({ infomaniak_api_key_hint: "ik…9", infomaniak_model: "mistral-large" }));
    const { user } = renderPage(<Settings />);
    let card = await llmCard();
    await user.selectOptions(card.getByLabelText("Fournisseur"), "infomaniak");
    card = await llmCard();
    await user.type(card.getByPlaceholderText("101112"), "42");
    await user.type(card.getByPlaceholderText("•••••••••• (laisser vide pour conserver)"), "ik-new");
    await user.click(card.getByRole("button", { name: /Appliquer/ }));
    expect(
      await card.findByText("Infomaniak AI (Suisse) appliqué avec le modèle mistral-large. Effectif immédiatement."),
    ).toBeInTheDocument();
    expect(mocked.patchSettings).toHaveBeenCalledWith({
      llm: {
        active_provider: "infomaniak",
        temperature: 0.7,
        max_tokens: 2048,
        infomaniak_api_key: "ik-new",
        infomaniak_product_id: "42",
        infomaniak_base_url: "",
        infomaniak_model: "mistral-large",
      },
    });
  });

  it("omits a blank key and surfaces a server rejection in the page banner", async () => {
    mocked.patchSettings.mockRejectedValueOnce(new Error("settings store locked"));
    const { user } = renderPage(<Settings />);
    const card = await llmCard();
    await user.click(card.getByRole("button", { name: /Appliquer/ }));
    expect(await screen.findByText("settings store locked")).toBeInTheDocument();
    expect(screen.getByText("settings store locked")).toHaveClass("error-banner");
    expect(mocked.patchSettings).toHaveBeenCalledWith({
      llm: { active_provider: "openai", temperature: 0.7, max_tokens: 2048, openai_base_url: "", openai_model: "" },
    });
  });
});

describe("OCR card", () => {
  it("renders the probe result and the saved settings", async () => {
    renderPage(<Settings />);
    const card = await ocrCard();
    expect(await card.findByText("⊘ Disponible")).toBeInTheDocument();
    expect(card.getByText("ocrmypdf: 16.1")).toBeInTheDocument();
    expect(card.getByText("Tesseract: ✓")).toBeInTheDocument();
    expect(card.getByText("Ghostscript: ✕")).toBeInTheDocument();
    expect(card.getByText("Non configuré")).toBeInTheDocument();
    expect(card.getByLabelText("Fournisseur OCR principal")).toHaveValue("tesseract");
    expect(card.getByRole("checkbox")).not.toBeChecked();
    expect(card.getByDisplayValue("200")).toBeInTheDocument();
    expect(card.getByText(/Si l'OCR retourne moins de 200 caractères/)).toBeInTheDocument();
    expect(card.getByDisplayValue("fra+eng")).toBeInTheDocument();
    expect(card.getByPlaceholderText("AIza…")).toBeInTheDocument();
    expect(card.getByRole("button", { name: /Sauvegarder/ })).toBeDisabled();
    expect(mocked.getOcrStatus).toHaveBeenCalledTimes(1);
  });

  it("falls back to the settings hint when the probe fails", async () => {
    mocked.getOcrStatus.mockRejectedValue(new Error("no probe"));
    serve(makeSettings({}, { google_api_key_hint: "AIza…xyz", auto_fallback: true, provider: "google_vision" }));
    renderPage(<Settings />);
    const card = await ocrCard();
    expect(card.getByText("⚠ Indisponible")).toBeInTheDocument();
    expect(card.getByText("ocrmypdf: —")).toBeInTheDocument();
    expect(card.getByText("Tesseract: ✕")).toBeInTheDocument();
    expect(await card.findByText("⊘ Configuré")).toBeInTheDocument();
    expect(card.getByText("AIza…xyz", { selector: "code" })).toBeInTheDocument();
    expect(card.getByPlaceholderText("••••••••••")).toBeInTheDocument();
    expect(card.getByLabelText("Fournisseur OCR principal")).toHaveValue("google_vision");
    expect(card.getByRole("checkbox")).toBeChecked();
  });

  it("patches the provider, the fallback toggle, the threshold and the languages", async () => {
    const { user } = renderPage(<Settings />);
    const card = await ocrCard();
    await user.selectOptions(card.getByLabelText("Fournisseur OCR principal"), "google_vision");
    await waitFor(() =>
      expect(mocked.patchSettings).toHaveBeenCalledWith({ ocr: { provider: "google_vision" } }),
    );
    expect(await card.findByDisplayValue("Google Cloud Vision")).toBeInTheDocument();

    await user.click(card.getByRole("checkbox"));
    await waitFor(() =>
      expect(mocked.patchSettings).toHaveBeenCalledWith({ ocr: { auto_fallback: true } }),
    );
    expect(card.getByRole("checkbox")).toBeChecked();

    fireEvent.change(card.getByDisplayValue("200"), { target: { value: "300" } });
    await waitFor(() =>
      expect(mocked.patchSettings).toHaveBeenCalledWith({ ocr: { min_text_threshold: 300 } }),
    );
    expect(await card.findByText(/Si l'OCR retourne moins de 300 caractères/)).toBeInTheDocument();

    fireEvent.change(card.getByDisplayValue("fra+eng"), { target: { value: "deu" } });
    await waitFor(() =>
      expect(mocked.patchSettings).toHaveBeenCalledWith({ ocr: { tesseract_languages: "deu" } }),
    );
  });

  it("saves the Google key once typed, then clears the field", async () => {
    const { user } = renderPage(<Settings />);
    const card = await ocrCard();
    const key = card.getByPlaceholderText("AIza…");
    const save = card.getByRole("button", { name: /Sauvegarder/ });
    expect(key).toHaveAttribute("type", "password");
    await user.click(card.getByTitle("Afficher"));
    expect(key).toHaveAttribute("type", "text");
    await user.type(key, "   ");
    expect(save).toBeDisabled();
    await user.type(key, "AIza-new ");
    expect(save).toBeEnabled();
    await user.click(save);
    await waitFor(() =>
      expect(mocked.patchSettings).toHaveBeenCalledWith({ ocr: { google_api_key: "AIza-new" } }),
    );
    await waitFor(() => expect(key).toHaveValue(""));
    expect(save).toBeDisabled();
  });

  it("re-probes the engines when the key hint changes", async () => {
    mocked.patchSettings.mockImplementation((patch: SettingsPatch) =>
      Promise.resolve(mergeSettings(makeSettings({}, { google_api_key_hint: "AIza…new" }), patch)),
    );
    mocked.getOcrStatus
      .mockResolvedValueOnce(okStatus)
      .mockResolvedValueOnce({ ...okStatus, google_vision: { configured: true, auth: "api_key" } });
    const { user } = renderPage(<Settings />);
    const card = await ocrCard();
    await user.type(card.getByPlaceholderText("AIza…"), "AIza-new");
    await user.click(card.getByRole("button", { name: /Sauvegarder/ }));
    expect(await card.findByText("⊘ Configuré")).toBeInTheDocument();
    expect(mocked.getOcrStatus).toHaveBeenCalledTimes(2);
  });
});
