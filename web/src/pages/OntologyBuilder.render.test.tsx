// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
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
    expect(screen.getByText("↑ 12% vs last run")).toBeInTheDocument();
    expect(screen.getByText("↓ 8% vs last run")).toBeInTheDocument();
    expect(screen.getByText("• 0% vs last run")).toBeInTheDocument();
    const classes = screen.getByText("Classes").closest(".insight-tile")!;
    expect(within(classes as HTMLElement).getByText("2")).toBeInTheDocument();
    // confidence = min(99, 70 + round(3000/1200*10)) = 95 -> High
    expect(screen.getByText("95%")).toBeInTheDocument();
    expect(screen.getByText(/^High/)).toBeInTheDocument();
    expect(screen.getByText(/Last updated: 0s ago/)).toBeInTheDocument();

    // Export link points at the API.
    expect(screen.getByRole("link", { name: /Export/ })).toHaveAttribute("href", expect.stringContaining("/export?format=jsonl"));

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
    expect(pdf.getByText("Processed")).toBeInTheDocument();
    expect(docx.getByText("W")).toBeInTheDocument();
    expect(docx.getByText("2.0 KB · TEXT")).toBeInTheDocument();
    expect(docx.getByText("Analyzed")).toBeInTheDocument();
    expect(json.getByText("{ }")).toBeInTheDocument();
    expect(json.getByText("3.0 MB · ONTOLOGY")).toBeInTheDocument();
    expect(bin.getByText("CUS")).toBeInTheDocument();
  });

  it("shows the load error and falls back to default insights", async () => {
    getStats.mockRejectedValueOnce(new Error("stats unavailable"));
    renderPage(<OntologyBuilder />);
    expect(await errorBanner()).toHaveTextContent("stats unavailable");
    expect(screen.getByText("92%")).toBeInTheDocument();
    expect(screen.getByText(/Last updated: —/)).toBeInTheDocument();
    expect(screen.queryByText("Uploaded Files")).toBeNull();
    // Generate is disabled without a description and without files.
    expect(screen.getByRole("button", { name: /Generate Ontology/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: /Clear All/ })).toBeDisabled();
  });

  it("rates a sparse graph as Medium confidence", async () => {
    getStats.mockResolvedValue({ ...stats, concepts: 10, relations: 0 });
    await renderLoaded();
    expect(screen.getByText("70%")).toBeInTheDocument();
    expect(screen.getByText(/^Medium/)).toBeInTheDocument();
  });

  it("formats the last-updated age in minutes, hours and days", async () => {
    const base = 1_800_000_000_000;
    const now = vi.spyOn(Date, "now").mockReturnValue(base);
    await renderLoaded();
    expect(screen.getByText(/Last updated: 0s ago/)).toBeInTheDocument();
    const rerender = () => fireEvent.click(screen.getByRole("button", { name: /Examples/ }));
    now.mockReturnValue(base + 2 * 60_000);
    rerender();
    expect(screen.getByText(/Last updated: 2 min ago/)).toBeInTheDocument();
    now.mockReturnValue(base + 3 * 3_600_000);
    rerender();
    expect(screen.getByText(/Last updated: 3 h ago/)).toBeInTheDocument();
    now.mockReturnValue(base + 4 * 86_400_000);
    rerender();
    expect(screen.getByText(/Last updated: 4 d ago/)).toBeInTheDocument();
  });

  it("refreshes from the toolbar buttons", async () => {
    const { user } = await renderLoaded();
    await user.click(screen.getByRole("button", { name: /Analyze Files/ }));
    await waitFor(() => expect(getOntology).toHaveBeenCalledTimes(2));
    await user.click(screen.getByRole("button", { name: /Merge Sources/ }));
    await waitFor(() => expect(getOntology).toHaveBeenCalledTimes(3));
    await user.click(screen.getByTitle("Refresh"));
    await waitFor(() => expect(getOntology).toHaveBeenCalledTimes(4));
  });
});

