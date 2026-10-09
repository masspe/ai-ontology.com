// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The Queries page in English: the French sentence is the key, the
// English dictionary supplies the rest.

import { beforeEach, describe, expect, it, vi } from "vitest";
import Queries from "./Queries";
import { renderPage, screen } from "../test/render";
import { setLang } from "../lib/i18n";
import type { SavedQuery } from "../api";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return { ...actual, getQueries: vi.fn(), createQuery: vi.fn(), deleteQuery: vi.fn(), runQuery: vi.fn() };
});
vi.mock("../components/StreamingAnswer", () => ({ default: () => <div data-testid="streaming-answer" /> }));

import * as api from "../api";

const getQueries = api.getQueries as unknown as ReturnType<typeof vi.fn>;

beforeEach(() => {
  setLang("en", false);
  getQueries.mockResolvedValue({
    queries: [{ id: 1, name: "Renewals", query: "Which contracts renew?", top_k: 8, last_run_at: null } as unknown as SavedQuery],
  });
});

describe("Queries page, English", () => {
  it("renders titles, labels and buttons in English", async () => {
    renderPage(<Queries />);
    expect(await screen.findByText("Renewals")).toBeInTheDocument();
    expect(screen.getByText("Ask a question")).toBeInTheDocument();
    expect(screen.getByText("Saved questions")).toBeInTheDocument();
    expect(screen.getByPlaceholderText("Renewal deadlines")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Run" })).toBeInTheDocument();
    expect(screen.queryByText("Enregistrer")).not.toBeInTheDocument();
  });
});
