// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The concept sheet in English.

import { beforeEach, describe, expect, it, vi } from "vitest";
import ConceptSheet from "./ConceptSheet";
import { renderPage, screen } from "../test/render";
import { setLang } from "../lib/i18n";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return {
    ...actual,
    getConcept: vi.fn(),
    getSubgraph: vi.fn(),
    listRules: vi.fn(),
    listActions: vi.fn(),
    getFiles: vi.fn(),
    updateConcept: vi.fn(),
  };
});

import * as api from "../api";

const mocked = api as unknown as Record<string, ReturnType<typeof vi.fn>>;
const acme = { id: 7, concept_type: "Company", name: "Acme SA", description: "", properties: {} };

beforeEach(() => {
  setLang("en", false);
  mocked.getConcept!.mockResolvedValue(acme);
  mocked.getSubgraph!.mockResolvedValue({ subgraph: { concepts: [acme], relations: [] } });
  mocked.listRules!.mockResolvedValue([]);
  mocked.listActions!.mockResolvedValue([]);
  mocked.getFiles!.mockResolvedValue({ files: [] });
});

describe("ConceptSheet, English", () => {
  it("renders the cards and empty states in English", async () => {
    renderPage(<ConceptSheet />, { route: "/concepts/7", path: "/concepts/:id" });
    expect(await screen.findByText("No description.")).toBeInTheDocument();
    expect(screen.getByText("Links (0)")).toBeInTheDocument();
    expect(screen.getByText("Source documents")).toBeInTheDocument();
    expect(screen.getByText("No structured information.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Edit" })).toBeInTheDocument();
  });
});
