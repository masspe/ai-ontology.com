// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Rendering tests of the Concepts page: stat tiles, the library list (server
// paging, debounced search, type / status filters, sort), row selection and
// bulk delete with the live refresh of the list, the create / edit modals
// (including the AI draft), single delete through the confirm dialog, the
// details and relations panels, the hierarchy tree, and every error banner.
//
// The API mock keeps a small in-memory store (`db`, `rels`) that the mutating
// calls edit, so a deletion really changes what the next `listConcepts` page
// returns — the same way the server behaves.

import { beforeEach, describe, expect, it, vi } from "vitest";
import Concepts from "./Concepts";
import { fireEvent, renderPage, screen, waitFor, within } from "../test/render";
import type { Concept, Ontology, Relation, Stats, StatsHistory } from "../api";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return {
    ...actual,
    createConcept: vi.fn(),
    createRelation: vi.fn(),
    deleteConcept: vi.fn(),
    deleteConcepts: vi.fn(),
    deleteRelation: vi.fn(),
    generateOntology: vi.fn(),
    getOntology: vi.fn(),
    getStats: vi.fn(),
    getStatsHistory: vi.fn(),
    getSubgraph: vi.fn(),
    listConcepts: vi.fn(),
    listRelations: vi.fn(),
    updateConcept: vi.fn(),
  };
});

import * as api from "../api";

const mocked = vi.mocked(api);

// The page is heavy (four sparklines, a 7-row table and two side panels
// re-render on every keystroke), so the scenarios that type through
// `userEvent` take 2–4 s in jsdom. Give them headroom on a loaded CI box:
// nothing here sleeps, every wait is a `findBy*` / `waitFor`.
vi.setConfig({ testTimeout: 20_000 });

// ---- Fixtures ---------------------------------------------------------------

const ontology: Ontology = {
  concept_types: {
    Entity: { name: "Entity" },
    Person: { name: "Person", parent: "Entity" },
    Company: { name: "Company", parent: "Entity" },
    Contract: { name: "Contract" },
  },
  relation_types: {
    works_for: { name: "works_for", domain: "Person", range: "Company", cardinality: "many-to-one", symmetric: false },
    knows: { name: "knows", domain: "Person", range: "Person", cardinality: "many-to-many", symmetric: true },
  },
};

// Noon UTC so the en-US date is the same in every time zone.
const T_ALICE = 1_709_985_600; // 2024-03-09T12:00:00Z
const LONG_DEF = "Lead architect. " + "x".repeat(200); // 216 chars → clipped at 160

function seed(): Concept[] {
  return [
    {
      id: 1,
      concept_type: "Person",
      name: "Alice",
      description: LONG_DEF,
      properties: {
        status: "reviewed",
        updated_at: T_ALICE,
        linked: 3,
        synonyms: ["Ally", "A."],
        owner: "Bob",
        score: 1234,
        tags: { vip: true },
      },
    },
    { id: 2, concept_type: "Company", name: "ACME", properties: { status: "draft", created_at: "2024-01-02T12:00:00Z" } },
    {
      id: 3,
      concept_type: "Contract",
      name: "Master SLA",
      description: "Service agreement",
      properties: { status: "archived", synonyms: "sla, agreement", parties: ["ACME", "Globex"] },
    },
    { id: 4, concept_type: "Person", name: "Bob" },
  ];
}

/** `n` people named "Concept 01" … whose update time decreases with the id. */
function many(n: number): Concept[] {
  return Array.from({ length: n }, (_, i) => ({
    id: i + 1,
    concept_type: "Person",
    name: `Concept ${String(i + 1).padStart(2, "0")}`,
    properties: { updated_at: 2_000_000_000 - i },
  }));
}

const stats: Stats = {
  concepts: 1234,
  relations: 56,
  rules: 0,
  actions: 0,
  concept_types: 4,
  relation_types: 2,
  rule_types: 0,
  action_types: 0,
  deltas: { concepts_pct: 0.1, relations_pct: -0.2, concept_types_pct: 0, relation_types_pct: 0 },
};

const history: StatsHistory = {
  samples: [
    { ts: 1, concepts: 10, relations: 5, concept_types: 2, relation_types: 1 },
    { ts: 2, concepts: 20, relations: 10, concept_types: 3, relation_types: 1 },
    { ts: 3, concepts: 0, relations: 0, concept_types: 4, relation_types: 2 },
  ],
};

let db: Concept[];
let rels: Relation[];

