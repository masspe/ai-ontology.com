// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// Rendering tests of the Graph View page: subgraph loading and refresh,
// client-side filters (search, node type, relation type, depth), the KPI
// tiles, the canvas toolbar, the node inspector (selection, properties,
// rules, actions), and the three bottom cards. The React Flow canvas has
// its own tests; here it is replaced by a stub that surfaces its callbacks
// as buttons and records the imperative handle calls.

import { forwardRef, useImperativeHandle } from "react";
import { Route, useLocation } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import GraphView from "./GraphView";
import { fireEvent, renderPage, screen, waitFor, within } from "../test/render";
import type { FileRecord, Ontology, SavedQuery, StatsHistory, Subgraph } from "../api";
import type { GraphCanvasHandle } from "../components/GraphCanvas";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return {
    ...actual,
    getSubgraph: vi.fn(),
    getOntology: vi.fn(),
    getStatsHistory: vi.fn(),
    getQueries: vi.fn(),
    getFiles: vi.fn(),
  };
});

// The canvas handle methods, recorded so the toolbar can be asserted on.
const handle = { fit: vi.fn(), zoomIn: vi.fn(), zoomOut: vi.fn(), focusNode: vi.fn() };

vi.mock("../components/GraphCanvas", () => {
  interface StubProps {
    subgraph: Subgraph | null;
    layoutDir?: string;
    showLabels?: boolean;
    highlightPaths?: boolean;
    selectedNodeId?: string | null;
    conceptTypeColors?: Record<string, string>;
    onNodeClick?: (id: string) => void;
    onPaneClick?: () => void;
    onNodeDoubleClick?: (id: string) => void;
  }
  const Stub = forwardRef<GraphCanvasHandle, StubProps>(function Stub(props, ref) {
    useImperativeHandle(ref, () => handle, []);
    return (
      <div data-testid="canvas">
        <div data-testid="canvas-nodes">{(props.subgraph?.concepts ?? []).map((c) => c.name).join(",")}</div>
        <div data-testid="canvas-rels">{(props.subgraph?.relations ?? []).map((r) => r.relation_type).join(",")}</div>
        <div data-testid="canvas-layout">{props.layoutDir}</div>
        <div data-testid="canvas-labels">{String(props.showLabels)}</div>
        <div data-testid="canvas-highlight">{String(props.highlightPaths)}</div>
        <div data-testid="canvas-selected">{props.selectedNodeId ?? "none"}</div>
        <div data-testid="canvas-colors">{JSON.stringify(props.conceptTypeColors ?? {})}</div>
        {(props.subgraph?.concepts ?? []).map((c) => (
          <button key={c.id} onClick={() => props.onNodeClick?.(String(c.id))}>{`node ${c.id}`}</button>
        ))}
        {(props.subgraph?.concepts ?? []).map((c) => (
          <button key={c.id} onClick={() => props.onNodeDoubleClick?.(String(c.id))}>{`dbl ${c.id}`}</button>
        ))}
        <button onClick={() => props.onPaneClick?.()}>pane</button>
      </div>
    );
  });
  return { default: Stub };
});

import * as api from "../api";

const mocked = api as unknown as {
  getSubgraph: ReturnType<typeof vi.fn>;
  getOntology: ReturnType<typeof vi.fn>;
  getStatsHistory: ReturnType<typeof vi.fn>;
  getQueries: ReturnType<typeof vi.fn>;
  getFiles: ReturnType<typeof vi.fn>;
};

// ---- fixtures ---------------------------------------------------------------

const NOW = Math.floor(Date.now() / 1000);

const subgraph: Subgraph = {
  concepts: [
    // Own properties covering every xsd mapping.
    {
      id: 1,
      concept_type: "Person",
      name: "Alice",
      description: "A person",
      properties: { age: 42, score: 1.5, active: true, born: "1984-02-03", nick: "ally", tags: ["a"], none: null },
    },
    // No own properties: falls back to the type schema.
    { id: 2, concept_type: "Company", name: "ACME" },
    // No own properties and no schema: nothing declared.
    { id: 3, concept_type: "City", name: "Geneva" },
    { id: 4, concept_type: "Person", name: "Bob", properties: {} },
  ],
  relations: [
    { id: 10, relation_type: "worksAt", source: 1, target: 2 },
    { id: 11, relation_type: "knows", source: 4, target: 1 },
    { id: 12, relation_type: "basedIn", source: 2, target: 3 },
  ],
};

