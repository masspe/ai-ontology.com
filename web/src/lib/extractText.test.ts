// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// `tesseract.js` and `pdfjs-dist` are heavy WASM/worker packages; they are
// replaced by in-memory fakes so the tests stay fast, offline and
// deterministic. DOCX handling is exercised end to end with a real zip built
// by `jszip` (a production dependency).

import JSZip from "jszip";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  IMAGE_EXTS,
  classifyFile,
  needsPreprocessing,
  prepareForIngest,
  terminateOcrWorker,
  type PrepareProgress,
} from "./extractText";

// ---------- fakes for the dynamically imported heavy dependencies ----------

const recognize = vi.fn(async (_img: unknown) => ({ data: { text: "  OCR TEXT  " } }));
const terminate = vi.fn(async () => {});
const createWorker = vi.fn(async (_langs: string) => ({ recognize, terminate }));
vi.mock("tesseract.js", () => ({ createWorker: (langs: string) => createWorker(langs) }));

interface FakePage {
  text: string;
  cleanup: ReturnType<typeof vi.fn>;
  render: ReturnType<typeof vi.fn>;
}
const fakeDoc = { pages: [] as FakePage[], destroy: vi.fn(async () => {}) };
const getDocument = vi.fn((_args: unknown) => ({
  promise: Promise.resolve({
    numPages: fakeDoc.pages.length,
    getPage: async (n: number) => {
      const p = fakeDoc.pages[n - 1];
      return {
        getTextContent: async () => ({ items: p.text.split("|").map((str) => ({ str })) }),
        getViewport: () => ({ width: 10.2, height: 20.7 }),
        render: p.render,
        cleanup: p.cleanup,
      };
    },
    destroy: fakeDoc.destroy,
  }),
}));
const GlobalWorkerOptions: { workerSrc: string } = { workerSrc: "" };
vi.mock("pdfjs-dist", () => ({ getDocument, GlobalWorkerOptions }));
vi.mock("pdfjs-dist/build/pdf.worker.min.mjs?url", () => ({ default: "blob:fake-pdf-worker" }));

function page(text: string): FakePage {
  return { text, cleanup: vi.fn(), render: vi.fn(() => ({ promise: Promise.resolve() })) };
}

const f = (name: string, content: BlobPart = "x") => new File([content], name);

beforeEach(() => {
  fakeDoc.pages = [];
  recognize.mockClear();
  terminate.mockClear();
  createWorker.mockClear();
  getDocument.mockClear();
  fakeDoc.destroy.mockClear();
});

afterEach(async () => {
  // The OCR worker is module-level state shared across tests.
  await terminateOcrWorker();
});

// ---------- classifyFile / needsPreprocessing ----------

describe("classifyFile", () => {
  it("detects PDF by extension, case-insensitively", () => {
    expect(classifyFile(f("report.pdf"))).toBe("pdf");
    expect(classifyFile(f("REPORT.PDF"))).toBe("pdf");
  });

  it("detects DOCX by extension", () => {
    expect(classifyFile(f("memo.docx"))).toBe("docx");
    expect(classifyFile(f("Memo.DocX"))).toBe("docx");
  });

  it("detects every supported image extension", () => {
    for (const ext of IMAGE_EXTS) expect(classifyFile(f(`scan.${ext}`))).toBe("image");
    expect([...IMAGE_EXTS].sort()).toEqual(["bmp", "gif", "jpeg", "jpg", "png", "tif", "tiff", "webp"]);
  });

  it("classifies textual formats as text", () => {
    const textual = [
      "txt", "md", "markdown", "csv", "tsv", "json", "jsonl", "ndjson", "triples",
      "html", "htm", "xml", "yaml", "yml", "log", "rtf",
    ];
    for (const ext of textual) expect(classifyFile(f(`a.${ext}`)), ext).toBe("text");
  });

  it("passes through xlsx, doc, unknown extensions and extension-less names", () => {
    expect(classifyFile(f("sheet.xlsx"))).toBe("passthrough");
    expect(classifyFile(f("old.doc"))).toBe("passthrough");
    expect(classifyFile(f("archive.tar.gz"))).toBe("passthrough");
    expect(classifyFile(f("README"))).toBe("passthrough");
    expect(classifyFile(f(".env"))).toBe("passthrough");
  });

  it("uses only the last extension of a dotted name", () => {
    expect(classifyFile(f("v1.2.final.pdf"))).toBe("pdf");
    expect(classifyFile(f("notes.pdf.txt"))).toBe("text");
  });
});

