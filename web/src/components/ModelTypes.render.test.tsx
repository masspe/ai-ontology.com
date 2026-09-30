// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// Rendering tests of the "Types du modèle" card: listing with sheet counts,
// search, edit, add, delete (blocked, confirmed, refused by the server) and
// the empty state.

import { beforeEach, describe, expect, it, vi } from "vitest";
import ModelTypes from "./ModelTypes";
import { renderPage, screen, waitFor, within } from "../test/render";
import type { Ontology } from "../api";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return { ...actual, listConcepts: vi.fn(), replaceOntology: vi.fn() };
});

import * as api from "../api";

const listConcepts = vi.mocked(api.listConcepts);
const replaceOntology = vi.mocked(api.replaceOntology);

const ontology: Ontology = {
  concept_types: {
    Chantier: { name: "Chantier", description: "un projet de construction", properties: ["adresse", "date"] },
    Entreprise: { name: "Entreprise", description: "une société" },
    Fournisseur: { name: "Fournisseur", parent: "Entreprise" },
    Note: { name: "Note", description: "un mémo" },
    Libre: { name: "Libre", disjoint_with: ["Note"] },
  },
  relation_types: {
    maitre_d_ouvrage: {
      name: "maitre_d_ouvrage",
      domain: "Chantier",
      range: "Entreprise",
      cardinality: "ManyToOne",
      symmetric: false,
      description: "commandé par",
    },
  },
};

const onChanged = vi.fn();

beforeEach(() => {
  vi.clearAllMocks();
  listConcepts.mockImplementation(async (p) => ({
    total: p?.type === "Chantier" ? 3 : 0,
    concepts: [],
    next_cursor: null,
  }));
  replaceOntology.mockImplementation(async (o) => o);
});

const mount = (o: Ontology | null = ontology) => renderPage(<ModelTypes ontology={o} onChanged={onChanged} />);
const row = (name: string) =>
  screen.getAllByRole("row").find((r) => r.querySelector("td strong")?.textContent === name)!;
const loaded = async () => {
  const r = mount();
  await waitFor(() => expect(within(row("Chantier")).getByText("3")).toBeInTheDocument());
  await waitFor(() => expect(screen.queryByText("…")).toBeNull());
  return r;
};