const ontology: Ontology = {
  concept_types: {
    Person: { name: "Person", description: "Human being", properties: { age: "integer", extra: 3 } },
    Company: { name: "Company", description: "A legal entity", properties: { vat: "string", founded: { nested: true } } },
    City: { name: "City" },
  },
  relation_types: {
    worksAt: { name: "worksAt", domain: "Person", range: "Company", cardinality: "n:1", symmetric: false },
    knows: { name: "knows", domain: "Person", range: "Person", cardinality: "n:n", symmetric: true },
    basedIn: { name: "basedIn", domain: "Company", range: "City", cardinality: "n:1", symmetric: false },
    owns: { name: "owns", domain: "Person", range: "Company", cardinality: "n:n", symmetric: false },
  },
  rule_types: {
    R1: { name: "R1", strict: true, description: "Strict one", when: "x", then: "y", applies_to: ["Person", "Company"] },
    R2: { name: "R2" },
  },
  action_types: {
    A1: { name: "A1", subject: "Person", object: "Company", parameters: ["p1", "p2"], effect: "hired", description: "Hire" },
    A2: { name: "A2", subject: "City" },
  },
};

const history: StatsHistory = {
  samples: [
    { ts: NOW - 200, concepts: 10, relations: 5, concept_types: 2, relation_types: 1 },
    { ts: NOW - 100, concepts: 12, relations: 8, concept_types: 3, relation_types: 2 },
  ],
};

const queries: SavedQuery[] = [
  { id: 1, name: "Just now", query: "q1", last_run_at: NOW - 5 },
  { id: 2, name: "Minutes", query: "q2", last_run_at: NOW - 120 },
  { id: 3, name: "Hours", query: "q3", last_run_at: NOW - 7200 },
  { id: 4, name: "Days", query: "q4", last_run_at: NOW - 200_000 },
  { id: 5, name: "Never (hidden)", query: "q5", last_run_at: null },
] as unknown as SavedQuery[];

const files: FileRecord[] = [
  { id: 1, name: "done.pdf", status: "processed", uploaded_at: NOW - 10 },
  { id: 2, name: "bad.csv", status: "failed", uploaded_at: NOW - 20 },
  { id: 3, name: "seen.json", status: "analyzed", uploaded_at: NOW - 30 },
  { id: 4, name: "wip.docx", status: "pending", uploaded_at: NOW - 40 },
  { id: 5, name: "old.txt", status: "processed", uploaded_at: NOW - 50 },
] as unknown as FileRecord[];

function LocationProbe() {
  const loc = useLocation();
  return <div data-testid="location">{loc.pathname + loc.search}</div>;
}
const probeRoutes = <Route path="/queries" element={<LocationProbe />} />;

const canvasNodes = () => screen.getByTestId("canvas-nodes").textContent;
const canvasRels = () => screen.getByTestId("canvas-rels").textContent;
const kpiValue = (label: string) =>
  screen.getByText(label, { selector: ".stat-label" }).parentElement!.querySelector(".stat-value")!.textContent;

async function mount(route = "/graph") {
  const page = renderPage(<GraphView />, { route, extraRoutes: probeRoutes });
  await waitFor(() => expect(canvasNodes()).toBe("Alice,ACME,Geneva,Bob"));
  return page;
}

beforeEach(() => {
  handle.fit.mockReset();
  handle.zoomIn.mockReset();
  handle.zoomOut.mockReset();
  handle.focusNode.mockReset();
  mocked.getSubgraph.mockResolvedValue({ subgraph });
  mocked.getOntology.mockResolvedValue(ontology);
  mocked.getStatsHistory.mockResolvedValue(history);
  mocked.getQueries.mockResolvedValue({ queries });
  mocked.getFiles.mockResolvedValue({ files });
});