describe("needsPreprocessing", () => {
  it("is true for pdf, docx and images", () => {
    expect(needsPreprocessing(f("a.pdf"))).toBe(true);
    expect(needsPreprocessing(f("a.docx"))).toBe(true);
    expect(needsPreprocessing(f("a.png"))).toBe(true);
  });

  it("is false for text and passthrough formats", () => {
    expect(needsPreprocessing(f("a.txt"))).toBe(false);
    expect(needsPreprocessing(f("a.xlsx"))).toBe(false);
    expect(needsPreprocessing(f("noext"))).toBe(false);
  });
});

// ---------- prepareForIngest ----------

describe("prepareForIngest — text and passthrough", () => {
  it("returns the very same File for textual input without touching OCR or PDF libraries", async () => {
    const file = f("a.md", "# hi");
    await expect(prepareForIngest(file)).resolves.toBe(file);
    expect(createWorker).not.toHaveBeenCalled();
    expect(getDocument).not.toHaveBeenCalled();
  });

  it("returns the very same File for passthrough input (xlsx)", async () => {
    const file = f("a.xlsx");
    await expect(prepareForIngest(file)).resolves.toBe(file);
    expect(createWorker).not.toHaveBeenCalled();
  });
});

describe("prepareForIngest — images (OCR)", () => {
  it("OCRs the image and returns a text/plain File named <base>.txt with a [source:] header", async () => {
    const out = await prepareForIngest(f("scan.png"));
    expect(out).toBeInstanceOf(File);
    expect(out.name).toBe("scan.txt");
    expect(out.type).toBe("text/plain");
    await expect(out.text()).resolves.toBe("[source: scan.png]\nOCR TEXT");
    expect(recognize).toHaveBeenCalledOnce();
  });

  it("loads the OCR worker with 'eng+fra' by default and with the given ocrLangs otherwise", async () => {
    await prepareForIngest(f("a.png"));
    expect(createWorker).toHaveBeenLastCalledWith("eng+fra");
    await prepareForIngest(f("b.png"), { ocrLangs: "deu" });
    expect(createWorker).toHaveBeenLastCalledWith("deu");
  });

  it("reuses one worker across files with the same languages", async () => {
    await prepareForIngest(f("a.png"));
    await prepareForIngest(f("b.jpg"));
    expect(createWorker).toHaveBeenCalledOnce();
    expect(recognize).toHaveBeenCalledTimes(2);
  });

  it("recreates the worker and terminates the old one when the languages change", async () => {
    await prepareForIngest(f("a.png"), { ocrLangs: "eng" });
    await prepareForIngest(f("b.png"), { ocrLangs: "fra" });
    expect(createWorker).toHaveBeenCalledTimes(2);
    expect(terminate).toHaveBeenCalledOnce();
  });

  it("reports an 'OCR <name>…' progress status", async () => {
    const seen: PrepareProgress[] = [];
    await prepareForIngest(f("scan.png"), { onProgress: (p) => seen.push(p) });
    expect(seen).toEqual([{ status: "OCR scan.png…" }]);
  });

  it("terminateOcrWorker tears the shared worker down and is a no-op afterwards", async () => {
    await prepareForIngest(f("a.png"));
    await terminateOcrWorker();
    await terminateOcrWorker();
    expect(terminate).toHaveBeenCalledOnce();
    await prepareForIngest(f("b.png"));
    expect(createWorker).toHaveBeenCalledTimes(2);
  });
});

// ---------- DOCX ----------

async function docx(
  name: string,
  documentXml: string | null,
  media: Record<string, BlobPart> = {},
): Promise<File> {
  const zip = new JSZip();
  if (documentXml !== null) zip.file("word/document.xml", documentXml);
  for (const [n, data] of Object.entries(media)) zip.file(`word/media/${n}`, data);
  const buf = await zip.generateAsync({ type: "arraybuffer" });
  return new File([buf], name);
}

const W = (body: string) =>
  `<?xml version="1.0"?><w:document xmlns:w="urn:w"><w:body>${body}</w:body></w:document>`;
const P = (...runs: string[]) => `<w:p>${runs.map((r) => `<w:r><w:t>${r}</w:t></w:r>`).join("")}</w:p>`;

