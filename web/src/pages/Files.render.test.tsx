// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// Rendering tests of the Files page: library table (types, sources,
// statuses, sizes, dates), filters/sort/pager, stats and side cards derived
// from the records, upload through the hidden input and the dropzone
// (success, concept-type guard, 422, network), delete behind the confirm
// dialog, and the file-picker entry points.

import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import Files from "./Files";
import { fireEvent, makeFile, renderPage, screen, waitFor, within } from "../test/render";
import { Route, useLocation } from "react-router-dom";
import { ApiError, type FileRecord, type Ontology, type UploadResponse } from "../api";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return {
    ...actual,
    getFiles: vi.fn(),
    getOntology: vi.fn(),
    upload: vi.fn(),
    deleteFile: vi.fn(),
  };
});

import * as api from "../api";

const mocked = {
  getFiles: api.getFiles as unknown as Mock,
  getOntology: api.getOntology as unknown as Mock,
  upload: api.upload as unknown as Mock,
  deleteFile: api.deleteFile as unknown as Mock,
};

// Fixed clock (2024-03-09T12:00:00Z): the relative ages ("à l'instant",
// "il y a 5 min", …) must not depend on how long the worker took to get here.
const NOW = 1_710_000_000;
const DAY = 86_400;

function rec(partial: Partial<FileRecord> & { id: number; name: string }): FileRecord {
  return {
    size: 100,
    kind: "text",
    status: "",
    uploaded_at: NOW - 60,
    concepts: 0,
    relations: 0,
    ontology_updates: 0,
    ...partial,
  };
}

// Ten records: every icon / type / status / size unit / age branch.
const files: FileRecord[] = [
  rec({ id: 1, name: "report.pdf", size: 2.5 * 1024 * 1024, status: "analyzed", uploaded_at: NOW - 30, concept_type: "Contract" }),
  rec({ id: 2, name: "data.csv", size: 512, concepts: 5, uploaded_at: NOW - 300 }),
  rec({ id: 3, name: "sheet.xlsx", size: 1536, status: "pending", uploaded_at: NOW - 7_200 }),
  rec({ id: 4, name: "notes.docx", size: 4096, status: "failed", uploaded_at: NOW - 3 * DAY }),
  rec({ id: 5, name: "graph.json", size: 2048, status: "processing", uploaded_at: NOW - 4 * DAY }),
  rec({ id: 6, name: "rows.jsonl", size: 8192, status: "queued", uploaded_at: NOW - 5 * DAY }),
  rec({ id: 7, name: "facts.triples", size: 300, status: "error", uploaded_at: NOW - 6 * DAY }),
  rec({ id: 8, name: "readme.md", size: 900, uploaded_at: NOW - 7 * DAY }),
  rec({ id: 9, name: "blob.bin", kind: "binary", size: 2 * 1024 * 1024 * 1024, uploaded_at: 0 }),
  rec({ id: 10, name: "old.txt", size: 10, relations: 2, uploaded_at: NOW - 40 * DAY }),
];

const ontology: Ontology = {
  concept_types: { Contract: { properties: [] } as unknown as Ontology["concept_types"][string] },
  relation_types: {},
};

const uploaded: UploadResponse = { file_id: 11, ingested: { concepts: 3, relations: 2, ontology_updates: 0 } };

function hiddenInput(container: HTMLElement): HTMLInputElement {
  return container.querySelector("input[type=file]") as HTMLInputElement;
}

function tableRows(): HTMLElement[] {
  const table = screen.getByRole("table");
  return within(table).getAllByRole("row").slice(1);
}

/** The library row of `name` (the name also shows in the side cards). */
function libraryRow(name: string): HTMLElement {
  return within(screen.getByRole("table")).getByText(name).closest("tr") as HTMLElement;
}

/** Wait for the first load: the library table is on screen. */
async function loaded(): Promise<void> {
  await screen.findByRole("table");
}

function rowNames(): string[] {
  return tableRows().map((r) => within(r).getByText(/\.[a-z]+$/, { selector: ".file-name" }).textContent!);
}

