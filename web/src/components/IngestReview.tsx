// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Shared review/apply-report UI for the LLM-assisted ingest pipeline.
//
// Extracted from `IngestWizard.tsx` so both `/ingest` (single-document
// wizard) and `/builder` (multi-file ontology builder) can render the
// same per-item preview, decision pickers and apply-report cards.

import { useMemo } from "react";
import type {
  ApplyOutcome,
  ApplyReport,
  ConflictInfo,
  DecisionAction,
  OntologyProposal,
  ProposalConcept,
  ProposalRelation,
} from "../lib/proposalTypes";
import Card from "./Card";

// ---------- decision heuristic ----------

/** Auto-pick a sensible decision for an item based on its conflict. */
export function defaultDecisionFor(conflict?: ConflictInfo | null): DecisionAction {
  if (!conflict) return "create_new";
  switch (conflict.kind.kind) {
    case "exists":
      return "merge";
    case "type_mismatch":
      return "create_new";
    case "dangling_ref":
      return "skip";
  }
}

// ---------- tiny UI primitives ----------

export function ConflictBadge({ conflict }: { conflict?: ConflictInfo | null }) {
  if (!conflict) {
    return (
      <span
        style={{
          fontSize: 11,
          padding: "2px 6px",
          borderRadius: 4,
          background: "#dcfce7",
          color: "#15803d",
        }}
      >
        new
      </span>
    );
  }
  const palette: Record<ConflictInfo["kind"]["kind"], { bg: string; fg: string; label: string }> = {
    exists: { bg: "#fef3c7", fg: "#a16207", label: "exists" },
    type_mismatch: { bg: "#fee2e2", fg: "#b91c1c", label: "type mismatch" },
    dangling_ref: { bg: "#fee2e2", fg: "#b91c1c", label: "dangling ref" },
  };
  const p = palette[conflict.kind.kind];
  return (
    <span
      title={conflict.summary}
      style={{
        fontSize: 11,
        padding: "2px 6px",
        borderRadius: 4,
        background: p.bg,
        color: p.fg,
      }}
    >
      {p.label}
    </span>
  );
}

export function DecisionPicker({
  value,
  onChange,
  allowMerge,
}: {
  value: DecisionAction;
  onChange: (v: DecisionAction) => void;
  allowMerge: boolean;
}) {
  return (
    <select
      value={value}
      onChange={(e) => onChange(e.target.value as DecisionAction)}
      style={{ fontSize: 12, padding: "2px 4px" }}
    >
      <option value="create_new">Create new</option>
      {allowMerge && <option value="merge">Merge with existing</option>}
      <option value="skip">Skip</option>
    </select>
  );
}

export function ConfidenceBar({ value }: { value?: number }) {
  const pct = Math.round(Math.max(0, Math.min(1, value ?? 0)) * 100);
  const color = pct >= 70 ? "#16a34a" : pct >= 40 ? "#eab308" : "#dc2626";
  return (
    <div title={`confidence ${pct}%`} style={{ width: 40, height: 4, background: "#e5e7eb", borderRadius: 2 }}>
      <div style={{ width: `${pct}%`, height: "100%", background: color, borderRadius: 2 }} />
    </div>
  );
}

export function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <Card>
      <h2 style={{ marginTop: 0, fontSize: 16 }}>{title}</h2>
      {children}
    </Card>
  );
}

