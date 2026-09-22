// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Rendering tests of the LLM-assisted ingest wizard: upload step (provider
// mirrored from Settings), analyze (through the mocked extractor and the
// real `ingestApi` client on a stubbed `fetch`), review, apply, the error
// step, and the sessionStorage draft.

import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import IngestWizard from "./IngestWizard";
import { flushPromises, makeFile, renderPage, screen, waitFor, within } from "../test/render";
import type { Settings } from "../api";
import type { ApplyReport, OntologyProposal } from "../lib/proposalTypes";
import type { ReviewPanelProps } from "../components/IngestReview";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return { ...actual, getSettings: vi.fn() };
});
// pdfjs / tesseract / jszip have their own tests: the extractor is a stub
// that records its input and can hand back a synthesized text file.
vi.mock("../lib/extractText", () => ({
  prepareForIngest: vi.fn(),
  terminateOcrWorker: vi.fn(() => Promise.resolve()),
}));
// The real review panel, plus a handle on `onEditRelation`, which the panel
// itself never calls (relations have no inline editor yet).
vi.mock("../components/IngestReview", async () => {
  const actual = await vi.importActual<typeof import("../components/IngestReview")>(
    "../components/IngestReview",
  );
  return {
    ...actual,
    ReviewPanel: (props: ReviewPanelProps) => (
      <>
        <actual.ReviewPanel {...props} />
        <button onClick={() => props.onEditRelation("r1", { relation_type: "supersedes" })}>
          edit relation
        </button>
      </>
    ),
  };
});

import * as api from "../api";
import * as extract from "../lib/extractText";

const mockedSettings = api.getSettings as unknown as Mock;
const mockedPrepare = extract.prepareForIngest as unknown as Mock;
const mockedTerminate = extract.terminateOcrWorker as unknown as Mock;

const STORAGE_KEY = "ingest.draft.v1";

function settingsWith(provider: string): Settings {
  return { llm: { active_provider: provider } } as unknown as Settings;
}

const proposal: OntologyProposal = {
  source: { name: "doc.txt", encoding: "utf-8" },
  language: { code: "en", script: "Latn", confidence: 0.99 },
  concept_types: [
    {
      client_ref: "ct1",
      name: "Contract",
      conflict: { kind: { kind: "exists", existing_id: "3", existing_display: "Contract" }, summary: "exists" },
    },
  ],
  relation_types: [{ client_ref: "rt1", name: "contains", domain: "Contract", range: "Clause" }],
  concepts: [
    {
      client_ref: "c1",
      concept_type: "Contract",
      name: "ACME master",
      conflict: { kind: { kind: "type_mismatch", existing_type: "Company", existing_id: "9" }, summary: "mismatch" },
    },
    { client_ref: "c2", concept_type: "Clause", name: "Termination" },
  ],
  relations: [
    {
      client_ref: "r1",
      relation_type: "contains",
      source_ref: "c1",
      target_ref: "c9",
      conflict: { kind: { kind: "dangling_ref", missing_ref: "c9" }, summary: "missing" },
    },
  ],
  rules: [{ client_ref: "ru1", rule_type: "constraint", name: "Notice" }],
  actions: [{ client_ref: "a1", action_type: "notify", name: "Warn", subject_ref: "c1" }],
};

const report: ApplyReport = {
  concept_types: [["ct1", { status: "merged", id: "3" }]],
  relation_types: [],
  concepts: [["c1", { status: "created", id: "10" }]],
  relations: [["r1", { status: "skipped" }]],
  rules: [],
  actions: [],
  created: 1,
  merged: 1,
  skipped: 1,
  failed: 0,
};

/** Route the stubbed `fetch` by URL suffix. Unknown URLs stay rejected. */
type Route = (init: RequestInit | undefined) => Response | Promise<Response>;
function routeFetch(routes: Record<string, Route>): Mock {
  const f = globalThis.fetch as unknown as Mock;
  f.mockImplementation((url: string, init?: RequestInit) => {
    for (const [suffix, handler] of Object.entries(routes)) {
      if (url.endsWith(suffix)) return Promise.resolve(handler(init));
    }
    return Promise.reject(new Error(`unexpected fetch ${url}`));
  });
  return f;
}

