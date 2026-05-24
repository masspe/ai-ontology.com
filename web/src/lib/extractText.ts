// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Client-side preprocessor that lifts plain text out of arbitrary file
// types before sending it to `POST /ingest/analyze`. The server-side
// `/ingest/analyze` handler only decodes bytes as UTF-8, so binary
// formats (PDF, DOCX, images) must be flattened to text in the browser.
//
// Supported inputs:
//   * Plain text-ish (txt, md, csv, jsonl, …): returned as-is.
//   * Images (png, jpg, jpeg, webp, bmp, gif, tiff): OCR via tesseract.js.
//   * DOCX: text from `word/document.xml` + OCR of every image stored in
//     `word/media/`.
//   * PDF: per-page text extraction via pdfjs-dist + OCR of every image
//     embedded in the page.
//
// The result is always a synthesized `text/plain` `File` whose name keeps
// the original basename with a `.txt` suffix so the LLM proposal carries
// useful provenance.

import JSZip from "jszip";

// ---------- format detection ----------

export const IMAGE_EXTS = new Set([
  "png",
  "jpg",
  "jpeg",
  "webp",
  "bmp",
  "gif",
  "tif",
  "tiff",
]);

const TEXTUAL_EXTS = new Set([
  "txt",
  "md",
  "markdown",
  "csv",
  "tsv",
  "json",
  "jsonl",
  "ndjson",
  "triples",
  "html",
  "htm",
  "xml",
  "yaml",
  "yml",
  "log",
  "rtf",
]);

export type PreprocessKind = "text" | "image" | "docx" | "pdf" | "passthrough";

export function classifyFile(file: File): PreprocessKind {
  const ext = extOf(file.name);
  if (ext === "pdf") return "pdf";
  if (ext === "docx") return "docx";
  if (IMAGE_EXTS.has(ext)) return "image";
  if (TEXTUAL_EXTS.has(ext)) return "text";
  // Anything else (xlsx, doc, etc.) goes through unchanged — the server
  // either understands it or returns a 4xx the caller can surface.
  return "passthrough";
}

function extOf(name: string): string {
  const i = name.lastIndexOf(".");
  return i >= 0 ? name.slice(i + 1).toLowerCase() : "";
}

function baseOf(name: string): string {
  const i = name.lastIndexOf(".");
  return i >= 0 ? name.slice(0, i) : name;
}

// ---------- progress reporting ----------

export interface PrepareProgress {
  /** Free-form status string for the UI. */
  status: string;
  /** 0..1 progress within the current file (when known). */
  fraction?: number;
}

export type ProgressFn = (p: PrepareProgress) => void;

// ---------- OCR worker (shared, lazily created) ----------

type TesseractWorker = {
  recognize: (image: Blob | ImageBitmap | HTMLCanvasElement | string) => Promise<{
    data: { text: string };
  }>;
  terminate: () => Promise<void>;
};

let workerPromise: Promise<TesseractWorker> | null = null;
let workerLangs = "";

async function getOcrWorker(langs: string): Promise<TesseractWorker> {
  if (workerPromise && workerLangs === langs) return workerPromise;
  if (workerPromise) {
    // language changed — tear down and recreate.
    const old = workerPromise;
    workerPromise = null;
    void old.then((w) => w.terminate()).catch(() => {});
  }
  workerLangs = langs;
  workerPromise = (async () => {
    const { createWorker } = await import("tesseract.js");
    const w = (await createWorker(langs)) as unknown as TesseractWorker;
    return w;
  })();
  return workerPromise;
}

/** Tear down the shared OCR worker. Call when the page unmounts so the
 * embedded WASM runtime releases memory. */
export async function terminateOcrWorker(): Promise<void> {
  if (!workerPromise) return;
  const p = workerPromise;
  workerPromise = null;
  workerLangs = "";
  try {
    const w = await p;
    await w.terminate();
  } catch {
    /* ignore */
  }
}

async function ocrBlob(
  blob: Blob,
  langs: string,
  onProgress?: ProgressFn,
  label?: string,
): Promise<string> {
  onProgress?.({ status: `OCR ${label ?? ""}…`.trim() });
  const w = await getOcrWorker(langs);
  const res = await w.recognize(blob);
  return (res.data.text ?? "").trim();
}

// ---------- PDF.js worker setup ----------

let pdfjsLoaded = false;
async function loadPdfjs(): Promise<typeof import("pdfjs-dist")> {
  const pdfjs = await import("pdfjs-dist");
  if (!pdfjsLoaded) {
    // Use the bundled worker so the build is self-contained.
    const workerSrc = (await import("pdfjs-dist/build/pdf.worker.min.mjs?url")).default;
    pdfjs.GlobalWorkerOptions.workerSrc = workerSrc;
    pdfjsLoaded = true;
  }
  return pdfjs;
}

// ---------- DOCX ----------

/** Extract visible text from `word/document.xml` (very lightweight — we
 * only care about feeding the LLM, not preserving layout). */
function docxBodyText(xml: string): string {
  // Replace paragraph/break boundaries with newlines before stripping tags.
  const withBreaks = xml
    .replace(/<\/w:p>/g, "\n")
    .replace(/<w:br\s*\/?>/g, "\n")
    .replace(/<w:tab\s*\/?>/g, "\t");
  const stripped = withBreaks.replace(/<[^>]+>/g, "");
  return decodeEntities(stripped).replace(/[\t ]+/g, " ").replace(/\n{3,}/g, "\n\n").trim();
}