describe("ModelTypes", () => {
  it("lists concept types with their sheet counts and relation types on their tab", async () => {
    const { user } = await loaded();
    expect(screen.getByRole("tab", { name: "Types de fiche (5)" })).toHaveAttribute("aria-selected", "true");
    expect(listConcepts).toHaveBeenCalledWith({ type: "Chantier", limit: 1, include_subtypes: false });
    expect(listConcepts).toHaveBeenCalledTimes(5);
    expect(within(row("Chantier")).getByTitle("adresse, date")).toHaveTextContent("2");
    expect(within(row("Fournisseur")).getByText("Entreprise")).toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: "Types de lien (1)" }));
    const r = row("maitre_d_ouvrage");
    expect(r).toHaveTextContent("Chantier → Entreprise");
    expect(within(r).getByText("ManyToOne")).toHaveClass("badge");
    expect(r).toHaveTextContent("commandé par");
  });

  it("shows loading then '?' when a count cannot be made", async () => {
    listConcepts.mockRejectedValue(new Error("HTTP 429"));
    mount();
    expect(within(row("Chantier")).getByText("…")).toBeInTheDocument();
    await waitFor(() => expect(within(row("Chantier")).getByText("?")).toBeInTheDocument());
    // Unknown count: the server decides, the button stays usable.
    expect(within(row("Note")).getByRole("button", { name: "Supprimer" })).toBeEnabled();
  });

  it("filters by name or description", async () => {
    const { user } = await loaded();
    const search = screen.getByPlaceholderText("Rechercher par nom ou description");
    await user.type(search, "société");
    expect(screen.getAllByRole("row")).toHaveLength(2);
    expect(row("Entreprise")).toBeInTheDocument();
    await user.clear(search);
    await user.type(search, "zzz");
    expect(screen.getByText("Aucun type ne correspond à « zzz ».")).toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: /Types de lien/ }));
    expect(screen.getByText("Aucun type ne correspond à « zzz ».")).toBeInTheDocument();
  });

  it("edits a concept type and saves the whole new model", async () => {
    const { user } = await loaded();
    await user.click(within(row("Note")).getByRole("button", { name: "Modifier" }));
    expect(screen.getByLabelText(/^Nom/)).toHaveAttribute("readonly");
    expect(screen.getByText(/Le nom ne peut pas être changé/)).toBeInTheDocument();
    const desc = screen.getByLabelText("Description");
    await user.clear(desc);
    await user.type(desc, "un mémo de chantier");
    await user.selectOptions(screen.getByLabelText("Parent"), "Entreprise");
    await user.type(screen.getByLabelText(/Propriétés/), "auteur, , date");
    await user.click(screen.getByRole("button", { name: "Enregistrer" }));
    await waitFor(() => expect(onChanged).toHaveBeenCalled());
    expect(replaceOntology).toHaveBeenCalledWith({
      ...ontology,
      concept_types: {
        ...ontology.concept_types,
        Note: { name: "Note", description: "un mémo de chantier", parent: "Entreprise", properties: ["auteur", "date"] },
      },
    });
    expect(screen.queryByText(/Modifier le type de fiche/)).toBeNull();
  });

  it("offers no parent that would make a cycle", async () => {
    const { user } = await loaded();
    await user.click(within(row("Entreprise")).getByRole("button", { name: "Modifier" }));
    const options = within(screen.getByLabelText("Parent")).getAllByRole("option").map((o) => o.textContent);
    expect(options).toEqual(["(aucun)", "Chantier", "Libre", "Note"]);
    await user.click(screen.getByRole("button", { name: "Annuler" }));
    expect(screen.queryByLabelText("Parent")).toBeNull();
  });

  it("edits a relation type's range", async () => {
    const { user } = await loaded();
    await user.click(screen.getByRole("tab", { name: /Types de lien/ }));
    await user.click(within(row("maitre_d_ouvrage")).getByRole("button", { name: "Modifier" }));
    await user.selectOptions(screen.getByLabelText(/^Vers/), "Fournisseur");
    await user.selectOptions(screen.getByLabelText("Cardinalité"), "OneToOne");
    await user.click(screen.getByRole("button", { name: "Enregistrer" }));
    await waitFor(() => expect(replaceOntology).toHaveBeenCalled());
    expect(replaceOntology).toHaveBeenCalledWith({
      ...ontology,
      relation_types: {
        maitre_d_ouvrage: { ...ontology.relation_types.maitre_d_ouvrage, range: "Fournisseur", cardinality: "OneToOne" },
      },
    });
  });

  it("blocks deleting a concept type with sheets, used by a relation type or parent of another", async () => {
    await loaded();
    const chantier = within(row("Chantier")).getByRole("button", { name: "Supprimer" });
    expect(chantier).toBeDisabled();
    expect(chantier).toHaveAttribute(
      "title",
      "Suppression impossible : 3 fiche(s) de ce type : supprimez-les ou changez leur type d'abord · utilisé par le(s) type(s) de lien : maitre_d_ouvrage",
    );
    const entreprise = within(row("Entreprise")).getByRole("button", { name: "Supprimer" });
    expect(entreprise).toBeDisabled();
    expect(entreprise.getAttribute("title")).toContain("parent de : Fournisseur");
    expect(within(row("Fournisseur")).getByRole("button", { name: "Supprimer" })).toBeEnabled();
  });

  it("blocks deleting a concept type that a rule type or an action type targets", async () => {
    renderPage(
      <ModelTypes
        ontology={{
          ...ontology,
          rule_types: { controle: { name: "controle", applies_to: ["Note"] } },
          action_types: { relancer: { name: "relancer", subject: "Fournisseur", object: null } },
        }}
        onChanged={onChanged}
      />,
    );
    await waitFor(() => expect(screen.queryByText("…")).toBeNull());
    const note = within(row("Note")).getByRole("button", { name: "Supprimer" });
    expect(note).toBeDisabled();
    expect(note.getAttribute("title")).toContain("visé par le(s) type(s) de règle : controle");
    const fournisseur = within(row("Fournisseur")).getByRole("button", { name: "Supprimer" });
    expect(fournisseur.getAttribute("title")).toContain("visé par le(s) type(s) d'action : relancer");
  });

  it("deletes a concept type after confirmation, and not when cancelled", async () => {
    const { user } = await loaded();
    await user.click(within(row("Note")).getByRole("button", { name: "Supprimer" }));
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Annuler" }));
    expect(replaceOntology).not.toHaveBeenCalled();

    await user.click(within(row("Note")).getByRole("button", { name: "Supprimer" }));
    expect(screen.getByRole("dialog")).toHaveTextContent("« Note » sera retiré du modèle de données.");
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Supprimer" }));
    await waitFor(() => expect(replaceOntology).toHaveBeenCalled());
    const { Note: _n, ...rest } = ontology.concept_types;
    expect(replaceOntology).toHaveBeenCalledWith({
      ...ontology,
      concept_types: { ...rest, Libre: { name: "Libre", disjoint_with: [] } },
    });
    expect(onChanged).toHaveBeenCalled();
  });

  it("shows the server's refusal when a relation type is deleted", async () => {
    replaceOntology.mockRejectedValue(new Error("des liens de ce type existent encore"));
    const { user } = await loaded();
    await user.click(screen.getByRole("tab", { name: /Types de lien/ }));
    await user.click(within(row("maitre_d_ouvrage")).getByRole("button", { name: "Supprimer" }));
    expect(screen.getByRole("dialog")).toHaveTextContent("Supprimer ce type de lien ?");
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Supprimer" }));
    expect(await screen.findByText("des liens de ce type existent encore")).toHaveClass("error-banner");
    expect(replaceOntology).toHaveBeenCalledWith({ ...ontology, relation_types: {} });
    expect(onChanged).not.toHaveBeenCalled();
  });

  it("adds a concept type, refusing an empty or taken name", async () => {
    const { user } = await loaded();
    await user.click(screen.getByRole("button", { name: "Ajouter un type de fiche" }));
    expect(screen.getByText("Nouveau type de fiche")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Enregistrer" }));
    expect(screen.getByText("Le nom est obligatoire.")).toHaveClass("error-banner");
    await user.type(screen.getByLabelText(/^Nom/), "Note");
    await user.click(screen.getByRole("button", { name: "Enregistrer" }));
    expect(screen.getByText("Le nom « Note » est déjà pris.")).toBeInTheDocument();
    await user.clear(screen.getByLabelText(/^Nom/));
    await user.type(screen.getByLabelText(/^Nom/), " Devis ");
    await user.click(screen.getByRole("button", { name: "Enregistrer" }));
    await waitFor(() => expect(replaceOntology).toHaveBeenCalled());
    expect(replaceOntology).toHaveBeenCalledWith({
      ...ontology,
      concept_types: {
        ...ontology.concept_types,
        Devis: { name: "Devis", description: "", parent: null, properties: null },
      },
    });
  });

  it("adds a relation type between existing concept types", async () => {
    const { user } = await loaded();
    await user.click(screen.getByRole("tab", { name: /Types de lien/ }));
    await user.click(screen.getByRole("button", { name: "Ajouter un type de lien" }));
    expect(screen.getByText("Nouveau type de lien")).toBeInTheDocument();
    await user.type(screen.getByLabelText(/^Nom/), "fournit");
    await user.type(screen.getByLabelText("Description"), "livre du matériel");
    await user.selectOptions(screen.getByLabelText("De (type de fiche)"), "Fournisseur");
    await user.click(screen.getByRole("button", { name: "Enregistrer" }));
    await waitFor(() => expect(replaceOntology).toHaveBeenCalled());
    expect(replaceOntology).toHaveBeenCalledWith({
      ...ontology,
      relation_types: {
        ...ontology.relation_types,
        fournit: {
          name: "fournit",
          domain: "Fournisseur",
          range: "Chantier",
          cardinality: "ManyToMany",
          description: "livre du matériel",
          symmetric: false,
        },
      },
    });
  });

  it("shows the empty state, and no relation type can be added without concept types", async () => {
    const { user } = mount({ concept_types: {}, relation_types: {} });
    expect(
      screen.getByText(
        "Aucun type pour l'instant : installez un modèle prêt à l'emploi depuis l'accueil, ou générez-en un ci-dessous.",
      ),
    ).toBeInTheDocument();
    expect(listConcepts).not.toHaveBeenCalled();
    await user.click(screen.getByRole("tab", { name: "Types de lien (0)" }));
    expect(screen.getByRole("button", { name: "Ajouter un type de lien" })).toBeDisabled();
  });

  it("renders nothing while the model is not loaded", () => {
    mount(null);
    expect(screen.queryByText("Types du modèle")).toBeNull();
  });
});