beforeEach(() => {
  db = seed();
  rels = [
    { id: 10, relation_type: "works_for", source: 1, target: 2 },
    { id: 11, relation_type: "knows", source: 4, target: 1 },
    { id: 12, relation_type: "knows", source: 1, target: 99 },
  ];

  mocked.getStats.mockResolvedValue(stats);
  mocked.getStatsHistory.mockResolvedValue(history);
  mocked.getOntology.mockResolvedValue(ontology);
  // 4 concepts, 2 of them linked → 50 % coverage.
  mocked.getSubgraph.mockResolvedValue({
    subgraph: { concepts: seed(), relations: [{ id: 10, relation_type: "works_for", source: 1, target: 2 }] },
  });

  // One implementation serves the three callers (page, recent activity,
  // per-type counts): filter by type and name, then slice.
  mocked.listConcepts.mockImplementation(async (params = {}) => {
    let rows = db;
    if (params.type) rows = rows.filter((c) => c.concept_type === params.type);
    if (params.q) {
      const q = params.q.toLowerCase();
      rows = rows.filter((c) => c.name.toLowerCase().includes(q));
    }
    const offset = params.offset ?? 0;
    const limit = params.limit ?? rows.length;
    return { total: rows.length, concepts: rows.slice(offset, offset + limit) };
  });
  mocked.listRelations.mockImplementation(async (params = {}) => {
    const out = rels.filter(
      (r) =>
        (params.source == null || r.source === params.source) &&
        (params.target == null || r.target === params.target),
    );
    return { total: out.length, relations: out };
  });

  mocked.createConcept.mockImplementation(async (c) => {
    const id = Math.max(0, ...db.map((x) => x.id)) + 1;
    db = [...db, { id, ...c }];
    return { id };
  });
  mocked.updateConcept.mockImplementation(async (id, patch) => {
    const updated = { ...db.find((c) => c.id === id)!, ...patch };
    db = db.map((c) => (c.id === id ? updated : c));
    return updated;
  });
  mocked.deleteConcept.mockImplementation(async (id) => {
    db = db.filter((c) => c.id !== id);
  });
  mocked.deleteConcepts.mockImplementation(async (ids) => {
    db = db.filter((c) => !ids.includes(c.id));
    return { deleted: ids.length, relations: ids.length > 1 ? 3 : 0, missing: [] };
  });
  mocked.createRelation.mockImplementation(async (r) => {
    const id = 100 + rels.length;
    rels = [...rels, { id, ...r }];
    return { id };
  });
  mocked.deleteRelation.mockImplementation(async (id) => {
    rels = rels.filter((r) => r.id !== id);
  });
  mocked.generateOntology.mockResolvedValue({
    ontology: { concept_types: { Person: { name: "Person", description: "A human being" } }, relation_types: {} },
    model: "test-model",
  });
});

// ---- Helpers ----------------------------------------------------------------

/** The library table (the details panel has its own, smaller table). */
const libraryTable = () => document.querySelector("table.table") as HTMLTableElement;
const libraryRows = () => within(libraryTable().tBodies[0]).getAllByRole("row");
const rowNames = () =>
  libraryRows().map((r) => r.querySelector(".concept-name-cell > span:last-child")!.textContent);
const details = () => document.querySelector(".concept-details") as HTMLElement;
const relPanel = () => document.querySelector(".rel-panel") as HTMLElement;

/** Mount and wait for the first page to be on screen. */
async function mountLoaded(firstName = "Alice") {
  const page = renderPage(<Concepts />);
  await within(libraryTable()).findByText(firstName);
  return page;
}

/** Calls of `listConcepts` that fetched a library page (not recent/counts). */
const pageCalls = () => mocked.listConcepts.mock.calls.map(([p]) => p!).filter((p) => p.limit === 7);

/** Click the confirm dialog's accept (or cancel) button. */
async function answerConfirm(user: ReturnType<typeof renderPage>["user"], label: string) {
  const dialog = await screen.findByRole("dialog");
  await user.click(within(dialog).getByRole("button", { name: label }));
}

// ---- Stat tiles ---------------------------------------------------------------

describe("Concepts page — stat tiles", () => {
  it("renders the counts, the month-over-month deltas and the coverage score", async () => {
    renderPage(<Concepts />);
    expect(await screen.findByText("1,234", { selector: ".stat-value" })).toBeInTheDocument();
    expect(screen.getByText("↑ 10%")).toBeInTheDocument(); // concepts +10 %
    expect(screen.getByText("↓ 20%")).toBeInTheDocument(); // relations −20 %
    expect(screen.getByText("• 0%")).toBeInTheDocument(); // groups flat
    expect(await screen.findByText("50%")).toBeInTheDocument();
    expect(mocked.getSubgraph).toHaveBeenCalledWith({ limit: 500, expansion_depth: 1 });
    // One trend line per tile (the sparkline component draws a polyline).
    expect(document.querySelectorAll(".stat-spark polyline")).toHaveLength(4);
  });

  it("shows a dash for the coverage when the subgraph is empty", async () => {
    mocked.getSubgraph.mockResolvedValue({ subgraph: { concepts: [], relations: [] } });
    renderPage(<Concepts />);
    await screen.findByText("1,234", { selector: ".stat-value" });
    expect(screen.getByText("Coverage Score").nextElementSibling).toHaveTextContent("—");
  });

  it("surfaces a failing stats call in the error banner", async () => {
    mocked.getStats.mockRejectedValue(new Error("stats down"));
    renderPage(<Concepts />);
    expect(await screen.findByText("stats down")).toBeInTheDocument();
    // A non-Error rejection is stringified.
    mocked.getStats.mockRejectedValue("boom");
    renderPage(<Concepts />);
    expect(await screen.findByText("boom")).toBeInTheDocument();
  });
});

// ---- Library list -------------------------------------------------------------