/** A response the test releases by hand, to observe the in-flight step. */
function deferred(): { route: Route; release: (r: Response) => void } {
  let release!: (r: Response) => void;
  const pending = new Promise<Response>((r) => (release = r));
  return { route: () => pending, release };
}

function json(body: unknown, status = 200, statusText = "OK"): Response {
  return new Response(JSON.stringify(body), {
    status,
    statusText,
    headers: { "content-type": "application/json" },
  });
}

async function pickAndAnalyze(user: ReturnType<typeof renderPage>["user"], file: File) {
  const analyze = screen.getByRole("button", { name: "Analyze document" });
  expect(analyze).toBeDisabled();
  await user.upload(screen.getByLabelText("Document"), file);
  expect(analyze).toBeEnabled();
  await user.click(analyze);
}

beforeEach(() => {
  mockedSettings.mockResolvedValue(settingsWith("openai"));
  // The real extractor reports progress; the wizard turns each step into a
  // toast (`toast.info`), so the mock reports one step too.
  mockedPrepare.mockImplementation(
    (file: File, opts?: { onProgress?: (p: { status: string }) => void }) => {
      opts?.onProgress?.({ status: `Parsing ${file.name}…` });
      return Promise.resolve(file);
    },
  );
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("IngestWizard — upload step", () => {
  it("mirrors the provider configured in Settings", async () => {
    renderPage(<IngestWizard />);
    expect(screen.getByRole("heading", { name: "LLM-assisted ingest" })).toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole("combobox")).toHaveValue("openai"));
    expect(screen.getByRole("button", { name: "Analyze document" })).toBeDisabled();
  });

  it("stays on the server default when Settings are unreachable", async () => {
    mockedSettings.mockRejectedValue(new Error("offline"));
    renderPage(<IngestWizard />);
    await flushPromises();
    expect(screen.getByRole("combobox")).toHaveValue("default");
  });

  it("never overwrites a provider the user picked, even if Settings answer later", async () => {
    let resolve!: (s: Settings) => void;
    mockedSettings.mockReturnValue(new Promise<Settings>((r) => (resolve = r)));
    const { user } = renderPage(<IngestWizard />);
    await user.selectOptions(screen.getByRole("combobox"), "anthropic");
    resolve(settingsWith("openai"));
    await flushPromises();
    expect(screen.getByRole("combobox")).toHaveValue("anthropic");
  });

  it("ignores an unknown provider coming from Settings", async () => {
    mockedSettings.mockResolvedValue(settingsWith("mystery"));
    renderPage(<IngestWizard />);
    await flushPromises();
    expect(screen.getByRole("combobox")).toHaveValue("default");
  });

  it("releases the OCR worker on unmount", async () => {
    const { unmount } = renderPage(<IngestWizard />);
    await flushPromises();
    unmount();
    expect(mockedTerminate).toHaveBeenCalledTimes(1);
  });
});

