// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The approvals card of the Actions page: hidden while nothing was asked,
// the statement that runs and its outcome are shown, a decision goes
// through the confirm dialog, and a refused one says why.

import { beforeEach, describe, expect, it, vi } from "vitest";
import Approvals from "./Approvals";
import { flushPromises, renderPage, screen, waitFor, within } from "../test/render";
import type { Approval } from "../api";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return { ...actual, listApprovals: vi.fn(), decideApproval: vi.fn() };
});
import * as api from "../api";
const m = api as unknown as Record<string, ReturnType<typeof vi.fn>>;

const SQL = "update orders set state = 'done' where code = 'O-7'";
const pending: Approval = {
  id: 5,
  action_id: 1,
  action_type: "close",
  action_name: "Close order 7",
  source_id: 1,
  source_name: "erp",
  statement_id: 1,
  sql: SQL,
  max_rows: 1,
  requested_by: "admin@example.com",
  requested_at: "2026-11-24T12:00:00Z",
  status: "pending",
};
const decided = (id: number, status: Approval["status"], more: Partial<Approval> = {}): Approval => ({
  ...pending,
  id,
  action_name: `Asked ${id}`,
  status,
  decided_by: "second@example.com",
  ...more,
});

beforeEach(() => {
  m.listApprovals!.mockReset().mockResolvedValue({ approvals: [pending] });
  m.decideApproval!.mockReset().mockResolvedValue({ ...pending, status: "done" });
});

describe("Approvals", () => {
  it("stays hidden while nothing was asked, and from who may not read the list", async () => {
    m.listApprovals!.mockResolvedValueOnce({ approvals: [] });
    const first = renderPage(<Approvals reload={0} />);
    await flushPromises();
    expect(screen.queryByText("Écritures à approuver")).not.toBeInTheDocument();
    first.unmount();

    m.listApprovals!.mockRejectedValueOnce(new Error("administrators only"));
    renderPage(<Approvals reload={0} />);
    await flushPromises();
    expect(screen.queryByText("Écritures à approuver")).not.toBeInTheDocument();
  });

  it("shows the statement that runs and what became of each request", async () => {
    m.listApprovals!.mockResolvedValue({
      approvals: [
        pending,
        decided(4, "running"),
        decided(3, "done", { rows_affected: 1 }),
        decided(2, "failed", { error: "would change 5 rows, at most 1 allowed: rolled back" }),
        decided(1, "rejected"),
      ],
    });
    renderPage(<Approvals reload={0} />);
    const row = (await screen.findByText("Close order 7")).closest("tr") as HTMLElement;
    expect(within(row).getByText(SQL)).toBeInTheDocument();
    expect(row).toHaveTextContent("erp");
    expect(row).toHaveTextContent("1 ligne(s) au plus");
    expect(row).toHaveTextContent("admin@example.com");
    expect(row).toHaveTextContent("en attente");

    const of = (name: string) => screen.getByText(name).closest("tr") as HTMLElement;
    expect(of("Asked 4")).toHaveTextContent("en cours");
    expect(of("Asked 3")).toHaveTextContent("exécutée");
    expect(of("Asked 3")).toHaveTextContent("1 ligne(s) modifiée(s)");
    expect(of("Asked 3")).toHaveTextContent("second@example.com");
    expect(of("Asked 2")).toHaveTextContent("échouée");
    expect(of("Asked 2")).toHaveTextContent("would change 5 rows");
    expect(of("Asked 1")).toHaveTextContent("refusée");
    // Only what still waits can be decided.
    expect(screen.getAllByRole("button", { name: "Approuver" })).toHaveLength(1);
    expect(within(of("Asked 3")).queryByRole("button")).not.toBeInTheDocument();
  });

  it("approves only once the dialog that shows the statement is accepted, then reads the list again", async () => {
    const { user } = renderPage(<Approvals reload={0} />);
    await user.click(await screen.findByRole("button", { name: "Approuver" }));
    let dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent(`Cette instruction sera exécutée sur erp : ${SQL}`);
    await user.click(within(dialog).getByRole("button", { name: "Annuler" }));
    expect(m.decideApproval).not.toHaveBeenCalled();

    m.listApprovals!.mockResolvedValue({ approvals: [decided(5, "done", { rows_affected: 1 })] });
    await user.click(screen.getByRole("button", { name: "Approuver" }));
    dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Approuver" }));
    await waitFor(() => expect(m.decideApproval).toHaveBeenCalledWith(5, "approve"));
    expect(await screen.findByText("exécutée")).toBeInTheDocument();
    expect(m.listApprovals).toHaveBeenCalledTimes(2);
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("rejects a request, and says why a decision was refused", async () => {
    const { user } = renderPage(<Approvals reload={0} />);
    await user.click(await screen.findByRole("button", { name: "Refuser" }));
    await user.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Refuser" }));
    await waitFor(() => expect(m.decideApproval).toHaveBeenCalledWith(5, "reject"));

    m.decideApproval!.mockRejectedValueOnce(new Error("another administrator must approve what you asked for"));
    await user.click(screen.getByRole("button", { name: "Approuver" }));
    await user.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Approuver" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("another administrator must approve what you asked for");

    m.decideApproval!.mockRejectedValueOnce("offline");
    await user.click(screen.getByRole("button", { name: "Refuser" }));
    await user.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Refuser" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("offline");
  });

  it("reads the list again when the page asked for a new write", async () => {
    const { rerender } = renderPage(<Approvals reload={0} />);
    await screen.findByText("Close order 7");
    rerender(<Approvals reload={1} />);
    await waitFor(() => expect(m.listApprovals).toHaveBeenCalledTimes(2));
  });
});