describe("OntologyBuilder — describe, generate, save", () => {
  it("offers examples that fill the description and updates the counter", async () => {
    const { user } = await renderLoaded();
    expect(screen.queryByText(/contract management system/)).toBeNull();
    await user.click(screen.getByRole("button", { name: /Examples/ }));
    const chip = screen.getByText(/contract management system/);
    await user.click(chip);
    const textarea = screen.getByPlaceholderText(/Describe the ontology structure/) as HTMLTextAreaElement;
    expect(textarea.value).toMatch(/^A contract management system/);
    expect(screen.getByText(`${textarea.value.length} / 4000`)).toBeInTheDocument();
    expect(screen.queryByText(/research knowledge base/)).toBeNull();
  });

  it("generates a draft from the description and the ingested files, then saves it", async () => {
    const pending = deferred<{ ontology: Ontology; model: string }>();
    generateOntology.mockReturnValueOnce(pending.promise);
    const { user } = await renderLoaded();
    const generate = screen.getByRole("button", { name: /Generate Ontology/ });
    // Files are present: the button is enabled even with an empty description.
    expect(generate).toBeEnabled();
    await user.type(screen.getByPlaceholderText(/Describe the ontology structure/), "Contracts and parties");
    await user.click(generate);
    expect(screen.getByRole("button", { name: /Generating…/ })).toBeDisabled();
    const prompt = generateOntology.mock.calls[0][0];
    expect(prompt).toMatch(/^Contracts and parties\n## Source documents already ingested into the graph/);
    expect(prompt).toContain("- report.pdf (text, ingested as Report): 3 concepts, 1 relations");
    expect(prompt).toContain("- notes.docx (text, uploaded): 0 concepts, 0 relations");

    pending.resolve({ ontology: draft, model: "test" });
    expect(await infoBanner()).toHaveTextContent("Draft generated from your description and 4 file(s). Click Save Ontology to apply.");
    expect(screen.getByText(/Proposed schema/)).toBeInTheDocument();
    expect(screen.getByText(/"Contract"/)).toBeInTheDocument();

    const save = screen.getByRole("button", { name: /Save Ontology/ });
    expect(save).toBeEnabled();
    await user.click(save);
    await waitFor(() => expect(replaceOntology).toHaveBeenCalledWith(draft));
    expect(await screen.findByText("Ontology saved.")).toBeInTheDocument();
    expect(screen.queryByText(/Proposed schema/)).toBeNull();
    expect(save).toBeDisabled();
    // The page reloads after a save.
    await waitFor(() => expect(getOntology).toHaveBeenCalledTimes(2));
  });

  it("generates from the description alone when no files are ingested", async () => {
    getFiles.mockResolvedValue({ files: [] });
    const { user } = renderPage(<OntologyBuilder />);
    await screen.findByText("1.2k");
    await user.type(screen.getByPlaceholderText(/Describe the ontology structure/), "  Papers  ");
    await user.click(screen.getByRole("button", { name: /Generate Ontology/ }));
    await waitFor(() => expect(generateOntology).toHaveBeenCalledWith("Papers"));
    expect(await infoBanner()).toHaveTextContent("Draft ontology generated. Click Save Ontology to apply.");
  });

  it("reports LLM failures in the error banner", async () => {
    generateOntology.mockRejectedValueOnce(new Error("LLM quota exceeded"));
    const { user } = await renderLoaded();
    await user.type(screen.getByPlaceholderText(/Describe the ontology structure/), "x");
    await user.click(screen.getByRole("button", { name: /Generate Ontology/ }));
    expect(await errorBanner()).toHaveTextContent("LLM quota exceeded");
    expect(screen.getByRole("button", { name: /Save Ontology/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: /Generate Ontology/ })).toBeEnabled();
  });

  it("keeps the draft and shows the schema-guard message when the save is refused", async () => {
    replaceOntology.mockRejectedValueOnce(
      new Error("schema change refused: concept type 'Person' still has 12 instances"),
    );
    const { user } = await renderLoaded();
    await user.type(screen.getByPlaceholderText(/Describe the ontology structure/), "x");
    await user.click(screen.getByRole("button", { name: /Generate Ontology/ }));
    await screen.findByText(/Proposed schema/);
    await user.click(screen.getByRole("button", { name: /Save Ontology/ }));
    expect(await errorBanner()).toHaveTextContent(
      "schema change refused: concept type 'Person' still has 12 instances",
    );
    // The unsaved draft survives so the user can adjust and retry.
    expect(screen.getByText(/Proposed schema/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Save Ontology/ })).toBeEnabled();
    expect(getOntology).toHaveBeenCalledTimes(1);

    // A non-Error rejection is stringified.
    replaceOntology.mockRejectedValueOnce("plain failure");
    await user.click(screen.getByRole("button", { name: /Save Ontology/ }));
    expect(await screen.findByText("plain failure")).toBeInTheDocument();
  });
});

