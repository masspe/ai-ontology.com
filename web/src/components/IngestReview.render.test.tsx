// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Rendering tests of the shared ingest review UI: badges, pickers, the
// review panel (sections, counters, bulk and per-item decisions, inline
// edits, apply/cancel) and the apply report.

import { describe, expect, it, vi } from "vitest";
import { render } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  ApplyReportView,
  ConfidenceBar,
  ConflictBadge,
  DecisionPicker,
  ReviewPanel,
  type ReviewPanelProps,
} from "./IngestReview";
import { screen, within } from "../test/render";
import type { ApplyReport, ConflictInfo, OntologyProposal } from "../lib/proposalTypes";

const exists: ConflictInfo = {
  kind: { kind: "exists", existing_id: "42", existing_display: "Person:Alice" },
  summary: "Alice already exists",
};
const mismatch: ConflictInfo = {
  kind: { kind: "type_mismatch", existing_type: "Company", existing_id: "7" },
  summary: "type differs",
};
const dangling: ConflictInfo = {
  kind: { kind: "dangling_ref", missing_ref: "c9" },
  summary: "missing ref",
};

const proposal: OntologyProposal = {
  source: { name: "contracts.pdf", encoding: "utf-8", had_bom: true },
  language: { code: "fr", script: "Latn", confidence: 0.875 },
  concept_types: [
    { client_ref: "ct1", name: "Contract", parent: "Document", confidence: 0.9, conflict: exists },
    { client_ref: "ct2", name: "Clause", confidence: 0.5 },
  ],
  relation_types: [
    { client_ref: "rt1", name: "contains", domain: "Contract", range: "Clause", confidence: 0.3, conflict: mismatch },
  ],
  concepts: [
    {
      client_ref: "c1",
      concept_type: "Contract",
      name: "ACME master",
      description: "Master agreement",
      properties: [["term", "3y"], ["value", "1M"]],
      confidence: 0.8,
      conflict: exists,
    },
    { client_ref: "c2", concept_type: "Clause", name: "Termination", confidence: 0.2 },
  ],
  relations: [
    { client_ref: "r1", relation_type: "contains", source_ref: "c1", target_ref: "c2", confidence: 0.7 },
  ],
  rules: [
    { client_ref: "ru1", rule_type: "constraint", name: "Notice period", when: "termination", then: "90 days", conflict: dangling },
    { client_ref: "ru2", rule_type: "constraint", name: "Empty rule" },
  ],
  actions: [
    { client_ref: "a1", action_type: "notify", name: "Warn legal", subject_ref: "c1", object_ref: "c2" },
    { client_ref: "a2", action_type: "notify", name: "No object", subject_ref: "c1" },
  ],
};

function panelProps(overrides: Partial<ReviewPanelProps> = {}): ReviewPanelProps {
  return {
    proposal,
    decisions: { ct1: "merge", ct2: "create_new", rt1: "create_new", c1: "merge", c2: "skip", r1: "create_new", ru1: "skip", ru2: "create_new", a1: "create_new" },
    onDecision: vi.fn(),
    onEditConcept: vi.fn(),
    onEditRelation: vi.fn(),
    onBulkDecision: vi.fn(),
    onApply: vi.fn(),
    onCancel: vi.fn(),
    ...overrides,
  };
}

describe("ConflictBadge", () => {
  it("labels each conflict kind, with the summary as tooltip", () => {
    const { rerender } = render(<ConflictBadge />);
    expect(screen.getByText("new")).toBeInTheDocument();
    rerender(<ConflictBadge conflict={null} />);
    expect(screen.getByText("new")).toBeInTheDocument();
    rerender(<ConflictBadge conflict={exists} />);
    expect(screen.getByText("exists")).toHaveAttribute("title", "Alice already exists");
    rerender(<ConflictBadge conflict={mismatch} />);
    expect(screen.getByText("type mismatch")).toBeInTheDocument();
    rerender(<ConflictBadge conflict={dangling} />);
    expect(screen.getByText("dangling ref")).toBeInTheDocument();
  });
});

describe("DecisionPicker", () => {
  it("offers merge only when allowed and reports the chosen action", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    const { rerender } = render(<DecisionPicker value="skip" onChange={onChange} allowMerge={false} />);
    expect(screen.queryByRole("option", { name: "Merge with existing" })).not.toBeInTheDocument();
    rerender(<DecisionPicker value="skip" onChange={onChange} allowMerge />);
    await user.selectOptions(screen.getByRole("combobox"), "merge");
    expect(onChange).toHaveBeenCalledWith("merge");
  });
});

