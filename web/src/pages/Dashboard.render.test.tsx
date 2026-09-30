// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// Rendering tests of the home page (ROADMAP §3.9 lot B, point 3): the
// question box, the stat tiles, what is to do (a review in progress, a
// failed import), this week's imports, the last questions, the rules, the
// first-day steps while the graph is empty, the 15 s poll, the error
// banner and the empty states.

import { Route, useLocation } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import Dashboard from "./Dashboard";
import { renderPage, screen, waitFor } from "../test/render";
import type { FileRecord, Ontology, Rule, SavedQuery, Stats, StatsHistory } from "../api";

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

const mocked = api as unknown as {
  getStats: ReturnType<typeof vi.fn>;
  getStatsHistory: ReturnType<typeof vi.fn>;
  getFiles: ReturnType<typeof vi.fn>;
  getQueries: ReturnType<typeof vi.fn>;
  getOntology: ReturnType<typeof vi.fn>;
  listRules: ReturnType<typeof vi.fn>;
};

const NOW = Math.floor(Date.now() / 1000);

const stats: Stats = {
  concepts: 1234,
  relations: 567,
  rules: 3,
  actions: 0,
  concept_types: 12,
  relation_types: 8,
  rule_types: 1,
  action_types: 0,
  deltas: { concepts_pct: 12.4, relations_pct: -7.6, concept_types_pct: 0.02, relation_types_pct: -0.04 },
};

const history: StatsHistory = {
  samples: [0, 1, 2, 3, 4, 5].map((i) => ({
    ts: NOW - (5 - i) * 86_400,
    concepts: 200 * i + 234,
    relations: 100 * i,
    concept_types: 2 + i,
    relation_types: 1 + i,
  })),
};

const ontology: Ontology = {
  concept_types: { Person: { name: "Person" } },
  relation_types: {},
};

const files: FileRecord[] = [
  { id: 1, name: "report.pdf", size: 512, kind: "pdf", status: "processed", uploaded_at: NOW - 5, concepts: 10, relations: 4 },
  { id: 2, name: "data.csv", size: 20_480, kind: "csv", status: "pending", uploaded_at: NOW - 120, concepts: 30, relations: 0 },
  { id: 3, name: "memo.docx", size: 3 * 1024 * 1024, kind: "docx", status: "failed", uploaded_at: NOW - 7_200, concepts: 0, relations: 0 },
  { id: 4, name: "old.xlsx", size: 1, kind: "xlsx", status: "analyzed", uploaded_at: NOW - 30 * 86_400, concepts: 99, relations: 99 },
] as unknown as FileRecord[];

const queries: SavedQuery[] = [
  { id: 1, name: "Renewals", query: "Which contracts renew?", created_at: NOW - 1000, last_run_at: NOW - 30 },
  { id: 2, name: "Never run", query: "x & y", created_at: NOW - 500, last_run_at: null },
] as unknown as SavedQuery[];

const rules: Rule[] = [
  { id: 1, rule_type: "check", name: "invoice has contract", applies_to: [], strict: true },
  { id: 2, rule_type: "check", name: "only Acme", applies_to: [7], strict: false },
  { id: 3, rule_type: "check", name: "soft", applies_to: [], strict: false },
] as unknown as Rule[];

function LocationProbe() {
  const loc = useLocation();
  return <div data-testid="location">{loc.pathname + loc.search}</div>;
}

const tile = (label: string) => screen.getByText(label, { selector: ".stat-label" }).closest(".stat-rich") as HTMLElement;
const card = (title: string) => screen.getByText(title, { selector: ".card-title > span" }).closest(".card") as HTMLElement;
const tileValue = (label: string) => tile(label).querySelector(".stat-value")!.textContent;

beforeEach(() => {
  mocked.getStats.mockResolvedValue(stats);
  mocked.getStatsHistory.mockResolvedValue(history);
  mocked.getFiles.mockResolvedValue({ files });
  mocked.getQueries.mockResolvedValue({ queries });
  mocked.getOntology.mockResolvedValue(ontology);
  mocked.listRules.mockResolvedValue(rules);
});

async function loaded() {
  await waitFor(() => expect(tileValue("Fiches")).toBe("1,234"));
}

