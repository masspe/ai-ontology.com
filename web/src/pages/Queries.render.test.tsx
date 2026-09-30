// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// Rendering tests of the Queries page: list, save, run, delete (with the
// confirm dialog), error banner, and the `?q=` prefill of the ask box.

import { beforeEach, describe, expect, it, vi } from "vitest";
import Queries from "./Queries";
import { flushPromises, renderPage, screen, waitFor } from "../test/render";
import type { RagAnswer, SavedQuery } from "../api";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return {
    ...actual,
    getQueries: vi.fn(),
    createQuery: vi.fn(),
    deleteQuery: vi.fn(),
    runQuery: vi.fn(),
  };
});
// The ask box streams through fetch; it has its own tests.
vi.mock("../components/StreamingAnswer", () => ({
  default: ({ defaultQuery }: { defaultQuery: string }) => (
    <div data-testid="streaming-answer">{defaultQuery}</div>
  ),
}));

import * as api from "../api";

const mocked = api as unknown as {
  getQueries: ReturnType<typeof vi.fn>;
  createQuery: ReturnType<typeof vi.fn>;
  deleteQuery: ReturnType<typeof vi.fn>;
  runQuery: ReturnType<typeof vi.fn>;
};

const saved: SavedQuery[] = [
  { id: 1, name: "Renewals", query: "Which contracts renew this quarter?", top_k: 8, last_run_at: 1_700_000_000 } as SavedQuery,
  { id: 2, name: "Late invoices", query: "x".repeat(70), top_k: 3, last_run_at: null } as unknown as SavedQuery,
];

const answer: RagAnswer = {
  answer: "Two contracts renew in Q4.",
  subgraph: {
    concepts: [
      { id: 10, concept_type: "Contract", name: "ACME master" },
      { id: 11, concept_type: "Contract", name: "Globex SLA" },
    ],
    relations: [],
  },
} as unknown as RagAnswer;

beforeEach(() => {
  mocked.getQueries.mockResolvedValue({ queries: saved });
  mocked.createQuery.mockResolvedValue({ id: 3 });
  mocked.deleteQuery.mockResolvedValue(undefined);
  mocked.runQuery.mockResolvedValue(answer);
});