export function Table({ headers, children }: { headers: string[]; children: React.ReactNode }) {
  return (
    <div style={{ overflowX: "auto" }}>
      <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 13 }}>
        <thead>
          <tr style={{ textAlign: "left", borderBottom: "1px solid #e5e7eb" }}>
            {headers.map((h) => (
              <th key={h} style={{ padding: "6px 8px", fontWeight: 600, color: "#475569" }}>
                {h}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>{children}</tbody>
      </table>
    </div>
  );
}

// ---------- Review panel ----------

export interface ReviewPanelProps {
  proposal: OntologyProposal;
  decisions: Record<string, DecisionAction>;
  onDecision: (ref: string, a: DecisionAction) => void;
  onEditConcept: (ref: string, patch: Partial<ProposalConcept>) => void;
  onEditRelation: (ref: string, patch: Partial<ProposalRelation>) => void;
  onBulkDecision: (a: DecisionAction) => void;
  onApply: () => void;
  onCancel: () => void;
  /** Label of the primary apply button (default `"Apply to graph"`). */
  applyLabel?: string;
  /** Label of the secondary button (default `"Cancel"`). */
  cancelLabel?: string;
  /** Disable the apply button (e.g. while a previous apply is in flight). */
  applyDisabled?: boolean;
}

export function ReviewPanel(props: ReviewPanelProps) {
  const { proposal, decisions, onDecision, onEditConcept } = props;

  const counters = useMemo(() => {
    let create = 0;
    let merge = 0;
    let skip = 0;
    for (const v of Object.values(decisions)) {
      if (v === "create_new") create++;
      else if (v === "merge") merge++;
      else skip++;
    }
    return { create, merge, skip };
  }, [decisions]);

  return (
    <div style={{ display: "grid", gap: 12 }}>
      <Card>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", flexWrap: "wrap", gap: 8 }}>
          <div style={{ display: "flex", gap: 16, alignItems: "center", flexWrap: "wrap" }}>
            <strong>{proposal.source?.name ?? "Document"}</strong>
            {proposal.source?.encoding && (
              <span style={{ fontSize: 12, color: "#475569" }}>
                encoding: <code>{proposal.source.encoding}</code>
                {proposal.source?.had_bom ? " (BOM)" : ""}
              </span>
            )}
            {proposal.language && (
              <span style={{ fontSize: 12, color: "#475569" }}>
                language: <code>{proposal.language.code}</code>{" "}
                ({Math.round(proposal.language.confidence * 100)}%)
              </span>
            )}
          </div>
          <div style={{ display: "flex", gap: 6 }}>
            <button onClick={() => props.onBulkDecision("create_new")}>Accept all</button>
            <button onClick={() => props.onBulkDecision("skip")}>Skip all</button>
          </div>
        </div>
        <div style={{ marginTop: 8, fontSize: 13, color: "#0f172a" }}>
          <span style={{ marginRight: 12 }}>Create: <strong>{counters.create}</strong></span>
          <span style={{ marginRight: 12 }}>Merge: <strong>{counters.merge}</strong></span>
          <span>Skip: <strong>{counters.skip}</strong></span>
        </div>
      </Card>

      {proposal.concept_types.length > 0 && (
        <Section title={`New concept types (${proposal.concept_types.length})`}>
          <Table headers={["Name", "Parent", "Conflict", "Confidence", "Decision"]}>
            {proposal.concept_types.map((ct) => (
              <tr key={ct.client_ref}>
                <td>{ct.name}</td>
                <td style={{ color: "#475569" }}>{ct.parent ?? "—"}</td>
                <td><ConflictBadge conflict={ct.conflict} /></td>
                <td><ConfidenceBar value={ct.confidence} /></td>
                <td>
                  <DecisionPicker
                    value={decisions[ct.client_ref] ?? "skip"}
                    onChange={(v) => onDecision(ct.client_ref, v)}
                    allowMerge={ct.conflict?.kind.kind === "exists"}
                  />
                </td>
              </tr>
            ))}
          </Table>
        </Section>
      )}

      {proposal.relation_types.length > 0 && (
        <Section title={`New relation types (${proposal.relation_types.length})`}>
          <Table headers={["Name", "Domain → Range", "Conflict", "Confidence", "Decision"]}>
            {proposal.relation_types.map((rt) => (
              <tr key={rt.client_ref}>
                <td>{rt.name}</td>
                <td style={{ color: "#475569" }}>{rt.domain} → {rt.range}</td>
                <td><ConflictBadge conflict={rt.conflict} /></td>
                <td><ConfidenceBar value={rt.confidence} /></td>
                <td>
                  <DecisionPicker
                    value={decisions[rt.client_ref] ?? "skip"}
                    onChange={(v) => onDecision(rt.client_ref, v)}
                    allowMerge={rt.conflict?.kind.kind === "exists"}
                  />
                </td>
              </tr>
            ))}
          </Table>
        </Section>
      )}

      {proposal.concepts.length > 0 && (
        <Section title={`Concepts (${proposal.concepts.length})`}>
          <Table headers={["Type", "Name", "Description", "Conflict", "Confidence", "Decision"]}>
            {proposal.concepts.map((c) => (
              <tr key={c.client_ref}>
                <td style={{ color: "#475569" }}>{c.concept_type}</td>
                <td>
                  <input
                    value={c.name}
                    onChange={(e) => onEditConcept(c.client_ref, { name: e.target.value })}
                    style={{ width: "100%", border: 0, background: "transparent" }}
                  />
                </td>
                <td>
                  <input
                    value={c.description ?? ""}
                    onChange={(e) =>
                      onEditConcept(c.client_ref, { description: e.target.value })
                    }
                    style={{ width: "100%", border: 0, background: "transparent", color: "#475569" }}
                    placeholder="—"
                  />
                </td>
                <td><ConflictBadge conflict={c.conflict} /></td>
                <td><ConfidenceBar value={c.confidence} /></td>
                <td>
                  <DecisionPicker
                    value={decisions[c.client_ref] ?? "skip"}
                    onChange={(v) => onDecision(c.client_ref, v)}
                    allowMerge={c.conflict?.kind.kind === "exists"}
                  />
                </td>
              </tr>
            ))}
          </Table>
        </Section>
      )}

      {proposal.relations.length > 0 && (
        <Section title={`Relations (${proposal.relations.length})`}>
          <Table headers={["Type", "Source", "Target", "Conflict", "Confidence", "Decision"]}>
            {proposal.relations.map((r) => (
              <tr key={r.client_ref}>
                <td style={{ color: "#475569" }}>{r.relation_type}</td>
                <td><code style={{ fontSize: 12 }}>{r.source_ref}</code></td>
                <td><code style={{ fontSize: 12 }}>{r.target_ref}</code></td>
                <td><ConflictBadge conflict={r.conflict} /></td>
                <td><ConfidenceBar value={r.confidence} /></td>
                <td>
                  <DecisionPicker
                    value={decisions[r.client_ref] ?? "skip"}
                    onChange={(v) => onDecision(r.client_ref, v)}
                    allowMerge={false}
                  />
                </td>
              </tr>
            ))}
          </Table>
        </Section>
      )}

      {proposal.rules.length > 0 && (
        <Section title={`Rules (${proposal.rules.length})`}>
          <Table headers={["Type", "Name", "When → Then", "Conflict", "Decision"]}>
            {proposal.rules.map((r) => (
              <tr key={r.client_ref}>
                <td style={{ color: "#475569" }}>{r.rule_type}</td>
                <td>{r.name}</td>
                <td style={{ color: "#475569", fontSize: 12 }}>
                  <em>when</em> {r.when || "—"} <em>then</em> {r.then || "—"}
                </td>
                <td><ConflictBadge conflict={r.conflict} /></td>
                <td>
                  <DecisionPicker
                    value={decisions[r.client_ref] ?? "skip"}
                    onChange={(v) => onDecision(r.client_ref, v)}
                    allowMerge={r.conflict?.kind.kind === "exists"}
                  />
                </td>
              </tr>
            ))}
          </Table>
        </Section>
      )}

      {proposal.actions.length > 0 && (
        <Section title={`Actions (${proposal.actions.length})`}>
          <Table headers={["Type", "Name", "Subject", "Object", "Conflict", "Decision"]}>
            {proposal.actions.map((a) => (
              <tr key={a.client_ref}>
                <td style={{ color: "#475569" }}>{a.action_type}</td>
                <td>{a.name}</td>
                <td><code style={{ fontSize: 12 }}>{a.subject_ref}</code></td>
                <td><code style={{ fontSize: 12 }}>{a.object_ref ?? "—"}</code></td>
                <td><ConflictBadge conflict={a.conflict} /></td>
                <td>
                  <DecisionPicker
                    value={decisions[a.client_ref] ?? "skip"}
                    onChange={(v) => onDecision(a.client_ref, v)}
                    allowMerge={false}
                  />
                </td>
              </tr>
            ))}
          </Table>
        </Section>
      )}

      <div style={{ display: "flex", justifyContent: "space-between", marginTop: 8 }}>
        <button onClick={props.onCancel}>{props.cancelLabel ?? "Cancel"}</button>
        <button
          onClick={props.onApply}
          disabled={props.applyDisabled}
          style={{
            padding: "8px 16px",
            background: props.applyDisabled ? "#94a3b8" : "#16a34a",
            color: "white",
            border: 0,
            borderRadius: 4,
            cursor: props.applyDisabled ? "not-allowed" : "pointer",
          }}
        >
          {props.applyLabel ?? "Apply to graph"}
        </button>
      </div>
    </div>
  );
}

// ---------- Apply report ----------

export function ApplyReportView({ report, onReset, resetLabel }: {
  report: ApplyReport;
  onReset: () => void;
  resetLabel?: string;
}) {
  return (
    <Card>
      <h2 style={{ marginTop: 0 }}>Apply complete</h2>
      <div style={{ display: "flex", gap: 24, marginBottom: 16 }}>
        <Stat label="Created" value={report.created} color="#16a34a" />
        <Stat label="Merged" value={report.merged} color="#2563eb" />
        <Stat label="Skipped" value={report.skipped} color="#94a3b8" />
        <Stat label="Failed" value={report.failed} color="#dc2626" />
      </div>

      <OutcomeList title="Concept types" rows={report.concept_types} />
      <OutcomeList title="Relation types" rows={report.relation_types} />
      <OutcomeList title="Concepts" rows={report.concepts} />
      <OutcomeList title="Relations" rows={report.relations} />
      <OutcomeList title="Rules" rows={report.rules} />
      <OutcomeList title="Actions" rows={report.actions} />

      <button onClick={onReset} style={{ marginTop: 12 }}>
        {resetLabel ?? "Ingest another document"}
      </button>
    </Card>
  );
}

function Stat({ label, value, color }: { label: string; value: number; color: string }) {
  return (
    <div>
      <div style={{ fontSize: 24, fontWeight: 600, color }}>{value}</div>
      <div style={{ fontSize: 12, color: "#475569" }}>{label}</div>
    </div>
  );
}

function OutcomeList({ title, rows }: { title: string; rows: [string, ApplyOutcome][] }) {
  if (rows.length === 0) return null;
  return (
    <details style={{ marginBottom: 8 }}>
      <summary style={{ cursor: "pointer", fontWeight: 600 }}>
        {title} ({rows.length})
      </summary>
      <table style={{ width: "100%", marginTop: 8, fontSize: 12 }}>
        <tbody>
          {rows.map(([ref, outcome]) => (
            <tr key={ref}>
              <td style={{ padding: "2px 4px" }}><code>{ref}</code></td>
              <td style={{ padding: "2px 4px" }}>
                {outcome.status === "failed" ? (
                  <span style={{ color: "#dc2626" }}>{outcome.error}</span>
                ) : outcome.status === "skipped" ? (
                  <span style={{ color: "#94a3b8" }}>skipped</span>
                ) : (
                  <span style={{ color: outcome.status === "merged" ? "#2563eb" : "#16a34a" }}>
                    {outcome.status} #{outcome.id}
                  </span>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </details>
  );
}
