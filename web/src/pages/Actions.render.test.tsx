// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Rendering tests of the Actions page: library table (status / trigger /
// date derivation, search, filters, sort), details panel with the status
// breakdown, delete through the confirm dialog, the create / edit modal and
// its validation, error banner and toasts.

import { beforeEach, describe, expect, it, vi } from "vitest";
import Actions from "./Actions";
import { fireEvent, renderPage, screen, waitFor, within } from "../test/render";
import type { Action, Concept, Ontology, Stats } from "../api";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return {
    ...actual,
    createAction: vi.fn(),
    deleteAction: vi.fn(),
    getOntology: vi.fn(),
    getStats: vi.fn(),
    listActions: vi.fn(),
    listConcepts: vi.fn(),
    updateAction: vi.fn(),
  };
});

import * as api from "../api";

const mocked = api as unknown as {
  createAction: ReturnType<typeof vi.fn>;
  deleteAction: ReturnType<typeof vi.fn>;
  getOntology: ReturnType<typeof vi.fn>;
  getStats: ReturnType<typeof vi.fn>;
  listActions: ReturnType<typeof vi.fn>;
  listConcepts: ReturnType<typeof vi.fn>;
  updateAction: ReturnType<typeof vi.fn>;
};

// Mid-day timestamps so the rendered day is stable in every timezone.
const actions: Action[] = [
  {
    id: 1,
    action_type: "Automation",
    name: "Notify owner",
    subject: 10,
    object: 11,
    effect: "send(email)",
    description: "Email the contract owner.",
    parameters: { trigger: "On update", updated_at: 1_700_050_000 }, // 2023-11-15 12:06 UTC
  },
  {
    id: 2,
    action_type: "Alert",
    name: "Escalate overdue",
    subject: 12,
    effect: "page(oncall)",
    parameters: { status: "reviewed", created_at: "2024-03-10T12:00:00Z" },
  },
  {
    id: 3,
    action_type: "Custom Sync",
    name: "Archive drafts",
    subject: 10,
    object: null,
    parameters: { status: "draft", updated_at: "not a date" },
  },
  {
    id: 4,
    action_type: "Automation",
    name: "Bill renewal",
    subject: 11,
    effect: "create(invoice)",
    parameters: { status: "paused" },
  },
];

const stats: Stats = {
  concepts: 42,
  relations: 7,
  rules: 4,
  actions: 4,
  concept_types: 3,
  relation_types: 2,
  rule_types: 3,
  action_types: 2,
  deltas: { concepts_pct: 0, relations_pct: 0, concept_types_pct: 0, relation_types_pct: 0 },
};

const ontology: Ontology = {
  concept_types: {},
  relation_types: {},
  action_types: {
    Automation: { name: "Automation", subject: "Contract" },
    Alert: { name: "Alert", subject: "Invoice" },
  },
};

const concepts: Concept[] = [
  { id: 10, concept_type: "Contract", name: "ACME master" },
  { id: 11, concept_type: "Contract", name: "Globex SLA" },
  { id: 12, concept_type: "Invoice", name: "INV-001" },
];

beforeEach(() => {
  mocked.listActions.mockResolvedValue(actions);
  mocked.getStats.mockResolvedValue(stats);
  mocked.getOntology.mockResolvedValue(ontology);
  mocked.listConcepts.mockResolvedValue({ total: concepts.length, concepts });
  mocked.createAction.mockResolvedValue({ id: 99 });
  mocked.updateAction.mockResolvedValue(undefined);
  mocked.deleteAction.mockResolvedValue(undefined);
});

function libraryTable(): HTMLElement {
  return screen.getByRole("table");
}

/** Wait for the library to be populated (the first action is also the selected one). */
async function loaded(): Promise<void> {
  await within(libraryTable()).findByText("Notify owner");
}

function rowNames(): string[] {
  return within(libraryTable())
    .getAllByRole("row")
    .slice(1)
    .map((tr) => tr.querySelector(".rule-name-text")?.textContent ?? "");
}

function detailsPanel(): HTMLElement {
  return screen.getByText("Action Details").closest("section") as HTMLElement;
}

async function openCreateModal(user: ReturnType<typeof renderPage>["user"]): Promise<HTMLFormElement> {
  await user.click(screen.getByRole("button", { name: "Create Action" }));
  const heading = await screen.findByRole("heading", { name: "Create Action" });
  return heading.closest("form") as HTMLFormElement;
}