describe("prepareForIngest — DOCX body text", () => {
  it("turns paragraphs into lines and joins runs of one paragraph without separators", async () => {
    const out = await prepareForIngest(await docx("m.docx", W(P("Hello", " world") + P("Second"))));
    expect(out.name).toBe("m.txt");
    await expect(out.text()).resolves.toBe("[source: m.docx]\nHello world\nSecond");
  });

  it("maps <w:br/> to a newline and <w:tab/> to a (collapsed) space", async () => {
    const xml = W(`<w:p><w:r><w:t>a</w:t><w:br/><w:t>b</w:t><w:tab/><w:t>c</w:t></w:r></w:p>`);
    const out = await prepareForIngest(await docx("m.docx", xml));
    await expect(out.text()).resolves.toBe("[source: m.docx]\na\nb c");
  });

  it("decodes XML entities (named, decimal and hex)", async () => {
    const xml = W(P("Tom &amp; Jerry &lt;3&gt; &quot;q&quot; &apos;a&apos; caf&#233; &#xE9;t&#xE9;"));
    const out = await prepareForIngest(await docx("m.docx", xml));
    await expect(out.text()).resolves.toBe("[source: m.docx]\nTom & Jerry <3> \"q\" 'a' café été");
  });

  it("collapses runs of spaces and caps blank lines at one", async () => {
    const xml = W(P("a   b") + P("") + P("") + P("") + P("c"));
    const out = await prepareForIngest(await docx("m.docx", xml));
    await expect(out.text()).resolves.toBe("[source: m.docx]\na b\n\nc");
  });

  it("strips tags it does not know, keeping only their text", async () => {
    const xml = W(`<w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:rPr><w:b/></w:rPr><w:t xml:space="preserve">bold</w:t></w:r></w:p>`);
    const out = await prepareForIngest(await docx("m.docx", xml));
    await expect(out.text()).resolves.toBe("[source: m.docx]\nbold");
  });

  it("yields only the header when word/document.xml is missing", async () => {
    const out = await prepareForIngest(await docx("empty.docx", null));
    await expect(out.text()).resolves.toBe("[source: empty.docx]\n");
    expect(createWorker).not.toHaveBeenCalled();
  });

  it("reports a 'Parsing <name>…' progress status", async () => {
    const seen: PrepareProgress[] = [];
    await prepareForIngest(await docx("m.docx", W(P("x"))), { onProgress: (p) => seen.push(p) });
    expect(seen).toEqual([{ status: "Parsing m.docx…" }]);
  });
});

