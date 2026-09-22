// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Rendering tests of the Rules page: library table (status / date
// derivation, search, filters, sort), details panel, delete through the
// confirm dialog, the create / edit modal with its validation and the
// LLM-assisted "Generate" box, error banner and toasts.

import { beforeEach, describe, expect, it, vi } from "vitest";
import Rules from "./Rules";
import { fireEvent, renderPage, screen, waitFor, within } from "../test/render";
import type { Concept, Ontology, Rule, Stats } from "../api";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return {
    ...actual,
    createRule: vi.fn(),
    deleteRule: vi.fn(),
    generateRule: vi.fn(),
    getOntology: vi.fn(),
    getStats: vi.fn(),
    listConcepts: vi.fn(),
    listRules: vi.fn(),
    updateRule: vi.fn(),
  };
});

import * as api from "../api";

const mocked = api as unknown as {
  createRule: ReturnType<typeof vi.fn>;
  deleteRule: ReturnType<typeof vi.fn>;
  generateRule: ReturnType<typeof vi.fn>;
  getOntology: ReturnType<typeof vi.fn>;
  getStats: ReturnType<typeof vi.fn>;
  listConcepts: ReturnType<typeof vi.fn>;
  listRules: ReturnType<typeof vi.fn>;
  updateRule: ReturnType<typeof vi.fn>;
};

// Mid-day timestamps so the rendered day is stable in every timezone.
const rules: Rule[] = [
  {
    id: 1,
    rule_type: "Validation",
    name: "Contract needs owner",
    when: "contract.owner is null",
    then: "flag(contract)",
    applies_to: [10, 11],
    strict: true,
    description: "Every contract must have an owner.",
    properties: { updated_at: 1_700_050_000 }, // 2023-11-15 12:06 UTC
  },
  {
    id: 2,
    rule_type: "Inference",
    name: "Bill on renewal",
    then: "create(invoice)",
    applies_to: [10],
    strict: false,
    properties: { status: "reviewed", created_at: "2024-03-10T12:00:00Z" },
  },
  {
    id: 3,
    rule_type: "Custom Audit",
    name: "Archive stale drafts",
    applies_to: [],
    strict: true,
    properties: { status: "draft", updated_at: "not a date" },
  },
  {
    id: 4,
    rule_type: "Constraint",
    name: "Zero-amount invoice",
    when: "invoice.amount == 0",
    applies_to: [12],
    strict: false,
    properties: { status: "disabled" },
  },
];

const stats: Stats = {
  concepts: 42,
  relations: 7,
  rules: 4,
  actions: 2,
  concept_types: 3,
  relation_types: 2,
  rule_types: 3,
  action_types: 1,
  deltas: { concepts_pct: 0, relations_pct: 0, concept_types_pct: 0, relation_types_pct: 0 },
};

const ontology: Ontology = {
  concept_types: {},
  relation_types: {},
  rule_types: {
    Validation: { name: "Validation" },
    Inference: { name: "Inference" },
    Constraint: { name: "Constraint" },
  },
};

const concepts: Concept[] = [
  { id: 10, concept_type: "Contract", name: "ACME master" },
  { id: 11, concept_type: "Contract", name: "Globex SLA" },
  { id: 12, concept_type: "Invoice", name: "INV-001" },
];

beforeEach(() => {
  mocked.listRules.mockResolvedValue(rules);
  mocked.getStats.mockResolvedValue(stats);
  mocked.getOntology.mockResolvedValue(ontology);
  mocked.listConcepts.mockResolvedValue({ total: concepts.length, concepts });
  mocked.createRule.mockResolvedValue({ id: 99 });
  mocked.updateRule.mockResolvedValue(undefined);
  mocked.deleteRule.mockResolvedValue(undefined);
  mocked.generateRule.mockResolvedValue({
    name: "Generated rule",
    when: "gen.when",
    then: "gen.then",
    description: "gen description",
    strict: true,
  });
});

function libraryTable(): HTMLElement {
  return screen.getByRole("table");
}

/** Wait for the library to be populated (the first rule is also the selected one). */
async function loaded(): Promise<void> {
  await within(libraryTable()).findByText("Contract needs owner");
}

function rowNames(): string[] {
  return within(libraryTable())
    .getAllByRole("row")
    .slice(1)
    .map((tr) => tr.querySelector(".rule-name-text")?.textContent ?? "");
}

