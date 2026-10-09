// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The sheet of one concept: what it is, its
// links both ways with a link to each neighbour's sheet, its structured
// information, the documents it came from, the rules and actions that
// concern it, and the correction made in place.

import { beforeEach, describe, expect, it, vi } from "vitest";
import { Route } from "react-router-dom";
import ConceptSheet from "./ConceptSheet";
import { renderPage, screen, waitFor } from "../test/render";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return {
    ...actual,
    getConcept: vi.fn(),
    getSubgraph: vi.fn(),
    listRules: vi.fn(),
    listActions: vi.fn(),
    getFiles: vi.fn(),
    updateConcept: vi.fn(),
  };
});

import * as api from "../api";

const mocked = api as unknown as Record<string, ReturnType<typeof vi.fn>>;

const acme = {
  id: 7,
  concept_type: "Entreprise",
  name: "Acme SA",
  description: "Sous-traitant électricité",
  properties: { ville: "Genève", montant: 1200.5, tags: ["a", "b"], source_file: "contrat.pdf", lang: "fr" },
};

beforeEach(() => {
  mocked.getConcept!.mockResolvedValue(acme);
  mocked.getSubgraph!.mockResolvedValue({
    subgraph: {
      concepts: [acme, { id: 1, concept_type: "Chantier", name: "Rue du Lac" }, { id: 2, concept_type: "Personne", name: "Ada" }],
      relations: [
        { id: 10, relation_type: "sous_traitant", source: 1, target: 7 },
        { id: 11, relation_type: "emploie", source: 7, target: 2 },
        { id: 12, relation_type: "autre", source: 1, target: 2 },
        { id: 13, relation_type: "emploie", source: 7, target: 99 },
      ],
    },
  });
  mocked.listRules!.mockResolvedValue([
    { id: 1, rule_type: "check", name: "règle générale", applies_to: [], strict: true },
    { id: 2, rule_type: "check", name: "règle sur Acme", applies_to: [7], strict: false },
    { id: 3, rule_type: "check", name: "ailleurs", applies_to: [1], strict: false },
  ]);
  mocked.listActions!.mockResolvedValue([
    { id: 1, action_type: "mail", name: "relancer", subject: 7, object: null },
    { id: 2, action_type: "mail", name: "autre", subject: 1, object: 2 },
  ]);
  mocked.getFiles!.mockResolvedValue({
    files: [
      { id: 1, name: "contrat.pdf", size: 1, kind: "pdf", status: "processed", uploaded_at: 1, concepts: 5, relations: 2 },
      { id: 2, name: "autre.pdf", size: 1, kind: "pdf", status: "processed", uploaded_at: 1, concepts: 1, relations: 0 },
    ],
  });
});

function mount(id = 7) {
  return renderPage(<ConceptSheet />, { route: `/concepts/${id}`, path: "/concepts/:id" });
}