describe("Concepts page — library list", () => {
  it("lists the first page sorted by last update, with domain, clipped definition, status, links and date", async () => {
    await mountLoaded();
    expect(mocked.listConcepts).toHaveBeenCalledWith({ type: undefined, q: undefined, limit: 7, offset: 0 });
    expect(rowNames()).toEqual(["Alice", "ACME", "Master SLA", "Bob"]);

    const [alice, acme, sla, bob] = libraryRows();
    expect(alice).toHaveClass("row-active"); // first row is selected by default
    expect(within(alice).getByText("Entity")).toBeInTheDocument(); // Person → Entity root
    const def = within(alice).getByText(/^Lead architect\. x+…$/);
    expect(def.textContent).toHaveLength(161);
    expect(def).toHaveAttribute("title", LONG_DEF); // 216 chars < 300: full title
    expect(within(alice).getByText("Reviewed")).toHaveClass("badge-accent");
    expect(within(alice).getByText("3")).toBeInTheDocument();
    expect(within(alice).getByText("Mar 9, 2024")).toBeInTheDocument();

    expect(within(acme).getByText("—")).toBeInTheDocument(); // no definition
    expect(within(acme).getByText("Draft")).toHaveClass("badge-warn");
    expect(within(acme).getByText("Jan 2, 2024")).toBeInTheDocument(); // created_at string
    expect(within(sla).getByText("Contract")).toBeInTheDocument(); // type without parent
    expect(within(sla).getByText("Archived")).toHaveClass("badge-danger");
    expect(within(bob).getByText("Active")).toHaveClass("badge-success");
    expect(within(bob).getAllByText("—")).toHaveLength(2); // no definition, no date

    expect(screen.getByText("Showing 1–4 of 4 concepts")).toBeInTheDocument();
  });

  it("shows the empty state, the details placeholder and disables select-all when nothing matches", async () => {
    db = [];
    renderPage(<Concepts />);
    expect(await screen.findByText("No concepts match the current filters.")).toBeInTheDocument();
    expect(screen.getByText("Select a concept from the library.")).toBeInTheDocument();
    expect(screen.getByText("Showing 0–0 of 0 concepts")).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "Select all concepts on this page" })).toBeDisabled();
    expect(await screen.findByText("No recent activity.")).toBeInTheDocument();
  });

  it("debounces the search and queries the server with the trimmed text", async () => {
    await mountLoaded();
    fireEvent.change(screen.getByPlaceholderText("Search concepts…"), { target: { value: "  ali " } });
    // Nothing is fetched synchronously: the query waits for the debounce.
    expect(pageCalls().some((p) => p.q)).toBe(false);
    await waitFor(() =>
      expect(mocked.listConcepts).toHaveBeenCalledWith({ type: undefined, q: "ali", limit: 7, offset: 0 }),
    );
    await waitFor(() => expect(rowNames()).toEqual(["Alice"]));
    expect(screen.getByText("Showing 1–1 of 1 concepts")).toBeInTheDocument();
  });

  it("filters by type on the server and by status on the client, and sorts by name or type", async () => {
    const { user } = await mountLoaded();
    await user.selectOptions(screen.getByDisplayValue("All Domains"), "Company");
    await waitFor(() =>
      expect(mocked.listConcepts).toHaveBeenCalledWith({ type: "Company", q: undefined, limit: 7, offset: 0 }),
    );
    await waitFor(() => expect(rowNames()).toEqual(["ACME"]));

    await user.selectOptions(screen.getByDisplayValue("Company"), "");
    await user.selectOptions(screen.getByDisplayValue("All Statuses"), "archived");
    await waitFor(() => expect(rowNames()).toEqual(["Master SLA"]));
    // The server never sees the status: the slice is filtered on the client.
    expect(pageCalls().every((p) => !("status" in p))).toBe(true);

    await user.selectOptions(screen.getByDisplayValue("Archived"), "");
    await user.selectOptions(screen.getByDisplayValue("Sort: Last Updated"), "name");
    await waitFor(() => expect(rowNames()).toEqual(["ACME", "Alice", "Bob", "Master SLA"]));
    await user.selectOptions(screen.getByDisplayValue("Sort: Name"), "type");
    await waitFor(() => expect(rowNames()).toEqual(["ACME", "Master SLA", "Alice", "Bob"]));
  });

  it("pages through a long list with previous / next / numbered / last buttons", async () => {
    db = many(40);
    const { user } = await mountLoaded("Concept 01");
    expect(screen.getByText("Showing 1–7 of 40 concepts")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "‹" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "5" })).toBeInTheDocument();
    expect(screen.getByText("…")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "6" })).toBeInTheDocument(); // last page shortcut
    expect(screen.queryByRole("button", { name: "7" })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "›" }));
    await waitFor(() => expect(mocked.listConcepts).toHaveBeenCalledWith(expect.objectContaining({ offset: 7 })));
    expect(await screen.findByText("Showing 8–14 of 40 concepts")).toBeInTheDocument();
    expect(rowNames()[0]).toBe("Concept 08");
    expect(screen.getByRole("button", { name: "2" })).toHaveClass("pager-active");

    await user.click(screen.getByRole("button", { name: "6" }));
    await waitFor(() => expect(mocked.listConcepts).toHaveBeenCalledWith(expect.objectContaining({ offset: 35 })));
    expect(await screen.findByText("Showing 36–40 of 40 concepts")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "›" })).toBeDisabled();

    await user.click(screen.getByRole("button", { name: "‹" }));
    await waitFor(() => expect(mocked.listConcepts).toHaveBeenCalledWith(expect.objectContaining({ offset: 28 })));
    expect(await screen.findByText("Showing 29–35 of 40 concepts")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "1" }));
    expect(await screen.findByText("Showing 1–7 of 40 concepts")).toBeInTheDocument();
  });

  it("clamps the page when the total shrinks below the current offset", async () => {
    db = many(40);
    const { user } = await mountLoaded("Concept 01");
    await user.click(screen.getByRole("button", { name: "6" }));
    await screen.findByText("Showing 36–40 of 40 concepts");
    // Someone else emptied most of the store meanwhile: the delete leaves 7.
    mocked.deleteConcept.mockImplementation(async (id) => {
      db = db.filter((c) => c.id !== id).slice(0, 7);
    });
    await user.click(screen.getByRole("button", { name: "Delete concept Concept 36" }));
    await answerConfirm(user, "Delete");
    expect(await screen.findByText("Showing 1–7 of 7 concepts")).toBeInTheDocument();
    expect(rowNames()[0]).toBe("Concept 01");
  });

  it("surfaces a failing page fetch in the error banner", async () => {
    mocked.listConcepts.mockImplementation(async (params = {}) => {
      if (params.limit === 7) throw new Error("list down");
      return { total: 0, concepts: [] };
    });
    renderPage(<Concepts />);
    expect(await screen.findByText("list down")).toBeInTheDocument();
  });
});

// ---- Selection & bulk delete ----------------------------------------------------

