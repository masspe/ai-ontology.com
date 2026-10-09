// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The Concepts page rendered in English: the dictionary reaches the screen.

import { expect, it, vi } from "vitest";
import Concepts from "./Concepts";
import { renderPage, screen } from "../test/render";
import { setLang } from "../lib/i18n";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return {
    ...actual,
    getOntology: vi.fn(),
    getStats: vi.fn(),
    getStatsHistory: vi.fn(),
    getSubgraph: vi.fn(),
    listConcepts: vi.fn(),
  };
});

import * as api from "../api";

it("renders the Concepts page in English", async () => {
  setLang("en", false);
  vi.mocked(api.getStats).mockResolvedValue({
    concepts: 0, relations: 0, rules: 0, actions: 0, concept_types: 1, relation_types: 0, rule_types: 0, action_types: 0,
    deltas: { concepts_pct: 0, relations_pct: 0, concept_types_pct: 0, relation_types_pct: 0 },
  });
  vi.mocked(api.getStatsHistory).mockResolvedValue({ samples: [] });
  vi.mocked(api.getOntology).mockResolvedValue({ concept_types: { Person: { name: "Person" } }, relation_types: {} });
  vi.mocked(api.getSubgraph).mockResolvedValue({ subgraph: { concepts: [], relations: [] } });
  vi.mocked(api.listConcepts).mockResolvedValue({ total: 0, concepts: [], next_cursor: null });
  renderPage(<Concepts />);
  expect(screen.getByRole("heading", { name: "Sheets" })).toBeInTheDocument();
  expect(screen.getByText("Sheet library")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: /Create a sheet/ })).toBeInTheDocument();
  expect(await screen.findByText(/No sheets yet\. The model is in place/)).toBeInTheDocument();
  expect(screen.getByText("Select a sheet in the library.")).toBeInTheDocument();
});
