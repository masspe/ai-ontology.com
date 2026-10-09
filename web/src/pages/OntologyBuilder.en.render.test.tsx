// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The data model page in English.

import { beforeEach, describe, expect, it, vi } from "vitest";
import OntologyBuilder from "./OntologyBuilder";
import { renderPage, screen } from "../test/render";
import { setLang } from "../lib/i18n";
import type { Stats } from "../api";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return {
    ...actual,
    getOntology: vi.fn(),
    getStats: vi.fn(),
    getFiles: vi.fn(),
    getSubgraph: vi.fn(),
    generateOntology: vi.fn(),
    replaceOntology: vi.fn(),
    listConcepts: vi.fn(),
    deleteFile: vi.fn(),
  };
});
vi.mock("../lib/ingestApi", () => ({ analyzeIngest: vi.fn(), applyIngest: vi.fn() }));
vi.mock("../lib/extractText", () => ({
  needsPreprocessing: vi.fn(),
  prepareForIngest: vi.fn(),
  terminateOcrWorker: vi.fn(),
}));
vi.mock("../components/OntologyGraph", () => ({ default: () => <div data-testid="ontology-graph" /> }));

import * as api from "../api";

const mocked = api as unknown as Record<string, ReturnType<typeof vi.fn>>;
const stats = {
  concepts: 10,
  relations: 5,
  rules: 0,
  actions: 0,
  concept_types: 2,
  relation_types: 0,
  rule_types: 0,
  action_types: 0,
  deltas: { concepts_pct: 0, relations_pct: 0, concept_types_pct: 0, relation_types_pct: 0 },
} as Stats;

beforeEach(() => {
  setLang("en", false);
  mocked.listConcepts!.mockResolvedValue({ total: 0, concepts: [], next_cursor: null });
  mocked.getOntology!.mockResolvedValue({ concept_types: { Person: { name: "Person" } }, relation_types: {} });
  mocked.getStats!.mockResolvedValue(stats);
  mocked.getFiles!.mockResolvedValue({ files: [] });
  mocked.getSubgraph!.mockResolvedValue({ subgraph: { concepts: [], relations: [] } });
});

describe("OntologyBuilder page, English", () => {
  it("renders the title, the cards and the footer buttons in English", async () => {
    renderPage(<OntologyBuilder />);
    expect(await screen.findByRole("heading", { name: "Data model" })).toBeInTheDocument();
    expect(screen.getByText("Import files (optional)")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Generate the model/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Save the data model/ })).toBeInTheDocument();
    expect(screen.getByText("Model figures")).toBeInTheDocument();
  });
});
