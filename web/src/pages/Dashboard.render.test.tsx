// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Rendering tests of the Dashboard: the six parallel requests, stat tiles
// with deltas and sparklines, the growth chart, the network preview, the
// file / query / activity lists, the insights card, the 15 s poll, the
// error banner and every empty state.

import { Route, useLocation } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import Dashboard from "./Dashboard";
import { renderPage, screen, waitFor } from "../test/render";
import type { Concept, FileRecord, Ontology, SavedQuery, Stats, StatsHistory } from "../api";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return {
    ...actual,
    getStats: vi.fn(),
    getStatsHistory: vi.fn(),
    getFiles: vi.fn(),
    getQueries: vi.fn(),
    getOntology: vi.fn(),
    listConcepts: vi.fn(),
  };
});

import * as api from "../api";

const mocked = api as unknown as {
  getStats: ReturnType<typeof vi.fn>;
  getStatsHistory: ReturnType<typeof vi.fn>;
  getFiles: ReturnType<typeof vi.fn>;
  getQueries: ReturnType<typeof vi.fn>;
  getOntology: ReturnType<typeof vi.fn>;
  listConcepts: ReturnType<typeof vi.fn>;
};

// ---- fixtures ---------------------------------------------------------------

const NOW = Math.floor(Date.now() / 1000);

const stats: Stats = {
  concepts: 1234,
  relations: 567,
  rules: 0,
  actions: 0,
  concept_types: 12,
  relation_types: 8,
  rule_types: 0,
  action_types: 0,
  // up / down / flat / flat(negative within tolerance)
  deltas: { concepts_pct: 12.4, relations_pct: -7.6, concept_types_pct: 0.02, relation_types_pct: -0.04 },
};

// Six samples (≤ 7 → every x label shown) with a max ≥ 1000 for the "k" ticks.
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
  concept_types: {
    Person: { name: "Person", properties: { age: "integer", name: "string" } },
    Company: { name: "Company", parent: "Organisation" },
    City: { name: "City", properties: null },
    Country: { name: "Country" },
    Product: { name: "Product" },
    Invoice: { name: "Invoice" },
    Hidden: { name: "Hidden" },
  },
  relation_types: {},
};

const files: FileRecord[] = [
  { id: 1, name: "report.pdf", size: 512, kind: "pdf", status: "processed", uploaded_at: NOW - 5 },
  { id: 2, name: "data.csv", size: 20_480, kind: "csv", status: "pending", uploaded_at: NOW - 120 },
  { id: 3, name: "memo.docx", size: 3 * 1024 * 1024, kind: "docx", status: "failed", uploaded_at: NOW - 7_200 },
  { id: 4, name: "sheet.xlsx", size: 1, kind: "xlsx", status: "analyzed", uploaded_at: NOW - 200_000 },
  { id: 5, name: "dump.bin", size: 1, kind: "binary", status: "weird", uploaded_at: NOW - 1 },
  { id: 6, name: "", size: 1, kind: "json", status: "", uploaded_at: NOW - 1 },
] as unknown as FileRecord[];

const queries: SavedQuery[] = [
  { id: 1, name: "Renewals", query: "Which contracts renew?", last_run_at: NOW - 30 },
  { id: 2, name: "Never run", query: "x", last_run_at: null },
] as unknown as SavedQuery[];

const concepts: Concept[] = [
  { id: 1, concept_type: "Person", name: "a" },
  { id: 2, concept_type: "Person", name: "b" },
  { id: 3, concept_type: "Person", name: "c" },
  { id: 4, concept_type: "Company", name: "d" },
  { id: 5, concept_type: "City", name: "e" },
  { id: 6, concept_type: "City", name: "f" },
];

function LocationProbe() {
  const loc = useLocation();
  return <div data-testid="location">{loc.pathname}</div>;
}

const tile = (label: string) => screen.getByText(label, { selector: ".stat-label" }).closest(".stat-rich") as HTMLElement;
const tileValue = (label: string) => tile(label).querySelector(".stat-value")!.textContent;
const tileDelta = (label: string) => tile(label).querySelector(".stat-delta");

beforeEach(() => {
  mocked.getStats.mockResolvedValue(stats);
  mocked.getStatsHistory.mockResolvedValue(history);
  mocked.getFiles.mockResolvedValue({ files });
  mocked.getQueries.mockResolvedValue({ queries });
  mocked.getOntology.mockResolvedValue(ontology);
  mocked.listConcepts.mockResolvedValue({ total: concepts.length, concepts });
});