function dropzone(): HTMLElement {
  return screen.getByText(/Glissez-déposez vos fichiers ici/).closest(".dz") as HTMLElement;
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"], now: NOW * 1000 });
  mocked.getFiles.mockResolvedValue({ files });
  mocked.getOntology.mockResolvedValue(ontology);
  mocked.upload.mockResolvedValue(uploaded);
  mocked.deleteFile.mockResolvedValue(undefined);
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("Files page — library", () => {
  it("lists the newest six files with type, source, status, size and date", async () => {
    renderPage(<Files />);
    await loaded();
    expect(mocked.getFiles).toHaveBeenCalledTimes(1);
    expect(mocked.getOntology).toHaveBeenCalledTimes(1);

    // Default sort: last updated, newest first; page size 6.
    expect(rowNames()).toEqual(["report.pdf", "data.csv", "sheet.xlsx", "notes.docx", "graph.json", "rows.jsonl"]);
    expect(screen.getByText("Fichiers 1 à 6 sur 10")).toBeInTheDocument();

    const row = libraryRow;
    expect(within(row("report.pdf")).getByText("PDF", { selector: "td" })).toBeInTheDocument();
    expect(within(row("report.pdf")).getByText("Contract", { selector: "td" })).toBeInTheDocument();
    expect(within(row("report.pdf")).getByText("type : Contract")).toBeInTheDocument();
    expect(within(row("report.pdf")).getByText("Analysé")).toHaveClass("info");
    expect(within(row("report.pdf")).getByText("2.5 Mo")).toBeInTheDocument();

    expect(within(row("data.csv")).getByText("CSV", { selector: "td" })).toBeInTheDocument();
    expect(within(row("data.csv")).getByText("Général")).toBeInTheDocument();
    expect(within(row("data.csv")).getByText("Traité")).toHaveClass("ok");
    expect(within(row("data.csv")).getByText("512 o")).toBeInTheDocument();

    expect(within(row("sheet.xlsx")).getByText("XLS")).toBeInTheDocument();
    expect(within(row("sheet.xlsx")).getByText("En attente")).toHaveClass("warn");
    expect(within(row("sheet.xlsx")).getByText("2 Ko")).toBeInTheDocument();

    expect(within(row("notes.docx")).getByText("DOC")).toBeInTheDocument();
    expect(within(row("notes.docx")).getByText("Échec")).toHaveClass("fail");
    expect(within(row("graph.json")).getByText("Analyse en cours")).toHaveClass("info");
    expect(within(row("rows.jsonl")).getByText("JSONL", { selector: "td" })).toBeInTheDocument();
    expect(within(row("rows.jsonl")).getByText("En attente")).toBeInTheDocument();

    const expectedDate = new Date((NOW - 30) * 1000).toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
    expect(within(row("report.pdf")).getByText(expectedDate)).toBeInTheDocument();
  });

  it("shows the remaining branches on page 2 and disables the pager at the ends", async () => {
    const { user } = renderPage(<Files />);
    await loaded();
    const prev = screen.getByRole("button", { name: "‹" });
    const next = screen.getByRole("button", { name: "›" });
    expect(prev).toBeDisabled();
    await user.click(next);
    expect(rowNames()).toEqual(["facts.triples", "readme.md", "old.txt", "blob.bin"]);
    expect(screen.getByText("Fichiers 7 à 10 sur 10")).toBeInTheDocument();
    expect(next).toBeDisabled();
    expect(screen.getByRole("button", { name: "2" })).toHaveClass("active");

    const row = libraryRow;
    expect(within(row("facts.triples")).getByText("TRIPLES")).toBeInTheDocument();
    expect(within(row("facts.triples")).getByText("Échec")).toBeInTheDocument();
    expect(within(row("readme.md")).getByText("TEXT")).toBeInTheDocument();
    expect(within(row("readme.md")).getByText("En attente")).toBeInTheDocument();
    expect(within(row("blob.bin")).getByText("BINARY")).toBeInTheDocument();
    expect(within(row("blob.bin")).getByText("2.0 Go")).toBeInTheDocument();
    expect(within(row("blob.bin")).getByText("—")).toBeInTheDocument();
    expect(within(row("old.txt")).getByText("Traité")).toBeInTheDocument();

    await user.click(prev);
    expect(screen.getByText("Fichiers 1 à 6 sur 10")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "2" }));
    expect(screen.getByText("Fichiers 7 à 10 sur 10")).toBeInTheDocument();
  });

  it("filters by search, type and status, sorts, and resets the page", async () => {
    const { user } = renderPage(<Files />);
    await loaded();
    await user.click(screen.getByRole("button", { name: "›" }));
    expect(screen.getByText("Fichiers 7 à 10 sur 10")).toBeInTheDocument();

    await user.type(screen.getByPlaceholderText("Rechercher un fichier…"), "CSV");
    expect(rowNames()).toEqual(["data.csv"]);
    expect(screen.getByText("Fichiers 1 à 1 sur 1")).toBeInTheDocument();
    await user.clear(screen.getByPlaceholderText("Rechercher un fichier…"));

    const selects = screen.getAllByRole("combobox");
    const [typeSelect, statusSelect, sortSelect] = selects;
    expect(within(typeSelect).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "Tous les types", "BINARY", "CSV", "DOCX", "JSON", "JSONL", "PDF", "TEXT", "TRIPLES", "XLSX",
    ]);
    await user.selectOptions(typeSelect, "PDF");
    expect(rowNames()).toEqual(["report.pdf"]);
    await user.selectOptions(typeSelect, "");

    await user.selectOptions(statusSelect, "échec");
    expect(rowNames()).toEqual(["notes.docx", "facts.triples"]);
    await user.selectOptions(statusSelect, "");

    await user.selectOptions(sortSelect, "name");
    expect(rowNames().slice(0, 3)).toEqual(["blob.bin", "data.csv", "facts.triples"]);
    await user.selectOptions(sortSelect, "size");
    expect(rowNames()[0]).toBe("blob.bin");
    expect(rowNames()[1]).toBe("report.pdf");
    await user.selectOptions(sortSelect, "type");
    expect(rowNames().slice(0, 2)).toEqual(["blob.bin", "data.csv"]);
    await user.selectOptions(sortSelect, "updated");
    expect(rowNames()[0]).toBe("report.pdf");

    await user.type(screen.getByPlaceholderText("Rechercher un fichier…"), "nothing-here");
    expect(screen.getByText(/Aucun fichier ne correspond/)).toBeInTheDocument();
    expect(screen.queryByText(/^Fichiers \d+ à/)).not.toBeInTheDocument();
  });

  it("derives stats, storage, folders and activity from the records", async () => {
    renderPage(<Files />);
    await loaded();
    const stat = (label: string) => screen.getByText(label, { selector: ".ts-label" }).nextElementSibling!;
    expect(stat("Fichiers au total")).toHaveTextContent("10");
    // Only data.csv and old.txt count as processed.
    expect(stat("Traités")).toHaveTextContent("2");
    expect(stat("En attente")).toHaveTextContent("8");
    expect(stat("Espace utilisé")).toHaveTextContent("2.0 Go");

    // Storage donut: top six types by bytes, BINARY first.
    const legend = screen.getByText("Total").closest(".storage")!.querySelector(".legend")!;
    const labels = Array.from(legend.querySelectorAll(".lbl")).map((e) => e.textContent);
    expect(labels[0]).toBe("BINARY");
    expect(labels).toHaveLength(6);
    expect(legend.querySelectorAll(".pct")[0]).toHaveTextContent("99.");

    // Folders: grouped by source.
    const folders = screen.getByText("Dossiers / collections").closest("section")!;
    expect(within(folders).getByText("Général").nextElementSibling).toHaveTextContent("9 fichiers");
    expect(within(folders).getByText("Contract").nextElementSibling).toHaveTextContent("1 fichier");

    // Activity: five newest, with verb and relative age.
    const activity = screen.getByText("Activité récente").closest("section")!;
    const items = within(activity).getAllByRole("listitem");
    expect(items).toHaveLength(5);
    expect(items[0]).toHaveTextContent("report.pdf est en cours d'analyse");
    expect(items[0]).toHaveTextContent("à l'instant");
    expect(items[1]).toHaveTextContent("data.csv a été traité");
    expect(items[1]).toHaveTextContent("il y a 5 min");
    expect(items[2]).toHaveTextContent("sheet.xlsx est en attente");
    expect(items[2]).toHaveTextContent("il y a 2 h");
    expect(items[3]).toHaveTextContent("notes.docx n'a pas pu être traité");
    expect(items[3]).toHaveTextContent("il y a 3 j");

    // Recent uploads fall back to the first three records: analyzing → bar, others → check.
    const recent = screen.getByText("Dépôts récents").closest("section")!;
    const rows = within(recent).getAllByRole("listitem");
    expect(rows).toHaveLength(3);
    expect(rows[0]).toHaveTextContent("report.pdf");
    expect(rows[0]).toHaveTextContent("100%");
    expect(rows[1]).toHaveTextContent("Traité");
    expect(within(rows[1]).getByText("✓")).toBeInTheDocument();
  });

  it("shows the empty states when nothing has been uploaded", async () => {
    mocked.getFiles.mockResolvedValue({ files: [] });
    renderPage(<Files />);
    expect(await screen.findByText(/Aucun fichier ne correspond/)).toBeInTheDocument();
    expect(screen.getByText("Aucun dépôt pour l'instant.")).toBeInTheDocument();
    expect(screen.getByText("Aucune activité pour l'instant.")).toBeInTheDocument();
    expect(screen.getByText("Aucune donnée")).toBeInTheDocument();
    expect(screen.getByText("Fichiers au total").nextElementSibling).toHaveTextContent("0");
  });

  it("surfaces a loading failure in the banner", async () => {
    mocked.getFiles.mockRejectedValueOnce(new Error("server down"));
    renderPage(<Files />);
    expect(await screen.findByText("server down")).toHaveClass("error-banner");
  });

  it("stringifies a non-Error loading failure", async () => {
    mocked.getOntology.mockRejectedValueOnce("nope");
    renderPage(<Files />);
    expect(await screen.findByText("nope")).toBeInTheDocument();
  });
});

