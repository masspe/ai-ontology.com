// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// Rendering tests of the Ontology Creation page: initial load (ontology,
// stats, files, subgraph), the LLM draft (generate → preview → save, with the
// schema-guard error path), the file list (icons, sizes, delete, clear-all
// behind the confirm dialog), the ingest preview (upload, ZIP and folder
// expansion, OCR notice, per-file failures, review decisions, apply/discard)
// and the insight tiles.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import JSZip from "jszip";
import OntologyBuilder from "./OntologyBuilder";
import { fireEvent, makeFile, renderPage, screen, waitFor, within } from "../test/render";
import type { FileRecord, Ontology, Stats, Subgraph } from "../api";
import type { ApplyReport, OntologyProposal } from "../lib/proposalTypes";
import type { ReviewPanelProps } from "../components/IngestReview";
import type { PrepareOptions } from "../lib/extractText";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return {
    ...actual,
    getOntology: vi.fn(),
    getStats: vi.fn(),
    getFiles: vi.fn(),
    getSubgraph: vi.fn(),
    generateOntology: vi.fn(),
    replaceOntology: vi.fn(),
    listConcepts: vi.fn(),
    deleteFile: vi.fn(),
  };
});
vi.mock("../lib/ingestApi", () => ({ analyzeIngest: vi.fn(), applyIngest: vi.fn() }));
vi.mock("../lib/extractText", () => ({
  needsPreprocessing: vi.fn(),
  prepareForIngest: vi.fn(),
  terminateOcrWorker: vi.fn(),
}));
// The SVG graph has its own rendering tests: here it only echoes its inputs.
vi.mock("../components/OntologyGraph", () => ({
  default: ({ subgraph, proposed }: { subgraph?: Subgraph | null; proposed?: OntologyProposal | null }) => (
    <div data-testid="ontology-graph">
      {(subgraph?.concepts ?? []).map((c) => c.name).join(",")}|{proposed ? proposed.concepts.length : "none"}
    </div>
  ),
}));
// The review panel is covered by its own tests: the stub exposes every
// callback through a button so the page's state logic can be exercised.
vi.mock("../components/IngestReview", async () => {
  const actual = await vi.importActual<typeof import("../components/IngestReview")>("../components/IngestReview");
  return {
    ...actual,
    ReviewPanel: (props: ReviewPanelProps) => (
      <div data-testid="review-panel">
        <span data-testid="decisions">{JSON.stringify(props.decisions)}</span>
        <span data-testid="concept-names">{props.proposal.concepts.map((c) => c.name).join(",")}</span>
        <span data-testid="relation-types">{props.proposal.relations.map((r) => r.relation_type).join(",")}</span>
        <button onClick={() => props.onDecision("f0/c0", "skip")}>stub-decide</button>
        <button onClick={() => props.onEditConcept("f0/c0", { name: "Renamed" })}>stub-edit-concept</button>
        <button onClick={() => props.onEditRelation("f0/r0", { relation_type: "renamed_rel" })}>stub-edit-relation</button>
        <button onClick={() => props.onBulkDecision("create_new")}>stub-bulk</button>
        <button onClick={props.onApply} disabled={props.applyDisabled}>
          {props.applyLabel}
        </button>
        <button onClick={props.onCancel}>{props.cancelLabel}</button>
      </div>
    ),
    ApplyReportView: ({ report, onReset, resetLabel }: { report: ApplyReport; onReset: () => void; resetLabel?: string }) => (
      <div data-testid="apply-report">
        created={report.created}
        <button onClick={onReset}>{resetLabel}</button>
      </div>
    ),
  };
});

import * as api from "../api";
import { analyzeIngest, applyIngest } from "../lib/ingestApi";
import { needsPreprocessing, prepareForIngest, terminateOcrWorker } from "../lib/extractText";

const getOntology = vi.mocked(api.getOntology);
const getStats = vi.mocked(api.getStats);
const getFiles = vi.mocked(api.getFiles);
const getSubgraph = vi.mocked(api.getSubgraph);
const generateOntology = vi.mocked(api.generateOntology);
const replaceOntology = vi.mocked(api.replaceOntology);
const listConcepts = vi.mocked(api.listConcepts);
const deleteFile = vi.mocked(api.deleteFile);
const analyze = vi.mocked(analyzeIngest);
const apply = vi.mocked(applyIngest);

