// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The home page in English.

import { beforeEach, describe, expect, it, vi } from "vitest";
import Dashboard from "./Dashboard";
import { renderPage, screen } from "../test/render";
import { setLang } from "../lib/i18n";
import type { Stats } from "../api";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return {
    ...actual,
    getStats: vi.fn(),
    getStatsHistory: vi.fn(),
    getFiles: vi.fn(),
    getQueries: vi.fn(),
    getOntology: vi.fn(),
    listRules: vi.fn(),
  };
});

import * as api from "../api";

const mocked = api as unknown as Record<string, ReturnType<typeof vi.fn>>;
const stats = {
  concepts: 1234,
  relations: 567,
  rules: 0,
  actions: 0,
  concept_types: 12,
  relation_types: 8,
  rule_types: 0,
  action_types: 0,
  deltas: { concepts_pct: 0, relations_pct: 0, concept_types_pct: 0, relation_types_pct: 0 },
} as Stats;

beforeEach(() => {
  setLang("en", false);
  mocked.getStats!.mockResolvedValue(stats);
  mocked.getStatsHistory!.mockResolvedValue({ samples: [] });
  mocked.getFiles!.mockResolvedValue({ files: [] });
  mocked.getQueries!.mockResolvedValue({ queries: [] });
  mocked.getOntology!.mockResolvedValue({ concept_types: { Person: { name: "Person" } }, relation_types: {} });
  mocked.listRules!.mockResolvedValue([]);
});

describe("Dashboard, English", () => {
  it("words the subtitle, the cards and the empty states in English", async () => {
    renderPage(<Dashboard />);
    expect(await screen.findByText("1,234 sheets and 567 links, 12 sheet types.")).toBeInTheDocument();
    expect(screen.getByText("Home")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Ask" })).toBeInTheDocument();
    expect(screen.getByText("Nothing pending. Drop a document or ask a question.")).toBeInTheDocument();
    expect(screen.getByText("No saved question. Ask the first one above.")).toBeInTheDocument();
  });
});
