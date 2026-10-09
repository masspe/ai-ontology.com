// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The Actions page in English.

import { beforeEach, describe, expect, it, vi } from "vitest";
import Actions from "./Actions";
import { renderPage, screen } from "../test/render";
import { setLang } from "../lib/i18n";
import type { Action, Stats } from "../api";

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

const mocked = api as unknown as Record<string, ReturnType<typeof vi.fn>>;
const actions = [
  { id: 2, action_type: "Alert", name: "Escalate overdue", subject: 12, effect: "page(oncall)", parameters: { status: "reviewed" } },
] as unknown as Action[];
const stats = { actions: 1, action_types: 1 } as unknown as Stats;

beforeEach(() => {
  setLang("en", false);
  mocked.listActions!.mockResolvedValue(actions);
  mocked.getStats!.mockResolvedValue(stats);
  mocked.getOntology!.mockResolvedValue({ concept_types: {}, relation_types: {}, action_types: {} });
  mocked.listConcepts!.mockResolvedValue({ total: 0, concepts: [] });
});

describe("Actions page, English", () => {
  it("renders the library, the tiles and the details panel in English", async () => {
    renderPage(<Actions />);
    expect(await screen.findByText("Escalate overdue", { selector: "strong" })).toBeInTheDocument();
    expect(screen.getByText("Action library")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Create an action/ })).toBeInTheDocument();
    expect(screen.getByText("Manual", { selector: "td" })).toBeInTheDocument();
    expect(screen.getByText("Reviewed", { selector: ".badge" })).toBeInTheDocument();
    expect(screen.getByText("Action details")).toBeInTheDocument();
  });
});