// ---- fixtures ---------------------------------------------------------------

const ontology: Ontology = {
  concept_types: { Person: { name: "Person" }, Org: { name: "Org" } },
  relation_types: {},
};

const stats: Stats = {
  concepts: 1200,
  relations: 3000,
  rules: 0,
  actions: 0,
  concept_types: 2,
  relation_types: 0,
  rule_types: 0,
  action_types: 0,
  deltas: { concepts_pct: 12, relations_pct: -8, concept_types_pct: 0.2, relation_types_pct: 0 },
};

const file = (over: Partial<FileRecord> & Pick<FileRecord, "id" | "name">): FileRecord => ({
  size: 0,
  kind: "text",
  status: "done",
  uploaded_at: 1_700_000_000,
  concepts: 0,
  relations: 0,
  ontology_updates: 0,
  ...over,
});
const files: FileRecord[] = [
  file({ id: 1, name: "report.pdf", size: 500, concepts: 3, relations: 1, concept_type: "Report" }),
  file({ id: 2, name: "notes.docx", size: 2048 }),
  file({ id: 3, name: "graph.json", kind: "ontology", size: 3 * 1024 * 1024 }),
  file({ id: 4, name: "weird.bin", kind: "custom", size: 10 }),
];

const subgraph: Subgraph = {
  concepts: [{ id: 1, concept_type: "Person", name: "Alice" }],
  relations: [],
};

const draft: Ontology = { concept_types: { Contract: { name: "Contract" } }, relation_types: {} };

const proposal: OntologyProposal = {
  concept_types: [{ client_ref: "ct0", name: "Contract" }],
  relation_types: [],
  concepts: [
    {
      client_ref: "c0",
      concept_type: "Contract",
      name: "MSA",
      conflict: { kind: { kind: "exists", existing_id: "7", existing_display: "MSA" }, summary: "exists" },
    },
    { client_ref: "c1", concept_type: "Contract", name: "SLA" },
  ],
  relations: [{ client_ref: "r0", relation_type: "amends", source_ref: "c0", target_ref: "c1" }],
  rules: [],
  actions: [],
};

const report: ApplyReport = {
  concept_types: [],
  relation_types: [],
  concepts: [],
  relations: [],
  rules: [],
  actions: [],
  created: 1,
  merged: 0,
  skipped: 1,
  failed: 0,
};

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** A binary `File` whose `arrayBuffer()` resolves to `bytes` (jsdom gap). */
function binaryFile(name: string, bytes: Uint8Array): File {
  const f = new File([bytes as unknown as BlobPart], name, { type: "application/zip" });
  Object.defineProperty(f, "arrayBuffer", { value: () => Promise.resolve(bytes.buffer) });
  return f;
}

async function zipFile(name: string, entries: Record<string, string | Uint8Array>, folders: string[] = []): Promise<File> {
  const zip = new JSZip();
  for (const folder of folders) zip.folder(folder);
  for (const [path, content] of Object.entries(entries)) zip.file(path, content);
  return binaryFile(name, await zip.generateAsync({ type: "uint8array" }));
}

function withRelPath(f: File, rel: string | undefined): File {
  Object.defineProperty(f, "webkitRelativePath", { value: rel, configurable: true });
  return f;
}

function dropzoneInput(container: HTMLElement): HTMLInputElement {
  return container.querySelector<HTMLInputElement>(".dropzone input[type=file]")!;
}
function folderInput(container: HTMLElement): HTMLInputElement {
  return container.querySelector<HTMLInputElement>("input[webkitdirectory]")!;
}
function upload(input: HTMLInputElement, list: File[]): void {
  fireEvent.change(input, { target: { files: list } });
}

const errorBanner = () => screen.findByText((_, el) => el?.className === "error-banner", { selector: "div" });
const infoBanner = () => screen.findByText((_, el) => el?.className === "success-banner", { selector: "div" });

