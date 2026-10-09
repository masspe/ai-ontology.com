// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The Graph View page rendered in English: the dictionary reaches the screen.

import { forwardRef } from "react";
import { expect, it, vi } from "vitest";
import GraphView from "./GraphView";
import { renderPage, screen, waitFor } from "../test/render";
import { setLang } from "../lib/i18n";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return {
    ...actual,
    getSubgraph: vi.fn(),
    getStats: vi.fn(),
    getOntology: vi.fn(),
    getStatsHistory: vi.fn(),
    getQueries: vi.fn(),
    getFiles: vi.fn(),
  };
});

vi.mock("../components/GraphCanvas", () => ({
  default: forwardRef(function Stub() {
    return <div data-testid="canvas" />;
  }),
}));

import * as api from "../api";

it("renders the Graph View page in English", async () => {
  setLang("en", false);
  vi.mocked(api.getStats).mockResolvedValue({
    concepts: 1, relations: 0, rules: 0, actions: 0, concept_types: 1, relation_types: 0, rule_types: 0, action_types: 0,
    deltas: { concepts_pct: 0, relations_pct: 0, concept_types_pct: 0, relation_types_pct: 0 },
  });
  vi.mocked(api.getOntology).mockResolvedValue({ concept_types: { Person: { name: "Person" } }, relation_types: {} });
  vi.mocked(api.getStatsHistory).mockResolvedValue({ samples: [] });
  vi.mocked(api.getQueries).mockResolvedValue({ queries: [] });
  vi.mocked(api.getFiles).mockResolvedValue({ files: [] });
  vi.mocked(api.getSubgraph).mockResolvedValue({ subgraph: { concepts: [], relations: [] } });
  renderPage(<GraphView />, { route: "/graph" });
  expect(screen.getByRole("heading", { name: "Graph" })).toBeInTheDocument();
  expect(screen.getByText("Ontology graph")).toBeInTheDocument();
  expect(screen.getByText("Click a sheet in the graph to see its type, properties and links.")).toBeInTheDocument();
  await waitFor(() => expect(api.getSubgraph).toHaveBeenCalled());
  expect(screen.getByText("No recent activity.")).toBeInTheDocument();
});