describe("IngestWizard — analyze", () => {
  it("prepares the file, posts it with the options and seeds the review decisions", async () => {
    const analyze = deferred();
    const fetchMock = routeFetch({ "/ingest/analyze": analyze.route });
    const { user } = renderPage(<IngestWizard />);
    await waitFor(() => expect(screen.getByRole("combobox")).toHaveValue("openai"));
    await user.type(screen.getByPlaceholderText(/gpt-4o-mini/), "  gpt-4o-mini ");
    await user.type(screen.getByPlaceholderText(/auto-detect/), "fr");

    const file = makeFile("doc.txt", "Some contract text.");
    await pickAndAnalyze(user, file);
    expect(await screen.findByText(/Calling the LLM/)).toBeInTheDocument();
    analyze.release(json(proposal));

    expect(await screen.findByText("New concept types (1)")).toBeInTheDocument();
    expect(mockedPrepare).toHaveBeenCalledWith(file, expect.objectContaining({ onProgress: expect.any(Function) }));

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toMatch(/\/ingest\/analyze$/);
    expect(init.method).toBe("POST");
    const form = init.body as FormData;
    expect((form.get("file") as File).name).toBe("doc.txt");
    expect(form.get("provider")).toBe("openai");
    expect(form.get("model")).toBe("gpt-4o-mini");
    expect(form.get("language_hint")).toBe("fr");

    // exists → merge, type_mismatch → create_new, dangling_ref → skip, none → create_new.
    expect(screen.getByText("Create:").parentElement).toHaveTextContent("Create: 5");
    expect(screen.getByText("Merge:").parentElement).toHaveTextContent("Merge: 1");
    expect(screen.getByText("Skip:").parentElement).toHaveTextContent("Skip: 1");
    // The draft is mirrored to sessionStorage while reviewing.
    const draft = JSON.parse(window.sessionStorage.getItem(STORAGE_KEY)!);
    expect(draft.decisions).toEqual({ ct1: "merge", rt1: "create_new", c1: "create_new", c2: "create_new", r1: "skip", ru1: "create_new", a1: "create_new" });
  });

  it.each([
    ["scan.pdf", "application/pdf", "scan.txt"],
    ["data.xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", "data.xlsx"],
    ["rows.jsonl", "application/x-ndjson", "rows.jsonl"],
  ])("sends whatever the extractor returns for %s", async (name, type, sentName) => {
    mockedPrepare.mockImplementation((file: File, opts: { onProgress?: (p: { status: string }) => void }) => {
      opts.onProgress?.({ status: `extracting ${file.name}` });
      if (file.name.endsWith(".pdf")) return Promise.resolve(makeFile("scan.txt", "[source: scan.pdf]\nocr text"));
      return Promise.resolve(file);
    });
    const fetchMock = routeFetch({ "/ingest/analyze": () => json(proposal) });
    const { user } = renderPage(<IngestWizard />);
    await pickAndAnalyze(user, makeFile(name, "payload", type));
    await screen.findByText("Concepts (2)");
    const form = (fetchMock.mock.calls[0] as [string, RequestInit])[1].body as FormData;
    expect((form.get("file") as File).name).toBe(sentName);
    // No model / hint typed: the optional fields are left out of the form.
    expect(form.get("model")).toBeNull();
    expect(form.get("language_hint")).toBeNull();
  });

  it("shows the server's 422 detail and lets the user start over", async () => {
    routeFetch({
      "/ingest/analyze": () =>
        new Response("no text could be extracted", { status: 422, statusText: "Unprocessable Entity" }),
    });
    const { user } = renderPage(<IngestWizard />);
    await pickAndAnalyze(user, makeFile("empty.txt", ""));
    expect(await screen.findByText("Something went wrong")).toBeInTheDocument();
    expect(
      screen.getByText("analyze failed: 422 Unprocessable Entity — no text could be extracted"),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Start over" }));
    // Back on the upload step, with the file cleared.
    expect(screen.getByRole("button", { name: "Analyze document" })).toBeDisabled();
    expect((screen.getByLabelText("Document") as HTMLInputElement).value).toBe("");
  });

  it("explains an unreachable API instead of a bare fetch error", async () => {
    // Default stub: fetch rejects.
    const { user } = renderPage(<IngestWizard />);
    await pickAndAnalyze(user, makeFile("doc.txt", "x"));
    const pre = await screen.findByText(/API injoignable/);
    expect(pre).toHaveTextContent("network access is disabled in unit tests");
  });

  it("stringifies a non-Error failure of the extractor", async () => {
    mockedPrepare.mockRejectedValue("worker crashed");
    const { user } = renderPage(<IngestWizard />);
    await pickAndAnalyze(user, makeFile("scan.pdf", "x", "application/pdf"));
    expect(await screen.findByText("worker crashed")).toBeInTheDocument();
  });
});

describe("IngestWizard — review and apply", () => {
  async function reachReview(routes: Record<string, Route> = {}) {
    const fetchMock = routeFetch({ "/ingest/analyze": () => json(proposal), ...routes });
    const page = renderPage(<IngestWizard />);
    await pickAndAnalyze(page.user, makeFile("doc.txt", "text"));
    await screen.findByText("Concepts (2)");
    return { ...page, fetchMock };
  }

  it("applies the reviewed proposal (edits, decisions, bulk) and shows the report", async () => {
    const apply = deferred();
    const { user, fetchMock } = await reachReview({ "/ingest/apply": apply.route });

    await user.click(screen.getByRole("button", { name: "Accept all" }));
    expect(screen.getByText("Create:").parentElement).toHaveTextContent("Create: 7");
    // Per-item override on the Clause concept, then an inline rename.
    const clause = screen.getByDisplayValue("Termination").closest("tr")!;
    await user.selectOptions(within(clause).getByRole("combobox"), "skip");
    await user.type(screen.getByDisplayValue("ACME master"), " v2");
    expect(screen.getByDisplayValue("ACME master v2")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "edit relation" }));
    expect(screen.getByText("supersedes")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Apply to graph" }));
    expect(await screen.findByText(/Writing accepted items/)).toBeInTheDocument();
    apply.release(json(report));
    expect(await screen.findByText("Apply complete")).toBeInTheDocument();

    const applyCall = fetchMock.mock.calls.find(([u]) => (u as string).endsWith("/ingest/apply"))!;
    const body = JSON.parse((applyCall[1] as RequestInit).body as string);
    expect(body.default_action).toBe("skip");
    expect(body.strict).toBe(false);
    expect(body.proposal.concepts[0].name).toBe("ACME master v2");
    expect(body.proposal.relations[0].relation_type).toBe("supersedes");
    expect(body.decisions).toEqual([
      { client_ref: "ct1", action: "create_new" },
      { client_ref: "rt1", action: "create_new" },
      { client_ref: "c1", action: "create_new" },
      { client_ref: "c2", action: "skip" },
      { client_ref: "r1", action: "create_new" },
      { client_ref: "ru1", action: "create_new" },
      { client_ref: "a1", action: "create_new" },
    ]);

    expect(screen.getByText("merged #3")).toBeInTheDocument();
    expect(window.sessionStorage.getItem(STORAGE_KEY)).toBeNull();
    await user.click(screen.getByRole("button", { name: "Ingest another document" }));
    expect(screen.getByRole("button", { name: "Analyze document" })).toBeDisabled();
  });

  it("goes to the error step when apply fails", async () => {
    const { user } = await reachReview({
      "/ingest/apply": () => json({ error: "boom" }, 500, "Internal Server Error"),
    });
    await user.click(screen.getByRole("button", { name: "Skip all" }));
    expect(screen.getByText("Skip:").parentElement).toHaveTextContent("Skip: 7");
    await user.click(screen.getByRole("button", { name: "Apply to graph" }));
    expect(await screen.findByText("apply failed: 500 Internal Server Error")).toBeInTheDocument();
  });

  it("cancelling the review clears the draft and returns to upload", async () => {
    const { user } = await reachReview();
    expect(window.sessionStorage.getItem(STORAGE_KEY)).not.toBeNull();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(window.sessionStorage.getItem(STORAGE_KEY)).toBeNull();
    expect(screen.getByRole("button", { name: "Analyze document" })).toBeInTheDocument();
  });
});

