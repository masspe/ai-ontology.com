// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The first-day steps: each opens after the previous one, the finance
// example loads in one click through the ordinary API, and the page is
// told to refresh afterwards.

import { beforeEach, describe, expect, it, vi } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import Onboarding from "./Onboarding";
import { renderPage } from "../test/render";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return { ...actual, upload: vi.fn() };
});

import * as api from "../api";

const mocked = api as unknown as { upload: ReturnType<typeof vi.fn> };

beforeEach(() => {
  vi.stubGlobal("fetch", vi.fn(async () => new Response(new Blob(["x"]))));
  mocked.upload.mockResolvedValue({ file_id: 1, ingested: { concepts: 3, relations: 5, ontology_updates: 0 } });
});

describe("Onboarding", () => {
  it("opens the steps one after the other", () => {
    const { rerender } = renderPage(<Onboarding hasModel={false} hasData={false} />);
    expect(screen.getByRole("heading", { name: "Bienvenue" })).toBeInTheDocument();
    const steps = screen.getAllByRole("listitem");
    expect(steps).toHaveLength(3);
    expect(steps[0]).toHaveAttribute("aria-current", "step");
    expect(steps[1]).toHaveClass("locked");
    expect(steps[2]).toHaveClass("locked");
    expect(screen.getByRole("link", { name: "Définir le modèle" })).toHaveAttribute("href", "/builder");
    expect(screen.getByRole("button", { name: /Essayer avec l'exemple finance/ })).toHaveTextContent(/sociétés, contrats, factures/);

    rerender(<Onboarding hasModel hasData={false} />);
    expect(screen.getAllByRole("listitem")[0]).toHaveClass("done");
    expect(screen.getAllByRole("listitem")[1]).toHaveAttribute("aria-current", "step");
    expect(screen.getByRole("link", { name: "Déposer des fichiers" })).toHaveAttribute("href", "/files");

    rerender(<Onboarding hasModel hasData />);
    expect(screen.getAllByRole("listitem")[1]).toHaveClass("done");
    expect(screen.getByRole("link", { name: "Poser une question" })).toHaveAttribute("href", "/queries");
  });

  it("loads the finance example through the API and reports back", async () => {
    const onLoaded = vi.fn();
    const { user } = renderPage(<Onboarding hasModel={false} hasData={false} onLoaded={onLoaded} />);
    await user.click(screen.getByRole("button", { name: /Essayer avec l'exemple finance/ }));
    await waitFor(() => expect(onLoaded).toHaveBeenCalledTimes(1));
    expect(mocked.upload).toHaveBeenCalledTimes(8);
    expect(mocked.upload.mock.calls[0]![0].name).toBe("ontology.json");
    expect(mocked.upload.mock.calls[7]![0].name).toBe("relations.jsonl");
    expect(screen.queryByText(/Chargement/)).not.toBeInTheDocument();
  });

  it("shows the API error and stays usable", async () => {
    mocked.upload.mockRejectedValue(new Error("API unreachable"));
    const { user } = renderPage(<Onboarding hasModel={false} hasData={false} />);
    await user.click(screen.getByRole("button", { name: /Essayer avec l'exemple finance/ }));
    expect(await screen.findByText("API unreachable")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Essayer avec l'exemple finance/ })).toBeEnabled();
  });
});