describe("prepareForIngest — DOCX embedded images", () => {
  it("OCRs images under word/media and appends them in an OCR section", async () => {
    const file = await docx("m.docx", W(P("Body")), { "image1.png": "png-bytes" });
    const seen: PrepareProgress[] = [];
    const out = await prepareForIngest(file, { onProgress: (p) => seen.push(p) });
    await expect(out.text()).resolves.toBe(
      "[source: m.docx]\nBody\n\n\n--- OCR of embedded images ---\n\n[image: image1.png]\nOCR TEXT",
    );
    expect(recognize).toHaveBeenCalledOnce();
    expect(seen).toEqual([
      { status: "Parsing m.docx…" },
      { status: "OCR m.docx · image 1/1", fraction: 1 },
    ]);
  });

  it("skips non-image media (e.g. .emf) without calling OCR", async () => {
    const file = await docx("m.docx", W(P("Body")), { "chart.emf": "emf-bytes" });
    const out = await prepareForIngest(file);
    await expect(out.text()).resolves.toBe("[source: m.docx]\nBody");
    expect(recognize).not.toHaveBeenCalled();
  });

  it("drops images whose OCR yields no text, so the OCR section is omitted", async () => {
    recognize.mockResolvedValueOnce({ data: { text: "   " } });
    const file = await docx("m.docx", W(P("Body")), { "blank.png": "x" });
    const out = await prepareForIngest(file);
    await expect(out.text()).resolves.toBe("[source: m.docx]\nBody");
  });

  it("swallows a failing OCR on one image and keeps the others", async () => {
    recognize.mockRejectedValueOnce(new Error("bad image"));
    const file = await docx("m.docx", W(P("Body")), { "a.png": "x", "b.png": "y" });
    const out = await prepareForIngest(file);
    const text = await out.text();
    expect(text).toContain("--- OCR of embedded images ---");
    expect(text.match(/\[image: /g)).toHaveLength(1);
    expect(recognize).toHaveBeenCalledTimes(2);
  });
});

// ---------- PDF ----------

describe("prepareForIngest — PDF", () => {
  it("points pdf.js at the bundled worker URL once and destroys the document afterwards", async () => {
    fakeDoc.pages = [page("Plenty of embedded text on this page, more than forty chars.")];
    await prepareForIngest(f("d.pdf"));
    expect(GlobalWorkerOptions.workerSrc).toBe("blob:fake-pdf-worker");
    expect(getDocument).toHaveBeenCalledOnce();
    expect(fakeDoc.destroy).toHaveBeenCalledOnce();
    expect(fakeDoc.pages[0].cleanup).toHaveBeenCalledOnce();
  });

  it("emits one '--- Page n ---' section per page with the items joined by single spaces", async () => {
    fakeDoc.pages = [
      page("First page has  quite|a lot of  text, easily over forty characters."),
      page("Second page also carries enough characters to skip OCR entirely!"),
    ];
    const out = await prepareForIngest(f("d.pdf"));
    expect(out.name).toBe("d.txt");
    await expect(out.text()).resolves.toBe(
      "[source: d.pdf]\n" +
        "--- Page 1 ---\nFirst page has quite a lot of text, easily over forty characters.\n\n" +
        "--- Page 2 ---\nSecond page also carries enough characters to skip OCR entirely!",
    );
    expect(recognize).not.toHaveBeenCalled();
  });

  it("does not OCR pages that already carry at least 40 characters of text", async () => {
    fakeDoc.pages = [page("x".repeat(40))];
    await prepareForIngest(f("d.pdf"));
    expect(recognize).not.toHaveBeenCalled();
    expect(fakeDoc.pages[0].render).not.toHaveBeenCalled();
  });

  it("rasterises and OCRs a page with fewer than 40 characters, merging text and OCR output", async () => {
    fakeDoc.pages = [page("short")];
    const ctx = {};
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(ctx as never);
    vi.spyOn(HTMLCanvasElement.prototype, "toBlob").mockImplementation(function (cb) {
      cb(new Blob(["png"], { type: "image/png" }));
    });
    const seen: PrepareProgress[] = [];
    const out = await prepareForIngest(f("scan.pdf"), { onProgress: (p) => seen.push(p) });
    await expect(out.text()).resolves.toBe("[source: scan.pdf]\n--- Page 1 ---\nshort\nOCR TEXT");
    const render = fakeDoc.pages[0].render;
    expect(render).toHaveBeenCalledOnce();
    const args = render.mock.calls[0][0] as { canvasContext: unknown; canvas: HTMLCanvasElement };
    expect(args.canvasContext).toBe(ctx);
    // viewport 10.2 x 20.7 at scale 2 → canvas rounded up
    expect(args.canvas.width).toBe(11);
    expect(args.canvas.height).toBe(21);
    expect(recognize).toHaveBeenCalledOnce();
    expect(seen).toEqual([
      { status: "Parsing scan.pdf…" },
      { status: "Reading scan.pdf · page 1/1", fraction: 1 },
      { status: "OCR scan.pdf · page 1/1", fraction: 1 },
      // the page-level OCR call passes no progress callback, so no "OCR…" entry
    ]);
  });

  it("skips OCR silently when no 2D canvas context is available", async () => {
    fakeDoc.pages = [page("short")];
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
    const out = await prepareForIngest(f("scan.pdf"));
    await expect(out.text()).resolves.toBe("[source: scan.pdf]\n--- Page 1 ---\nshort");
    expect(recognize).not.toHaveBeenCalled();
  });

  it("omits pages that end up with neither text nor OCR output", async () => {
    fakeDoc.pages = [page(""), page("Enough text on the second page to be kept as is, definitely.")];
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
    const out = await prepareForIngest(f("d.pdf"));
    await expect(out.text()).resolves.toBe(
      "[source: d.pdf]\n--- Page 2 ---\nEnough text on the second page to be kept as is, definitely.",
    );
  });

  it("reports per-page reading progress with the page fraction", async () => {
    fakeDoc.pages = [page("a".repeat(50)), page("b".repeat(50))];
    const seen: PrepareProgress[] = [];
    await prepareForIngest(f("d.pdf"), { onProgress: (p) => seen.push(p) });
    expect(seen).toEqual([
      { status: "Parsing d.pdf…" },
      { status: "Reading d.pdf · page 1/2", fraction: 0.5 },
      { status: "Reading d.pdf · page 2/2", fraction: 1 },
    ]);
  });
});