describe("ConceptSheet", () => {
  it("shows the concept, its links both ways, its information, its origin, its rules and actions", async () => {
    mount();
    expect(await screen.findByRole("heading", { name: "Acme SA" })).toBeInTheDocument();
    expect(mocked.getSubgraph).toHaveBeenCalledWith({ seed_concept_ids: [7], expansion_depth: 1, limit: 200 });
    expect(screen.getByText("Sous-traitant électricité")).toBeInTheDocument();
    expect(screen.getByText("Liens (3)")).toBeInTheDocument();
    // Outgoing first, then incoming; a neighbour outside the subgraph keeps a link.
    expect(screen.getByRole("link", { name: /Ada/ })).toHaveAttribute("href", "/concepts/2");
    expect(screen.getByRole("link", { name: /Rue du Lac/ })).toHaveAttribute("href", "/concepts/1");
    expect(screen.getByRole("link", { name: "fiche 99" })).toHaveAttribute("href", "/concepts/99");
    expect(screen.queryByText("autre")).toBeNull();
    // Structured information, without the bookkeeping properties.
    expect(screen.getByText("Informations (3)")).toBeInTheDocument();
    expect(screen.getByText("Genève")).toBeInTheDocument();
    expect(screen.getByText("1200.5")).toBeInTheDocument();
    expect(screen.getByText("a, b")).toBeInTheDocument();
    expect(screen.queryByText("source_file")).toBeNull();
    // Origin: the file it was imported from.
    expect(screen.getByRole("link", { name: "contrat.pdf" })).toHaveAttribute("href", "/files");
    expect(screen.queryByText("autre.pdf")).toBeNull();
    // Rules: the general one and the one scoped to it; actions on it.
    expect(screen.getByText("Règles et actions (3)")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "règle générale" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "règle sur Acme" })).toBeInTheDocument();
    expect(screen.queryByText("ailleurs")).toBeNull();
    expect(screen.getByRole("link", { name: "relancer" })).toHaveAttribute("href", "/actions");
    expect(screen.getByText("stricte")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Voir dans le graphe" })).toHaveAttribute("href", "/graph?focus=7");
  });

  it("corrects the name and the description in place", async () => {
    mocked.updateConcept!.mockResolvedValue({ ...acme, name: "Acme Suisse SA", description: "" });
    const { user } = mount();
    await screen.findByRole("heading", { name: "Acme SA" });
    await user.click(screen.getByRole("button", { name: "Corriger" }));
    const name = screen.getByRole("textbox", { name: "Nom" });
    await user.clear(name);
    await user.type(name, "Acme Suisse SA");
    await user.clear(screen.getByRole("textbox", { name: "Description" }));
    await user.click(screen.getByRole("button", { name: "Enregistrer" }));
    await waitFor(() => expect(screen.getByRole("heading", { name: "Acme Suisse SA" })).toBeInTheDocument());
    expect(mocked.updateConcept).toHaveBeenCalledWith(7, { name: "Acme Suisse SA", description: "" });
    expect(screen.getByText("Pas de description.")).toBeInTheDocument();
    // Cancel leaves everything as it was.
    await user.click(screen.getByRole("button", { name: "Corriger" }));
    await user.click(screen.getByRole("button", { name: "Annuler" }));
    expect(screen.getByRole("heading", { name: "Acme Suisse SA" })).toBeInTheDocument();
  });

  it("shows the save error and keeps the form", async () => {
    mocked.updateConcept!.mockRejectedValue(new Error("name taken"));
    const { user } = mount();
    await screen.findByRole("heading", { name: "Acme SA" });
    await user.click(screen.getByRole("button", { name: "Corriger" }));
    await user.click(screen.getByRole("button", { name: "Enregistrer" }));
    expect(await screen.findByText("name taken")).toHaveClass("error-banner");
    expect(screen.getByRole("textbox", { name: "Nom" })).toBeInTheDocument();
  });

  it("shows the empty states and the unknown origin", async () => {
    mocked.getConcept!.mockResolvedValue({ id: 8, concept_type: "Personne", name: "Ada", properties: {} });
    mocked.getSubgraph!.mockResolvedValue({ subgraph: { concepts: [], relations: [] } });
    mocked.listRules!.mockRejectedValue(new Error("no rules"));
    mocked.listActions!.mockRejectedValue(new Error("no actions"));
    mocked.getFiles!.mockRejectedValue(new Error("no files"));
    mount(8);
    expect(await screen.findByRole("heading", { name: "Ada" })).toBeInTheDocument();
    expect(screen.getByText(/Aucun lien pour l'instant/)).toBeInTheDocument();
    expect(screen.getByText("Aucune information structurée.")).toBeInTheDocument();
    expect(screen.getByText(/Origine inconnue/)).toBeInTheDocument();
    expect(screen.getByText(/Aucune règle ni action/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Définir une règle" })).toHaveAttribute("href", "/rules");
  });

  it("names the source file when the file record is gone, and reports a missing concept", async () => {
    mocked.getFiles!.mockResolvedValue({ files: [] });
    mount();
    expect(await screen.findByText("Importée depuis contrat.pdf.")).toBeInTheDocument();

    mocked.getConcept!.mockRejectedValue(new Error("not found: concept 404"));
    renderPage(<ConceptSheet />, {
      route: "/concepts/404",
      path: "/concepts/:id",
      extraRoutes: <Route path="/concepts" element={<div>liste</div>} />,
    });
    expect(await screen.findByText("not found: concept 404")).toHaveClass("error-banner");
    expect(screen.getByRole("link", { name: "Retour aux fiches" })).toHaveAttribute("href", "/concepts");
  });
});

describe("ConceptSheet — guards", () => {
  it("refuses a route id that is not a number without calling the API", async () => {
    mount("abc" as unknown as number);
    expect(await screen.findByText("Fiche inconnue : abc")).toHaveClass("error-banner");
    expect(mocked.getConcept).not.toHaveBeenCalled();
  });

  it("lists a self-loop once", async () => {
    mocked.getSubgraph!.mockResolvedValue({
      subgraph: { concepts: [acme], relations: [{ id: 20, relation_type: "boucle", source: 7, target: 7 }] },
    });
    mount();
    await screen.findByRole("heading", { name: "Acme SA" });
    expect(screen.getByText("Liens (1)")).toBeInTheDocument();
    expect(screen.getAllByText("boucle")).toHaveLength(1);
  });
});