describe("Accueil", () => {
  it("fills the tiles and words the subtitle from the stats", async () => {
    renderPage(<Dashboard />);
    await loaded();
    expect(screen.getByText("1,234 fiches et 567 liens, 12 types de fiches.")).toBeInTheDocument();
    expect(tileValue("Liens")).toBe("567");
    expect(tileValue("Types de fiches")).toBe("12");
    expect(tileValue("Règles")).toBe("3");
    expect(tile("Fiches").querySelector(".stat-delta")).toHaveClass("up");
    expect(tile("Fiches").querySelector("polyline")!.getAttribute("points")!.split(" ")).toHaveLength(6);
  });

  it("sends the question to the Questions page, and ignores a blank one", async () => {
    const { user } = renderPage(<Dashboard />, {
      extraRoutes: <Route path="*" element={<LocationProbe />} />,
      path: "/",
    });
    await loaded();
    const ask = screen.getByRole("button", { name: "Demander" });
    expect(ask).toBeDisabled();
    await user.type(screen.getByRole("textbox", { name: "Question" }), "  ");
    expect(ask).toBeDisabled();
    await user.type(screen.getByRole("textbox", { name: "Question" }), "contrats & échéances{Enter}");
    // The probe is mounted on "*": leaving "/" renders it.
    expect(await screen.findByTestId("location")).toHaveTextContent("/queries?q=contrats%20%26%20%C3%A9ch%C3%A9ances");
  });

  it("lists what is to do: the review in progress and the failed imports", async () => {
    window.sessionStorage.setItem(
      "ingest.draft.v1",
      JSON.stringify({ proposal: { concepts: [1, 2, 3], relations: [1], source: { name: "contrat.pdf" } }, decisions: {} }),
    );
    renderPage(<Dashboard />);
    await loaded();
    expect(card("À faire (2)")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Relecture en cours : contrat.pdf" })).toHaveAttribute("href", "/ingest");
    expect(screen.getByText(/3 fiche\(s\) et 1 lien\(s\) proposés/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Import en échec : memo.docx" })).toHaveAttribute("href", "/files");
  });

  it("sums this week's imports and lists the latest files with their status", async () => {
    renderPage(<Dashboard />);
    await loaded();
    // old.xlsx is 30 days old: not this week, but still among the latest.
    const week = card("Cette semaine") as HTMLElement;
    expect(week).toHaveTextContent("3 document(s) importé(s) : 40 fiche(s) et 4 lien(s) ajoutés.");
    expect(week).toHaveTextContent("old.xlsx");
    expect(week).toHaveTextContent("report.pdf");
    expect(screen.getByRole("link", { name: "Déposer des fichiers" })).toHaveAttribute("href", "/files");
  });

  it("orders the last questions by their last run and links each to its query", async () => {
    renderPage(<Dashboard />);
    await loaded();
    const items = card("Dernières questions")!.querySelectorAll("li");
    expect(items).toHaveLength(2);
    expect(items[0]).toHaveTextContent("Renewals");
    expect(screen.getByRole("link", { name: "Never run" })).toHaveAttribute("href", "/queries?q=x%20%26%20y");
  });

  it("counts the rules, the strict and the general ones", async () => {
    renderPage(<Dashboard />);
    await loaded();
    const rulesCard = card("Règles");
    expect(rulesCard).toHaveTextContent("3 règle(s), dont 1 stricte(s) ; 2 générale(s).");
    expect(screen.getByRole("link", { name: "Gérer" })).toHaveAttribute("href", "/rules");
  });

  it("shows the empty states when nothing happened yet (but there is data)", async () => {
    mocked.getFiles.mockResolvedValue({ files: [] });
    mocked.getQueries.mockRejectedValue(new Error("no queries"));
    mocked.listRules.mockRejectedValue(new Error("no rules"));
    mocked.getStatsHistory.mockResolvedValue({ samples: [] });
    renderPage(<Dashboard />);
    await loaded();
    expect(card("À faire (0)")).toBeInTheDocument();
    expect(screen.getByText(/Rien en attente/)).toBeInTheDocument();
    expect(screen.getByText(/Aucun document importé ces 7 derniers jours/)).toBeInTheDocument();
    expect(screen.getByText(/Aucune question enregistrée/)).toBeInTheDocument();
    expect(screen.getByText(/Aucune règle\./)).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("shows the first-day steps while the graph is empty, the dashboard once there is data", async () => {
    mocked.getStats.mockResolvedValue({ ...stats, concepts: 0 });
    renderPage(<Dashboard />);
    expect(await screen.findByRole("heading", { name: "Bienvenue" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Accueil" })).not.toBeInTheDocument();
  });

  it("shows the error banner when a required request fails, and clears it on the next tick", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      mocked.getStats.mockRejectedValueOnce(new Error("stats down"));
      renderPage(<Dashboard />);
      expect(await screen.findByText("stats down")).toHaveClass("error-banner");
      await vi.advanceTimersByTimeAsync(15_000);
      await loaded();
      expect(screen.queryByText("stats down")).toBeNull();
      expect(mocked.getStats).toHaveBeenCalledTimes(2);
    } finally {
      vi.useRealTimers();
    }
  });

  it("ignores responses that arrive after unmount", async () => {
    let resolve: (v: Stats) => void = () => {};
    mocked.getStats.mockReturnValue(new Promise<Stats>((r) => (resolve = r)));
    const { unmount } = renderPage(<Dashboard />);
    unmount();
    resolve(stats);
    await Promise.resolve();
    expect(screen.queryByText(/fiches et/)).toBeNull();
  });
});