function decodeEntities(s: string): string {
  return s
    .replace(/&amp;/g, "&")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&#(\d+);/g, (_, n) => String.fromCodePoint(Number(n)))
    .replace(/&#x([0-9a-fA-F]+);/g, (_, n) => String.fromCodePoint(parseInt(n, 16)));
}

async function extractDocx(
  file: File,
  langs: string,
  onProgress?: ProgressFn,
): Promise<string> {
  onProgress?.({ status: `Parsing ${file.name}…` });
  const zip = await JSZip.loadAsync(await file.arrayBuffer());
  const docXml = await zip.file("word/document.xml")?.async("string");
  const body = docXml ? docxBodyText(docXml) : "";

  // OCR every image in word/media/.
  const ocrParts: string[] = [];
  const mediaEntries = Object.values(zip.files).filter(
    (e) => !e.dir && /^word\/media\//.test(e.name),
  );
  for (let i = 0; i < mediaEntries.length; i++) {
    const entry = mediaEntries[i];
    const ext = extOf(entry.name);
    if (!IMAGE_EXTS.has(ext)) continue;
    onProgress?.({
      status: `OCR ${file.name} · image ${i + 1}/${mediaEntries.length}`,
      fraction: (i + 1) / Math.max(1, mediaEntries.length),
    });
    try {
      const blob = await entry.async("blob");
      const text = await ocrBlob(blob, langs, undefined);
      if (text) {
        ocrParts.push(
          `\n[image: ${entry.name.replace("word/media/", "")}]\n${text}`,
        );
      }
    } catch {
      /* skip unreadable images */
    }
  }

  const parts = [body];
  if (ocrParts.length > 0) {
    parts.push("\n\n--- OCR of embedded images ---");
    parts.push(...ocrParts);
  }
  return parts.join("\n").trim();
}

// ---------- PDF ----------

async function extractPdf(
  file: File,
  langs: string,
  onProgress?: ProgressFn,
): Promise<string> {
  const pdfjs = await loadPdfjs();
  onProgress?.({ status: `Parsing ${file.name}…` });
  const buf = await file.arrayBuffer();
  const doc = await pdfjs.getDocument({ data: buf }).promise;
  const pages: string[] = [];
  for (let pageNum = 1; pageNum <= doc.numPages; pageNum++) {
    onProgress?.({
      status: `Reading ${file.name} · page ${pageNum}/${doc.numPages}`,
      fraction: pageNum / doc.numPages,
    });
    const page = await doc.getPage(pageNum);
    const content = await page.getTextContent();
    const pageText = content.items
      .map((it) => (it as { str?: string }).str ?? "")
      .join(" ")
      .replace(/\s+/g, " ")
      .trim();

    let pageOcr = "";
    // When a page has little or no embedded text, fall back to OCR by
    // rasterizing the whole page. This handles scanned PDFs and PDFs
    // whose text content is encoded as glyph paths.
    if (pageText.length < 40) {
      try {
        const viewport = page.getViewport({ scale: 2 });
        const canvas = document.createElement("canvas");
        canvas.width = Math.ceil(viewport.width);
        canvas.height = Math.ceil(viewport.height);
        const ctx = canvas.getContext("2d");
        if (ctx) {
          onProgress?.({
            status: `OCR ${file.name} · page ${pageNum}/${doc.numPages}`,
            fraction: pageNum / doc.numPages,
          });
          await page.render({ canvasContext: ctx, viewport, canvas }).promise;
          const blob: Blob = await new Promise((resolve, reject) => {
            canvas.toBlob((b) => (b ? resolve(b) : reject(new Error("toBlob failed"))), "image/png");
          });
          pageOcr = await ocrBlob(blob, langs);
        }
      } catch {
        /* skip OCR failures */
      }
    }

    const merged = [pageText, pageOcr].filter(Boolean).join("\n");
    if (merged) pages.push(`\n--- Page ${pageNum} ---\n${merged}`);
    page.cleanup();
  }
  await doc.destroy();
  return pages.join("\n").trim();
}

// ---------- top-level entry ----------

export interface PrepareOptions {
  /** Tesseract language packs to load (e.g. `"eng"`, `"eng+fra"`). */
  ocrLangs?: string;
  /** Progress callback for the UI. */
  onProgress?: ProgressFn;
}

const DEFAULT_OCR_LANGS = "eng+fra";

/** Preprocess a single file: extract text, OCR images. The returned file
 * is always something `POST /ingest/analyze` can decode as UTF-8 — for
 * formats we don't recognize the original file is returned untouched. */
export async function prepareForIngest(
  file: File,
  opts: PrepareOptions = {},
): Promise<File> {
  const langs = opts.ocrLangs ?? DEFAULT_OCR_LANGS;
  const kind = classifyFile(file);

  if (kind === "text" || kind === "passthrough") return file;

  let text = "";
  if (kind === "image") {
    text = await ocrBlob(file, langs, opts.onProgress, file.name);
  } else if (kind === "docx") {
    text = await extractDocx(file, langs, opts.onProgress);
  } else if (kind === "pdf") {
    text = await extractPdf(file, langs, opts.onProgress);
  }

  const header = `[source: ${file.name}]\n`;
  const synthetic = new File([header + text], `${baseOf(file.name)}.txt`, {
    type: "text/plain",
  });
  return synthetic;
}

/** True when `file` requires client-side preprocessing (so the UI can
 * warn the user up-front about a potentially slow OCR pass). */
export function needsPreprocessing(file: File): boolean {
  const k = classifyFile(file);
  return k === "image" || k === "docx" || k === "pdf";
}
