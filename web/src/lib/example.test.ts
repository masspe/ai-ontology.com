// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.

import { beforeEach, describe, expect, it, vi } from "vitest";
import { FINANCE_FILES, loadFinanceExample } from "./example";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return { ...actual, upload: vi.fn() };
});

import * as api from "../api";

const mocked = api as unknown as { upload: ReturnType<typeof vi.fn> };

beforeEach(() => {
  // Not a real Response: on Node 22 it cannot read a jsdom Blob.
  vi.stubGlobal("fetch", vi.fn(async (url: string) => ({ blob: async () => new Blob([`body of ${url}`]) })));
  mocked.upload.mockResolvedValue({ file_id: 1, ingested: { concepts: 3, relations: 5, ontology_updates: 0 } });
});

describe("finance example loader", () => {
  it("uploads the eight files in the README order, relations last, and adds up the report", async () => {
    const seen: [number, number][] = [];
    const r = await loadFinanceExample((done, total) => seen.push([done, total]));
    expect(FINANCE_FILES.map((f) => f.name)).toEqual([
      "ontology.json",
      "seed.jsonl",
      "C-2025-001.txt",
      "C-2025-002.txt",
      "C-2025-003.txt",
      "invoices.xlsx",
      "line_items.xlsx",
      "relations.jsonl",
    ]);
    expect(mocked.upload).toHaveBeenCalledTimes(8);
    const [file, opts] = mocked.upload.mock.calls[5]!;
    expect(file.name).toBe("invoices.xlsx");
    expect(opts).toEqual({ kind: "xlsx", conceptType: "Invoice" });
    expect(mocked.upload.mock.calls[0]![1]).toEqual({ kind: "ontology", conceptType: undefined });
    expect(r).toEqual({ concepts: 24, relations: 40 });
    expect(seen).toEqual(FINANCE_FILES.map((_, i) => [i + 1, 8]));
  });

  it("stops at the first failed upload", async () => {
    mocked.upload.mockResolvedValueOnce({ file_id: 1, ingested: { concepts: 0, relations: 0, ontology_updates: 5 } });
    mocked.upload.mockRejectedValueOnce(new Error("upload failed: 400"));
    await expect(loadFinanceExample()).rejects.toThrow("upload failed: 400");
    expect(mocked.upload).toHaveBeenCalledTimes(2);
  });
});