describe("OntologyBuilder — file records", () => {
  it("removes a single file record and reloads", async () => {
    const { user } = await renderLoaded();
    await user.click(screen.getAllByTitle("Remove")[1]);
    await waitFor(() => expect(deleteFile).toHaveBeenCalledWith(2));
    await waitFor(() => expect(getFiles).toHaveBeenCalledTimes(2));
  });

  it("surfaces a failed removal", async () => {
    deleteFile.mockRejectedValueOnce(new Error("record locked"));
    const { user } = await renderLoaded();
    await user.click(screen.getAllByTitle("Remove")[0]);
    expect(await errorBanner()).toHaveTextContent("record locked");
    expect(getFiles).toHaveBeenCalledTimes(1);
  });

  it("clears all records only after the confirm dialog is accepted, ignoring individual failures", async () => {
    const { user } = await renderLoaded();
    await user.click(screen.getByRole("button", { name: /Clear All/ }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Remove file records")).toBeInTheDocument();
    expect(within(dialog).getByText("Remove 4 file records? Ingested data stays in the graph.")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(deleteFile).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).toBeNull();

    deleteFile.mockRejectedValueOnce(new Error("gone already"));
    await user.click(screen.getByRole("button", { name: /Clear All/ }));
    await user.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Remove" }));
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
      "deal.txt: previewed 2 concept(s) and 1 relation(s) from 1/1 file(s). Review below, then click Apply.",
    );
    expect(analyze).toHaveBeenCalledTimes(1);
    expect(analyze.mock.calls[0][0].file.name).toBe("deal.txt");
    // Header counts + the graph overlay receive the merged proposal.
    expect(screen.getByText(/2 concept\(s\) · 1 relation\(s\) · 1 new concept type\(s\)/)).toBeInTheDocument();
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

    await user.click(within(panel).getByRole("button", { name: "Apply to graph" }));
    await waitFor(() => expect(apply).toHaveBeenCalledTimes(1));
    const opts = apply.mock.calls[0][0];
    expect(opts.defaultAction).toBe("skip");
    expect(opts.decisions).toEqual(
      expect.arrayContaining([{ client_ref: "f0/c0", action: "create_new" }, { client_ref: "f0/r0", action: "create_new" }]),
    );
    expect(opts.proposal.concepts[0].name).toBe("Renamed");
    expect(await screen.findByText("Applied: 1 created, 0 merged, 1 skipped, 0 failed.")).toBeInTheDocument();
    expect(screen.queryByTestId("review-panel")).toBeNull();
    const reportView = screen.getByTestId("apply-report");
    expect(reportView).toHaveTextContent("created=1");
    await waitFor(() => expect(getOntology).toHaveBeenCalledTimes(2));
    await user.click(within(reportView).getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByTestId("apply-report")).toBeNull();
  });

  it("discards the preview and keeps it when the apply call fails", async () => {
    const { user, container } = await renderLoaded();
    upload(dropzoneInput(container), [makeFile("deal.txt", "hello")]);
    const panel = await screen.findByTestId("review-panel");
    await user.click(within(panel).getByRole("button", { name: "Discard" }));
    expect(screen.queryByTestId("review-panel")).toBeNull();
    expect(screen.queryByText(/previewed/)).toBeNull();

    apply.mockRejectedValueOnce(new Error("apply exploded"));
    upload(dropzoneInput(container), [makeFile("deal.txt", "hello")]);
    await user.click(within(await screen.findByTestId("review-panel")).getByRole("button", { name: "Apply to graph" }));
    expect(await errorBanner()).toHaveTextContent("apply exploded");
    expect(screen.getByTestId("review-panel")).toBeInTheDocument();
  });

  it("warns about OCR and reports the progress of the preprocessing step", async () => {
    vi.mocked(needsPreprocessing).mockReturnValue(true);
    const pending = deferred<OntologyProposal>();
    analyze.mockReturnValueOnce(pending.promise);
    const { container } = await renderLoaded();
    upload(dropzoneInput(container), [makeFile("scan.png", "img")]);
    // The status goes: OCR notice -> progress callback -> "Analyzing 1/1: scan.png…".
    expect(await screen.findByText("Analyzing 1/1: scan.png…")).toBeInTheDocument();
    expect(vi.mocked(prepareForIngest)).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: /Generate Ontology/ })).toBeDisabled();
    pending.resolve(proposal);
    await screen.findByTestId("review-panel");
    expect(screen.getByRole("button", { name: /Generate Ontology/ })).toBeEnabled();
  });

  it("shows the OCR notice while the first file is being prepared", async () => {
    vi.mocked(needsPreprocessing).mockReturnValue(true);
    const pending = deferred<File>();
    vi.mocked(prepareForIngest).mockReturnValueOnce(pending.promise);
    const { container } = await renderLoaded();
    const img = makeFile("scan.png", "img");
    upload(dropzoneInput(container), [img]);
    expect(await screen.findByText(/scan\.png: preparing 1 file\(s\) — OCR may take a moment/)).toBeInTheDocument();
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
      "Archive pack.zip: previewed 2 concept(s) and 1 relation(s) from 1/2 file(s) · 1 failed. Review below, then click Apply.",
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
    expect(await infoBanner()).toHaveTextContent("Archive bundle.zip: previewed");
  });

  it("tells the user when an archive holds nothing ingestible, and when it cannot be read", async () => {
    const { container } = await renderLoaded();
    upload(dropzoneInput(container), [await zipFile("empty.zip", { ".DS_Store": "x" })]);
    expect(await screen.findByText("Archive empty.zip: no ingestible files found.")).toBeInTheDocument();
    expect(analyze).not.toHaveBeenCalled();

    upload(dropzoneInput(container), [makeFile("broken.zip", "definitely not a zip")]);
    expect(await errorBanner()).toBeInTheDocument();
    expect(screen.queryByText(/no ingestible files/)).toBeNull();
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
    expect(await infoBanner()).toHaveTextContent("Folder proj: previewed");
    expect(folderInput(container).value).toBe("");
  });

  it("falls back to a generic folder name and ignores empty selections", async () => {
    const { container } = await renderLoaded();
    upload(folderInput(container), []);
    expect(analyze).not.toHaveBeenCalled();

    upload(folderInput(container), [withRelPath(makeFile("a.txt", "a"), undefined)]);
    await screen.findByTestId("review-panel");
    expect(await infoBanner()).toHaveTextContent("Folder folder: previewed");
  });

  it("reports an unreadable archive inside a folder", async () => {
    const { container } = await renderLoaded();
    upload(folderInput(container), [withRelPath(makeFile("bad.zip", "nope"), "proj/bad.zip")]);
    expect(await errorBanner()).toBeInTheDocument();
    expect(analyze).not.toHaveBeenCalled();
  });
});