beforeEach(() => {
  listConcepts.mockResolvedValue({ total: 0, concepts: [], next_cursor: null });
  getOntology.mockResolvedValue(ontology);
  getStats.mockResolvedValue(stats);
  getFiles.mockResolvedValue({ files });
  getSubgraph.mockResolvedValue({ subgraph });
  generateOntology.mockResolvedValue({ ontology: draft, model: "test" });
  replaceOntology.mockResolvedValue(draft);
  deleteFile.mockResolvedValue(undefined);
  analyze.mockResolvedValue(proposal);
  apply.mockResolvedValue(report);
  vi.mocked(needsPreprocessing).mockReturnValue(false);
  vi.mocked(prepareForIngest).mockImplementation(async (f: File, opts?: PrepareOptions) => {
    opts?.onProgress?.({ status: "ocr 50%" });
    return f;
  });
  vi.mocked(terminateOcrWorker).mockResolvedValue(undefined);
});

afterEach(() => {
  vi.restoreAllMocks();
});

async function renderLoaded() {
  const rendered = renderPage(<OntologyBuilder />);
  await screen.findByText("report.pdf");
  return rendered;
}

// ---- tests ------------------------------------------------------------------

describe("OntologyBuilder — load and insights", () => {
  it("loads ontology, stats, files and subgraph and renders the insights", async () => {
    const { unmount } = await renderLoaded();
    expect(getOntology).toHaveBeenCalledTimes(1);
    expect(getSubgraph).toHaveBeenCalledWith({ limit: 150, expansion_depth: 1 });
    expect(screen.getByTestId("ontology-graph")).toHaveTextContent("Alice|none");

    // Insight tiles: 1200 -> "1.2k", deltas up / down / flat, classes from the ontology.
    expect(screen.getByText("1.2k")).toBeInTheDocument();
    expect(screen.getByText("3.0k")).toBeInTheDocument();
    expect(screen.getByText("↑ 12 % vs dernière exécution")).toBeInTheDocument();
    expect(screen.getByText("↓ 8 % vs dernière exécution")).toBeInTheDocument();
    expect(screen.getByText("• 0 % vs dernière exécution")).toBeInTheDocument();
    const classes = screen.getByText("Types de fiches").closest(".insight-tile")!;
    expect(within(classes as HTMLElement).getByText("2")).toBeInTheDocument();
    // confidence = min(99, 70 + round(3000/1200*10)) = 95 -> Élevée
    expect(screen.getByText("95%")).toBeInTheDocument();
    expect(screen.getByText(/^Élevée/)).toBeInTheDocument();
    expect(screen.getByText(/Dernière mise à jour : il y a 0 s/)).toBeInTheDocument();

    // Export link points at the API.
    expect(screen.getByRole("link", { name: /Exporter/ })).toHaveAttribute("href", expect.stringContaining("/export?format=jsonl"));

    // The OCR worker is released on unmount.
    unmount();
    expect(terminateOcrWorker).toHaveBeenCalledTimes(1);
  });

  it("lists files with per-kind icons, sizes and processing status", async () => {
    await renderLoaded();
    const items = screen.getAllByRole("listitem").filter((li) => li.classList.contains("upload-item"));
    expect(items).toHaveLength(4);
    const [pdf, docx, json, bin] = items.map((li) => within(li as HTMLElement));
    expect(pdf.getByText("PDF")).toBeInTheDocument();
    expect(pdf.getByText("500 B · TEXT")).toBeInTheDocument();
    expect(pdf.getByText("Traité")).toBeInTheDocument();
    expect(docx.getByText("W")).toBeInTheDocument();
    expect(docx.getByText("2.0 KB · TEXT")).toBeInTheDocument();
    expect(docx.getByText("Analysé")).toBeInTheDocument();
    expect(json.getByText("{ }")).toBeInTheDocument();
    expect(json.getByText("3.0 MB · ONTOLOGY")).toBeInTheDocument();
    expect(bin.getByText("CUS")).toBeInTheDocument();
  });

  it("shows the load error and falls back to default insights", async () => {
    getStats.mockRejectedValueOnce(new Error("stats unavailable"));
    renderPage(<OntologyBuilder />);
    expect(await errorBanner()).toHaveTextContent("stats unavailable");
    expect(screen.getByText("92%")).toBeInTheDocument();
    expect(screen.getByText(/Dernière mise à jour : —/)).toBeInTheDocument();
    expect(screen.queryByText("Fichiers importés")).toBeNull();
    // Generate is disabled without a description and without files.
    expect(screen.getByRole("button", { name: /Générer le modèle/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: /Tout retirer/ })).toBeDisabled();
  });

  it("rates a sparse graph as Medium confidence", async () => {
    getStats.mockResolvedValue({ ...stats, concepts: 10, relations: 0 });
    await renderLoaded();
    expect(screen.getByText("70%")).toBeInTheDocument();
    expect(screen.getByText(/^Moyenne/)).toBeInTheDocument();
  });

  it("formats the last-updated age in minutes, hours and days", async () => {
    const base = 1_800_000_000_000;
    const now = vi.spyOn(Date, "now").mockReturnValue(base);
    await renderLoaded();
    expect(screen.getByText(/Dernière mise à jour : il y a 0 s/)).toBeInTheDocument();
    const rerender = () => fireEvent.click(screen.getByRole("button", { name: /Exemples/ }));
    now.mockReturnValue(base + 2 * 60_000);
    rerender();
    expect(screen.getByText(/Dernière mise à jour : il y a 2 min/)).toBeInTheDocument();
    now.mockReturnValue(base + 3 * 3_600_000);
    rerender();
    expect(screen.getByText(/Dernière mise à jour : il y a 3 h/)).toBeInTheDocument();
    now.mockReturnValue(base + 4 * 86_400_000);
    rerender();
    expect(screen.getByText(/Dernière mise à jour : il y a 4 j/)).toBeInTheDocument();
  });

  it("refreshes from the toolbar buttons", async () => {
    const { user } = await renderLoaded();
    await user.click(screen.getByRole("button", { name: /Analyser les fichiers/ }));
    await waitFor(() => expect(getOntology).toHaveBeenCalledTimes(2));
    await user.click(screen.getByRole("button", { name: /Fusionner les sources/ }));
    await waitFor(() => expect(getOntology).toHaveBeenCalledTimes(3));
    await user.click(screen.getByTitle("Actualiser"));
    await waitFor(() => expect(getOntology).toHaveBeenCalledTimes(4));
  });
});

