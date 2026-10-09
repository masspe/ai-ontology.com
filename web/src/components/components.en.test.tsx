// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The shared components in English: each renders a few sentences with the
// language set to `en` (the other tests of this folder run in French).

import { describe, expect, it, vi } from "vitest";
import { render } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { setLang } from "../lib/i18n";
import { act, renderPage, screen, waitFor } from "../test/render";
import Tour, { openTour } from "./Tour";
import Onboarding from "./Onboarding";
import { ApplyReportView, ReviewPanel } from "./IngestReview";
import ModelTypes from "./ModelTypes";
import FeedbackModal from "./FeedbackModal";
import MultiSelect from "./MultiSelect";
import StreamingAnswer from "./StreamingAnswer";
import GraphCanvas from "./GraphCanvas";
import Dropzone from "./Dropzone";
// @ts-expect-error JSX module
import { useConfirm } from "./ConfirmDialog.jsx";
import type { ApplyReport, OntologyProposal } from "../lib/proposalTypes";
import type { Ontology } from "../api";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return { ...actual, listConcepts: vi.fn(async () => ({ total: 0, concepts: [], next_cursor: null })) };
});

describe("components in English", () => {
  it("Tour: welcome step, counter and buttons", () => {
    setLang("en", false);
    renderPage(<Tour />);
    act(() => openTour());
    expect(screen.getByText("Welcome")).toBeInTheDocument();
    expect(screen.getByText(/^Step 1 of \d+$/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Next" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Close the guide" })).toBeInTheDocument();
  });

  it("Onboarding: the three steps and the ready-made models", () => {
    setLang("en", false);
    renderPage(<Onboarding hasModel={false} hasData={false} />);
    expect(screen.getByRole("heading", { name: "Describe your data" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "Ready-made model…" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "Contracts and invoices" })).toBeInTheDocument();
    expect(screen.getAllByText("After the previous step.")).toHaveLength(2);
  });

  it("IngestReview: review panel and apply report", () => {
    setLang("en", false);
    const proposal = {
      source: { name: "a.pdf" },
      concept_types: [],
      relation_types: [],
      concepts: [{ client_ref: "c1", concept_type: "Contract", name: "ACME" }],
      relations: [],
      rules: [],
      actions: [],
    } as unknown as OntologyProposal;
    const noop = () => {};
    const { rerender } = render(
      <ReviewPanel
        proposal={proposal}
        decisions={{ c1: "create_new" }}
        onDecision={noop}
        onEditConcept={noop}
        onEditRelation={noop}
        onBulkDecision={noop}
        onApply={noop}
        onCancel={noop}
      />,
    );
    expect(screen.getByText("Sheets (1)")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Accept all" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add to the data" })).toBeInTheDocument();
    expect(screen.getByText("new")).toBeInTheDocument();
    const report = {
      created: 1,
      merged: 0,
      skipped: 0,
      failed: 0,
      concept_types: [],
      relation_types: [],
      concepts: [],
      relations: [],
      rules: [],
      actions: [],
    } as unknown as ApplyReport;
    rerender(<ApplyReportView report={report} onReset={noop} />);
    expect(screen.getByText("Import complete")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Import another document" })).toBeInTheDocument();
  });

  it("ModelTypes: title, tab count and add button", async () => {
    setLang("en", false);
    const ontology = {
      concept_types: { Chantier: { name: "Chantier", description: "d" } },
      relation_types: {},
    } as unknown as Ontology;
    renderPage(<ModelTypes ontology={ontology} onChanged={() => {}} />);
    expect(screen.getByText("Model types")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Sheet types (1)" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add a sheet type" })).toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole("button", { name: "Delete" })).toBeInTheDocument());
  });

  it("FeedbackModal: title, fields and buttons", () => {
    setLang("en", false);
    render(<FeedbackModal open onClose={() => {}} />);
    expect(screen.getByRole("heading", { name: /Send feedback/ })).toBeInTheDocument();
    expect(screen.getByPlaceholderText("Summarize in a few words…")).toBeInTheDocument();
    expect(screen.getByText("Improvement")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Send" })).toBeInTheDocument();
  });

  it("MultiSelect: search box, select-all and summary", async () => {
    setLang("en", false);
    const user = userEvent.setup();
    render(
      <MultiSelect
        label="Sheet type"
        options={[
          { value: "a", label: "A" },
          { value: "b", label: "B" },
          { value: "c", label: "C" },
        ]}
        selected={["a", "b"]}
        onChange={() => {}}
        allLabel="All"
      />,
    );
    await user.click(screen.getByRole("button", { name: /Sheet type/ }));
    expect(screen.getByText("2 of 3")).toBeInTheDocument();
    expect(screen.getByPlaceholderText("Search…")).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "Select all" })).toBeInTheDocument();
  });

  it("StreamingAnswer: placeholder and button", () => {
    setLang("en", false);
    render(<StreamingAnswer />);
    expect(screen.getByPlaceholderText("Ask your question…")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Ask the question" })).toBeInTheDocument();
  });

  it("GraphCanvas: the empty state with its links", () => {
    setLang("en", false);
    renderPage(<GraphCanvas subgraph={null} />);
    expect(screen.getByText(/Nothing to show: the graph is empty\./)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Drop files" })).toHaveAttribute("href", "/files");
    expect(screen.getByRole("link", { name: "define the model" })).toHaveAttribute("href", "/builder");
  });

  it("Dropzone: the default sentences", () => {
    setLang("en", false);
    render(<Dropzone onFile={() => {}} />);
    expect(screen.getByText("Drop files here or click to upload")).toBeInTheDocument();
    expect(screen.getByText("JSONL, CSV, XLSX, triples, text or ontology JSON")).toBeInTheDocument();
  });

  it("ConfirmDialog: default copy", async () => {
    setLang("en", false);
    function Ask() {
      const confirm = useConfirm();
      return <button onClick={() => void confirm()}>ask</button>;
    }
    const { user } = renderPage(<Ask />);
    await user.click(screen.getByRole("button", { name: "ask" }));
    expect(screen.getByRole("heading", { name: "Confirm" })).toBeInTheDocument();
    expect(screen.getByText("Are you sure?")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
  });
});
