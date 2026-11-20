// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The jobs light: off while nothing runs or failed, on with the count,
// the menu lists the jobs and can stop a running one.

import { describe, expect, it, vi } from "vitest";
import JobsIndicator from "./JobsIndicator";
import { fireEvent, renderPage, screen, waitFor } from "../test/render";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return { ...actual, listJobs: vi.fn(), cancelJob: vi.fn() };
});
import * as api from "../api";
const mocked = api as unknown as { listJobs: ReturnType<typeof vi.fn>; cancelJob: ReturnType<typeof vi.fn> };

const running = { id: 1, kind: "sync", label: "orders", status: "running", created_at: new Date().toISOString(), progress: 40, total: 100, message: "batch 2", log: [], by: "ann" };
const failed = { id: 2, kind: "sync", label: "tickets", status: "failed", created_at: new Date().toISOString(), finished_at: new Date().toISOString(), progress: 0, message: "connection refused", log: [], by: "ann" };
const old = { ...failed, id: 3, finished_at: new Date(Date.now() - 2 * 3600 * 1000).toISOString() };

describe("JobsIndicator", () => {
  it("stays off without jobs or when the server cannot be asked", async () => {
    mocked.listJobs.mockResolvedValue({ jobs: [old] });
    const { unmount } = renderPage(<JobsIndicator />);
    await waitFor(() => expect(mocked.listJobs).toHaveBeenCalled());
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
    unmount();
    mocked.listJobs.mockRejectedValue(new Error("offline"));
    renderPage(<JobsIndicator />);
    await waitFor(() => expect(mocked.listJobs).toHaveBeenCalledTimes(2));
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });

  it("shows the running count, lists the jobs and stops one", async () => {
    mocked.listJobs.mockResolvedValue({ jobs: [running, failed] });
    mocked.cancelJob.mockResolvedValue({ ok: true });
    renderPage(<JobsIndicator />);
    const light = await screen.findByRole("button", { name: "1 tâche(s) en cours" });
    fireEvent.click(light);
    expect(screen.getByText("orders")).toBeInTheDocument();
    expect(screen.getByText("40 / 100 batch 2")).toBeInTheDocument();
    expect(screen.getByText("tickets")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Arrêter" }));
    await waitFor(() => expect(mocked.cancelJob).toHaveBeenCalledWith(1));
  });

  it("turns red with the recent failures only", async () => {
    mocked.listJobs.mockResolvedValue({ jobs: [failed, old] });
    renderPage(<JobsIndicator />);
    const light = await screen.findByRole("button", { name: "1 tâche(s) en échec" });
    expect(light.className).toContain("jobs-failed");
  });
});