describe("OntologyBuilder — describe, generate, save", () => {
  it("offers examples that fill the description and updates the counter", async () => {
    const { user } = await renderLoaded();
    expect(screen.queryByText(/suivi des contrats/)).toBeNull();
    await user.click(screen.getByRole("button", { name: /Exemples/ }));
    const chip = screen.getByText(/suivi des contrats/);
    await user.click(chip);
    const textarea = screen.getByPlaceholderText(/Décrivez la structure du modèle/) as HTMLTextAreaElement;
    expect(textarea.value).toMatch(/^Un suivi des contrats/);
    expect(screen.getByText(`${textarea.value.length} / 4000`)).toBeInTheDocument();
    expect(screen.queryByText(/base de connaissances de recherche/)).toBeNull();
  });

  it("generates a draft from the description and the ingested files, then saves it", async () => {
    const pending = deferred<{ ontology: Ontology; model: string }>();
    generateOntology.mockReturnValueOnce(pending.promise);
    const { user } = await renderLoaded();
    const generate = screen.getByRole("button", { name: /Générer le modèle/ });
    // Files are present: the button is enabled even with an empty description.
    expect(generate).toBeEnabled();
    await user.type(screen.getByPlaceholderText(/Décrivez la structure du modèle/), "Contracts and parties");
    await user.click(generate);
    expect(screen.getByRole("button", { name: /Génération…/ })).toBeDisabled();
    const prompt = generateOntology.mock.calls[0][0];
    expect(prompt).toMatch(/^Contracts and parties\n## Source documents already ingested into the graph/);
    expect(prompt).toContain("- report.pdf (text, ingested as Report): 3 concepts, 1 relations");
    expect(prompt).toContain("- notes.docx (text, uploaded): 0 concepts, 0 relations");

    pending.resolve({ ontology: draft, model: "test" });
    expect(await infoBanner()).toHaveTextContent("Brouillon généré à partir de votre description et de 4 fichier(s). Cliquez sur Enregistrer le modèle pour l'appliquer.");
    expect(screen.getByText(/Schéma proposé/)).toBeInTheDocument();
    expect(screen.getByText(/"Contract"/)).toBeInTheDocument();

    const save = screen.getByRole("button", { name: /Enregistrer le modèle/ });
    expect(save).toBeEnabled();
    await user.click(save);
    await waitFor(() => expect(replaceOntology).toHaveBeenCalledWith(draft));
    expect(await screen.findByText("Modèle enregistré. Rien à migrer : aucune fiche ne perd son type.")).toBeInTheDocument();
    expect(screen.queryByText(/Schéma proposé/)).toBeNull();
    expect(save).toBeDisabled();
    // The page reloads after a save.
    await waitFor(() => expect(getOntology).toHaveBeenCalledTimes(2));
  });

  it("generates from the description alone when no files are ingested", async () => {
    getFiles.mockResolvedValue({ files: [] });
    const { user } = renderPage(<OntologyBuilder />);
    await screen.findByText("1.2k");
    await user.type(screen.getByPlaceholderText(/Décrivez la structure du modèle/), "  Papers  ");
    await user.click(screen.getByRole("button", { name: /Générer le modèle/ }));
    await waitFor(() => expect(generateOntology).toHaveBeenCalledWith("Papers"));
    expect(await infoBanner()).toHaveTextContent("Brouillon du modèle généré. Cliquez sur Enregistrer le modèle pour l'appliquer.");
  });

  it("reports LLM failures in the error banner", async () => {
    generateOntology.mockRejectedValueOnce(new Error("LLM quota exceeded"));
    const { user } = await renderLoaded();
    await user.type(screen.getByPlaceholderText(/Décrivez la structure du modèle/), "x");
    await user.click(screen.getByRole("button", { name: /Générer le modèle/ }));
    expect(await errorBanner()).toHaveTextContent("LLM quota exceeded");
    expect(screen.getByRole("button", { name: /Enregistrer le modèle/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: /Générer le modèle/ })).toBeEnabled();
  });

  it("keeps the draft and shows the schema-guard message when the save is refused", async () => {
    replaceOntology.mockRejectedValueOnce(
      new Error("schema change refused: concept type 'Person' still has 12 instances"),
    );
    const { user } = await renderLoaded();
    await user.type(screen.getByPlaceholderText(/Décrivez la structure du modèle/), "x");
    await user.click(screen.getByRole("button", { name: /Générer le modèle/ }));
    await screen.findByText(/Schéma proposé/);
    await user.click(screen.getByRole("button", { name: /Enregistrer le modèle/ }));
    expect(await errorBanner()).toHaveTextContent(
      "schema change refused: concept type 'Person' still has 12 instances",
    );
    // The unsaved draft survives so the user can adjust and retry.
    expect(screen.getByText(/Schéma proposé/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Enregistrer le modèle/ })).toBeEnabled();
    expect(getOntology).toHaveBeenCalledTimes(1);

    // A non-Error rejection is stringified.
    replaceOntology.mockRejectedValueOnce("plain failure");
    await user.click(screen.getByRole("button", { name: /Enregistrer le modèle/ }));
    expect(await screen.findByText("plain failure")).toBeInTheDocument();
  });
});

