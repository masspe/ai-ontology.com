// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The Data sources page: sources listed with their mappings, a source
// added and tested, a mapping added with its link rule, previewed and
// synced, the errors shown.

import { beforeEach, describe, expect, it, vi } from "vitest";
import Sources from "./Sources";
import { fireEvent, renderPage, screen, waitFor } from "../test/render";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return {
    ...actual,
    listSources: vi.fn(),
    createSource: vi.fn(),
    updateSource: vi.fn(),
    deleteSource: vi.fn(),
    testSource: vi.fn(),
    listSourceTables: vi.fn(),
    listSourceColumns: vi.fn(),
    createMapping: vi.fn(),
    updateMapping: vi.fn(),
    deleteMapping: vi.fn(),
    previewMapping: vi.fn(),
    runMapping: vi.fn(),
    createNamedQuery: vi.fn(),
    updateNamedQuery: vi.fn(),
    deleteNamedQuery: vi.fn(),
    tryNamedQuery: vi.fn(),
  };
});
import * as api from "../api";
const m = api as unknown as Record<string, ReturnType<typeof vi.fn>>;

const source = {
  id: 1, name: "erp", kind: "sqlserver", location: "", host: "db.example", port: 1433, database: "erp", user: "ro",
  password_hint: "s3c...pass", encrypt: true, trust_cert: false, allow_private_hosts: false, timeout_secs: 60, created_at: "2026-11-20T10:00:00Z",
  mappings: [
    { id: 1, name: "orders", from: "dbo.orders", concept_type: "Order", key_column: "id", property_columns: [], relations: [], schedule_minutes: 30,
      last_run_at: "2026-11-20T10:30:00Z", last_status: "ok", last_report: { rows: 42 } },
  ],
  queries: [
    { id: 1, name: "ticket", description: "", trigger: "ticket (?P<id>\\d+)", sql: "SELECT * FROM tickets WHERE id = {id}", max_rows: 20, admin_only: true, enabled: false },
  ],
};

beforeEach(() => {
  m.listSources.mockResolvedValue({ sources: [source] });
  m.listSourceTables.mockResolvedValue({ tables: [{ schema: "dbo", name: "orders" }, { name: "customers" }] });
  m.listSourceColumns.mockResolvedValue({ columns: [{ name: "id", data_type: "int" }, { name: "label", data_type: "text" }] });
});

