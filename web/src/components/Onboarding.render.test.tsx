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
  return { ...actual, upload: vi.fn(), replaceOntology: vi.fn() };
});

import * as api from "../api";

const mocked = api as unknown as { upload: ReturnType<typeof vi.fn>; replaceOntology: ReturnType<typeof vi.fn> };

beforeEach(() => {
  vi.stubGlobal("fetch", vi.fn(async () => ({ blob: async () => new Blob(["x"]) })));
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

describe("Onboarding — ready-made models", () => {
  it("installs the chosen model's schema and tells the page to refresh", async () => {
    mocked.replaceOntology.mockResolvedValue(undefined);
    const onLoaded = vi.fn();
    const { user } = renderPage(<Onboarding hasModel={false} hasData={false} onLoaded={onLoaded} />);
    const select = screen.getByRole("combobox", { name: "Modèle prêt à l'emploi" });
    expect(select).toHaveDisplayValue("Modèle prêt à l'emploi…");
    await user.selectOptions(select, "chantiers");
    await waitFor(() => expect(onLoaded).toHaveBeenCalledTimes(1));
    const schema = mocked.replaceOntology.mock.calls[0]![0];
    expect(Object.keys(schema.concept_types)).toContain("Chantier");
    expect(schema.relation_types.sous_traitant.domain).toBe("Chantier");
    // The finance model is the example's own schema.
    await user.selectOptions(select, "finance");
    await waitFor(() => expect(mocked.replaceOntology).toHaveBeenCalledTimes(2));
    expect(Object.keys(mocked.replaceOntology.mock.calls[1]![0].concept_types)).toContain("Invoice");
  });

  it("shows the API error when the model cannot be installed", async () => {
    mocked.replaceOntology.mockRejectedValue(new Error("schema refused"));
    const { user } = renderPage(<Onboarding hasModel={false} hasData={false} />);
    await user.selectOptions(screen.getByRole("combobox", { name: "Modèle prêt à l'emploi" }), "personnes");
    expect(await screen.findByText("schema refused")).toBeInTheDocument();
  });
});
