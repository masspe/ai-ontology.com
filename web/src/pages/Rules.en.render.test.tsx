// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The Rules page rendered in English: the dictionary reaches the screen.

import { expect, it, vi } from "vitest";
import Rules from "./Rules";
import { renderPage, screen, within } from "../test/render";
import { setLang } from "../lib/i18n";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return { ...actual, getOntology: vi.fn(), getStats: vi.fn(), listConcepts: vi.fn(), listRules: vi.fn() };
});

import * as api from "../api";

it("renders the Rules page in English", async () => {
  setLang("en", false);
  vi.mocked(api.listRules).mockResolvedValue([
    { id: 1, rule_type: "Validation", name: "Contract needs owner", applies_to: [10], strict: true, properties: {} },
    { id: 2, rule_type: "Inference", name: "Bill on renewal", applies_to: [], strict: false, properties: { status: "reviewed" } },
  ]);
  vi.mocked(api.getStats).mockResolvedValue({
    concepts: 1, relations: 0, rules: 2, actions: 0, concept_types: 1, relation_types: 0, rule_types: 2, action_types: 0,
    deltas: { concepts_pct: 0, relations_pct: 0, concept_types_pct: 0, relation_types_pct: 0 },
  });
  vi.mocked(api.getOntology).mockResolvedValue({ concept_types: {}, relation_types: {}, rule_types: {} });
  vi.mocked(api.listConcepts).mockResolvedValue({ total: 0, concepts: [], next_cursor: null });
  renderPage(<Rules />);
  const table = screen.getByRole("table");
  await within(table).findByText("Contract needs owner");
  expect(screen.getByRole("heading", { name: "Rules" })).toBeInTheDocument();
  expect(screen.getByText("Rule library")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: /Create a rule/ })).toBeInTheDocument();
  expect(within(table).getByText("Reviewed")).toBeInTheDocument();
  expect(within(table).getByText("1 sheet(s)")).toBeInTheDocument();
});
