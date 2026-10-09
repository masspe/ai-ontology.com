// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The Files page in English.

import { beforeEach, describe, expect, it, vi } from "vitest";
import Files from "./Files";
import { renderPage, screen } from "../test/render";
import { setLang } from "../lib/i18n";
import type { FileRecord } from "../api";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return { ...actual, getFiles: vi.fn(), getOntology: vi.fn(), upload: vi.fn(), deleteFile: vi.fn() };
});

import * as api from "../api";

const mocked = api as unknown as Record<string, ReturnType<typeof vi.fn>>;
const files = [
  { id: 1, name: "report.pdf", size: 2048, kind: "text", status: "failed", uploaded_at: 1_700_000_000, concepts: 0, relations: 0, ontology_updates: 0 },
] as FileRecord[];

beforeEach(() => {
  setLang("en", false);
  mocked.getFiles!.mockResolvedValue({ files });
  mocked.getOntology!.mockResolvedValue({ concept_types: {}, relation_types: {} });
});

describe("Files page, English", () => {
  it("renders the library, the status and the side cards in English", async () => {
    renderPage(<Files />);
    expect(await screen.findByRole("table")).toBeInTheDocument();
    expect(screen.getByText("File library")).toBeInTheDocument();
    expect(screen.getByText("Total files")).toBeInTheDocument();
    expect(screen.getByText("Failed", { selector: ".status-pill" })).toBeInTheDocument();
    expect(screen.getByText("Quick actions")).toBeInTheDocument();
  });
});
