// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// "Essayer avec l'exemple finance" (ROADMAP §3.9 lot A, point 2): the
// example shipped in `examples/finance` is sent to the server through the
// ordinary upload endpoint, from the browser, in the order its README gives
// (the same order `ontology serve --seed` uses). No server change: the
// files are bundled with the UI at build time.

import ontologyUrl from "../../../examples/finance/ontology.json?url";
import seedUrl from "../../../examples/finance/seed.jsonl?url";
import c1Url from "../../../examples/finance/contracts/C-2025-001.txt?url";
import c2Url from "../../../examples/finance/contracts/C-2025-002.txt?url";
import c3Url from "../../../examples/finance/contracts/C-2025-003.txt?url";
import invoicesUrl from "../../../examples/finance/invoices.xlsx?url";
import lineItemsUrl from "../../../examples/finance/line_items.xlsx?url";
import relationsUrl from "../../../examples/finance/relations.jsonl?url";
import { upload } from "../api";

/** The files, in loading order: relations last so every endpoint exists. */
export const FINANCE_FILES: { url: string; name: string; kind: string; conceptType?: string }[] = [
  { url: ontologyUrl, name: "ontology.json", kind: "ontology" },
  { url: seedUrl, name: "seed.jsonl", kind: "jsonl" },
  { url: c1Url, name: "C-2025-001.txt", kind: "text", conceptType: "Contract" },
  { url: c2Url, name: "C-2025-002.txt", kind: "text", conceptType: "Contract" },
  { url: c3Url, name: "C-2025-003.txt", kind: "text", conceptType: "Contract" },
  { url: invoicesUrl, name: "invoices.xlsx", kind: "xlsx", conceptType: "Invoice" },
  { url: lineItemsUrl, name: "line_items.xlsx", kind: "xlsx", conceptType: "LineItem" },
  { url: relationsUrl, name: "relations.jsonl", kind: "jsonl" },
];

export interface ExampleReport {
  concepts: number;
  relations: number;
}

/**
 * Load the finance example. `onProgress` is called after every file with
 * the number done and the total, so the UI can show where it is.
 */
export async function loadFinanceExample(
  onProgress?: (done: number, total: number) => void,
): Promise<ExampleReport> {
  const report = { concepts: 0, relations: 0 };
  let done = 0;
  for (const f of FINANCE_FILES) {
    const blob = await (await fetch(f.url)).blob();
    const res = await upload(new File([blob], f.name), { kind: f.kind, conceptType: f.conceptType });
    report.concepts += res.ingested.concepts;
    report.relations += res.ingested.relations;
    onProgress?.(++done, FINANCE_FILES.length);
  }
  return report;
}