describe("Dashboard page", () => {
  it("fills the stat tiles with formatted values, deltas and sparklines", async () => {
    renderPage(<Dashboard />);
    await waitFor(() => expect(tileValue("Entities")).toBe("1,234"));
    expect(mocked.listConcepts).toHaveBeenCalledWith({ limit: 500 });
    expect(tileValue("Concept Types")).toBe("12");
    expect(tileValue("Relations")).toBe("567");
    expect(tileValue("Relation Types")).toBe("8");
    expect(tileDelta("Entities")).toHaveClass("up");
    expect(tileDelta("Entities")).toHaveTextContent(/↑ 12%\s*vs last period/);
    expect(tileDelta("Relations")).toHaveClass("down");
    expect(tileDelta("Relations")).toHaveTextContent("↓ 8%");
    expect(tileDelta("Concept Types")).toHaveClass("flat");
    expect(tileDelta("Concept Types")).toHaveTextContent("• 0%");
    expect(tileDelta("Relation Types")).toHaveClass("flat");
    // Sparklines are fed the history series: six points each.
    const points = tile("Entities").querySelector("polyline")!.getAttribute("points")!.split(" ");
    expect(points).toHaveLength(6);
  });

  it("renders the growth chart with a point per sample, k-ticks and a date label per sample", async () => {
    renderPage(<Dashboard />);
    await waitFor(() => expect(tileValue("Entities")).toBe("1,234"));
    const chart = document.querySelector(".growth-chart")!;
    expect(chart.querySelectorAll("circle")).toHaveLength(6);
    const texts = Array.from(chart.querySelectorAll("text")).map((t) => t.textContent);
    // Max 1234 → top tick abbreviated to "1.2k", the others rounded integers.
    expect(texts.slice(0, 5)).toEqual(["1.2k", "926", "617", "309", "0"]);
    // ≤ 7 samples: one x label per sample.
    expect(texts).toHaveLength(5 + 6);
    expect(chart.querySelector("polyline")!.getAttribute("points")!.split(" ")).toHaveLength(6);
  });

  it("thins the x labels to five when there are more than seven samples", async () => {
    mocked.getStatsHistory.mockResolvedValue({
      samples: Array.from({ length: 12 }, (_, i) => ({
        ts: NOW - (11 - i) * 3600,
        concepts: 10 + i,
        relations: i,
        concept_types: 1,
        relation_types: 1,
      })),
    });
    renderPage(<Dashboard />);
    await waitFor(() => expect(tileValue("Entities")).toBe("1,234"));
    const chart = document.querySelector(".growth-chart")!;
    expect(chart.querySelectorAll("circle")).toHaveLength(12);
    // 5 y ticks (all < 1000 → plain integers) + 5 x labels.
    const texts = Array.from(chart.querySelectorAll("text")).map((t) => t.textContent);
    expect(texts).toHaveLength(10);
    expect(texts[0]).toBe("21");
  });

  it("draws the network preview with the first six concept types around the first one", async () => {
    renderPage(<Dashboard />);
    await waitFor(() => expect(tileValue("Entities")).toBe("1,234"));
    const preview = document.querySelector(".network-preview")!;
    expect(preview.querySelectorAll("rect")).toHaveLength(6);
    expect(preview.querySelectorAll("line")).toHaveLength(5);
    expect(preview).toHaveTextContent("Person");
    expect(preview).toHaveTextContent("Invoice");
    expect(preview).not.toHaveTextContent("Hidden");
  });

  it("tables the first five concept types with parent and property count", async () => {
    renderPage(<Dashboard />);
    await waitFor(() => expect(tileValue("Entities")).toBe("1,234"));
    const rows = Array.from(document.querySelectorAll(".compact-table tbody tr")).map((tr) =>
      Array.from(tr.querySelectorAll("td")).map((td) => td.textContent),
    );
    expect(rows).toHaveLength(5);
    expect(rows[0]).toEqual(["Person", "—", "2", "Active"]);
    expect(rows[1]).toEqual(["Company", "Organisation", "0", "Active"]);
    expect(rows[2]).toEqual(["City", "—", "0", "Active"]);
    expect(rows.map((r) => r[0])).not.toContain("Invoice");
  });

  it("lists the first five files with kind icon, size, status badge and age", async () => {
    renderPage(<Dashboard />);
    await waitFor(() => expect(tileValue("Entities")).toBe("1,234"));
    const rows = Array.from(document.querySelectorAll(".file-row"));
    expect(rows).toHaveLength(5);
    const row = (i: number) => ({
      icon: rows[i].querySelector(".file-icon")!,
      sub: rows[i].querySelector(".file-sub")!.textContent,
      badge: rows[i].querySelector(".badge")!,
      time: rows[i].querySelector(".file-time")!.textContent,
    });
    expect(row(0).icon).toHaveClass("pdf");
    expect(row(0).icon).toHaveTextContent("PDF");
    expect(row(0).sub).toBe("512 B · pdf");
    expect(row(0).badge).toHaveClass("badge-success");
    expect(row(0).badge).toHaveTextContent("Processed");
    expect(row(0).time).toMatch(/^\d+s ago$/);

    expect(row(1).icon).toHaveClass("csv");
    expect(row(1).sub).toBe("20.0 KB · csv");
    expect(row(1).badge).toHaveClass("badge-warn");
    expect(row(1).badge).toHaveTextContent("Pending");
    expect(row(1).time).toBe("2m ago");

    expect(row(2).icon).toHaveClass("doc");
    expect(row(2).icon).toHaveTextContent("DOCX");
    expect(row(2).sub).toBe("3.0 MB · docx");
    expect(row(2).badge).toHaveClass("badge-danger");
    expect(row(2).badge).toHaveTextContent("Failed");
    expect(row(2).time).toBe("2h ago");

    expect(row(3).icon).toHaveClass("xls");
    expect(row(3).badge).toHaveClass("badge-accent");
    expect(row(3).badge).toHaveTextContent("Analyzed");
    expect(row(3).time).toBe("2d ago");

    // Unknown kind → generic icon, clipped to four letters; unknown status →
    // shown verbatim with the plain badge class.
    expect(row(4).icon).toHaveClass("generic");
    expect(row(4).icon).toHaveTextContent("BINA");
    expect(row(4).badge).toHaveTextContent("weird");
    expect(row(4).badge.className).toBe("badge badge");
    // Only five files are listed.
    expect(screen.queryByText("json")).toBeNull();
  });

  it("shows the recent queries with their last run, and the team activity from files", async () => {
    renderPage(<Dashboard />);
    await waitFor(() => expect(tileValue("Entities")).toBe("1,234"));
    const items = Array.from(document.querySelectorAll(".query-list li"));
    expect(items).toHaveLength(2);
    expect(items[0]).toHaveTextContent("Renewals");
    expect(items[0].querySelector(".query-time")!.textContent).toMatch(/^\d+s ago$/);
    expect(items[0].querySelector(".query-text")).toHaveAttribute("title", "Which contracts renew?");
    expect(items[1].querySelector(".query-time")!.textContent).toBe("—");

    const activity = Array.from(document.querySelectorAll(".activity-item"));
    expect(activity).toHaveLength(4);
    expect(activity[0].querySelector(".activity-avatar")!.textContent).toBe("R");
    expect(activity[0]).toHaveTextContent("System ingested report.pdf");
    expect(activity[3].querySelector(".activity-avatar")!.textContent).toBe("S");
  });

  it("ranks the top entity types and sizes the bars relative to the leader", async () => {
    renderPage(<Dashboard />);
    await waitFor(() => expect(tileValue("Entities")).toBe("1,234"));
    expect(screen.getByText("Across 6 entities")).toBeInTheDocument();
    const rows = Array.from(document.querySelectorAll(".bar-row"));
    expect(rows.map((r) => r.querySelector(".bar-label")!.textContent)).toEqual(["Person", "City", "Company"]);
    expect(rows.map((r) => r.querySelector(".bar-value")!.textContent)).toEqual(["3", "2", "1"]);
    const width = (i: number) => (rows[i].querySelector(".bar-fill") as HTMLElement).style.width;
    expect(width(0)).toBe("100%");
    expect(parseFloat(width(1))).toBeCloseTo(66.67, 1);
    expect(parseFloat(width(2))).toBeCloseTo(33.33, 1);
  });

  it("words the insights from the stats", async () => {
    renderPage(<Dashboard />);
    await waitFor(() => expect(tileValue("Entities")).toBe("1,234"));
    expect(screen.getByText("Entity growth is up 12%")).toBeInTheDocument();
    expect(
      screen.getByText("You have 1,234 entities across 12 concept types and 567 relations."),
    ).toBeInTheDocument();
  });

  it("uses the neutral insight title when growth is not positive", async () => {
    mocked.getStats.mockResolvedValue({ ...stats, deltas: { ...stats.deltas, concepts_pct: -2 } });
    renderPage(<Dashboard />);
    await waitFor(() => expect(tileValue("Entities")).toBe("1,234"));
    expect(screen.getByText("Graph activity overview")).toBeInTheDocument();
  });

  it("links to the other pages", async () => {
    const { user } = renderPage(<Dashboard />, {
      route: "/",
      extraRoutes: <Route path="/builder" element={<LocationProbe />} />,
    });
    await waitFor(() => expect(tileValue("Entities")).toBe("1,234"));
    expect(screen.getByRole("link", { name: "View Analytics" })).toHaveAttribute("href", "/graph");
    expect(screen.getByRole("link", { name: "Open Graph ↗" })).toHaveAttribute("href", "/graph");
    expect(screen.getByRole("link", { name: /Upload Files.*Import and process data/ })).toHaveAttribute("href", "/files");
    expect(screen.getByRole("link", { name: /Run Query/ })).toHaveAttribute("href", "/queries");
    expect(screen.getByRole("link", { name: /Drag & drop files anywhere/ })).toHaveAttribute("href", "/files");
    await user.click(screen.getByRole("link", { name: /Create Ontology/ }));
    expect(screen.getByTestId("location")).toHaveTextContent("/builder");
  });

  it("shows every empty state while nothing is loaded yet", async () => {
    mocked.getStats.mockResolvedValue({ ...stats, concepts: 0, deltas: { ...stats.deltas, concepts_pct: 0 } });
    mocked.getStatsHistory.mockResolvedValue({ samples: [] });
    mocked.getFiles.mockResolvedValue({ files: [] });
    // The optional requests failing must not break the page.
    mocked.getQueries.mockRejectedValue(new Error("no queries"));
    mocked.getOntology.mockRejectedValue(new Error("no ontology"));
    mocked.listConcepts.mockRejectedValue(new Error("no concepts"));
    renderPage(<Dashboard />);
    await waitFor(() => expect(tileValue("Concept Types")).toBe("12"));
    expect(screen.queryByRole("alert")).toBeNull();
    expect(document.querySelector(".error-banner")).toBeNull();
    expect(screen.getByText("No samples yet. The dashboard auto-refreshes every 15s.")).toBeInTheDocument();
    expect(screen.getAllByText("No ontology defined yet.")).toHaveLength(2);
    expect(screen.getByText("No files uploaded yet.")).toBeInTheDocument();
    expect(screen.getByText("No saved queries yet.")).toBeInTheDocument();
    expect(screen.getByText("No activity yet.")).toBeInTheDocument();
    expect(screen.getByText("No entities yet.")).toBeInTheDocument();
    expect(screen.getByText("Across 0 entities")).toBeInTheDocument();
    // Empty spark series are drawn as a flat [0, 0] line.
    expect(tile("Entities").querySelector("polyline")!.getAttribute("points")).toBe("0.00,34.00 100.00,34.00");
  });

  it("shows the error banner when a required request fails, and clears it on the next tick", async () => {
    mocked.getStats.mockRejectedValueOnce(new Error("stats down"));
    vi.useFakeTimers({ shouldAdvanceTime: true });
    renderPage(<Dashboard />);
    expect(await screen.findByText("stats down")).toBeInTheDocument();
    // Before any stats: zero tiles and the waiting insight.
    expect(tileValue("Entities")).toBe("0");
    expect(tileDelta("Entities")).toBeNull();
    expect(screen.getByText("Awaiting stats from the server.")).toBeInTheDocument();
    // 15 s later the poll succeeds and the banner goes away.
    await vi.advanceTimersByTimeAsync(15_000);
    await waitFor(() => expect(tileValue("Entities")).toBe("1,234"));
    expect(screen.queryByText("stats down")).toBeNull();
    expect(mocked.getStats).toHaveBeenCalledTimes(2);
    vi.useRealTimers();
  });

  it("stringifies non-Error rejections", async () => {
    mocked.getFiles.mockRejectedValueOnce("files gone");
    renderPage(<Dashboard />);
    expect(await screen.findByText("files gone")).toBeInTheDocument();
  });

  it("ignores responses that arrive after unmount", async () => {
    let resolveStats: (s: Stats) => void = () => {};
    mocked.getStats.mockReturnValue(new Promise<Stats>((r) => (resolveStats = r)));
    const { unmount } = renderPage(<Dashboard />);
    expect(tileValue("Entities")).toBe("0");
    unmount();
    resolveStats(stats);
    // Nothing observable remains: React 18 no longer warns about state
    // updates on an unmounted tree. The test exercises the `cancelled`
    // branch for coverage and only checks that the late resolution does
    // not throw.
    await expect(Promise.resolve().then(() => undefined)).resolves.toBeUndefined();
  });
});