describe("GraphView page", () => {
  it("loads the subgraph with the default request and fills the KPI tiles", async () => {
    await mount();
    // The mount effect and the depth effect both fetch on the first render.
    expect(mocked.getSubgraph).toHaveBeenCalledWith({
      seed_query: undefined,
      seed_concept_types: [],
      expansion_depth: 3,
      limit: 250,
    });
    expect(kpiValue("Fiches")).toBe("4");
    expect(kpiValue("Liens")).toBe("3");
    expect(kpiValue("Filtres actifs")).toBe("0");
    // 3 relations / 4 nodes * 50 = 37.5 → 38%
    expect(kpiValue("Santé du graphe")).toBe("38%");
    // The canvas gets a colour per concept type, in ontology order.
    expect(JSON.parse(screen.getByTestId("canvas-colors").textContent!)).toEqual({
      Person: "#2563eb",
      Company: "#7c3aed",
      City: "#16a34a",
    });
    // Ontology types populate the two selects.
    expect(screen.getByRole("option", { name: "Company" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "owns" })).toBeInTheDocument();
  });

  it("shows the error banner when the subgraph request fails, and survives the side requests failing", async () => {
    mocked.getSubgraph.mockRejectedValue(new Error("graph exploded"));
    mocked.getOntology.mockRejectedValue(new Error("no ontology"));
    mocked.getStatsHistory.mockRejectedValue("nope");
    mocked.getQueries.mockRejectedValue("nope");
    mocked.getFiles.mockRejectedValue("nope");
    renderPage(<GraphView />);
    expect(await screen.findByText("graph exploded")).toBeInTheDocument();
    expect(kpiValue("Fiches")).toBe("0");
    expect(kpiValue("Santé du graphe")).toBe("0%");
    expect(screen.getByText("Aucune vue enregistrée pour l'instant.")).toBeInTheDocument();
    expect(screen.getByText("Aucune activité récente.")).toBeInTheDocument();
    expect(screen.getByText("Aucune suggestion disponible.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^Règles/ }).querySelector(".gv-tab-count")).toHaveTextContent("0");
    // A non-Error rejection is stringified.
    mocked.getSubgraph.mockRejectedValue("plain failure");
    renderPage(<GraphView />);
    expect(await screen.findByText("plain failure")).toBeInTheDocument();
  });

  it("filters nodes client-side by search text (name or type) and counts the filter", async () => {
    const { user } = await mount();
    const input = screen.getByPlaceholderText("Rechercher une fiche…");
    await user.type(input, "ali");
    expect(canvasNodes()).toBe("Alice");
    expect(canvasRels()).toBe("");
    expect(kpiValue("Filtres actifs")).toBe("1");
    await user.clear(input);
    await user.type(input, "person");
    expect(canvasNodes()).toBe("Alice,Bob");
    expect(canvasRels()).toBe("knows");
    // Refresh sends the search text as the seed query.
    await user.click(screen.getByRole("button", { name: "Actualiser" }));
    await waitFor(() =>
      expect(mocked.getSubgraph).toHaveBeenLastCalledWith(expect.objectContaining({ seed_query: "person", expansion_depth: 3 })),
    );
  });

  it("filters by node type and relation type", async () => {
    const { user } = await mount();
    const [nodeSelect, relSelect] = screen.getAllByRole("combobox");
    await user.selectOptions(nodeSelect, "Person");
    expect(canvasNodes()).toBe("Alice,Bob");
    expect(canvasRels()).toBe("knows");
    expect(kpiValue("Filtres actifs")).toBe("1");
    await user.selectOptions(nodeSelect, "Tous les types");
    await user.selectOptions(relSelect, "basedIn");
    expect(canvasNodes()).toBe("Alice,ACME,Geneva,Bob");
    expect(canvasRels()).toBe("basedIn");
    expect(kpiValue("Filtres actifs")).toBe("1");
    // The node type is sent as a seed type on refresh.
    await user.selectOptions(nodeSelect, "Company");
    await user.click(screen.getByRole("button", { name: "Actualiser" }));
    await waitFor(() =>
      expect(mocked.getSubgraph).toHaveBeenLastCalledWith(expect.objectContaining({ seed_concept_types: ["Company"] })),
    );
    expect(kpiValue("Filtres actifs")).toBe("2");
  });

  it("refetches with the new depth when the slider moves", async () => {
    await mount();
    const calls = mocked.getSubgraph.mock.calls.length;
    fireEvent.change(screen.getByRole("slider"), { target: { value: "5" } });
    expect(screen.getByText("5 niveaux")).toBeInTheDocument();
    await waitFor(() => expect(mocked.getSubgraph).toHaveBeenCalledTimes(calls + 1));
    expect(mocked.getSubgraph).toHaveBeenLastCalledWith(expect.objectContaining({ expansion_depth: 5 }));
    expect(kpiValue("Filtres actifs")).toBe("1");
  });

  it("wires the toggles to the canvas", async () => {
    const { user } = await mount();
    const sw = (label: string) => screen.getByText(label).closest("label")!.querySelector("[role=switch]")!;
    expect(screen.getByTestId("canvas-labels")).toHaveTextContent("true");
    expect(screen.getByTestId("canvas-highlight")).toHaveTextContent("true");
    await user.click(sw("Afficher les libellés"));
    expect(screen.getByTestId("canvas-labels")).toHaveTextContent("false");
    expect(sw("Afficher les libellés")).toHaveAttribute("aria-checked", "false");
    await user.click(sw("Surligner les chemins"));
    expect(screen.getByTestId("canvas-highlight")).toHaveTextContent("false");
    // The funnel toolbar button toggles the same flag.
    await user.click(screen.getByRole("button", { name: "Basculer le surlignage des chemins" }));
    expect(screen.getByTestId("canvas-highlight")).toHaveTextContent("true");
    // The two decorative toggles flip their own state.
    await user.click(sw("Vue groupée"));
    expect(sw("Vue groupée")).toHaveAttribute("aria-checked", "true");
    await user.click(sw("Afficher les contraintes"));
    expect(sw("Afficher les contraintes")).toHaveAttribute("aria-checked", "true");
  });

  it("drives the canvas from the toolbar: layout menu, zoom, fit, fullscreen", async () => {
    const { user } = await mount();
    expect(screen.getByTestId("canvas-layout")).toHaveTextContent("LR");
    await user.click(screen.getByRole("button", { name: /Disposition/ }));
    const menu = document.querySelector(".gv-layout-menu") as HTMLElement;
    expect(within(menu).getByRole("button", { name: "Gauche → Droite" })).toHaveClass("active");
    await user.click(within(menu).getByRole("button", { name: "Haut → Bas" }));
    expect(screen.getByTestId("canvas-layout")).toHaveTextContent("TB");
    expect(screen.queryByRole("button", { name: "Haut → Bas" })).toBeNull();
    // Reopen and leave with the mouse: the menu closes without a change.
    await user.click(screen.getByRole("button", { name: /Disposition/ }));
    fireEvent.mouseLeave(screen.getByRole("button", { name: "Droite → Gauche" }).closest("ul")!);
    expect(screen.queryByRole("button", { name: "Droite → Gauche" })).toBeNull();
    expect(screen.getByTestId("canvas-layout")).toHaveTextContent("TB");

    await user.click(screen.getByRole("button", { name: "Zoom avant" }));
    await user.click(screen.getByRole("button", { name: "Zoom arrière" }));
    await user.click(screen.getByRole("button", { name: "Ajuster la vue" }));
    expect(handle.zoomIn).toHaveBeenCalledTimes(1);
    expect(handle.zoomOut).toHaveBeenCalledTimes(1);
    expect(handle.fit).toHaveBeenCalledTimes(1);

    // Fullscreen: enter when nothing is fullscreen, exit otherwise.
    const wrap = document.getElementById("gv-canvas-wrap")!;
    const request = vi.fn();
    (wrap as unknown as { requestFullscreen: () => void }).requestFullscreen = request;
    const exit = vi.fn();
    document.exitFullscreen = exit;
    await user.click(screen.getByRole("button", { name: "Plein écran" }));
    expect(request).toHaveBeenCalledTimes(1);
    expect(exit).not.toHaveBeenCalled();
    Object.defineProperty(document, "fullscreenElement", { configurable: true, get: () => wrap });
    await user.click(screen.getByRole("button", { name: "Plein écran" }));
    expect(exit).toHaveBeenCalledTimes(1);
    Object.defineProperty(document, "fullscreenElement", { configurable: true, get: () => null });
  });

  it("selects a node on click, toggles it off on a second click, clears on pane click", async () => {
    const { user } = await mount();
    expect(screen.getByText(/Cliquez sur une fiche du graphe/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "node 1" }));
    expect(screen.getByTestId("canvas-selected")).toHaveTextContent("1");
    expect(screen.getByText("Alice", { selector: ".gv-inspector-name" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "node 1" }));
    expect(screen.getByTestId("canvas-selected")).toHaveTextContent("none");
    expect(screen.getByText(/Cliquez sur une fiche du graphe/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "node 2" }));
    expect(screen.getByTestId("canvas-selected")).toHaveTextContent("2");
    await user.click(screen.getByRole("button", { name: "pane" }));
    expect(screen.getByTestId("canvas-selected")).toHaveTextContent("none");
    // Double-click selects (never toggles) and centres the canvas on the node.
    await user.click(screen.getByRole("button", { name: "dbl 3" }));
    await user.click(screen.getByRole("button", { name: "dbl 3" }));
    expect(screen.getByTestId("canvas-selected")).toHaveTextContent("3");
    expect(handle.focusNode).toHaveBeenCalledTimes(2);
    expect(handle.focusNode).toHaveBeenLastCalledWith("3");
  });

  it("inspects a concept: description, connection counts and its own typed properties", async () => {
    const { user } = await mount();
    await user.click(screen.getByRole("button", { name: "node 1" }));
    expect(screen.getByRole("link", { name: "Ouvrir la fiche" })).toHaveAttribute("href", "/concepts/1");
    expect(screen.getByText("A person")).toBeInTheDocument();
    const overview = document.querySelector(".gv-overview")!;
    expect(overview).toHaveTextContent("Type de fichePerson");
    // Alice: outgoing worksAt, incoming knows.
    expect(overview).toHaveTextContent("Liens au total2");
    expect(overview).toHaveTextContent("Liens entrants1");
    expect(overview).toHaveTextContent("Liens sortants1");
    expect(screen.getByText("Propriétés (7)")).toBeInTheDocument();
    const props = Object.fromEntries(
      Array.from(document.querySelectorAll(".gv-props li")).map((li) => [
        li.querySelector(".gv-prop-name")!.textContent,
        li.querySelector(".gv-prop-type")!.textContent,
      ]),
    );
    expect(props).toEqual({
      age: "xsd:integer",
      score: "xsd:decimal",
      active: "xsd:boolean",
      born: "xsd:date",
      nick: "xsd:string",
      tags: "xsd:any",
      none: "xsd:string",
    });
  });

  it("falls back to the type schema, then to 'no properties', and uses the type description", async () => {
    const { user } = await mount();
    await user.click(screen.getByRole("button", { name: "node 2" }));
    expect(screen.getByText("A legal entity")).toBeInTheDocument();
    expect(screen.getByText("Propriétés (2)")).toBeInTheDocument();
    expect(screen.getByText("vat").nextElementSibling).toHaveTextContent("xsd:string");
    expect(screen.getByText("founded").nextElementSibling).toHaveTextContent("xsd:any");
    // Geneva: no properties anywhere, no description.
    await user.click(screen.getByRole("button", { name: "node 3" }));
    expect(screen.getByText("Propriétés (0)")).toBeInTheDocument();
    expect(screen.getByText("Aucune propriété déclarée.")).toBeInTheDocument();
    expect(document.querySelector(".gv-inspector-desc")).toBeNull();
    // Bob: empty own properties → schema of Person (string + non-string values).
    await user.click(screen.getByRole("button", { name: "node 4" }));
    expect(screen.getByText("Human being")).toBeInTheDocument();
    expect(screen.getByText("age").nextElementSibling).toHaveTextContent("xsd:integer");
    expect(screen.getByText("extra").nextElementSibling).toHaveTextContent("xsd:integer");
  });

  it("hides the inspector content when the selection is filtered out", async () => {
    const { user } = await mount();
    await user.click(screen.getByRole("button", { name: "node 3" }));
    expect(screen.getByRole("link", { name: "Ouvrir la fiche" })).toHaveAttribute("href", "/concepts/3");
    await user.selectOptions(screen.getAllByRole("combobox")[0], "Person");
    expect(screen.queryByRole("link", { name: "Ouvrir la fiche" })).toBeNull();
    expect(screen.getByText(/Cliquez sur une fiche du graphe/)).toBeInTheDocument();
  });

  it("focuses, expands and queries the selected node", async () => {
    const { user } = await mount();
    await user.click(screen.getByRole("button", { name: "node 1" }));
    await user.click(screen.getByRole("button", { name: /Centrer la fiche/ }));
    expect(handle.focusNode).toHaveBeenCalledWith("1");
    // Expand: restrict to the node's type and go one level deeper.
    await user.click(screen.getByRole("button", { name: /Étendre le voisinage/ }));
    expect(screen.getByText("4 niveaux")).toBeInTheDocument();
    expect(screen.getAllByRole("combobox")[0]).toHaveValue("Person");
    await waitFor(() =>
      expect(mocked.getSubgraph).toHaveBeenLastCalledWith(expect.objectContaining({ expansion_depth: 4, seed_concept_types: ["Person"] })),
    );
    // Depth is capped at 5.
    fireEvent.change(screen.getByRole("slider"), { target: { value: "5" } });
    await user.click(screen.getByRole("button", { name: /Étendre le voisinage/ }));
    expect(screen.getByText("5 niveaux")).toBeInTheDocument();
    // Run Query navigates with the concept name.
    await user.click(screen.getByRole("button", { name: /Lancer la requête/ }));
    expect(screen.getByTestId("location")).toHaveTextContent("/queries?q=Alice");
  });

  it("collapses and reopens the inspector; a node click reopens it too", async () => {
    const { user } = await mount();
    const row = document.querySelector(".gv-row-main")!;
    await user.click(screen.getByRole("button", { name: "Précédent" }));
    expect(row).toHaveClass("inspector-collapsed");
    await user.click(screen.getByRole("button", { name: "Suivant" }));
    expect(row).not.toHaveClass("inspector-collapsed");
    await user.click(screen.getByRole("button", { name: "Précédent" }));
    await user.click(screen.getByRole("button", { name: "node 1" }));
    expect(row).not.toHaveClass("inspector-collapsed");
  });

  it("lists rules and actions in their tabs, and comes back to the inspector on a node click", async () => {
    const { user } = await mount();
    await user.click(screen.getByRole("button", { name: /^Règles/ }));
    expect(screen.getByText("R1")).toBeInTheDocument();
    expect(screen.getByText("stricte")).toBeInTheDocument();
    expect(screen.getByText("Strict one")).toBeInTheDocument();
    expect(screen.getByText("SI").nextElementSibling).toHaveTextContent("x");
    expect(screen.getByText("ALORS").nextElementSibling).toHaveTextContent("y");
    expect(screen.getByText("R1").closest("li")!.querySelectorAll(".gv-rule-tags .badge")).toHaveLength(2);
    // R2 has none of the optional fields and is advisory.
    expect(screen.getByText("indicative")).toBeInTheDocument();
    expect(screen.getByText("R2").closest("li")!.querySelector(".gv-rule-row")).toBeNull();

    await user.click(screen.getByRole("button", { name: /^Actions/ }));
    expect(screen.queryByText("R1")).toBeNull();
    const a1 = within(screen.getByText("A1").closest("li") as HTMLElement);
    expect(a1.getByText("Hire")).toBeInTheDocument();
    expect(a1.getByText("SUJET").nextElementSibling).toHaveTextContent("Person → Company");
    expect(a1.getByText("PARAMÈTRES").nextElementSibling!.querySelectorAll(".badge")).toHaveLength(2);
    expect(a1.getByText("EFFET").nextElementSibling).toHaveTextContent("hired");
    // A2: subject only.
    const a2 = screen.getByText("A2").closest("li")!;
    expect(a2.querySelectorAll(".gv-rule-row")).toHaveLength(1);
    expect(a2.querySelector("strong")).toBeNull();

    await user.click(screen.getByRole("button", { name: "node 2" }));
    expect(screen.getByRole("link", { name: "Ouvrir la fiche" })).toHaveAttribute("href", "/concepts/2");
    expect(screen.queryByText("A1")).toBeNull();
    // The Inspector tab itself can be chosen explicitly.
    await user.click(screen.getByRole("button", { name: /^Règles/ }));
    expect(screen.queryByRole("link", { name: "Ouvrir la fiche" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Inspecteur" }));
    expect(screen.getByRole("link", { name: "Ouvrir la fiche" })).toHaveAttribute("href", "/concepts/2");
  });

  it("shows the empty rules / actions states when the ontology declares none", async () => {
    mocked.getOntology.mockResolvedValue({ concept_types: {}, relation_types: {} });
    const { user } = await mount();
    await user.click(screen.getByRole("button", { name: /^Règles/ }));
    expect(screen.getByText("Aucune règle déclarée dans cette ontologie.")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /^Actions/ }));
    expect(screen.getByText("Aucune action déclarée dans cette ontologie.")).toBeInTheDocument();
    // Without types, the canvas colour map is empty and no suggestion is built.
    expect(screen.getByTestId("canvas-colors")).toHaveTextContent("{}");
    expect(screen.getByText("Aucune suggestion disponible.")).toBeInTheDocument();
  });

  it("lists the four most recent saved views with relative times", async () => {
    await mount();
    const list = screen.getByText("Just now").closest("ul")!;
    const rows = Array.from(list.querySelectorAll("li")).map((li) => li.textContent);
    expect(rows).toHaveLength(4);
    expect(rows[0]).toMatch(/Just nowil y a [0-9]+ s/);
    expect(rows[1]).toMatch(/Minutesil y a 2 min/);
    expect(rows[2]).toMatch(/Hoursil y a 2 h/);
    expect(rows[3]).toMatch(/Daysil y a 2 j/);
    expect(screen.queryByText("Never (hidden)")).toBeNull();
  });

  it("lists the four most recent files as activity, worded by status", async () => {
    await mount();
    const items = Array.from(document.querySelectorAll(".gv-activity-item"));
    expect(items).toHaveLength(4);
    expect(items[0]).toHaveTextContent("Import terminé pour done.pdf");
    expect(items[0].querySelector(".gv-act-ok")).not.toBeNull();
    expect(items[1]).toHaveTextContent("Validation échouée sur bad.csv");
    expect(items[1].querySelector(".gv-act-err")).not.toBeNull();
    expect(items[2]).toHaveTextContent("Fiche mise à jour depuis seen.json");
    expect(items[2].querySelector(".gv-act-info")).not.toBeNull();
    expect(items[3]).toHaveTextContent("Ingestion en cours pour wip.docx");
    expect(items[3].querySelector(".gv-act-warn")).not.toBeNull();
    expect(screen.queryByText("old.txt")).toBeNull();
  });

  it("builds query suggestions from the ontology and navigates on click", async () => {
    const { user } = await mount();
    const suggestions = Array.from(document.querySelectorAll(".gv-sug-text")).map((b) => b.textContent);
    expect(suggestions).toEqual([
      "Trouver tous les Person qui works at un Company donné",
      "Trouver tous les Person qui knows un Person donné",
      "Trouver tous les Company qui based in un City donné",
      "Afficher tous les Person créés par un Company",
      "Lister tous les Person liés à Company",
    ]);
    await user.click(screen.getByRole("button", { name: "Lister tous les Person liés à Company" }));
    expect(screen.getByTestId("location")).toHaveTextContent("/queries?q=" + encodeURIComponent("Lister tous les Person liés à Company"));
  });

  it("'Tout voir' on the saved views goes to the queries page", async () => {
    const { user } = await mount();
    await user.click(screen.getAllByRole("button", { name: "Tout voir" })[0]);
    expect(screen.getByTestId("location")).toHaveTextContent("/queries");
  });

  it("'Tout voir' on the suggestions goes to the queries page", async () => {
    const { user } = await mount();
    await user.click(screen.getAllByRole("button", { name: "Tout voir" })[2]);
    expect(screen.getByTestId("location")).toHaveTextContent("/queries");
  });

  it("navigates to the files page from the activity 'Tout voir'", async () => {
    // Run Query only exists with a selection in the inspector, but the
    // activity 'View All' goes to /files: cover the second navigate target.
    const { user } = renderPage(<GraphView />, {
      route: "/graph",
      extraRoutes: <Route path="/files" element={<LocationProbe />} />,
    });
    await waitFor(() => expect(canvasNodes()).toBe("Alice,ACME,Geneva,Bob"));
    await user.click(screen.getAllByRole("button", { name: "Tout voir" })[1]);
    expect(screen.getByTestId("location")).toHaveTextContent("/files");
  });

  it("polls the subgraph every 30 s while mounted", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { unmount } = renderPage(<GraphView />);
    await waitFor(() => expect(canvasNodes()).toBe("Alice,ACME,Geneva,Bob"));
    const calls = mocked.getSubgraph.mock.calls.length;
    await vi.advanceTimersByTimeAsync(30_000);
    expect(mocked.getSubgraph).toHaveBeenCalledTimes(calls + 1);
    unmount();
    await vi.advanceTimersByTimeAsync(30_000);
    expect(mocked.getSubgraph).toHaveBeenCalledTimes(calls + 1);
    vi.useRealTimers();
  });
});

