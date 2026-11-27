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
    listSourceIndexes: vi.fn(),
    createMapping: vi.fn(),
    updateMapping: vi.fn(),
    deleteMapping: vi.fn(),
    previewMapping: vi.fn(),
    runMapping: vi.fn(),
    createNamedQuery: vi.fn(),
    updateNamedQuery: vi.fn(),
    deleteNamedQuery: vi.fn(),
    tryNamedQuery: vi.fn(),
    odbcCatalog: vi.fn(),
    uploadSourceFile: vi.fn(),
  };
});
import * as api from "../api";
const m = api as unknown as Record<string, ReturnType<typeof vi.fn>>;

const source = {
  id: 1, name: "erp", kind: "sqlserver", location: "", host: "db.example", port: 1433, database: "erp", user: "ro",
  password_hint: "••••••••", encrypt: true, trust_cert: false, allow_private_hosts: false, timeout_secs: 60, created_at: "2026-11-20T10:00:00Z",
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
  m.listSourceIndexes.mockResolvedValue({ indexes: [] });
  m.odbcCatalog.mockResolvedValue({ available: false, drivers: [], dsns: [] });
});

describe("Sources page", () => {
  it("lists the sources with their mappings and tests a connection", async () => {
    m.testSource.mockResolvedValue({ ok: true, ms: 12 });
    renderPage(<Sources />);
    expect(await screen.findByText("erp")).toBeInTheDocument();
    expect(screen.getByText("db.example:1433/erp")).toBeInTheDocument();
    expect(screen.getByText("orders")).toBeInTheDocument();
    expect(screen.getByText(/toutes les 30 min/)).toBeInTheDocument();
    expect(screen.getByText("OK")).toHaveClass("badge-success");
    expect(screen.getByText(/42 lignes lues : 0 fiches créées/)).toBeInTheDocument();
    // While the engine is asked, the button says so and waits; the result
    // takes its place, beside the button.
    let answer: (v: { ok: boolean; ms?: number; error?: string }) => void = () => {};
    m.testSource.mockImplementationOnce(() => new Promise((r) => { answer = r; }));
    fireEvent.click(screen.getByRole("button", { name: "Tester" }));
    expect(await screen.findByText("Test en cours… (jusqu'à 60 s)")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Tester" })).toBeDisabled();
    answer({ ok: true, ms: 12 });
    expect(await screen.findByText("Connexion réussie (12 ms)")).toHaveClass("badge-success");
    m.testSource.mockResolvedValue({ ok: false, error: "login failed" });
    fireEvent.click(screen.getByRole("button", { name: "Tester" }));
    expect(await screen.findByText("Échec : login failed")).toHaveClass("badge-danger");
    m.testSource.mockRejectedValue(new Error("API injoignable"));
    fireEvent.click(screen.getByRole("button", { name: "Tester" }));
    expect(await screen.findByText("Échec : API injoignable")).toBeInTheDocument();
  });

  it("adds a database by host, the port following the engine", async () => {
    m.createSource.mockResolvedValue({ ...source, id: 3, name: "pg", kind: "postgres", port: 5432 });
    renderPage(<Sources />);
    await screen.findByText("erp");
    fireEvent.click(screen.getByRole("button", { name: "Ajouter une source" }));
    expect(screen.queryByLabelText("Host")).toBeNull();
    fireEvent.change(screen.getByLabelText("Source name"), { target: { value: "pg" } });
    fireEvent.change(screen.getByLabelText("Type"), { target: { value: "mysql" } });
    expect(screen.getByLabelText("Port")).toHaveValue(3306);
    fireEvent.change(screen.getByLabelText("Type"), { target: { value: "postgres" } });
    expect(screen.getByLabelText("Port")).toHaveValue(5432);
    fireEvent.change(screen.getByLabelText("Host"), { target: { value: "pg.example" } });
    fireEvent.change(screen.getByLabelText("Type"), { target: { value: "sqlserver" } });
    expect(screen.getByLabelText("Port")).toHaveValue(1433);
    fireEvent.change(screen.getByLabelText("Type"), { target: { value: "postgres" } });
    fireEvent.click(screen.getByRole("button", { name: "Enregistrer" }));
    await waitFor(() => expect(m.createSource).toHaveBeenCalledWith(expect.objectContaining({ name: "pg", kind: "postgres", host: "pg.example", port: 5432, encrypt: true })));
  });

  it("adds a source and a mapping with a link rule, previews and syncs", async () => {
    m.createSource.mockResolvedValue({ ...source, id: 2, name: "files" });
    m.createMapping.mockResolvedValue({});
    m.previewMapping.mockResolvedValue({ columns: [{ name: "id", data_type: "int" }], rows: [[1], [2]], sheets: [{ name: "1", description: "", properties: {} }], links: 0, skipped: 1 });
    m.runMapping.mockResolvedValue({ job: 7 });
    renderPage(<Sources />);
    await screen.findByText("erp");

    fireEvent.click(screen.getByRole("button", { name: "Ajouter une source" }));
    fireEvent.change(screen.getByLabelText("Source name"), { target: { value: "files" } });
    fireEvent.change(screen.getByLabelText("Type"), { target: { value: "csv" } });
    fireEvent.change(screen.getByLabelText("File path or HTTPS URL"), { target: { value: "/data/x.csv" } });
    fireEvent.click(screen.getByRole("button", { name: "Enregistrer" }));
    await waitFor(() => expect(m.createSource).toHaveBeenCalledWith(expect.objectContaining({ name: "files", kind: "csv", location: "/data/x.csv" })));
    expect(await screen.findByText("Source enregistrée.")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Ajouter un mapping" }));
    expect(await screen.findByLabelText("Mapping name")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Mapping name"), { target: { value: "customers" } });
    fireEvent.change(screen.getByLabelText("Table or SELECT"), { target: { value: "customers" } });
    fireEvent.change(screen.getByLabelText("Sheet type"), { target: { value: "Customer" } });
    // Columns are picked one by one: Enter adds, a comma adds, × takes back.
    const key = () => screen.getByRole("group", { name: "Key column(s)" }).querySelector("input")!;
    fireEvent.change(key(), { target: { value: "tbl" } });
    fireEvent.keyDown(key(), { key: "Enter" });
    fireEvent.change(key(), { target: { value: "id" } });
    fireEvent.keyDown(key(), { key: "Enter" });
    fireEvent.click(screen.getByRole("button", { name: "Retirer tbl" }));
    const props = () => screen.getByRole("group", { name: "Property columns (empty: all)" }).querySelector("input")!;
    fireEvent.change(props(), { target: { value: "label," } });
    fireEvent.change(props(), { target: { value: "city" } });
    fireEvent.blur(props());
    // A column twice is once; Backspace on an empty field takes the last back.
    fireEvent.change(props(), { target: { value: "city" } });
    fireEvent.keyDown(props(), { key: "Enter" });
    fireEvent.change(props(), { target: { value: "zip" } });
    fireEvent.keyDown(props(), { key: "Enter" });
    fireEvent.keyDown(props(), { key: "Backspace" });
    expect(screen.getByRole("group", { name: "Property columns (empty: all)" })).toHaveTextContent("label×city×");
    fireEvent.click(screen.getByRole("button", { name: "Add link" }));
    fireEvent.change(screen.getByLabelText("Column"), { target: { value: "rep" } });
    fireEvent.change(screen.getByLabelText("Link type"), { target: { value: "managed_by" } });
    fireEvent.change(screen.getByLabelText("Target type"), { target: { value: "Employee" } });
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
    fireEvent.change(screen.getByLabelText("Query name"), { target: { value: "client" } });
    fireEvent.change(screen.getByLabelText("Trigger (regex with named groups)"), { target: { value: "client (?P<name>\\w+)" } });
    fireEvent.change(screen.getByLabelText("SELECT with {group} blanks"), { target: { value: "SELECT * FROM c WHERE name = {name}" } });
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

  it("says what a failed or stopped sync leaves behind", async () => {
    m.listSources.mockResolvedValue({
      sources: [{
        ...source,
        mappings: [
          { ...source.mappings[0], id: 1, name: "bad", last_status: "failed: graph: duplicate concept name `X` for type `T`", last_report: null },
          { ...source.mappings[0], id: 2, name: "halted", last_status: "cancelled", last_report: null },
          { ...source.mappings[0], id: 3, name: "partial", last_status: "ok", last_report: { rows: 10, sheets_new: 7, sheets_updated: 1, unchanged: 0, skipped: 2, links: 3, duplicates: 4 } },
        ],
      }],
    });
    renderPage(<Sources />);
    expect(await screen.findByText("Échec")).toHaveClass("badge-danger");
    expect(screen.getByRole("alert")).toHaveTextContent("graph: duplicate concept name `X` for type `T`");
    expect(screen.getByText(/lots de 2 000 lignes écrits avant l'échec sont gardés/)).toBeInTheDocument();
    expect(screen.getByText("Arrêtée")).toHaveClass("badge-warn");
    expect(screen.getByText(/10 lignes lues : 7 fiches créées, 1 mises à jour, 0 inchangées, 3 liens · 2 lignes sans clé ignorées · 4 doublons fusionnés/)).toBeInTheDocument();
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

  it("edits a source field by field, an ODBC string, and gives up without saving", async () => {
    m.updateSource.mockResolvedValue({});
    renderPage(<Sources />);
    await screen.findByText("erp");
    const edit = () => fireEvent.click(screen.getAllByRole("button", { name: "Modifier" })[0]);
    edit();
    fireEvent.change(screen.getByLabelText("Host"), { target: { value: "db2.example" } });
    fireEvent.change(screen.getByLabelText("Port"), { target: { value: "1444" } });
    fireEvent.change(screen.getByLabelText("Database"), { target: { value: "erp2" } });
    fireEvent.change(screen.getByLabelText("User (read-only)"), { target: { value: "reader" } });
    // The stored password is never shown, only that one is there; what is
    // typed is hidden until the eye.
    expect(screen.getByLabelText("Password")).toHaveAttribute("placeholder", "••••••••");
    expect(screen.getByLabelText("Password")).toHaveAttribute("type", "password");
    fireEvent.change(screen.getByLabelText("Password"), { target: { value: "n3w" } });
    fireEvent.click(screen.getByRole("button", { name: "Afficher" }));
    expect(screen.getByLabelText("Password")).toHaveAttribute("type", "text");
    expect(screen.getByLabelText("Password")).toHaveValue("n3w");
    fireEvent.click(screen.getByRole("button", { name: "Masquer" }));
    expect(screen.getByLabelText("Password")).toHaveAttribute("type", "password");
    fireEvent.click(screen.getByLabelText("Encrypt (TLS)"));
    fireEvent.click(screen.getByLabelText("Trust server certificate"));
    fireEvent.click(screen.getByLabelText("Allow private hosts (the server is on the local network)"));
    fireEvent.change(screen.getByLabelText("Timeout (seconds)"), { target: { value: "30" } });
    fireEvent.click(screen.getByRole("button", { name: "Enregistrer" }));
    await waitFor(() =>
      expect(m.updateSource).toHaveBeenCalledWith(
        1,
        expect.objectContaining({ host: "db2.example", port: 1444, database: "erp2", user: "reader", password: "n3w", encrypt: false, trust_cert: true, allow_private_hosts: true, timeout_secs: 30 }),
      ),
    );
    await waitFor(() => expect(screen.queryByLabelText("Source name")).toBeNull());

    edit();
    fireEvent.change(screen.getByLabelText("Type"), { target: { value: "odbc" } });
    expect(screen.queryByLabelText("Host")).toBeNull();
    fireEvent.change(screen.getByLabelText("Connection string"), { target: { value: "Driver={x};Server=db.example;" } });
    expect(screen.getByLabelText("Connection string")).toHaveValue("Driver={x};Server=db.example;");
    fireEvent.click(screen.getByRole("button", { name: "Annuler" }));
    expect(screen.queryByLabelText("Source name")).toBeNull();
    // The same button closes the form it opened.
    edit();
    expect(screen.getByLabelText("Source name")).toBeInTheDocument();
    edit();
    expect(screen.queryByLabelText("Source name")).toBeNull();
    expect(m.updateSource).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole("button", { name: "Ajouter une source" }));
    expect(screen.getByText("Nouvelle source")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Annuler" }));
    expect(screen.queryByText("Nouvelle source")).toBeNull();
    expect(m.createSource).not.toHaveBeenCalled();
  });

  it("connects ODBC by a data source name or a driver, as the ODBC administrator does", async () => {
    m.odbcCatalog.mockResolvedValue({ available: true, drivers: ["Progress OpenEdge 12.2 Driver"], dsns: [{ name: "erp-live", driver: "Progress OpenEdge 12.2 Driver" }] });
    m.listSources.mockResolvedValue({
      sources: [
        { ...source, id: 4, name: "by-dsn", kind: "odbc", dsn: "erp-live", driver: "", mappings: [], queries: [] },
        { ...source, id: 5, name: "by-driver", kind: "odbc", dsn: "", driver: "OE", host: "h", port: 20931, database: "d", mappings: [], queries: [] },
      ],
    });
    m.createSource.mockResolvedValue({});
    const { container } = renderPage(<Sources />);
    expect(await screen.findByText("DSN erp-live")).toBeInTheDocument();
    expect(screen.getByText("OE · h:20931/d")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Ajouter une source" }));
    fireEvent.change(screen.getByLabelText("Source name"), { target: { value: "erp" } });
    fireEvent.change(screen.getByLabelText("Type"), { target: { value: "odbc" } });
    await waitFor(() => expect(container.querySelector('#odbc-dsns option[value="erp-live"]')).not.toBeNull());
    expect(container.querySelector('#odbc-drivers option[value="Progress OpenEdge 12.2 Driver"]')).not.toBeNull();
    // The Windows ODBC administrator's names, in its order.
    const want = ["Data Source Name", "Driver", "Host Name", "Port Number", "Database Name", "User ID", "Password"];
    const labels = () => [...container.querySelectorAll(".source-form label > span, .source-form label[for]")].map((x) => x.textContent ?? "").filter((x) => want.includes(x));
    expect(labels()).toEqual(want);
    expect(screen.getByLabelText("Port Number")).toHaveValue(null);
    expect(screen.getByLabelText("Data Source Name")).toHaveAttribute("placeholder", "erp-live");
    // A Data Source Name carries the driver, host, port and database: only the account is left.
    fireEvent.change(screen.getByLabelText("Data Source Name"), { target: { value: "erp-live" } });
    expect(labels()).toEqual(["Data Source Name", "User ID", "Password"]);
    fireEvent.change(screen.getByLabelText("User ID"), { target: { value: "reader" } });
    fireEvent.change(screen.getByLabelText("Password"), { target: { value: "pw" } });
    // No TLS switches: the driver or the DSN says how it connects.
    expect(screen.queryByLabelText("Encrypt (TLS)")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Enregistrer" }));
    await waitFor(() =>
      expect(m.createSource).toHaveBeenCalledWith(expect.objectContaining({ kind: "odbc", dsn: "erp-live", user: "reader", password: "pw", port: 0 })),
    );
    // A blank connection string is not sent: the stored one is kept.
    expect(m.createSource.mock.calls[0]![0].location).toBeUndefined();
  });

  it("says when the server lists no ODBC driver, or cannot be asked", async () => {
    m.odbcCatalog.mockRejectedValueOnce(new Error("down"));
    renderPage(<Sources />);
    await screen.findByText("erp");
    fireEvent.click(screen.getByRole("button", { name: "Ajouter une source" }));
    fireEvent.change(screen.getByLabelText("Type"), { target: { value: "odbc" } });
    expect(await screen.findByText(/ne liste aucun pilote ODBC/)).toBeInTheDocument();
  });

  it("sends a CSV of this computer to the server and takes its path", async () => {
    m.uploadSourceFile.mockResolvedValue({ path: "C:/data/source-files/clients.csv", bytes: 10 });
    renderPage(<Sources />);
    await screen.findByText("erp");
    fireEvent.click(screen.getByRole("button", { name: "Ajouter une source" }));
    const file = new File(["code,name\n"], "clients.csv", { type: "text/csv" });
    fireEvent.change(screen.getByTestId("csv-file"), { target: { files: [file] } });
    await waitFor(() => expect(screen.getByLabelText("File path or HTTPS URL")).toHaveValue("C:/data/source-files/clients.csv"));
    expect(m.uploadSourceFile).toHaveBeenCalledWith(file);
    m.uploadSourceFile.mockRejectedValue(new Error("a .csv, .tsv or .txt file is required"));
    fireEvent.change(screen.getByTestId("csv-file"), { target: { files: [file] } });
    expect(await screen.findByText("a .csv, .tsv or .txt file is required")).toBeInTheDocument();
    // Nothing picked: nothing sent.
    fireEvent.change(screen.getByTestId("csv-file"), { target: { files: [] } });
    expect(m.uploadSourceFile).toHaveBeenCalledTimes(2);
  });

  it("offers the table's indexes as the key and warns of a watermark without one", async () => {
    m.listSourceIndexes.mockResolvedValue({
      indexes: [
        { name: "pk", columns: ["tbl", "id"], unique: true },
        { name: "by_label", columns: ["label"], unique: false },
      ],
    });
    m.updateMapping.mockResolvedValue({});
    renderPage(<Sources />);
    await screen.findByText("erp");
    fireEvent.click(screen.getAllByRole("button", { name: "Modifier" })[1]);
    await screen.findByLabelText("Mapping name");
    const pk = await screen.findByRole("button", { name: /pk: tbl, id/ });
    expect(pk).toHaveAttribute("title", "Index unique");
    fireEvent.click(pk);
    expect(screen.getByRole("group", { name: "Key column(s)" })).toHaveTextContent("tbl×id×");
    // A watermark column no index starts with: a sync will scan.
    fireEvent.change(screen.getByLabelText("Watermark column (incremental sync)"), { target: { value: "updated" } });
    expect(screen.getByRole("status")).toHaveTextContent(/Aucun index ne commence par cette colonne/);
    fireEvent.change(screen.getByLabelText("Watermark column (incremental sync)"), { target: { value: "label" } });
    expect(screen.queryByRole("status")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Enregistrer" }));
    await waitFor(() => expect(m.updateMapping).toHaveBeenCalledWith(1, 1, expect.objectContaining({ key_column: "tbl, id", watermark_column: "label" })));
  });

  it("edits a mapping: its columns, its schedule, a link taken back", async () => {
    m.updateMapping.mockResolvedValue({});
    m.listSourceColumns.mockRejectedValue(new Error("no such table"));
    renderPage(<Sources />);
    await screen.findByText("erp");
    fireEvent.click(screen.getAllByRole("button", { name: "Modifier" })[1]);
    expect(await screen.findByLabelText("Mapping name")).toHaveValue("orders");
    await waitFor(() => expect(m.listSourceColumns).toHaveBeenCalledWith(1, "dbo.orders"));
    fireEvent.change(screen.getByLabelText("Name column (default: the key)"), { target: { value: "label" } });
    fireEvent.change(screen.getByLabelText("Description column"), { target: { value: "notes" } });
    fireEvent.change(screen.getByLabelText("Watermark column (incremental sync)"), { target: { value: "updated" } });
    fireEvent.change(screen.getByLabelText("Schedule (minutes, 0: manual)"), { target: { value: "15" } });
    fireEvent.click(screen.getByRole("button", { name: "Add link" }));
    fireEvent.change(screen.getByLabelText("Column"), { target: { value: "rep" } });
    fireEvent.click(screen.getByRole("button", { name: "×" }));
    expect(screen.queryByLabelText("Column")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Enregistrer" }));
    await waitFor(() =>
      expect(m.updateMapping).toHaveBeenCalledWith(
        1,
        1,
        expect.objectContaining({ name: "orders", name_column: "label", description_column: "notes", watermark_column: "updated", schedule_minutes: 15, relations: [] }),
      ),
    );
    expect(await screen.findByText("Mapping enregistré.")).toBeInTheDocument();

    // The tables that cannot be listed are an error on the form, which can still be left.
    m.listSourceTables.mockRejectedValueOnce(new Error("no driver"));
    fireEvent.click(screen.getByRole("button", { name: "Ajouter un mapping" }));
    expect(await screen.findByText("no driver")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Annuler" }));
    expect(screen.queryByLabelText("Mapping name")).toBeNull();
    expect(m.createMapping).not.toHaveBeenCalled();
  });

  it("deletes a source, a mapping and a named query only once confirmed", async () => {
    m.deleteSource.mockResolvedValue({});
    m.deleteMapping.mockResolvedValue({});
    m.deleteNamedQuery.mockRejectedValue(new Error("query in use"));
    renderPage(<Sources />);
    await screen.findByText("erp");
    const remove = async (i: number, answer: string) => {
      fireEvent.click(screen.getAllByRole("button", { name: "Supprimer" })[i]);
      fireEvent.click(await screen.findByRole("button", { name: answer }));
    };
    await remove(1, "Annuler");
    await waitFor(() => expect(screen.queryByText("Supprimer le mapping ?")).toBeNull());
    expect(m.deleteMapping).not.toHaveBeenCalled();
    await remove(1, "Confirmer");
    await waitFor(() => expect(m.deleteMapping).toHaveBeenCalledWith(1, 1));
    await remove(0, "Confirmer");
    await waitFor(() => expect(m.deleteSource).toHaveBeenCalledWith(1));
    await remove(2, "Confirmer");
    await waitFor(() => expect(m.deleteNamedQuery).toHaveBeenCalledWith(1, 1));
    expect(await screen.findByText("query in use")).toBeInTheDocument();
  });

  it("describes and caps a named query, and closes what it opened", async () => {
    m.createNamedQuery.mockResolvedValue({});
    m.previewMapping.mockResolvedValue({ columns: [{ name: "id", data_type: "int" }, { name: "note", data_type: "text" }], rows: [[1, null]], sheets: [], links: 0, skipped: 0 });
    renderPage(<Sources />);
    await screen.findByText("erp");
    fireEvent.click(screen.getByRole("button", { name: "Ajouter une requête" }));
    fireEvent.change(screen.getByLabelText("Query name"), { target: { value: "client" } });
    fireEvent.change(screen.getByLabelText("Trigger (regex with named groups)"), { target: { value: "client (?P<name>\\w+)" } });
    fireEvent.change(screen.getByLabelText("SELECT with {group} blanks"), { target: { value: "SELECT * FROM c WHERE name = {name}" } });
    fireEvent.change(screen.getByLabelText("Description"), { target: { value: "A customer by name" } });
    fireEvent.change(screen.getByLabelText("Max rows (200 max)"), { target: { value: "50" } });
    fireEvent.click(screen.getByRole("button", { name: "Enregistrer" }));
    await waitFor(() => expect(m.createNamedQuery).toHaveBeenCalledWith(1, expect.objectContaining({ name: "client", description: "A customer by name", max_rows: 50 })));
    await waitFor(() => expect(screen.queryByLabelText("Query name")).toBeNull());

    fireEvent.click(screen.getByRole("button", { name: "Ajouter une requête" }));
    fireEvent.click(screen.getByRole("button", { name: "Annuler" }));
    expect(screen.queryByLabelText("Query name")).toBeNull();
    expect(m.createNamedQuery).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole("button", { name: "Essayer" }));
    expect(screen.getByText("Essayer : ticket")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Fermer" }));
    expect(screen.queryByText("Essayer : ticket")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Aperçu" }));
    expect(await screen.findByText("Aperçu : orders")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Fermer" }));
    expect(screen.queryByText("Aperçu : orders")).toBeNull();
  });
});