describe("Files page — upload", () => {
  it("uploads a JSONL file through the hidden input and reports what was ingested", async () => {
    const { user, container } = renderPage(<Files />);
    await loaded();
    const file = makeFile("rows.jsonl", '{"a":1}\n', "application/x-ndjson");
    await user.upload(hiddenInput(container), file);
    expect(await screen.findByText("3 fiches et 2 liens importés depuis rows.jsonl.")).toHaveClass("success-banner");
    expect(mocked.upload).toHaveBeenCalledWith(file, { kind: "jsonl", conceptType: undefined });
    await waitFor(() => expect(mocked.getFiles).toHaveBeenCalledTimes(2));

    const recent = screen.getByText("Dépôts récents").closest("section")!;
    const row = within(recent).getAllByRole("listitem")[0];
    expect(row).toHaveTextContent("rows.jsonl");
    expect(row).toHaveTextContent("Traité");
    expect(within(row).getByText("✓")).toBeInTheDocument();
    expect(hiddenInput(container).value).toBe("");
  });

  it.each([
    ["data.csv", "csv"],
    ["sheet.xlsx", "xlsx"],
  ])("asks the sheet type for %s, then imports with it", async (name, kind) => {
    const { user, container } = renderPage(<Files />);
    await loaded();
    const file = makeFile(name, "x");
    await user.upload(hiddenInput(container), file);
    const form = await screen.findByRole("form", { name: "Type de fiche du fichier déposé" });
    expect(form).toHaveTextContent(`Chaque ligne de « ${name} » devient une fiche : de quel type ?`);
    expect(mocked.upload).not.toHaveBeenCalled();
    expect(screen.queryByText(/demande un type de fiche/)).toBeNull();
    const go = within(form).getByRole("button", { name: "Importer" });
    expect(go).toBeDisabled();
    await user.selectOptions(within(form).getByRole("combobox", { name: "Type de fiche" }), "Contract");
    await user.click(go);
    await waitFor(() => expect(mocked.upload).toHaveBeenCalledWith(file, { kind, conceptType: "Contract" }));
    expect(screen.queryByRole("form", { name: "Type de fiche du fichier déposé" })).toBeNull();
    const recent = screen.getByText("Dépôts récents").closest("section")!;
    const row = within(recent).getAllByRole("listitem")[0];
    expect(row).toHaveTextContent(name);
    expect(row).toHaveTextContent("Traité");
  });

  it("shows the server's 422 and an unreachable API in the banner", async () => {
    mocked.upload.mockRejectedValueOnce(new ApiError("upload failed: 422 Unprocessable Entity", 422, null));
    const { user, container } = renderPage(<Files />);
    await loaded();
    await user.upload(hiddenInput(container), makeFile("a.jsonl", "{}"));
    expect(await screen.findByText("upload failed: 422 Unprocessable Entity")).toBeInTheDocument();

    mocked.upload.mockRejectedValueOnce(new ApiError("API injoignable sur le proxy Vite (Failed to fetch).", 0, null));
    await user.upload(hiddenInput(container), makeFile("b.jsonl", "{}"));
    expect(await screen.findByText(/API injoignable/)).toBeInTheDocument();
    // A non-Error rejection is stringified.
    mocked.upload.mockRejectedValueOnce("odd");
    await user.upload(hiddenInput(container), makeFile("c.jsonl", "{}"));
    expect(await screen.findByText("odd")).toBeInTheDocument();
  });

  it("accepts drops on the dropzone, shows progress, and ignores drops while busy", async () => {
    let finish!: (r: UploadResponse) => void;
    mocked.upload.mockReturnValueOnce(new Promise<UploadResponse>((r) => (finish = r)));
    renderPage(<Files />);
    await loaded();
    const dz = dropzone();
    fireEvent.dragOver(dz);
    expect(dz).toHaveClass("active");
    fireEvent.dragLeave(dz);
    expect(dz).not.toHaveClass("active");

    fireEvent.drop(dz, { dataTransfer: { files: [makeFile("first.jsonl", "{}")] } });
    expect(dz).not.toHaveClass("active");
    expect(await screen.findByText("70%")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Déposer des fichiers" })).toBeDisabled();

    fireEvent.drop(dz, { dataTransfer: { files: [makeFile("second.jsonl", "{}")] } });
    fireEvent.drop(dz, { dataTransfer: { files: [] } });
    expect(mocked.upload).toHaveBeenCalledTimes(1);

    finish(uploaded);
    expect(await screen.findByText(/3 fiches et 2 liens importés/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Déposer des fichiers" })).toBeEnabled();
    expect(mocked.upload).toHaveBeenCalledTimes(1);
  });
});

describe("Files page — delete and actions", () => {
  it("removes a record only after the confirm dialog is accepted", async () => {
    const { user } = renderPage(<Files />);
    await loaded();
    await user.click(screen.getAllByRole("button", { name: "Actions" })[0]);
    expect(screen.getByRole("dialog")).toHaveTextContent("Retirer le fichier de la liste");
    await user.click(screen.getByRole("button", { name: "Annuler" }));
    expect(mocked.deleteFile).not.toHaveBeenCalled();

    await user.click(screen.getAllByRole("button", { name: "Actions" })[1]);
    await user.click(screen.getByRole("button", { name: "Retirer" }));
    await waitFor(() => expect(mocked.deleteFile).toHaveBeenCalledWith(2));
    expect(mocked.getFiles).toHaveBeenCalledTimes(2);
  });

  it("surfaces a delete failure", async () => {
    mocked.deleteFile.mockRejectedValueOnce(new Error("cannot delete"));
    const { user } = renderPage(<Files />);
    await loaded();
    await user.click(screen.getAllByRole("button", { name: "Actions" })[0]);
    await user.click(screen.getByRole("button", { name: "Retirer" }));
    expect(await screen.findByText("cannot delete")).toBeInTheDocument();
  });

  it("opens the file picker from every entry point", async () => {
    const { user } = renderPage(<Files />);
    await loaded();
    const click = vi.spyOn(HTMLInputElement.prototype, "click");
    await user.click(screen.getByRole("button", { name: "Déposer des fichiers" }));
    await user.click(screen.getByRole("button", { name: "Parcourir" }));
    await user.click(screen.getByText(/Glissez-déposez vos fichiers ici/));
    await user.click(screen.getByText("Nouveau dossier"));
    await user.click(screen.getByText("Créer un dossier"));
    await user.click(screen.getByText("Ajouter des fichiers à votre bibliothèque"));
    // The dropzone's Browse button stops propagation: one click each.
    expect(click).toHaveBeenCalledTimes(6);
  });

  it("reloads on 'Relancer les échecs', keeps 'Tout voir' on the page and links the export", async () => {
    const { user } = renderPage(<Files />);
    await loaded();
    await user.click(screen.getByText("Relancer les échecs"));
    await waitFor(() => expect(mocked.getFiles).toHaveBeenCalledTimes(2));
    for (const link of screen.getAllByRole("link", { name: "Tout voir" })) {
      await user.click(link);
    }
    expect(screen.getByRole("link", { name: /Exporter les métadonnées/ })).toHaveAttribute(
      "href",
      expect.stringContaining("/export?format=jsonl"),
    );
  });
});

describe("Files page — documents go to the review", () => {
  function Probe() {
    const loc = useLocation();
    const file = (loc.state as { file?: File } | null)?.file;
    return <div data-testid="handed">{loc.pathname}:{file?.name ?? "none"}</div>;
  }

  it.each(["contrat.pdf", "notes.txt", "memo.docx", "readme.md"])(
    "hands %s to the assistant instead of loading it as one sheet",
    async (name) => {
      const { user, container } = renderPage(<Files />, {
        extraRoutes: <Route path="/ingest" element={<Probe />} />,
      });
      await loaded();
      await user.upload(hiddenInput(container), makeFile(name, "x"));
      expect(await screen.findByTestId("handed")).toHaveTextContent(`/ingest:${name}`);
      expect(mocked.upload).not.toHaveBeenCalled();
    },
  );
});