describe("Concepts page — selection and bulk delete", () => {
  it("ticks rows, selects all on the page and clears the selection without changing the selected row", async () => {
    const { user } = await mountLoaded();
    expect(screen.queryByRole("toolbar")).not.toBeInTheDocument();

    await user.click(screen.getByRole("checkbox", { name: "Select concept Bob" }));
    const toolbar = screen.getByRole("toolbar", { name: "Bulk actions" });
    expect(toolbar).toHaveTextContent("1 selected");
    expect(screen.getByText("Delete 1 selected")).toBeInTheDocument(); // quick action mirrors it
    expect(libraryRows()[3]).toHaveClass("row-checked");
    // Ticking does not select the row for the details panel.
    expect(within(details()).getByRole("heading", { level: 3 })).toHaveTextContent("Alice");

    const all = screen.getByRole("checkbox", { name: "Select all concepts on this page" });
    await user.click(all);
    expect(toolbar).toHaveTextContent("4 selected");
    expect(all).toBeChecked();
    await user.click(all);
    expect(screen.queryByRole("toolbar")).not.toBeInTheDocument();

    await user.click(screen.getByRole("checkbox", { name: "Select concept Alice" }));
    await user.click(screen.getByRole("checkbox", { name: "Select concept Alice" }));
    expect(screen.queryByRole("toolbar")).not.toBeInTheDocument();

    await user.click(screen.getByRole("checkbox", { name: "Select concept Alice" }));
    await user.click(screen.getByRole("button", { name: "Clear selection" }));
    expect(screen.queryByRole("toolbar")).not.toBeInTheDocument();
    expect(screen.getByText("Select concepts, then delete")).toBeInTheDocument();
  });

  it("drops ticked rows that leave the page when a filter changes", async () => {
    const { user } = await mountLoaded();
    await user.click(screen.getByRole("checkbox", { name: "Select concept Alice" }));
    expect(screen.getByRole("toolbar")).toBeInTheDocument();
    await user.selectOptions(screen.getByDisplayValue("All Domains"), "Company");
    await waitFor(() => expect(rowNames()).toEqual(["ACME"]));
    expect(screen.queryByRole("toolbar")).not.toBeInTheDocument();
  });

  it("bulk-deletes the ticked concepts only after confirmation, then refreshes the list", async () => {
    const { user } = await mountLoaded();
    await user.click(screen.getByRole("checkbox", { name: "Select concept Alice" }));
    await user.click(screen.getByRole("checkbox", { name: "Select concept ACME" }));
    await user.click(screen.getByRole("button", { name: "Delete selected" }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Delete selected concepts")).toBeInTheDocument();
    expect(
      within(dialog).getByText("Delete 2 concepts and every relation attached to them? This cannot be undone."),
    ).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(mocked.deleteConcepts).not.toHaveBeenCalled();
    expect(rowNames()).toHaveLength(4);

    await user.click(screen.getByRole("button", { name: "Delete selected" }));
    await answerConfirm(user, "Delete 2");
    await waitFor(() => expect(mocked.deleteConcepts).toHaveBeenCalledWith([1, 2]));
    expect(await screen.findByText("Deleted 2 concepts and 3 relations.")).toBeInTheDocument();
    await waitFor(() => expect(rowNames()).toEqual(["Master SLA", "Bob"]));
    expect(screen.getByText("Showing 1–2 of 2 concepts")).toBeInTheDocument();
    expect(screen.queryByRole("toolbar")).not.toBeInTheDocument();
    // The deleted selection is replaced by the first remaining row.
    await waitFor(() => expect(within(details()).getByRole("heading", { level: 3 })).toHaveTextContent("Master SLA"));
    // Stats and recent activity are refreshed.
    await waitFor(() => expect(mocked.getStats).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.queryByText("“Alice”")).not.toBeInTheDocument());
  });

  it("words a single deletion without relations in the singular", async () => {
    const { user } = await mountLoaded();
    await user.click(screen.getByRole("checkbox", { name: "Select concept Bob" }));
    await user.click(screen.getByRole("button", { name: "Delete selected" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/^Delete 1 concept and every relation/)).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Delete 1" }));
    expect(await screen.findByText("Deleted 1 concept.")).toBeInTheDocument();
    expect(mocked.deleteConcepts).toHaveBeenCalledWith([4]);
  });

  it("shows the API error when the bulk delete fails and keeps the rows", async () => {
    mocked.deleteConcepts.mockRejectedValue(new Error("bulk failed"));
    const { user } = await mountLoaded();
    await user.click(screen.getByRole("checkbox", { name: "Select concept Bob" }));
    await user.click(screen.getByRole("button", { name: "Delete selected" }));
    await answerConfirm(user, "Delete 1");
    expect(await screen.findByText("bulk failed")).toBeInTheDocument();
    expect(rowNames()).toHaveLength(4);
  });

  it("quick action: hints when nothing is ticked, otherwise opens the bulk confirmation", async () => {
    const { user } = await mountLoaded();
    const quick = screen.getByRole("button", { name: /Bulk Delete/ });
    await user.click(quick);
    expect(
      screen.getByText("Tick the boxes in the library to select concepts, then delete them together."),
    ).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    await user.click(screen.getByRole("checkbox", { name: "Select concept Bob" }));
    await user.click(quick);
    await answerConfirm(user, "Delete 1");
    await waitFor(() => expect(mocked.deleteConcepts).toHaveBeenCalledWith([4]));

    await user.click(screen.getByRole("button", { name: /Validate Definitions/ }));
    expect(screen.getByText("Definition validation coming soon.")).toBeInTheDocument();
  });
});

// ---- Create ----------------------------------------------------------------------

describe("Concepts page — create concept", () => {
  it("refuses to open the form when the ontology has no concept types", async () => {
    mocked.getOntology.mockResolvedValue({ concept_types: {}, relation_types: {} });
    const { user } = await mountLoaded();
    expect(await screen.findByText("No concept types defined yet.")).toBeInTheDocument();
    expect(screen.getByText("Loading domains…")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Create Concept" }));
    expect(
      screen.getByText("No concept types are defined yet. Create one in Ontology Builder first."),
    ).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("creates a concept from the form, closes the modal and refreshes the list", async () => {
    const { user } = await mountLoaded();
    await screen.findByText("Coverage Score"); // ontology loaded → types known
    await waitFor(() => expect(screen.getByRole("option", { name: "Person" })).toBeInTheDocument());
    await user.click(screen.getByRole("button", { name: "Create Concept" }));
    const dialog = await screen.findByRole("dialog", { name: "Create Concept" });
    const create = within(dialog).getByRole("button", { name: "Create" });
    expect(create).toBeDisabled();
    expect(within(dialog).getByLabelText("Concept type")).toHaveValue("Company"); // first sorted type

    // The submit guard also holds when the form is forced through.
    fireEvent.submit(dialog.querySelector("form.modal-form")!);
    expect(mocked.createConcept).not.toHaveBeenCalled();

    await user.type(within(dialog).getByLabelText("Concept name"), "  Globex ");
    await user.selectOptions(within(dialog).getByLabelText("Concept type"), "Person");
    await user.type(within(dialog).getByLabelText("Definition (optional)"), "A rival ");
    expect(create).toBeEnabled();
    await user.click(create);

    await waitFor(() =>
      expect(mocked.createConcept).toHaveBeenCalledWith({ name: "Globex", concept_type: "Person", description: "A rival" }),
    );
    expect(await screen.findByText('Concept "Globex" created.')).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await waitFor(() => expect(rowNames()).toContain("Globex"));
    expect(await screen.findByText("“Globex”")).toBeInTheDocument(); // recent activity
    await waitFor(() => expect(mocked.getStats).toHaveBeenCalledTimes(2));
  });

  it("keeps the form open and shows the error when the creation fails", async () => {
    mocked.createConcept.mockRejectedValue(new Error("create failed"));
    const { user } = await mountLoaded();
    await waitFor(() => expect(screen.getByRole("option", { name: "Person" })).toBeInTheDocument());
    await user.click(screen.getByRole("button", { name: "Create Concept" }));
    const dialog = await screen.findByRole("dialog", { name: "Create Concept" });
    await user.type(within(dialog).getByLabelText("Concept name"), "Globex");
    await user.click(within(dialog).getByRole("button", { name: "Create" }));
    expect(await screen.findByText("create failed")).toBeInTheDocument();
    expect(screen.getByRole("dialog", { name: "Create Concept" })).toBeInTheDocument();
  });

  it("closes through Cancel, the × button and the backdrop, but not from inside", async () => {
    const { user } = await mountLoaded();
    await waitFor(() => expect(screen.getByRole("option", { name: "Person" })).toBeInTheDocument());
    const open = () => user.click(screen.getByRole("button", { name: "Create Concept" }));

    await open();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    await open();
    await user.click(screen.getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    await open();
    await user.click(screen.getByRole("dialog", { name: "Create Concept" }));
    expect(screen.getByRole("dialog", { name: "Create Concept" })).toBeInTheDocument();
    await user.click(document.querySelector(".modal-backdrop")!);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("drafts the fields from an AI-generated ontology without overwriting a typed name", async () => {
    let resolve!: (v: { ontology: Ontology; model: string }) => void;
    mocked.generateOntology.mockReturnValue(new Promise((r) => (resolve = r)));
    const { user } = await mountLoaded();
    await waitFor(() => expect(screen.getByRole("option", { name: "Person" })).toBeInTheDocument());
    await user.click(screen.getByRole("button", { name: "Create Concept" }));
    const dialog = await screen.findByRole("dialog", { name: "Create Concept" });

    const generate = within(dialog).getByRole("button", { name: "Generate" });
    expect(generate).toBeDisabled();
    await user.type(within(dialog).getByPlaceholderText(/French software vendor/), "  a person ");
    expect(generate).toBeEnabled();
    await user.click(generate);
    expect(await within(dialog).findByText("Generating…")).toBeInTheDocument();
    expect(mocked.generateOntology).toHaveBeenCalledWith("a person");
    resolve({
      ontology: { concept_types: { Person: { name: "Person", description: "A human being" } }, relation_types: {} },
      model: "m",
    });
    await waitFor(() => expect(within(dialog).getByLabelText("Concept name")).toHaveValue("Person"));
    expect(within(dialog).getByLabelText("Concept type")).toHaveValue("Person");
    expect(within(dialog).getByLabelText("Definition (optional)")).toHaveValue("A human being");

    // A second draft of an unknown type keeps the name, type and definition already set.
    mocked.generateOntology.mockResolvedValue({
      ontology: { concept_types: { Alien: { name: "Alien", description: "Other" } }, relation_types: {} },
      model: "m",
    });
    await user.click(within(dialog).getByRole("button", { name: "Generate" }));
    await waitFor(() => expect(mocked.generateOntology).toHaveBeenCalledTimes(2));
    expect(within(dialog).getByLabelText("Concept name")).toHaveValue("Person");
    expect(within(dialog).getByLabelText("Concept type")).toHaveValue("Person");
    expect(within(dialog).getByLabelText("Definition (optional)")).toHaveValue("A human being");
  });

  it("reports an empty AI answer and a failed AI call next to the button", async () => {
    mocked.generateOntology.mockResolvedValueOnce({ ontology: { concept_types: {}, relation_types: {} }, model: "m" });
    const { user } = await mountLoaded();
    await waitFor(() => expect(screen.getByRole("option", { name: "Person" })).toBeInTheDocument());
    await user.click(screen.getByRole("button", { name: "Create Concept" }));
    const dialog = await screen.findByRole("dialog", { name: "Create Concept" });
    expect(within(dialog).getByText(/Drafts will populate the fields below/)).toBeInTheDocument();
    await user.type(within(dialog).getByPlaceholderText(/French software vendor/), "nothing");
    await user.click(within(dialog).getByRole("button", { name: "Generate" }));
    expect(
      await within(dialog).findByText("AI did not return any concepts. Try a more specific prompt."),
    ).toBeInTheDocument();
    expect(within(dialog).queryByText(/Drafts will populate/)).not.toBeInTheDocument();

    mocked.generateOntology.mockRejectedValueOnce(new Error("llm down"));
    await user.click(within(dialog).getByRole("button", { name: "Generate" }));
    expect(await within(dialog).findByText("llm down")).toBeInTheDocument();
  });
});

// ---- Edit -------------------------------------------------------------------------

describe("Concepts page — edit concept", () => {
  it("edits from the row's pencil button and saves the trimmed name and definition", async () => {
    const { user } = await mountLoaded();
    await user.click(screen.getAllByRole("button", { name: "Edit" })[0]);
    const dialog = await screen.findByRole("dialog", { name: "Edit Concept" });
    const name = within(dialog).getByLabelText("Name");
    expect(name).toHaveValue("Alice");
    expect(within(dialog).getByLabelText("Definition")).toHaveValue(LONG_DEF);
    await user.clear(name);
    await user.type(name, " Alicia ");
    await user.clear(within(dialog).getByLabelText("Definition"));
    await user.type(within(dialog).getByLabelText("Definition"), "Architect ");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));

    await waitFor(() => expect(mocked.updateConcept).toHaveBeenCalledWith(1, { name: "Alicia", description: "Architect" }));
    expect(await screen.findByText('Concept "Alicia" updated.')).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(rowNames()[0]).toBe("Alicia");
    expect(within(details()).getByRole("heading", { level: 3 })).toHaveTextContent("Alicia");
    expect(within(details()).getByText("Architect")).toBeInTheDocument();
  });

  it("edits from the details panel; Cancel discards, an empty name cannot be saved", async () => {
    const { user } = await mountLoaded();
    await user.click(within(details()).getByRole("button", { name: "Edit Concept" }));
    const dialog = await screen.findByRole("dialog", { name: "Edit Concept" });
    const save = within(dialog).getByRole("button", { name: "Save" });
    await user.clear(within(dialog).getByLabelText("Name"));
    expect(save).toBeDisabled();
    fireEvent.submit(dialog.querySelector("form")!);
    expect(mocked.updateConcept).not.toHaveBeenCalled();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    // The × button closes too.
    await user.click(within(details()).getByRole("button", { name: "Edit Concept" }));
    await user.click(screen.getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(rowNames()[0]).toBe("Alice");
  });

  it("shows the error when the update fails", async () => {
    mocked.updateConcept.mockRejectedValue(new Error("update failed"));
    const { user } = await mountLoaded();
    await user.click(screen.getAllByRole("button", { name: "Edit" })[1]);
    const dialog = await screen.findByRole("dialog", { name: "Edit Concept" });
    await user.type(within(dialog).getByLabelText("Name"), " Corp");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(await screen.findByText("update failed")).toBeInTheDocument();
    expect(mocked.updateConcept).toHaveBeenCalledWith(2, { name: "ACME Corp", description: "" });
  });
});

// ---- Delete one -------------------------------------------------------------------

describe("Concepts page — delete a concept", () => {
  it("deletes only after confirmation, removes the row and lets the next one fill the page", async () => {
    db = many(9);
    const { user } = await mountLoaded("Concept 01");
    expect(rowNames()).toHaveLength(7);
    await user.click(screen.getByRole("button", { name: "Delete concept Concept 01" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText('Delete concept "Concept 01"? This cannot be undone.')).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(mocked.deleteConcept).not.toHaveBeenCalled();
    expect(rowNames()).toHaveLength(7);

    await user.click(screen.getByRole("button", { name: "Delete concept Concept 01" }));
    await answerConfirm(user, "Delete");
    await waitFor(() => expect(mocked.deleteConcept).toHaveBeenCalledWith(1));
    expect(await screen.findByText('Deleted "Concept 01".')).toBeInTheDocument();
    // No manual refresh: the eighth concept fills the page.
    await waitFor(() => expect(rowNames()).toContain("Concept 08"));
    expect(rowNames()).not.toContain("Concept 01");
    expect(rowNames()).toHaveLength(7);
    expect(screen.getByText("Showing 1–7 of 8 concepts")).toBeInTheDocument();
    await waitFor(() => expect(mocked.getStats).toHaveBeenCalledTimes(2));
  });

  it("deletes the selected concept from the details panel and selects the next row", async () => {
    const { user } = await mountLoaded();
    await user.click(within(details()).getByRole("button", { name: "Delete" }));
    await answerConfirm(user, "Delete");
    await waitFor(() => expect(mocked.deleteConcept).toHaveBeenCalledWith(1));
    await waitFor(() => expect(within(details()).getByRole("heading", { level: 3 })).toHaveTextContent("ACME"));
    expect(rowNames()).toEqual(["ACME", "Master SLA", "Bob"]);
  });

  it("shows the error when the delete fails", async () => {
    mocked.deleteConcept.mockRejectedValue(new Error("delete failed"));
    const { user } = await mountLoaded();
    await user.click(screen.getByRole("button", { name: "Delete concept Bob" }));
    await answerConfirm(user, "Delete");
    expect(await screen.findByText("delete failed")).toBeInTheDocument();
    expect(rowNames()).toHaveLength(4);
  });
});

// ---- Details panel -------------------------------------------------------------------

describe("Concepts page — details panel", () => {
  it("shows the definition, URI, synonyms, owner, date and every other property", async () => {
    await mountLoaded();
    const d = details();
    expect(within(d).getByRole("heading", { level: 3 })).toHaveTextContent("Alice");
    expect(within(d).getByText("Person")).toHaveClass("badge-accent");
    expect(within(d).getByText(LONG_DEF)).toBeInTheDocument(); // 216 < 2000: not clipped
    expect(within(d).getByText("urn:concept:person:1")).toBeInTheDocument();
    expect(within(d).getByText("Ally")).toHaveClass("tag-chip");
    expect(within(d).getByText("A.")).toHaveClass("tag-chip");
    expect(within(d).getByText("Bob")).toBeInTheDocument(); // owner
    expect(within(d).getByText("Mar 9, 2024")).toBeInTheDocument();
    expect(within(d).getByText("Properties (5)")).toBeInTheDocument();
    const props = within(d).getByRole("table");
    expect([...props.querySelectorAll("th")].map((th) => th.textContent)).toEqual([
      "linked",
      "score",
      "status",
      "tags",
      "updated_at",
    ]);
    expect(within(props).getByText("1,234")).toBeInTheDocument(); // number → formatted
    expect(within(props).getByText('{"vip":true}')).toBeInTheDocument(); // object → JSON
    expect(within(props).getByText("1,709,985,600")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "View Relations" })).toHaveAttribute("href", "/graph?seed=1");
  });

  it("switches on row click and falls back for a concept without properties", async () => {
    const { user } = await mountLoaded();
    await user.click(within(libraryRows()[3]).getByText("Bob"));
    const d = details();
    expect(within(d).getByRole("heading", { level: 3 })).toHaveTextContent("Bob");
    expect(within(d).getByText("No definition provided.")).toBeInTheDocument();
    expect(within(d).getByText("Properties (0)")).toBeInTheDocument();
    expect(within(d).getAllByText("—")).toHaveLength(4); // synonyms, owner, date, properties
    expect(libraryRows()[3]).toHaveClass("row-active");

    // Comma-separated synonyms and array-valued properties.
    await user.click(within(libraryRows()[2]).getByText("Master SLA"));
    expect(within(details()).getByText("sla")).toHaveClass("tag-chip");
    expect(within(details()).getByText("agreement")).toHaveClass("tag-chip");
    expect(within(details()).getByText("ACME, Globex")).toBeInTheDocument();
  });
});

// ---- Relations panel --------------------------------------------------------------------

describe("Concepts page — relations panel", () => {
  it("lists outgoing and incoming relations, naming the peer when it is on the page", async () => {
    await mountLoaded();
    const panel = relPanel();
    expect(await within(panel).findByText("Outgoing")).toBeInTheDocument();
    expect(within(panel).getByText("Incoming")).toBeInTheDocument();
    expect(mocked.listRelations).toHaveBeenCalledWith({ source: 1, limit: 200 });
    expect(mocked.listRelations).toHaveBeenCalledWith({ target: 1, limit: 200 });
    const rows = within(panel).getAllByRole("listitem");
    expect(rows).toHaveLength(3);
    expect(rows[0]).toHaveTextContent("→works_forCompany: ACME");
    expect(rows[1]).toHaveTextContent("→knowsConcept #99"); // peer not on this page
    expect(rows[2]).toHaveTextContent("←knowsPerson: Bob");
  });

  it("shows 'No relations.' and disables + Add when no relation type has the concept's type as domain", async () => {
    const { user } = await mountLoaded();
    await within(relPanel()).findByText("Outgoing");
    await user.click(within(libraryRows()[2]).getByText("Master SLA"));
    expect(await within(relPanel()).findByText("No relations.")).toBeInTheDocument();
    const add = within(relPanel()).getByRole("button", { name: "+ Add" });
    expect(add).toBeDisabled();
    expect(add).toHaveAttribute("title", "No relation types defined with this concept's type as domain.");
  });

  it("adds a relation whose targets are restricted to the chosen type's range", async () => {
    const { user } = await mountLoaded();
    const panel = relPanel();
    await within(panel).findByText("Outgoing");
    const add = within(panel).getByRole("button", { name: "+ Add" });
    expect(add).toBeEnabled();
    await user.click(add);
    expect(within(panel).getByRole("button", { name: "Cancel" })).toBeInTheDocument();

    const [typeSel, targetSel] = within(panel).getAllByRole("combobox");
    expect(within(typeSel).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "Relation type…",
      "works_for (Person → Company)",
      "knows (Person → Person)",
    ]);
    // No type yet: every other concept is a candidate.
    expect(within(targetSel).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "Target concept…",
      "Company: ACME",
      "Contract: Master SLA",
      "Person: Bob",
    ]);
    // The submit guard holds while nothing is chosen.
    fireEvent.submit(panel.querySelector("form.rel-form")!);
    expect(mocked.createRelation).not.toHaveBeenCalled();

    await user.selectOptions(typeSel, "works_for");
    expect(within(targetSel).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "Target concept…",
      "Company: ACME",
    ]);
    await user.selectOptions(targetSel, "2");
    await user.click(within(panel).getByRole("button", { name: "Add" }));
    await waitFor(() =>
      expect(mocked.createRelation).toHaveBeenCalledWith({ relation_type: "works_for", source: 1, target: 2 }),
    );
    await waitFor(() => expect(within(panel).getAllByRole("listitem")).toHaveLength(4));
    expect(within(panel).queryByRole("combobox")).not.toBeInTheDocument(); // form closed
    expect(within(panel).getByRole("button", { name: "+ Add" })).toBeInTheDocument();

    // Cancel folds the form away again.
    await user.click(within(panel).getByRole("button", { name: "+ Add" }));
    await user.click(within(panel).getByRole("button", { name: "Cancel" }));
    expect(within(panel).queryByRole("combobox")).not.toBeInTheDocument();
  });

  it("toasts when adding a relation fails", async () => {
    mocked.createRelation.mockRejectedValue(new Error("duplicate edge"));
    const { user } = await mountLoaded();
    const panel = relPanel();
    await within(panel).findByText("Outgoing");
    await user.click(within(panel).getByRole("button", { name: "+ Add" }));
    const [typeSel, targetSel] = within(panel).getAllByRole("combobox");
    await user.selectOptions(typeSel, "knows");
    await user.selectOptions(targetSel, "4");
    await user.click(within(panel).getByRole("button", { name: "Add" }));
    expect(await screen.findByRole("status")).toHaveTextContent("Add failed: duplicate edge");
  });

  it("deletes a relation after confirmation and toasts when that fails", async () => {
    const { user } = await mountLoaded();
    const panel = relPanel();
    await within(panel).findByText("Outgoing");
    await user.click(within(panel).getAllByRole("button", { name: "Delete relation" })[0]);
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Delete this relation?")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(mocked.deleteRelation).not.toHaveBeenCalled();

    await user.click(within(panel).getAllByRole("button", { name: "Delete relation" })[0]);
    await answerConfirm(user, "Delete");
    await waitFor(() => expect(mocked.deleteRelation).toHaveBeenCalledWith(10));
    await waitFor(() => expect(within(panel).getAllByRole("listitem")).toHaveLength(2));

    mocked.deleteRelation.mockRejectedValue(new Error("locked"));
    await user.click(within(panel).getAllByRole("button", { name: "Delete relation" })[0]);
    await answerConfirm(user, "Delete");
    expect(await screen.findByRole("status")).toHaveTextContent("Delete failed: locked");
  });

  it("shows the error when the relations fail to load", async () => {
    mocked.listRelations.mockRejectedValue(new Error("relations down"));
    await mountLoaded();
    expect(await within(relPanel()).findByText("relations down")).toBeInTheDocument();
    expect(within(relPanel()).getByText("No relations.")).toBeInTheDocument();
  });
});

// ---- Hierarchy, recent activity, top domains -------------------------------------------

describe("Concepts page — side cards", () => {
  it("renders the type tree and toggles a parent node", async () => {
    const { user } = await mountLoaded();
    const tree = (await screen.findByText("Entity", { selector: ".tree-label" })).closest(".concept-tree")!;
    const labels = () => within(tree as HTMLElement).getAllByText(/.+/, { selector: ".tree-label" }).map((e) => e.textContent);
    expect(labels()).toEqual(["Contract", "Entity", "Company", "Person"]);
    // Only Entity has children, so it is the only toggle.
    const toggle = within(tree as HTMLElement).getByRole("button", { name: "Collapse" });
    await user.click(toggle);
    expect(labels()).toEqual(["Contract", "Entity"]);
    await user.click(within(tree as HTMLElement).getByRole("button", { name: "Expand" }));
    expect(labels()).toEqual(["Contract", "Entity", "Company", "Person"]);
  });

  it("lists the five most recent concepts and the per-domain counts", async () => {
    db = [...seed(), ...many(9).slice(4).map((c) => ({ ...c, id: c.id + 10 }))]; // ids 15..19 newest
    await mountLoaded();
    const activity = (await screen.findByText("“Concept 09”")).closest(".activity-list")!;
    expect(within(activity as HTMLElement).getAllByRole("listitem")).toHaveLength(5);
    expect(within(activity as HTMLElement).queryByText("“Alice”")).not.toBeInTheDocument();

    // Types are counted with limit 1 and grouped under their root type.
    expect(await screen.findByText("8 (88.9%)")).toBeInTheDocument(); // Entity = 7 persons + 1 company
    expect(screen.getByText("1 (11.1%)")).toBeInTheDocument(); // Contract
    expect(mocked.listConcepts).toHaveBeenCalledWith({ type: "Person", limit: 1 });
    expect(mocked.listConcepts).toHaveBeenCalledWith({ type: "Entity", limit: 1 });
  });

  it("counts a type whose parent chain loops under the type itself", async () => {
    mocked.getOntology.mockResolvedValue({
      concept_types: { Loop1: { name: "Loop1", parent: "Loop2" }, Loop2: { name: "Loop2", parent: "Loop1" } },
      relation_types: {},
    });
    db = [
      { id: 1, concept_type: "Loop1", name: "First" },
      { id: 2, concept_type: "Loop2", name: "Second" },
    ];
    await mountLoaded("First");
    // Neither type reaches a root, so each is its own bucket.
    expect(await screen.findAllByText("1 (50.0%)")).toHaveLength(2);
    expect(screen.getByText("Loop1", { selector: ".domain-name" })).toBeInTheDocument();
    expect(screen.getByText("Loop2", { selector: ".domain-name" })).toBeInTheDocument();
    // The library shows the direct type when the chain loops.
    expect(within(libraryRows()[0]).getByText("Loop1")).toBeInTheDocument();
  });

  it("ignores a failing per-type count and a failing recent fetch", async () => {
    mocked.listConcepts.mockImplementation(async (params = {}) => {
      if (params.limit === 500) throw new Error("recent down");
      if (params.limit === 1 && params.type === "Person") throw new Error("count down");
      let rows = db;
      if (params.type) rows = rows.filter((c) => c.concept_type === params.type);
      return { total: rows.length, concepts: rows.slice(params.offset ?? 0, (params.offset ?? 0) + (params.limit ?? 7)) };
    });
    await mountLoaded();
    expect(await screen.findByText("No recent activity.")).toBeInTheDocument();
    // Entity = Company (1) only, Contract = 1 → 50 / 50.
    expect(await screen.findAllByText("1 (50.0%)")).toHaveLength(2);
    expect(screen.queryByText(/down/)).not.toBeInTheDocument();
  });
});