describe("Queries page", () => {
  it("lists saved queries, truncating long text and formatting the last run", async () => {
    renderPage(<Queries />);
    expect(await screen.findByText("Renewals")).toBeInTheDocument();
    expect(screen.getByText("Late invoices")).toBeInTheDocument();
    // 70 chars → 60 + ellipsis; never-run → dash.
    expect(screen.getByText(`${"x".repeat(60)}…`)).toBeInTheDocument();
    expect(screen.getByText("—")).toBeInTheDocument();
    expect(mocked.getQueries).toHaveBeenCalledTimes(1);
  });

  it("shows the empty state when nothing is saved", async () => {
    mocked.getQueries.mockResolvedValue({ queries: [] });
    renderPage(<Queries />);
    expect(await screen.findByText(/Aucune question enregistrée/)).toBeInTheDocument();
  });

  it("prefills the ask box from ?q=", async () => {
    renderPage(<Queries />, { route: "/queries?q=hello%20graph" });
    expect(await screen.findByTestId("streaming-answer")).toHaveTextContent("hello graph");
  });

  it("saves a query only once both fields are filled, then clears and reloads", async () => {
    const { user } = renderPage(<Queries />);
    await screen.findByText("Renewals");
    const save = screen.getByRole("button", { name: "Enregistrer" });
    expect(save).toBeDisabled();
    await user.type(screen.getByPlaceholderText("Échéances de renouvellement"), "Mine");
    expect(save).toBeDisabled();
    await user.type(screen.getByPlaceholderText(/prochains renouvellements/), "who owes what");
    await user.clear(screen.getByRole("spinbutton"));
    await user.type(screen.getByRole("spinbutton"), "5");
    expect(save).toBeEnabled();
    await user.click(save);
    await waitFor(() =>
      expect(mocked.createQuery).toHaveBeenCalledWith({ name: "Mine", query: "who owes what", top_k: 5 }),
    );
    expect(mocked.getQueries).toHaveBeenCalledTimes(2);
    expect(screen.getByPlaceholderText("Échéances de renouvellement")).toHaveValue("");
  });

  it("runs a query and shows the answer with citations", async () => {
    const { user } = renderPage(<Queries />);
    await screen.findByText("Renewals");
    await user.click(screen.getAllByRole("button", { name: "Exécuter" })[0]);
    expect(await screen.findByText("Réponse · Renewals")).toBeInTheDocument();
    expect(screen.getByText("Two contracts renew in Q4.")).toBeInTheDocument();
    expect(screen.getByText("Contract · ACME master")).toBeInTheDocument();
    expect(mocked.runQuery).toHaveBeenCalledWith(1);
  });

  it("deletes only after the confirm dialog is accepted", async () => {
    const { user } = renderPage(<Queries />);
    await screen.findByText("Renewals");
    await user.click(screen.getAllByRole("button", { name: "Supprimer" })[0]);
    // Cancel first: nothing happens.
    await user.click(screen.getByRole("button", { name: "Annuler" }));
    expect(mocked.deleteQuery).not.toHaveBeenCalled();
    // Accept: the API is called and the list reloads.
    await user.click(screen.getAllByRole("button", { name: "Supprimer" })[0]);
    const dialogDelete = screen.getAllByRole("button", { name: "Supprimer" }).at(-1)!;
    await user.click(dialogDelete);
    await waitFor(() => expect(mocked.deleteQuery).toHaveBeenCalledWith(1));
    expect(mocked.getQueries).toHaveBeenCalledTimes(2);
  });

  it("surfaces API errors in the banner", async () => {
    mocked.getQueries.mockRejectedValueOnce(new Error("server down"));
    renderPage(<Queries />);
    expect(await screen.findByText("server down")).toBeInTheDocument();
    // A non-Error rejection is stringified.
    mocked.runQuery.mockRejectedValueOnce("boom");
    mocked.getQueries.mockResolvedValue({ queries: saved });
    const { user } = renderPage(<Queries />);
    await user.click((await screen.findAllByRole("button", { name: "Exécuter" }))[0]);
    expect(await screen.findByText("boom")).toBeInTheDocument();
  });

  it("reloads on demand", async () => {
    const { user } = renderPage(<Queries />);
    await screen.findByText("Renewals");
    await user.click(screen.getByRole("button", { name: "Recharger" }));
    await waitFor(() => expect(mocked.getQueries).toHaveBeenCalledTimes(2));
  });
});

describe("Queries — pinned to the home page", () => {
  it("pins and unpins a saved question, remembered in the browser", async () => {
    const { user } = renderPage(<Queries />);
    const pin = (await screen.findAllByRole("button", { name: "☆" }))[0]!;
    expect(pin).toHaveAttribute("aria-pressed", "false");
    await user.click(pin);
    expect(screen.getAllByRole("button", { name: "★" })).toHaveLength(1);
    expect(JSON.parse(window.localStorage.getItem("queries.pinned.v1")!)).toEqual([saved[0]!.id]);
    await user.click(screen.getByRole("button", { name: "★" }));
    expect(JSON.parse(window.localStorage.getItem("queries.pinned.v1")!)).toEqual([]);
  });

  it("replays the question named by ?run once", async () => {
    // A fresh array per call: `refresh()` after the run re-renders for real.
    mocked.getQueries.mockImplementation(async () => ({ queries: [...saved] }));
    const { rerender } = renderPage(<Queries />, { route: `/queries?run=${saved[0]!.id}` });
    await waitFor(() => expect(mocked.runQuery).toHaveBeenCalledWith(saved[0]!.id));
    rerender(<Queries />);
    await flushPromises();
    expect(mocked.runQuery).toHaveBeenCalledTimes(1);
  });
});