describe("ConfidenceBar", () => {
  it("clamps the value to 0..100 % and treats a missing value as 0", () => {
    const { rerender } = render(<ConfidenceBar value={0.85} />);
    expect(screen.getByTitle("confidence 85%")).toBeInTheDocument();
    rerender(<ConfidenceBar value={1.7} />);
    expect(screen.getByTitle("confidence 100%")).toBeInTheDocument();
    rerender(<ConfidenceBar value={-3} />);
    expect(screen.getByTitle("confidence 0%")).toBeInTheDocument();
    rerender(<ConfidenceBar />);
    expect(screen.getByTitle("confidence 0%")).toBeInTheDocument();
    rerender(<ConfidenceBar value={0.5} />);
    expect(screen.getByTitle("confidence 50%")).toBeInTheDocument();
  });
});

describe("ReviewPanel", () => {
  it("renders the source, language, counters and every section", () => {
    render(<ReviewPanel {...panelProps()} />);
    expect(screen.getByText("contracts.pdf")).toBeInTheDocument();
    expect(screen.getByText("utf-8")).toBeInTheDocument();
    expect(screen.getByText(/\(BOM\)/)).toBeInTheDocument();
    expect(screen.getByText("fr")).toBeInTheDocument();
    expect(screen.getByText(/\(88%\)/)).toBeInTheDocument();

    // ct2, rt1, r1, ru2, a1 create; ct1, c1 merge; c2, ru1 skip.
    expect(screen.getByText("Create:").parentElement).toHaveTextContent("Create: 5");
    expect(screen.getByText("Merge:").parentElement).toHaveTextContent("Merge: 2");
    expect(screen.getByText("Skip:").parentElement).toHaveTextContent("Skip: 2");

    expect(screen.getByText("New concept types (2)")).toBeInTheDocument();
    expect(screen.getByText("New relation types (1)")).toBeInTheDocument();
    expect(screen.getByText("Concepts (2)")).toBeInTheDocument();
    expect(screen.getByText("Relations (1)")).toBeInTheDocument();
    expect(screen.getByText("Rules (2)")).toBeInTheDocument();
    expect(screen.getByText("Actions (2)")).toBeInTheDocument();

    // Parent fallback, domain → range, property chips, rule when/then, action object fallback.
    expect(screen.getByText("Document")).toBeInTheDocument();
    expect(screen.getByText("Contract → Clause")).toBeInTheDocument();
    expect(screen.getByTitle("term: 3y")).toBeInTheDocument();
    expect(screen.getByTitle("value: 1M")).toBeInTheDocument();
    expect(screen.getByText(/termination/).textContent).toContain("90 days");
    const emptyRule = screen.getByText("Empty rule").closest("tr")!;
    expect(emptyRule).toHaveTextContent("when — then —");
    const noObject = screen.getByText("No object").closest("tr")!;
    expect(within(noObject).getByText("—")).toBeInTheDocument();
    // a2 has no decision → picker falls back to "skip".
    expect(within(noObject).getByRole("combobox")).toHaveValue("skip");
    // Merge is only offered on "exists" conflicts.
    const ctSection = screen.getByText("New concept types (2)").closest("section")!;
    const ct1 = within(ctSection).getByText("Contract").closest("tr")!;
    expect(within(ct1).getByRole("option", { name: "Merge with existing" })).toBeInTheDocument();
    const rtSection = screen.getByText("New relation types (1)").closest("section")!;
    const rt1 = within(rtSection).getByText("contains").closest("tr")!;
    expect(within(rt1).queryByRole("option", { name: "Merge with existing" })).not.toBeInTheDocument();
  });

  it("falls back to 'Document' and hides the sections of an empty proposal", () => {
    const empty: OntologyProposal = {
      concept_types: [],
      relation_types: [],
      concepts: [],
      relations: [],
      rules: [],
      actions: [],
    };
    render(<ReviewPanel {...panelProps({ proposal: empty, decisions: {} })} />);
    expect(screen.getByText("Document")).toBeInTheDocument();
    expect(screen.queryByText(/encoding:/)).not.toBeInTheDocument();
    expect(screen.queryByText(/language:/)).not.toBeInTheDocument();
    expect(screen.queryByText(/Concepts \(/)).not.toBeInTheDocument();
    expect(screen.getByText("Create:").parentElement).toHaveTextContent("Create: 0");
  });

  it("forwards bulk decisions, per-item decisions and concept edits", async () => {
    const user = userEvent.setup();
    const props = panelProps();
    render(<ReviewPanel {...props} />);

    await user.click(screen.getByRole("button", { name: "Accept all" }));
    expect(props.onBulkDecision).toHaveBeenCalledWith("create_new");
    await user.click(screen.getByRole("button", { name: "Skip all" }));
    expect(props.onBulkDecision).toHaveBeenCalledWith("skip");

    const ctSection = screen.getByText("New concept types (2)").closest("section")!;
    const clauseRow = within(ctSection).getByText("Clause").closest("tr")!;
    await user.selectOptions(within(clauseRow).getByRole("combobox"), "skip");
    expect(props.onDecision).toHaveBeenCalledWith("ct2", "skip");

    const rtSection = screen.getByText("New relation types (1)").closest("section")!;
    const rtRow = within(rtSection).getByText("contains").closest("tr")!;
    await user.selectOptions(within(rtRow).getByRole("combobox"), "skip");
    expect(props.onDecision).toHaveBeenCalledWith("rt1", "skip");

    const relRow = screen.getByText("Relations (1)").closest("section")!.querySelector("tbody tr")!;
    await user.selectOptions(within(relRow as HTMLElement).getByRole("combobox"), "skip");
    expect(props.onDecision).toHaveBeenCalledWith("r1", "skip");

    const ruleRow = screen.getByText("Notice period").closest("tr")!;
    await user.selectOptions(within(ruleRow).getByRole("combobox"), "create_new");
    expect(props.onDecision).toHaveBeenCalledWith("ru1", "create_new");

    const actionRow = screen.getByText("Warn legal").closest("tr")!;
    await user.selectOptions(within(actionRow).getByRole("combobox"), "skip");
    expect(props.onDecision).toHaveBeenCalledWith("a1", "skip");

    const conceptRow = screen.getByDisplayValue("ACME master").closest("tr")!;
    await user.selectOptions(within(conceptRow).getByRole("combobox"), "create_new");
    expect(props.onDecision).toHaveBeenCalledWith("c1", "create_new");

    // Inline edits: the panel is controlled, so each keystroke reports a patch.
    await user.type(screen.getByDisplayValue("ACME master"), "!");
    expect(props.onEditConcept).toHaveBeenCalledWith("c1", { name: "ACME master!" });
    await user.type(screen.getByDisplayValue("Master agreement"), "?");
    expect(props.onEditConcept).toHaveBeenCalledWith("c1", { description: "Master agreement?" });
    const emptyDescription = within(screen.getByDisplayValue("Termination").closest("tr")!).getByPlaceholderText("—");
    await user.type(emptyDescription, "x");
    expect(props.onEditConcept).toHaveBeenCalledWith("c2", { description: "x" });
  });

  it("wires apply / cancel with default labels", async () => {
    const user = userEvent.setup();
    const props = panelProps();
    render(<ReviewPanel {...props} />);
    await user.click(screen.getByRole("button", { name: "Apply to graph" }));
    expect(props.onApply).toHaveBeenCalledTimes(1);
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(props.onCancel).toHaveBeenCalledTimes(1);
  });

  it("honours custom labels and the disabled apply button", async () => {
    const user = userEvent.setup();
    const props = panelProps({ applyLabel: "Build", cancelLabel: "Back", applyDisabled: true });
    render(<ReviewPanel {...props} />);
    const apply = screen.getByRole("button", { name: "Build" });
    expect(apply).toBeDisabled();
    await user.click(apply);
    expect(props.onApply).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Back" }));
    expect(props.onCancel).toHaveBeenCalledTimes(1);
  });
});

describe("ApplyReportView", () => {
  const report: ApplyReport = {
    concept_types: [["ct1", { status: "created", id: "1" }]],
    relation_types: [],
    concepts: [
      ["c1", { status: "merged", id: "42" }],
      ["c2", { status: "skipped" }],
      ["c3", { status: "failed", error: "duplicate name" }],
    ],
    relations: [],
    rules: [],
    actions: [],
    created: 1,
    merged: 1,
    skipped: 1,
    failed: 1,
  };

  it("shows the counters and one collapsible list per non-empty family", async () => {
    const user = userEvent.setup();
    const onReset = vi.fn();
    render(<ApplyReportView report={report} onReset={onReset} />);
    expect(screen.getByText("Apply complete")).toBeInTheDocument();
    for (const label of ["Created", "Merged", "Skipped", "Failed"]) {
      expect(screen.getByText(label).previousElementSibling).toHaveTextContent("1");
    }
    expect(screen.getByText("Concept types (1)")).toBeInTheDocument();
    expect(screen.getByText("Concepts (3)")).toBeInTheDocument();
    expect(screen.queryByText(/Relation types/)).not.toBeInTheDocument();
    expect(screen.queryByText(/^Rules/)).not.toBeInTheDocument();
    expect(screen.getByText("created #1")).toBeInTheDocument();
    expect(screen.getByText("merged #42")).toBeInTheDocument();
    expect(screen.getByText("skipped")).toBeInTheDocument();
    expect(screen.getByText("duplicate name")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Ingest another document" }));
    expect(onReset).toHaveBeenCalledTimes(1);
  });

  it("accepts a custom reset label", () => {
    render(<ApplyReportView report={report} onReset={() => {}} resetLabel="Again" />);
    expect(screen.getByRole("button", { name: "Again" })).toBeInTheDocument();
  });
});