describe("Sources page", () => {
  it("lists the sources with their mappings and tests a connection", async () => {
    m.testSource.mockResolvedValue({ ok: true, ms: 12 });
    renderPage(<Sources />);
    expect(await screen.findByText("erp")).toBeInTheDocument();
    expect(screen.getByText("db.example:1433/erp")).toBeInTheDocument();
    expect(screen.getByText("orders")).toBeInTheDocument();
    expect(screen.getByText(/toutes les 30 min/)).toBeInTheDocument();
    expect(screen.getByText(/42 lignes/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Tester" }));
    expect(await screen.findByText("Connexion réussie.")).toBeInTheDocument();
    m.testSource.mockResolvedValue({ ok: false, error: "login failed" });
    fireEvent.click(screen.getByRole("button", { name: "Tester" }));
    expect(await screen.findByText("login failed")).toBeInTheDocument();
  });

  it("adds a source and a mapping with a link rule, previews and syncs", async () => {
    m.createSource.mockResolvedValue({ ...source, id: 2, name: "files" });
    m.createMapping.mockResolvedValue({});
    m.previewMapping.mockResolvedValue({ columns: [{ name: "id", data_type: "int" }], rows: [[1], [2]], sheets: [{ name: "1", description: "", properties: {} }], links: 0, skipped: 1 });
    m.runMapping.mockResolvedValue({ job: 7 });
    renderPage(<Sources />);
    await screen.findByText("erp");

    fireEvent.click(screen.getByRole("button", { name: "Ajouter une source" }));
    fireEvent.change(screen.getByLabelText("Nom de la source"), { target: { value: "files" } });
    fireEvent.change(screen.getByLabelText("Type"), { target: { value: "csv" } });
    fireEvent.change(screen.getByLabelText("Chemin ou URL HTTPS du fichier"), { target: { value: "/data/x.csv" } });
    fireEvent.click(screen.getByRole("button", { name: "Enregistrer" }));
    await waitFor(() => expect(m.createSource).toHaveBeenCalledWith(expect.objectContaining({ name: "files", kind: "csv", location: "/data/x.csv" })));
    expect(await screen.findByText("Source enregistrée.")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Ajouter un mapping" }));
    expect(await screen.findByLabelText("Nom du mapping")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Nom du mapping"), { target: { value: "customers" } });
    fireEvent.change(screen.getByLabelText("Table ou SELECT"), { target: { value: "customers" } });
    fireEvent.change(screen.getByLabelText("Type de fiche"), { target: { value: "Customer" } });
    fireEvent.change(screen.getByLabelText("Colonne clé"), { target: { value: "id" } });
    fireEvent.change(screen.getByLabelText("Colonnes gardées en propriétés (vide : toutes)"), { target: { value: "label, city" } });
    fireEvent.click(screen.getByRole("button", { name: "Ajouter un lien" }));
    fireEvent.change(screen.getByLabelText("Colonne"), { target: { value: "rep" } });
    fireEvent.change(screen.getByLabelText("Type de lien"), { target: { value: "managed_by" } });
    fireEvent.change(screen.getByLabelText("Type cible"), { target: { value: "Employee" } });
    fireEvent.click(screen.getByRole("button", { name: "Enregistrer" }));
    await waitFor(() =>
      expect(m.createMapping).toHaveBeenCalledWith(
        1,
        expect.objectContaining({
          name: "customers",
          from: "customers",
          concept_type: "Customer",
          key_column: "id",
          property_columns: ["label", "city"],
          relations: [{ column: "rep", relation_type: "managed_by", target_type: "Employee", create_target: true }],
          name_column: null,
        }),
      ),
    );

    fireEvent.click(screen.getByRole("button", { name: "Aperçu" }));
    expect(await screen.findByText("Aperçu : orders")).toBeInTheDocument();
    expect(screen.getByText(/1 fiche\(s\), 0 lien\(s\), 1 ligne\(s\) sans clé/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Synchroniser" }));
    await waitFor(() => expect(m.runMapping).toHaveBeenCalledWith(1, 1));
    expect(await screen.findByText(/Synchronisation lancée/)).toBeInTheDocument();
  });

  it("lists, adds, edits and tries a named query", async () => {
    m.createNamedQuery.mockResolvedValue({});
    m.updateNamedQuery.mockResolvedValue({});
    m.tryNamedQuery.mockResolvedValueOnce({ matched: false }).mockResolvedValueOnce({ matched: true, sql: "SELECT * FROM tickets WHERE id = '12'", context: "id | title\n12 | Printer" });
    renderPage(<Sources />);
    await screen.findByText("erp");
    expect(screen.getByText("ticket")).toBeInTheDocument();
    expect(screen.getByText("désactivée", { exact: false })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Ajouter une requête" }));
    fireEvent.change(screen.getByLabelText("Nom de la requête"), { target: { value: "client" } });
    fireEvent.change(screen.getByLabelText("Déclencheur (expression régulière, groupes nommés)"), { target: { value: "client (?P<name>\\w+)" } });
    fireEvent.change(screen.getByLabelText("SELECT avec des blancs : le nom du groupe entre accolades"), { target: { value: "SELECT * FROM c WHERE name = {name}" } });
    fireEvent.click(screen.getByLabelText("Administrateurs seulement"));
    fireEvent.click(screen.getByRole("button", { name: "Enregistrer" }));
    await waitFor(() => expect(m.createNamedQuery).toHaveBeenCalledWith(1, expect.objectContaining({ name: "client", trigger: "client (?P<name>\\w+)", sql: "SELECT * FROM c WHERE name = {name}", admin_only: true, enabled: true, max_rows: 20 })));
    expect(await screen.findByText("Requête enregistrée.")).toBeInTheDocument();

    fireEvent.click(screen.getAllByRole("button", { name: "Modifier" })[2]);
    fireEvent.click(screen.getByLabelText("Activée"));
    fireEvent.click(screen.getByRole("button", { name: "Enregistrer" }));
    await waitFor(() => expect(m.updateNamedQuery).toHaveBeenCalledWith(1, 1, expect.objectContaining({ name: "ticket", enabled: true })));

    fireEvent.click(screen.getByRole("button", { name: "Essayer" }));
    expect(await screen.findByText("Essayer : ticket")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Question"), { target: { value: "rien" } });
    fireEvent.submit(screen.getByLabelText("Question").closest("form")!);
    expect(await screen.findByText("Le déclencheur ne correspond pas à cette question.")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Question"), { target: { value: "ticket 12" } });
    fireEvent.submit(screen.getByLabelText("Question").closest("form")!);
    expect(await screen.findByText("SELECT * FROM tickets WHERE id = '12'")).toBeInTheDocument();
    expect(screen.getByText(/12 \| Printer/)).toBeInTheDocument();
    await waitFor(() => expect(m.tryNamedQuery).toHaveBeenLastCalledWith(1, 1, "ticket 12"));
  });

  it("shows the listing error and the empty state", async () => {
    m.listSources.mockRejectedValueOnce(new Error("data sources are for administrators"));
    const { unmount } = renderPage(<Sources />);
    expect(await screen.findByText("data sources are for administrators")).toBeInTheDocument();
    unmount();
    m.listSources.mockResolvedValue({ sources: [] });
    renderPage(<Sources />);
    expect(await screen.findByText("Aucune source pour l'instant.")).toBeInTheDocument();
  });
});
