// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The ingest wizard in English.

import { beforeEach, describe, expect, it, vi } from "vitest";
import IngestWizard from "./IngestWizard";
import { renderPage, screen } from "../test/render";
import { setLang } from "../lib/i18n";
import type { Settings } from "../api";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return { ...actual, getSettings: vi.fn() };
});
vi.mock("../lib/extractText", () => ({
  prepareForIngest: vi.fn(),
  terminateOcrWorker: vi.fn(() => Promise.resolve()),
}));

import * as api from "../api";

beforeEach(() => {
  setLang("en", false);
  (api.getSettings as unknown as ReturnType<typeof vi.fn>).mockResolvedValue({ llm: { active_provider: "default" } } as unknown as Settings);
});

describe("IngestWizard, English", () => {
  it("renders the upload step in English", async () => {
    renderPage(<IngestWizard />);
    expect(await screen.findByRole("heading", { name: "Import a document" })).toBeInTheDocument();
    expect(screen.getByText("AI provider")).toBeInTheDocument();
    expect(screen.getByText("Model (optional)")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Analyse the document" })).toBeDisabled();
  });
});