describe("Actions page", () => {
  it("shows the loading state, then the library with derived status, trigger and dates", async () => {
    renderPage(<Actions />);
    expect(screen.getByText("Loading actions…")).toBeInTheDocument();
    expect(screen.getByText("Loading…")).toBeInTheDocument();

    await loaded();
    expect(mocked.listActions).toHaveBeenCalledTimes(1);
    expect(mocked.getStats).toHaveBeenCalledTimes(1);
    expect(mocked.getOntology).toHaveBeenCalledTimes(1);
    expect(mocked.listConcepts).toHaveBeenCalledWith({ limit: 500 });

    const table = libraryTable();
    // Status from parameters.status, default Active.
    expect(within(table).getByText("Active")).toHaveClass("badge-success");
    expect(within(table).getByText("Reviewed")).toHaveClass("badge-accent");
    expect(within(table).getByText("Draft")).toHaveClass("badge-warn");
    expect(within(table).getByText("Paused")).toHaveClass("badge-danger");
    // Trigger from parameters, default Manual; description falls back to effect.
    expect(within(table).getByText("On update")).toBeInTheDocument();
    expect(within(table).getAllByText("Manual")).toHaveLength(3);
    expect(within(table).getByText("Email the contract owner.")).toBeInTheDocument();
    expect(within(table).getByText("page(oncall)")).toBeInTheDocument();
    // Dates: epoch seconds, ISO string, unparsable string, missing.
    expect(within(table).getByText("Nov 15, 2023")).toBeInTheDocument();
    expect(within(table).getByText("Mar 10, 2024")).toBeInTheDocument();
    expect(within(table).getAllByText("—")).toHaveLength(2);
    expect(screen.getByText("Showing 4 of 4 actions")).toBeInTheDocument();
  });

  it("renders stat tiles from /stats and the paused/draft count", async () => {
    renderPage(<Actions />);
    await loaded();
    const tile = (label: string) => screen.getByText(label).closest(".stat-rich") as HTMLElement;
    expect(within(tile("Total Actions")).getByText("4")).toBeInTheDocument();
    expect(within(tile("Active Automations")).getByText("1")).toBeInTheDocument();
    expect(within(tile("Action Types")).getByText("2")).toBeInTheDocument();
    expect(within(tile("Paused/Draft")).getByText("2")).toBeInTheDocument();
  });

  it("falls back to the row count when /stats is missing", async () => {
    mocked.getStats.mockResolvedValue(null);
    renderPage(<Actions />);
    await loaded();
    const tile = (label: string) => screen.getByText(label).closest(".stat-rich") as HTMLElement;
    expect(within(tile("Total Actions")).getByText("4")).toBeInTheDocument();
    expect(within(tile("Action Types")).getByText("0")).toBeInTheDocument();
  });

  it("shows the empty states when there are no actions", async () => {
    mocked.listActions.mockResolvedValue([]);
    renderPage(<Actions />);
    expect(await screen.findByText("No actions match the current filters.")).toBeInTheDocument();
    expect(screen.getByText("No action selected.")).toBeInTheDocument();
    expect(screen.getByText("No activity yet.")).toBeInTheDocument();
    expect(screen.getByText("No types.")).toBeInTheDocument();
    expect(screen.getByText("Showing 0 of 0 actions")).toBeInTheDocument();
  });

  it("shows the error banner when the action list fails", async () => {
    mocked.listActions.mockRejectedValue(new Error("server down"));
    renderPage(<Actions />);
    expect(await screen.findByText("Failed to load actions: server down")).toBeInTheDocument();
    expect(screen.getByText("No actions match the current filters.")).toBeInTheDocument();
  });

  it("selects the first action and shows its details, effect, status breakdown and top types", async () => {
    renderPage(<Actions />);
    await loaded();
    const details = detailsPanel();
    expect(within(details).getByRole("heading", { level: 3 })).toHaveTextContent("Notify owner");
    expect(within(details).getByText("action:1")).toBeInTheDocument();
    expect(within(details).getByText("On update")).toBeInTheDocument();
    expect(within(details).getByText("Concept #10")).toBeInTheDocument();
    expect(within(details).getByText("Concept #11")).toBeInTheDocument();
    expect(within(details).getByText("Nov 15, 2023")).toBeInTheDocument();
    expect(details.querySelector(".rd-code")).toHaveTextContent("send(email)");
    // Status breakdown: one entry per non-empty status, percentages of the total.
    const legend = details.querySelector(".rd-cat-legend") as HTMLElement;
    expect(within(legend).getAllByRole("listitem").map((li) => li.textContent)).toEqual([
      "Active125%", "Reviewed125%", "Draft125%", "Paused125%",
    ]);
    expect(details.querySelectorAll(".donut circle")).toHaveLength(5);
    // Top Action Types sorted by count.
    const types = screen.getByText("Top Action Types").closest("section") as HTMLElement;
    const rows = within(types).getAllByRole("listitem");
    expect(rows[0]).toHaveTextContent("Automation");
    expect(rows[0]).toHaveTextContent("2 (50.0%)");
    expect(rows).toHaveLength(3);
    // Recent activity lists every action.
    expect(screen.getByText('Action "Archive drafts" present')).toBeInTheDocument();
  });

  it("switches the details panel when a row is clicked", async () => {
    const { user } = renderPage(<Actions />);
    await loaded();
    await user.click(within(libraryTable()).getByText("Archive drafts"));
    const details = detailsPanel();
    expect(within(details).getByRole("heading", { level: 3 })).toHaveTextContent("Archive drafts");
    expect(within(details).getByText("action:3")).toBeInTheDocument();
    expect(within(details).getByText("Manual")).toBeInTheDocument();
    // No object, no date, no effect.
    expect(within(details).getAllByText("—")).toHaveLength(2);
    expect(details.querySelector(".rd-code")).toHaveTextContent("(no effect declared)");
    expect(within(libraryTable()).getByText("Archive drafts").closest("tr")).toHaveClass("is-selected");
  });

  it("filters by search, type and status, and sorts by name or type", async () => {
    const { user } = renderPage(<Actions />);
    await loaded();
    const [typeSel, statusSel, sortSel] = screen.getAllByRole("combobox") as [
      HTMLSelectElement, HTMLSelectElement, HTMLSelectElement,
    ];
    expect(within(typeSel).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "All Types", "Alert", "Automation", "Custom Sync",
    ]);

    await user.type(screen.getByPlaceholderText("Search actions..."), "OWNER");
    expect(rowNames()).toEqual(["Notify owner"]);
    expect(screen.getByText("Showing 1 of 4 actions")).toBeInTheDocument();
    await user.clear(screen.getByPlaceholderText("Search actions..."));

    await user.selectOptions(typeSel, "Automation");
    expect(rowNames()).toEqual(["Notify owner", "Bill renewal"]);
    await user.selectOptions(typeSel, "All Types");

    await user.selectOptions(statusSel, "Paused");
    expect(rowNames()).toEqual(["Bill renewal"]);
    await user.selectOptions(statusSel, "All Statuses");

    await user.selectOptions(sortSel, "Sort: Name");
    expect(rowNames()).toEqual(["Archive drafts", "Bill renewal", "Escalate overdue", "Notify owner"]);
    await user.selectOptions(sortSel, "Sort: Type");
    expect(rowNames()).toEqual(["Escalate overdue", "Notify owner", "Bill renewal", "Archive drafts"]);
    await user.selectOptions(sortSel, "Sort: Last Updated");
    expect(rowNames()).toEqual(["Notify owner", "Escalate overdue", "Archive drafts", "Bill renewal"]);

    await user.selectOptions(typeSel, "Alert");
    await user.selectOptions(statusSel, "Draft");
    expect(screen.getByText("No actions match the current filters.")).toBeInTheDocument();
  });

  it("deletes an action from the row only after the confirm dialog is accepted", async () => {
    const { user } = renderPage(<Actions />);
    await loaded();
    expect(screen.getAllByRole("button", { name: "Delete action" })).toHaveLength(4);

    await user.click(screen.getAllByRole("button", { name: "Delete action" })[1]!);
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Delete this action?")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(mocked.deleteAction).not.toHaveBeenCalled();
    expect(screen.getByText("Escalate overdue")).toBeInTheDocument();
    // The row click did not bubble: the selection is unchanged.
    expect(within(libraryTable()).getByText("Notify owner").closest("tr")).toHaveClass("is-selected");

    await user.click(screen.getAllByRole("button", { name: "Delete action" })[1]!);
    await user.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(mocked.deleteAction).toHaveBeenCalledWith(2));
    expect(await screen.findByRole("status")).toHaveTextContent("Action deleted.");
    expect(screen.queryByText("Escalate overdue")).not.toBeInTheDocument();
    expect(screen.getByText("Showing 3 of 3 actions")).toBeInTheDocument();
  });

  it("deletes the selected action from the details panel and reports failures", async () => {
    mocked.deleteAction.mockRejectedValue(new Error("locked"));
    const { user } = renderPage(<Actions />);
    await loaded();
    await user.click(within(detailsPanel()).getByRole("button", { name: "Delete" }));
    await user.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(mocked.deleteAction).toHaveBeenCalledWith(1));
    expect(await screen.findByRole("status")).toHaveTextContent("Delete failed: locked");
    expect(within(libraryTable()).getByText("Notify owner")).toBeInTheDocument();
  });

  it("creates an action through the modal and selects it", async () => {
    const { user } = renderPage(<Actions />);
    await loaded();
    const form = await openCreateModal(user);

    const typeSelect = within(form).getByLabelText("Action Type") as HTMLSelectElement;
    expect(typeSelect).toBeEnabled();
    expect(typeSelect.value).toBe("Alert"); // sorted first
    await user.selectOptions(typeSelect, "Automation");
    await user.type(within(form).getByLabelText("Name"), "  Ping owner  ");
    const subject = within(form).getByLabelText("Subject (concept)") as HTMLSelectElement;
    expect(subject.value).toBe("");
    expect(within(subject).getByRole("option", { name: "Select a subject concept…" })).toBeDisabled();
    await user.selectOptions(subject, "12");
    const object = within(form).getByLabelText("Object (concept, optional)") as HTMLSelectElement;
    expect(within(object).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "(none)", "Contract: ACME master", "Contract: Globex SLA", "Invoice: INV-001",
    ]);
    await user.selectOptions(object, "10");
    await user.type(within(form).getByLabelText("Effect"), "notify()");
    await user.type(within(form).getByLabelText("Description"), "desc");
    await user.click(within(form).getByRole("button", { name: "Create" }));

    await waitFor(() =>
      expect(mocked.createAction).toHaveBeenCalledWith({
        action_type: "Automation",
        name: "Ping owner",
        subject: 12,
        object: 10,
        parameters: {},
        effect: "notify()",
        description: "desc",
      }),
    );
    await waitFor(() => expect(screen.queryByRole("heading", { name: "Create Action" })).not.toBeInTheDocument());
    const details = detailsPanel();
    expect(within(details).getByRole("heading", { level: 3 })).toHaveTextContent("Ping owner");
    expect(within(details).getByText("action:99")).toBeInTheDocument();
    expect(within(details).getByText("Concept #12")).toBeInTheDocument();
    expect(screen.getByText("Showing 5 of 5 actions")).toBeInTheDocument();
  });

  it("creates with a null object when none is picked", async () => {
    const { user } = renderPage(<Actions />);
    await loaded();
    const form = await openCreateModal(user);
    await user.type(within(form).getByLabelText("Name"), "Solo");
    await user.selectOptions(within(form).getByLabelText("Subject (concept)"), "10");
    await user.click(within(form).getByRole("button", { name: "Create" }));
    await waitFor(() =>
      expect(mocked.createAction).toHaveBeenCalledWith(
        expect.objectContaining({ action_type: "Alert", name: "Solo", subject: 10, object: null, effect: "" }),
      ),
    );
  });

  it("blocks submission without a name or without a subject", async () => {
    const { user } = renderPage(<Actions />);
    await loaded();
    const form = await openCreateModal(user);

    fireEvent.submit(form);
    expect(mocked.createAction).not.toHaveBeenCalled();

    await user.type(within(form).getByLabelText("Name"), "No subject");
    fireEvent.submit(form);
    expect(mocked.createAction).not.toHaveBeenCalled();
    expect(screen.getByRole("heading", { name: "Create Action" })).toBeInTheDocument();
  });

  it("closes the modal from Cancel or the backdrop, but not from inside the card", async () => {
    const { user } = renderPage(<Actions />);
    await loaded();
    let form = await openCreateModal(user);
    await user.click(within(form).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("heading", { name: "Create Action" })).not.toBeInTheDocument();

    form = await openCreateModal(user);
    await user.click(screen.getByRole("heading", { name: "Create Action" }));
    expect(screen.getByRole("heading", { name: "Create Action" })).toBeInTheDocument();
    await user.click(form.parentElement as HTMLElement);
    expect(screen.queryByRole("heading", { name: "Create Action" })).not.toBeInTheDocument();
    expect(mocked.createAction).not.toHaveBeenCalled();
  });

  it("edits the selected action with an immutable type and patches it", async () => {
    const updated: Action = { ...actions[0]!, name: "Renamed", object: null };
    mocked.updateAction.mockResolvedValue(updated);
    const { user } = renderPage(<Actions />);
    await loaded();
    await user.click(screen.getByRole("button", { name: "Edit Action" }));
    const form = (await screen.findByRole("heading", { name: "Edit Action" })).closest("form") as HTMLFormElement;

    const typeSelect = within(form).getByLabelText(/^Action Type/) as HTMLSelectElement;
    expect(typeSelect).toBeDisabled();
    expect(typeSelect.value).toBe("Automation");
    expect(within(form).getByText("Type is immutable.")).toBeInTheDocument();
    expect(within(form).getByLabelText("Name")).toHaveValue("Notify owner");
    expect(within(form).getByLabelText("Subject (concept)")).toHaveValue("10");
    expect(within(form).getByLabelText("Object (concept, optional)")).toHaveValue("11");
    expect(within(form).getByLabelText("Effect")).toHaveValue("send(email)");

    await user.clear(within(form).getByLabelText("Name"));
    await user.type(within(form).getByLabelText("Name"), "Renamed");
    await user.selectOptions(within(form).getByLabelText("Object (concept, optional)"), "");
    await user.click(within(form).getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(mocked.updateAction).toHaveBeenCalledWith(1, {
        name: "Renamed",
        subject: 10,
        object: null,
        parameters: { trigger: "On update", updated_at: 1_700_050_000 },
        effect: "send(email)",
        description: "Email the contract owner.",
      }),
    );
    await waitFor(() => expect(screen.queryByRole("heading", { name: "Edit Action" })).not.toBeInTheDocument());
    expect(within(libraryTable()).getByText("Renamed")).toBeInTheDocument();
    expect(screen.queryByText("Notify owner")).not.toBeInTheDocument();
    // The object is gone from the details.
    expect(within(detailsPanel()).queryByText("Concept #11")).not.toBeInTheDocument();
  });

  it("keeps the modal open and toasts when saving fails", async () => {
    mocked.createAction.mockRejectedValue(new Error("duplicate name"));
    const { user } = renderPage(<Actions />);
    await loaded();
    const form = await openCreateModal(user);
    await user.type(within(form).getByLabelText("Name"), "Dup");
    await user.selectOptions(within(form).getByLabelText("Subject (concept)"), "10");
    await user.click(within(form).getByRole("button", { name: "Create" }));
    expect(await screen.findByRole("status")).toHaveTextContent("Save failed: duplicate name");
    expect(screen.getByRole("heading", { name: "Create Action" })).toBeInTheDocument();
    expect(screen.getByText("Showing 4 of 4 actions")).toBeInTheDocument();
  });

  it("falls back to a free-text action type when the ontology declares none", async () => {
    mocked.getOntology.mockRejectedValue(new Error("no ontology"));
    mocked.listConcepts.mockResolvedValue({ total: 0, concepts: [] });
    const { user } = renderPage(<Actions />);
    await loaded();
    const form = await openCreateModal(user);

    const typeInput = within(form).getByLabelText("Action Type");
    expect(typeInput.tagName).toBe("INPUT");
    expect(typeInput).toHaveValue("");
    // No concepts loaded: only the placeholder option remains.
    expect(within(within(form).getByLabelText("Subject (concept)")).getAllByRole("option")).toHaveLength(1);

    await user.type(typeInput, "Custom Sync");
    await user.type(within(form).getByLabelText("Name"), "Free type");
    fireEvent.submit(form);
    // No subject can be chosen, so nothing is created.
    expect(mocked.createAction).not.toHaveBeenCalled();
    expect(typeInput).toHaveValue("Custom Sync");
  });

  it("keeps the quick-action links inert", async () => {
    const { user } = renderPage(<Actions />);
    await loaded();
    for (const title of ["Import Actions", "Generate with AI", "Bulk Edit", "Run All Active"]) {
      const link = screen.getByText(title).closest("a") as HTMLAnchorElement;
      expect(link).toHaveAttribute("href", "#");
      await user.click(link);
    }
    await user.click(screen.getByRole("button", { name: "Run Now" }));
    expect(screen.getByRole("heading", { name: "Actions" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Create Action" })).not.toBeInTheDocument();
  });
});