describe("GraphView — opened on a sheet", () => {
  it("seeds the graph with ?focus, selects the node and focuses it", async () => {
    const { user } = await mount("/graph?focus=2");
    expect(mocked.getSubgraph).toHaveBeenCalledWith(expect.objectContaining({ seed_concept_ids: [2], seed_query: undefined }));
    await waitFor(() => expect(handle.focusNode).toHaveBeenCalledWith("2"));
    expect(screen.getByRole("link", { name: "Ouvrir la fiche" })).toHaveAttribute("href", "/concepts/2");
    // A later reload (the depth slider) leaves the selection alone.
    handle.focusNode.mockClear();
    await user.click(screen.getByRole("button", { name: "node 1" }));
    expect(screen.getByRole("link", { name: "Ouvrir la fiche" })).toHaveAttribute("href", "/concepts/1");
  });

  it("ignores a focus that is not an id or not in the subgraph", async () => {
    await mount("/graph?focus=abc");
    expect(mocked.getSubgraph).toHaveBeenCalledWith(expect.objectContaining({ seed_concept_ids: undefined }));
    expect(handle.focusNode).not.toHaveBeenCalled();
  });

  it("ignores a focus that is not in the subgraph", async () => {
    await mount("/graph?focus=999");
    expect(mocked.getSubgraph).toHaveBeenCalledWith(expect.objectContaining({ seed_concept_ids: [999] }));
    expect(handle.focusNode).not.toHaveBeenCalled();
    expect(screen.queryByRole("link", { name: "Ouvrir la fiche" })).toBeNull();
  });
});

describe("GraphView — a model without sheets", () => {
  it("says the model is in place and the graph waits for files", async () => {
    mocked.getSubgraph.mockResolvedValue({ subgraph: { concepts: [], relations: [] } });
    renderPage(<GraphView />, { route: "/graph", extraRoutes: probeRoutes });
    expect(await screen.findByTestId("model-only")).toHaveTextContent(/Le modèle est en place \(\d+ type\(s\) de fiche\)/);
  });
});