describe("OntologyBuilder — file records", () => {
  it("removes a single file record and reloads", async () => {
    const { user } = await renderLoaded();
    await user.click(screen.getAllByTitle("Retirer")[1]);
    await waitFor(() => expect(deleteFile).toHaveBeenCalledWith(2));
    await waitFor(() => expect(getFiles).toHaveBeenCalledTimes(2));
  });

  it("surfaces a failed removal", async () => {
    deleteFile.mockRejectedValueOnce(new Error("record locked"));
    const { user } = await renderLoaded();
    await user.click(screen.getAllByTitle("Retirer")[0]);
    expect(await errorBanner()).toHaveTextContent("record locked");
    expect(getFiles).toHaveBeenCalledTimes(1);
  });

  it("clears all records only after the confirm dialog is accepted, ignoring individual failures", async () => {
    const { user } = await renderLoaded();
    await user.click(screen.getByRole("button", { name: /Tout retirer/ }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Retirer les fichiers importés")).toBeInTheDocument();
    expect(within(dialog).getByText("Retirer 4 fichier(s) de la liste ? Les données déjà importées restent dans le graphe.")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Annuler" }));
    expect(deleteFile).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).toBeNull();

    deleteFile.mockRejectedValueOnce(new Error("gone already"));
    await user.click(screen.getByRole("button", { name: /Tout retirer/ }));
    await user.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Retirer" }));
    await waitFor(() => expect(deleteFile).toHaveBeenCalledTimes(4));
    expect(deleteFile.mock.calls.map((c) => c[0])).toEqual([1, 2, 3, 4]);
    await waitFor(() => expect(getFiles).toHaveBeenCalledTimes(2));
    expect(screen.queryByText("gone already")).toBeNull();
  });
});

describe("OntologyBuilder — ingest preview", () => {
  it("analyzes an uploaded file, lets the user review decisions and applies them", async () => {
    const { user, container } = await renderLoaded();
    upload(dropzoneInput(container), [makeFile("deal.txt", "hello")]);

    const panel = await screen.findByTestId("review-panel");
    expect(await infoBanner()).toHaveTextContent(
      "deal.txt : 2 fiche(s) et 1 lien(s) proposés à partir de 1/1 fichier(s). Vérifiez ci-dessous, puis cliquez sur Appliquer.",
    );
    expect(analyze).toHaveBeenCalledTimes(1);
    expect(analyze.mock.calls[0][0].file.name).toBe("deal.txt");
    // Header counts + the graph overlay receive the merged proposal.
    expect(screen.getByText(/2 fiche\(s\) · 1 lien\(s\) · 1 nouveau\(x\) type\(s\) de fiche/)).toBeInTheDocument();
    expect(screen.getByTestId("ontology-graph")).toHaveTextContent("Alice|2");
    // Default decisions: "exists" conflict -> merge, otherwise create_new.
    expect(JSON.parse(screen.getByTestId("decisions").textContent!)).toEqual({
      "f0/ct0": "create_new",
      "f0/c0": "merge",
      "f0/c1": "create_new",
      "f0/r0": "create_new",
    });

    await user.click(within(panel).getByText("stub-decide"));
    expect(JSON.parse(screen.getByTestId("decisions").textContent!)["f0/c0"]).toBe("skip");
    await user.click(within(panel).getByText("stub-edit-concept"));
    expect(screen.getByTestId("concept-names")).toHaveTextContent("Renamed,SLA");
    await user.click(within(panel).getByText("stub-edit-relation"));
    expect(screen.getByTestId("relation-types")).toHaveTextContent("renamed_rel");
    await user.click(within(panel).getByText("stub-bulk"));
    expect(Object.values(JSON.parse(screen.getByTestId("decisions").textContent!))).toEqual([
      "create_new",
      "create_new",
      "create_new",
      "create_new",
    ]);

    await user.click(within(panel).getByRole("button", { name: "Appliquer au graphe" }));
    await waitFor(() => expect(apply).toHaveBeenCalledTimes(1));
    const opts = apply.mock.calls[0][0];
    expect(opts.defaultAction).toBe("skip");
    expect(opts.decisions).toEqual(
      expect.arrayContaining([{ client_ref: "f0/c0", action: "create_new" }, { client_ref: "f0/r0", action: "create_new" }]),
    );
    expect(opts.proposal.concepts[0].name).toBe("Renamed");
    expect(await screen.findByText("Appliqué : 1 créé(s), 0 fusionné(s), 1 ignoré(s), 0 en échec.")).toBeInTheDocument();
    expect(screen.queryByTestId("review-panel")).toBeNull();
    const reportView = screen.getByTestId("apply-report");
    expect(reportView).toHaveTextContent("created=1");
    await waitFor(() => expect(getOntology).toHaveBeenCalledTimes(2));
    await user.click(within(reportView).getByRole("button", { name: "Fermer" }));
    expect(screen.queryByTestId("apply-report")).toBeNull();
  });

  it("discards the preview and keeps it when the apply call fails", async () => {
    const { user, container } = await renderLoaded();
    upload(dropzoneInput(container), [makeFile("deal.txt", "hello")]);
    const panel = await screen.findByTestId("review-panel");
    await user.click(within(panel).getByRole("button", { name: "Abandonner" }));
    expect(screen.queryByTestId("review-panel")).toBeNull();
    expect(screen.queryByText(/proposés à partir de/)).toBeNull();

    apply.mockRejectedValueOnce(new Error("apply exploded"));
    upload(dropzoneInput(container), [makeFile("deal.txt", "hello")]);
    await user.click(within(await screen.findByTestId("review-panel")).getByRole("button", { name: "Appliquer au graphe" }));
    expect(await errorBanner()).toHaveTextContent("apply exploded");
    expect(screen.getByTestId("review-panel")).toBeInTheDocument();
  });

  it("warns about OCR and reports the progress of the preprocessing step", async () => {
    vi.mocked(needsPreprocessing).mockReturnValue(true);
    const pending = deferred<OntologyProposal>();
    analyze.mockReturnValueOnce(pending.promise);
    const { container } = await renderLoaded();
    upload(dropzoneInput(container), [makeFile("scan.png", "img")]);
    // The status goes: OCR notice -> progress callback -> "Analyse 1/1 : scan.png…".
    expect(await screen.findByText("Analyse 1/1 : scan.png…")).toBeInTheDocument();
    expect(vi.mocked(prepareForIngest)).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: /Générer le modèle/ })).toBeDisabled();
    pending.resolve(proposal);
    await screen.findByTestId("review-panel");
    expect(screen.getByRole("button", { name: /Générer le modèle/ })).toBeEnabled();
  });

  it("shows the OCR notice while the first file is being prepared", async () => {
    vi.mocked(needsPreprocessing).mockReturnValue(true);
    const pending = deferred<File>();
    vi.mocked(prepareForIngest).mockReturnValueOnce(pending.promise);
    const { container } = await renderLoaded();
    const img = makeFile("scan.png", "img");
    upload(dropzoneInput(container), [img]);
    expect(await screen.findByText(/scan\.png : préparation de 1 fichier\(s\) — l'OCR peut prendre un moment/)).toBeInTheDocument();
    pending.resolve(img);
    await screen.findByTestId("review-panel");
  });

  it("reports analysis failures: total failure as an error, partial failure as a tail", async () => {
    analyze.mockRejectedValueOnce(new Error("LLM offline"));
    const { container } = await renderLoaded();
    upload(dropzoneInput(container), [makeFile("a.txt", "a")]);
    expect(await errorBanner()).toHaveTextContent("a.txt: LLM offline");
    expect(screen.queryByTestId("review-panel")).toBeNull();

    // Two files in a ZIP, the first rejects with a non-Error value.
    analyze.mockRejectedValueOnce("boom");
    const zip = await zipFile("pack.zip", { "docs/a.txt": "a", "docs/b.txt": "b" });
    upload(dropzoneInput(container), [zip]);
    await screen.findByTestId("review-panel");
    expect(await infoBanner()).toHaveTextContent(
      "Archive pack.zip : 2 fiche(s) et 1 lien(s) proposés à partir de 1/2 fichier(s) · 1 en échec. Vérifiez ci-dessous, puis cliquez sur Appliquer.",
    );
    expect(screen.getByText("a.txt: boom")).toBeInTheDocument();
  });

  it("expands ZIP archives (nested too), skipping directories, hidden and system files", async () => {
    const inner = await zipFile("inner.zip", { "deep/inner.md": "# hi" });
    const zip = await zipFile(
      "bundle.zip",
      {
        "docs/a.txt": "a",
        "__MACOSX/._a.txt": "junk",
        "docs/.hidden": "x",
        "docs/Thumbs.db": "x",
        "docs/inner.zip": new Uint8Array(await inner.arrayBuffer()),
      },
      ["empty-dir"],
    );
    const { container } = await renderLoaded();
    upload(dropzoneInput(container), [zip]);
    await screen.findByTestId("review-panel");
    expect(analyze.mock.calls.map((c) => c[0].file.name).sort()).toEqual(["a.txt", "inner.md"]);
    expect(await infoBanner()).toHaveTextContent("Archive bundle.zip : 2 fiche(s)");
  });

  it("tells the user when an archive holds nothing ingestible, and when it cannot be read", async () => {
    const { container } = await renderLoaded();
    upload(dropzoneInput(container), [await zipFile("empty.zip", { ".DS_Store": "x" })]);
    expect(await screen.findByText("Archive empty.zip : aucun fichier importable trouvé.")).toBeInTheDocument();
    expect(analyze).not.toHaveBeenCalled();

    upload(dropzoneInput(container), [makeFile("broken.zip", "definitely not a zip")]);
    expect(await errorBanner()).toBeInTheDocument();
    expect(screen.queryByText(/aucun fichier importable/)).toBeNull();
  });

  it("scans a connected folder, filtering by extension and expanding archives", async () => {
    const zip = withRelPath(await zipFile("pack.zip", { "b.md": "b" }), "proj/pack.zip");
    const { container } = await renderLoaded();
    upload(folderInput(container), [
      withRelPath(makeFile("a.txt", "a"), "proj/a.txt"),
      withRelPath(makeFile("Thumbs.db", "x"), "proj/Thumbs.db"),
      withRelPath(makeFile("tool.exe", "x"), "proj/tool.exe"),
      withRelPath(makeFile("ignored.txt", "x"), "proj/.git/ignored.txt"),
      zip,
    ]);
    await screen.findByTestId("review-panel");
    expect(analyze.mock.calls.map((c) => c[0].file.name).sort()).toEqual(["a.txt", "b.md", "ignored.txt"]);
    expect(await infoBanner()).toHaveTextContent("Dossier proj : 2 fiche(s)");
    expect(folderInput(container).value).toBe("");
  });

  it("falls back to a generic folder name and ignores empty selections", async () => {
    const { container } = await renderLoaded();
    upload(folderInput(container), []);
    expect(analyze).not.toHaveBeenCalled();

    upload(folderInput(container), [withRelPath(makeFile("a.txt", "a"), undefined)]);
    await screen.findByTestId("review-panel");
    expect(await infoBanner()).toHaveTextContent("Dossier dossier : 2 fiche(s)");
  });

  it("reports an unreadable archive inside a folder", async () => {
    const { container } = await renderLoaded();
    upload(folderInput(container), [withRelPath(makeFile("bad.zip", "nope"), "proj/bad.zip")]);
    expect(await errorBanner()).toBeInTheDocument();
    expect(analyze).not.toHaveBeenCalled();
  });
});

describe("OntologyBuilder — what a model change touches", () => {
  it("counts the sheets whose type the new model drops and refuses to save with what to do", async () => {
    listConcepts.mockImplementation(async (params) => ({
      total: params?.type === "Person" ? 12 : 0,
      concepts: [],
      next_cursor: null,
    }));
    const { user } = await renderLoaded();
    await user.type(screen.getByPlaceholderText(/Décrivez la structure du modèle/), "x");
    await user.click(screen.getByRole("button", { name: /Générer le modèle/ }));
    await screen.findByText(/Schéma proposé/);
    await user.click(screen.getByRole("button", { name: /Enregistrer le modèle/ }));
    expect(await errorBanner()).toHaveTextContent(
      "12 fiche(s) concernée(s) : Person (12). Un type encore utilisé ne peut pas être retiré : réaffectez ou supprimez ces fiches d'abord.",
    );
    expect(listConcepts).toHaveBeenCalledWith({ type: "Person", limit: 1, include_subtypes: false });
    expect(listConcepts).toHaveBeenCalledWith({ type: "Org", limit: 1, include_subtypes: false });
    expect(replaceOntology).not.toHaveBeenCalled();
    expect(screen.getByText(/Schéma proposé/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Enregistrer le modèle/ })).toBeEnabled();
  });

  it("lets the server decide when the count cannot be made", async () => {
    listConcepts.mockRejectedValue(new Error("HTTP 429"));
    const { user } = await renderLoaded();
    await user.type(screen.getByPlaceholderText(/Décrivez la structure du modèle/), "x");
    await user.click(screen.getByRole("button", { name: /Générer le modèle/ }));
    await screen.findByText(/Schéma proposé/);
    await user.click(screen.getByRole("button", { name: /Enregistrer le modèle/ }));
    await waitFor(() => expect(replaceOntology).toHaveBeenCalledWith(draft));
    expect(await screen.findByText("Modèle enregistré.")).toBeInTheDocument();
  });
});