describe("IngestWizard — draft persistence", () => {
  it("restores an in-flight review from sessionStorage and keeps it in sync", async () => {
    window.sessionStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ proposal, decisions: { ct1: "merge", c1: "skip" } }),
    );
    const { user } = renderPage(<IngestWizard />);
    expect(await screen.findByText("Concepts (2)")).toBeInTheDocument();
    expect(screen.getByText("Merge:").parentElement).toHaveTextContent("Merge: 1");
    await user.click(screen.getByRole("button", { name: "Skip all" }));
    const saved = JSON.parse(window.sessionStorage.getItem(STORAGE_KEY)!);
    expect(Object.values(saved.decisions)).toEqual(Array(7).fill("skip"));
  });

  it("ignores a corrupt draft", async () => {
    window.sessionStorage.setItem(STORAGE_KEY, "{not json");
    renderPage(<IngestWizard />);
    await flushPromises();
    expect(screen.getByRole("button", { name: "Analyze document" })).toBeInTheDocument();
  });

  it("survives a storage that throws on write and on remove", async () => {
    window.sessionStorage.setItem(STORAGE_KEY, JSON.stringify({ proposal, decisions: {} }));
    const setItem = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("quota");
    });
    const removeItem = vi.spyOn(Storage.prototype, "removeItem").mockImplementation(() => {
      throw new Error("locked");
    });
    const { user } = renderPage(<IngestWizard />);
    expect(await screen.findByText("Concepts (2)")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Accept all" }));
    expect(setItem).toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(removeItem).toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Analyze document" })).toBeInTheDocument();
  });
});
