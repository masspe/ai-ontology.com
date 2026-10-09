// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The Settings page rendered in English: the dictionary reaches the screen.

import { expect, it, vi } from "vitest";
import Settings from "./Settings";
import { renderPage, screen } from "../test/render";
import { setLang } from "../lib/i18n";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return { ...actual, getSettings: vi.fn(), getOcrStatus: vi.fn() };
});

import * as api from "../api";

it("renders the Settings page in English", async () => {
  setLang("en", false);
  vi.mocked(api.getSettings).mockResolvedValue({
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
    ocr: { provider: "tesseract", auto_fallback: false, min_text_threshold: 200, tesseract_languages: "fra+eng", google_api_key_hint: "" },
  });
  vi.mocked(api.getOcrStatus).mockResolvedValue({
    tesseract: { available: true, ocrmypdf_version: "16.1", ghostscript_available: true },
    google_vision: { configured: false, auth: "api_key" },
  });
  renderPage(<Settings />);
  expect(screen.getByRole("heading", { name: "Settings" })).toBeInTheDocument();
  expect(await screen.findByText("Lexical weight")).toBeInTheDocument();
  expect(screen.getByText("Search settings")).toBeInTheDocument();
  expect(screen.getByText("Danger zone")).toBeInTheDocument();
  expect(screen.queryByText("Réglages de recherche")).not.toBeInTheDocument();
});