async function openCreateModal(user: ReturnType<typeof renderPage>["user"]): Promise<HTMLFormElement> {
  await user.click(screen.getByRole("button", { name: "Create Rule" }));
  const heading = await screen.findByRole("heading", { name: "Create Rule" });
  return heading.closest("form") as HTMLFormElement;
}

describe("Rules page", () => {
  it("shows the loading state, then the library with derived status, scope and dates", async () => {
    renderPage(<Rules />);
    expect(screen.getByText("Loading rules…")).toBeInTheDocument();
    expect(screen.getByText("Loading…")).toBeInTheDocument();

    await loaded();
    expect(mocked.listRules).toHaveBeenCalledTimes(1);
    expect(mocked.getStats).toHaveBeenCalledTimes(1);
    expect(mocked.getOntology).toHaveBeenCalledTimes(1);
    expect(mocked.listConcepts).toHaveBeenCalledWith({ limit: 500 });

    const table = libraryTable();
    // Status derived from properties.status, else strict → Active / Draft.
    expect(within(table).getByText("Active")).toHaveClass("badge-success");
    expect(within(table).getByText("Reviewed")).toHaveClass("badge-accent");
    expect(within(table).getByText("Draft")).toHaveClass("badge-warn");
    expect(within(table).getByText("Disabled")).toHaveClass("badge-danger");
    // Scope = applies_to count or dash; description falls back to `when`.
    expect(within(table).getByText("2 concept(s)")).toBeInTheDocument();
    expect(within(table).getByText("Every contract must have an owner.")).toBeInTheDocument();
    expect(within(table).getByText("invoice.amount == 0")).toBeInTheDocument();
    // Dates: epoch seconds, ISO string, unparsable string, missing.
    expect(within(table).getByText("Nov 15, 2023")).toBeInTheDocument();
    expect(within(table).getByText("Mar 10, 2024")).toBeInTheDocument();
    expect(within(table).getAllByText("—").length).toBeGreaterThanOrEqual(3);
    expect(screen.getByText("Showing 4 of 4 rules")).toBeInTheDocument();
  });

  it("renders stat tiles from /stats and the applied-concept total", async () => {
    renderPage(<Rules />);
    await loaded();
    const tile = (label: string) => screen.getByText(label).closest(".stat-rich") as HTMLElement;
    expect(within(tile("Total Rules")).getByText("4")).toBeInTheDocument();
    expect(within(tile("Active Rules")).getByText("1")).toBeInTheDocument();
    expect(within(tile("Rule Types")).getByText("3")).toBeInTheDocument();
    // 2 + 1 + 0 + 1 concepts referenced.
    expect(within(tile("Applied Concepts")).getByText("4")).toBeInTheDocument();
  });

  it("falls back to row counts when /stats is missing", async () => {
    mocked.getStats.mockResolvedValue(null);
    renderPage(<Rules />);
    await loaded();
    const tile = (label: string) => screen.getByText(label).closest(".stat-rich") as HTMLElement;
    expect(within(tile("Total Rules")).getByText("4")).toBeInTheDocument();
    // Validation, Inference, Custom Audit, Constraint.
    expect(within(tile("Rule Types")).getByText("4")).toBeInTheDocument();
  });

  it("shows the empty states when there are no rules", async () => {
    mocked.listRules.mockResolvedValue([]);
    renderPage(<Rules />);
    expect(await screen.findByText("No rules match the current filters.")).toBeInTheDocument();
    expect(screen.getByText("No rule selected.")).toBeInTheDocument();
    expect(screen.getByText("No activity yet.")).toBeInTheDocument();
    expect(screen.getByText("No scopes.")).toBeInTheDocument();
    expect(screen.getByText("Showing 0 of 0 rules")).toBeInTheDocument();
  });

  it("shows the error banner when the rule list fails", async () => {
    mocked.listRules.mockRejectedValue(new Error("server down"));
    renderPage(<Rules />);
    expect(await screen.findByText("Failed to load rules: server down")).toBeInTheDocument();
    expect(screen.getByText("No rules match the current filters.")).toBeInTheDocument();
  });

  it("selects the first rule and shows its details, logic, categories and scopes", async () => {
    renderPage(<Rules />);
    await loaded();
    const details = screen.getByText("Rule Details").closest("section") as HTMLElement;
    expect(within(details).getByRole("heading", { level: 3 })).toHaveTextContent("Contract needs owner");
    expect(within(details).getByText("rule:1")).toBeInTheDocument();
    expect(within(details).getByText("Yes")).toBeInTheDocument();
    expect(within(details).getByText("Concept #10")).toBeInTheDocument();
    expect(within(details).getByText("Concept #11")).toBeInTheDocument();
    const code = details.querySelector(".rd-code") as HTMLElement;
    expect(code).toHaveTextContent("WHEN contract.owner is null");
    expect(code).toHaveTextContent("THEN flag(contract)");
    // Rule Categories legend: one entry per type with its share.
    const legend = details.querySelector(".rd-cat-legend") as HTMLElement;
    expect(within(legend).getByText("Validation")).toBeInTheDocument();
    expect(within(legend).getByText("Custom Audit")).toBeInTheDocument();
    expect(within(legend).getAllByText("25.0%")).toHaveLength(4);
    expect(details.querySelectorAll(".donut circle")).toHaveLength(5);
    // Top Scopes grouped by first applies_to concept.
    const scopes = screen.getByText("Top Scopes").closest("section") as HTMLElement;
    expect(within(scopes).getByText("Concept #10")).toBeInTheDocument();
    expect(within(scopes).getByText("2 (50.0%)")).toBeInTheDocument();
    expect(within(scopes).getByText("Unscoped")).toBeInTheDocument();
    expect(within(scopes).getByText("Concept #12")).toBeInTheDocument();
    // Recent activity lists every rule.
    expect(screen.getByText('Rule "Archive stale drafts" present')).toBeInTheDocument();
  });

  it("switches the details panel when a row is clicked", async () => {
    const { user } = renderPage(<Rules />);
    await loaded();
    await user.click(within(libraryTable()).getByText("Archive stale drafts"));
    const details = screen.getByText("Rule Details").closest("section") as HTMLElement;
    expect(within(details).getByRole("heading", { level: 3 })).toHaveTextContent("Archive stale drafts");
    expect(within(details).getByText("rule:3")).toBeInTheDocument();
    expect(within(details).getByText("Yes")).toBeInTheDocument();
    // No applies_to, no when/then, no description.
    expect(within(details).getAllByText("—").length).toBeGreaterThanOrEqual(2);
    expect(details.querySelector(".rd-code")).toHaveTextContent("WHEN (no condition)");
    expect(details.querySelector(".rd-code")).toHaveTextContent("THEN (no action)");
    expect(within(libraryTable()).getByText("Archive stale drafts").closest("tr")).toHaveClass("is-selected");
  });

  it("filters by search, type and status, and sorts by name or type", async () => {
    const { user } = renderPage(<Rules />);
    await loaded();
    const selects = screen.getAllByRole("combobox");
    const [typeSel, statusSel, sortSel] = selects as [HTMLSelectElement, HTMLSelectElement, HTMLSelectElement];
    expect(within(typeSel).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "All Types", "Constraint", "Custom Audit", "Inference", "Validation",
    ]);

    // Search matches name or description, case-insensitive.
    await user.type(screen.getByPlaceholderText("Search rules..."), "OWNER");
    expect(rowNames()).toEqual(["Contract needs owner"]);
    expect(screen.getByText("Showing 1 of 4 rules")).toBeInTheDocument();
    await user.clear(screen.getByPlaceholderText("Search rules..."));

    await user.selectOptions(typeSel, "Inference");
    expect(rowNames()).toEqual(["Bill on renewal"]);
    await user.selectOptions(typeSel, "All Types");

    await user.selectOptions(statusSel, "Disabled");
    expect(rowNames()).toEqual(["Zero-amount invoice"]);
    await user.selectOptions(statusSel, "Active");
    expect(rowNames()).toEqual(["Contract needs owner"]);
    await user.selectOptions(statusSel, "All Statuses");

    await user.selectOptions(sortSel, "Sort: Name");
    expect(rowNames()).toEqual([
      "Archive stale drafts", "Bill on renewal", "Contract needs owner", "Zero-amount invoice",
    ]);
    await user.selectOptions(sortSel, "Sort: Type");
    expect(rowNames()).toEqual([
      "Zero-amount invoice", "Archive stale drafts", "Bill on renewal", "Contract needs owner",
    ]);
    await user.selectOptions(sortSel, "Sort: Last Updated");
    expect(rowNames()).toEqual([
      "Contract needs owner", "Bill on renewal", "Archive stale drafts", "Zero-amount invoice",
    ]);

    // Combined filters with no match.
    await user.selectOptions(typeSel, "Validation");
    await user.selectOptions(statusSel, "Draft");
    expect(screen.getByText("No rules match the current filters.")).toBeInTheDocument();
  });

  it("deletes a rule from the row only after the confirm dialog is accepted", async () => {
    const { user } = renderPage(<Rules />);
    await loaded();
    const deleteButtons = screen.getAllByRole("button", { name: "Delete rule" });
    expect(deleteButtons).toHaveLength(4);

    await user.click(deleteButtons[1]!);
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Delete this rule?")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(mocked.deleteRule).not.toHaveBeenCalled();
    expect(screen.getByText("Bill on renewal")).toBeInTheDocument();
    // The row click did not bubble: the selection is unchanged.
    expect(within(libraryTable()).getByText("Contract needs owner").closest("tr")).toHaveClass("is-selected");

    await user.click(screen.getAllByRole("button", { name: "Delete rule" })[1]!);
    await user.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(mocked.deleteRule).toHaveBeenCalledWith(2));
    expect(await screen.findByRole("status")).toHaveTextContent("Rule deleted.");
    expect(screen.queryByText("Bill on renewal")).not.toBeInTheDocument();
    expect(screen.getByText("Showing 3 of 3 rules")).toBeInTheDocument();
  });

  it("deletes the selected rule from the details panel and reports failures", async () => {
    mocked.deleteRule.mockRejectedValue(new Error("locked"));
    const { user } = renderPage(<Rules />);
    await loaded();
    const details = screen.getByText("Rule Details").closest("section") as HTMLElement;
    await user.click(within(details).getByRole("button", { name: "Delete" }));
    await user.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(mocked.deleteRule).toHaveBeenCalledWith(1));
    expect(await screen.findByRole("status")).toHaveTextContent("Delete failed: locked");
    expect(within(libraryTable()).getByText("Contract needs owner")).toBeInTheDocument();
  });

  it("creates a rule through the modal and selects it", async () => {
    const { user } = renderPage(<Rules />);
    await loaded();
    const form = await openCreateModal(user);

    const typeSelect = within(form).getByLabelText("Rule Type") as HTMLSelectElement;
    expect(typeSelect).toBeEnabled();
    expect(typeSelect.value).toBe("Constraint"); // sorted first
    await user.selectOptions(typeSelect, "Inference");
    await user.type(within(form).getByLabelText("Name"), "  New rule  ");
    await user.type(within(form).getByLabelText("Description"), "desc");
    await user.type(within(form).getByLabelText("When"), "a");
    await user.type(within(form).getByLabelText("Then"), "b");
    const applies = within(form).getByLabelText(/Applies To/) as HTMLSelectElement;
    expect(within(applies).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "Contract: ACME master", "Contract: Globex SLA", "Invoice: INV-001",
    ]);
    await user.selectOptions(applies, ["10", "12"]);
    await user.click(within(form).getByRole("checkbox"));
    await user.click(within(form).getByRole("button", { name: "Create" }));

    await waitFor(() =>
      expect(mocked.createRule).toHaveBeenCalledWith({
        rule_type: "Inference",
        name: "New rule",
        when: "a",
        then: "b",
        applies_to: [10, 12],
        strict: true,
        description: "desc",
        properties: {},
      }),
    );
    await waitFor(() => expect(screen.queryByRole("heading", { name: "Create Rule" })).not.toBeInTheDocument());
    const details = screen.getByText("Rule Details").closest("section") as HTMLElement;
    expect(within(details).getByRole("heading", { level: 3 })).toHaveTextContent("New rule");
    expect(within(details).getByText("rule:99")).toBeInTheDocument();
    expect(screen.getByText("Showing 5 of 5 rules")).toBeInTheDocument();
  });

  it("blocks submission without a name or without a concept", async () => {
    const { user } = renderPage(<Rules />);
    await loaded();
    const form = await openCreateModal(user);

    // No name: nothing happens.
    fireEvent.submit(form);
    expect(mocked.createRule).not.toHaveBeenCalled();
    expect(screen.queryByText("Select at least one concept.")).not.toBeInTheDocument();

    // Name but no concept: inline error, select flagged invalid.
    await user.type(within(form).getByLabelText("Name"), "Nameless scope");
    fireEvent.submit(form);
    expect(mocked.createRule).not.toHaveBeenCalled();
    expect(screen.getByText("Select at least one concept.")).toBeInTheDocument();
    const applies = within(form).getByLabelText(/Applies To/);
    expect(applies).toHaveAttribute("aria-invalid", "true");

    // Picking a concept clears the error.
    await user.selectOptions(applies, "11");
    expect(screen.queryByText("Select at least one concept.")).not.toBeInTheDocument();
    expect(applies).toHaveAttribute("aria-invalid", "false");
  });

  it("closes the modal from Cancel or the backdrop, but not from inside the card", async () => {
    const { user } = renderPage(<Rules />);
    await loaded();
    let form = await openCreateModal(user);
    await user.click(within(form).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("heading", { name: "Create Rule" })).not.toBeInTheDocument();

    form = await openCreateModal(user);
    await user.click(screen.getByRole("heading", { name: "Create Rule" }));
    expect(screen.getByRole("heading", { name: "Create Rule" })).toBeInTheDocument();
    await user.click(form.parentElement as HTMLElement);
    expect(screen.queryByRole("heading", { name: "Create Rule" })).not.toBeInTheDocument();
    expect(mocked.createRule).not.toHaveBeenCalled();
  });

  it("edits the selected rule with an immutable type and patches it", async () => {
    const updated: Rule = { ...rules[0]!, name: "Renamed", strict: false };
    mocked.updateRule.mockResolvedValue(updated);
    const { user } = renderPage(<Rules />);
    await loaded();
    await user.click(screen.getByRole("button", { name: "Edit Rule" }));
    const form = (await screen.findByRole("heading", { name: "Edit Rule" })).closest("form") as HTMLFormElement;

    const typeSelect = within(form).getByLabelText(/^Rule Type/) as HTMLSelectElement;
    expect(typeSelect).toBeDisabled();
    expect(typeSelect.value).toBe("Validation");
    expect(within(form).getByText("Type is immutable.")).toBeInTheDocument();
    expect(within(form).getByLabelText("Name")).toHaveValue("Contract needs owner");
    expect(within(form).getByLabelText("When")).toHaveValue("contract.owner is null");
    expect(within(form).getByRole("checkbox")).toBeChecked();
    const applies = within(form).getByLabelText(/Applies To/) as HTMLSelectElement;
    expect(Array.from(applies.selectedOptions).map((o) => o.value)).toEqual(["10", "11"]);

    await user.clear(within(form).getByLabelText("Name"));
    await user.type(within(form).getByLabelText("Name"), "Renamed");
    await user.click(within(form).getByRole("checkbox"));
    await user.click(within(form).getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(mocked.updateRule).toHaveBeenCalledWith(1, {
        name: "Renamed",
        when: "contract.owner is null",
        then: "flag(contract)",
        applies_to: [10, 11],
        strict: false,
        description: "Every contract must have an owner.",
        properties: { updated_at: 1_700_050_000 },
      }),
    );
    await waitFor(() => expect(screen.queryByRole("heading", { name: "Edit Rule" })).not.toBeInTheDocument());
    expect(within(libraryTable()).getByText("Renamed")).toBeInTheDocument();
    expect(screen.queryByText("Contract needs owner")).not.toBeInTheDocument();
    const details = screen.getByText("Rule Details").closest("section") as HTMLElement;
    expect(within(details).getByText("No")).toBeInTheDocument();
  });

  it("keeps the modal open and toasts when saving fails", async () => {
    mocked.createRule.mockRejectedValue(new Error("duplicate name"));
    const { user } = renderPage(<Rules />);
    await loaded();
    const form = await openCreateModal(user);
    await user.type(within(form).getByLabelText("Name"), "Dup");
    await user.selectOptions(within(form).getByLabelText(/Applies To/), "10");
    await user.click(within(form).getByRole("button", { name: "Create" }));
    expect(await screen.findByRole("status")).toHaveTextContent("Save failed: duplicate name");
    expect(screen.getByRole("heading", { name: "Create Rule" })).toBeInTheDocument();
    expect(screen.getByText("Showing 4 of 4 rules")).toBeInTheDocument();
  });

  it("generates the rule fields from a prompt once a concept and prompt are given", async () => {
    let resolveGen: (v: unknown) => void = () => {};
    mocked.generateRule.mockImplementation(
      () => new Promise((resolve) => { resolveGen = resolve; }),
    );
    const { user } = renderPage(<Rules />);
    await loaded();
    const form = await openCreateModal(user);

    const generate = within(form).getByRole("button", { name: "Generate" });
    expect(generate).toBeDisabled();
    expect(within(form).getByText("Select at least one concept under Applies To before generating.")).toBeInTheDocument();

    await user.selectOptions(within(form).getByLabelText(/Applies To/), "11");
    expect(within(form).getByText("Describe the rule to enable generation.")).toBeInTheDocument();
    expect(generate).toBeDisabled();

    await user.type(within(form).getByPlaceholderText("Describe the rule you want to create…"), "  flag orphans ");
    expect(within(form).getByText("Fills in Name, When, Then, Description, Strict.")).toBeInTheDocument();
    expect(generate).toBeEnabled();

    await user.click(generate);
    expect(mocked.generateRule).toHaveBeenCalledWith("flag orphans", "Constraint", [11]);
    expect(await within(form).findByRole("button", { name: "Generating…" })).toBeDisabled();

    resolveGen({ name: "Generated rule", when: "gen.when", then: "gen.then", description: "gen description", strict: true });
    expect(await within(form).findByRole("button", { name: "Generate" })).toBeEnabled();
    expect(within(form).getByLabelText("Name")).toHaveValue("Generated rule");
    expect(within(form).getByLabelText("When")).toHaveValue("gen.when");
    expect(within(form).getByLabelText("Then")).toHaveValue("gen.then");
    expect(within(form).getByLabelText("Description")).toHaveValue("gen description");
    expect(within(form).getByRole("checkbox")).toBeChecked();
  });

  it("shows the generation error and leaves the fields untouched", async () => {
    mocked.generateRule.mockRejectedValue(new Error("LLM unavailable"));
    const { user } = renderPage(<Rules />);
    await loaded();
    const form = await openCreateModal(user);
    await user.selectOptions(within(form).getByLabelText(/Applies To/), "10");
    await user.type(within(form).getByPlaceholderText("Describe the rule you want to create…"), "x");
    await user.click(within(form).getByRole("button", { name: "Generate" }));
    expect(await within(form).findByText("LLM unavailable")).toBeInTheDocument();
    expect(within(form).getByLabelText("Name")).toHaveValue("");
    expect(within(form).getByRole("button", { name: "Generate" })).toBeEnabled();
  });

  it("falls back to a free-text rule type when the ontology declares none", async () => {
    mocked.getOntology.mockRejectedValue(new Error("no ontology"));
    mocked.listConcepts.mockResolvedValue({ total: 0, concepts: [] });
    const { user } = renderPage(<Rules />);
    await loaded();
    const form = await openCreateModal(user);

    const typeInput = within(form).getByLabelText("Rule Type");
    expect(typeInput.tagName).toBe("INPUT");
    expect(typeInput).toHaveValue("");
    // No concepts loaded: the multi-select is empty and nothing can be generated.
    expect(within(within(form).getByLabelText(/Applies To/)).queryAllByRole("option")).toHaveLength(0);

    await user.type(typeInput, "Custom Audit");
    await user.type(within(form).getByLabelText("Name"), "Free type");
    fireEvent.submit(form);
    expect(screen.getByText("Select at least one concept.")).toBeInTheDocument();
    expect(mocked.createRule).not.toHaveBeenCalled();
  });

  it("keeps the quick-action links inert", async () => {
    const { user } = renderPage(<Rules />);
    await loaded();
    for (const title of ["Import Rules", "Generate with AI", "Bulk Edit", "Run Validation"]) {
      const link = screen.getByText(title).closest("a") as HTMLAnchorElement;
      expect(link).toHaveAttribute("href", "#");
      await user.click(link);
    }
    // Still on the page, no modal opened, nothing fetched.
    expect(screen.getByRole("heading", { name: "Rules" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Create Rule" })).not.toBeInTheDocument();
    expect(mocked.generateRule).not.toHaveBeenCalled();
  });

  it("asks for a rule type before generating when the type is blank", async () => {
    mocked.getOntology.mockResolvedValue({ concept_types: {}, relation_types: {} });
    const { user } = renderPage(<Rules />);
    await loaded();
    const form = await openCreateModal(user);
    await user.selectOptions(within(form).getByLabelText(/Applies To/), "10");
    expect(within(form).getByText("Pick a rule type first.")).toBeInTheDocument();
    expect(within(form).getByRole("button", { name: "Generate" })).toBeDisabled();
    await user.type(within(form).getByLabelText("Rule Type"), "Validation");
    expect(within(form).getByText("Describe the rule to enable generation.")).toBeInTheDocument();
  });
});
