import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Shared review/apply-report UI for the LLM-assisted ingest pipeline.
//
// Extracted from `IngestWizard.tsx` so both `/ingest` (single-document
// wizard) and `/builder` (multi-file ontology builder) can render the
// same per-item preview, decision pickers and apply-report cards.
import { useMemo } from "react";
import Card from "./Card";
// ---------- decision heuristic ----------
/** Auto-pick a sensible decision for an item based on its conflict. */
export function defaultDecisionFor(conflict) {
    if (!conflict)
        return "create_new";
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
export function ConflictBadge({ conflict }) {
    if (!conflict) {
        return (_jsx("span", { style: {
                fontSize: 11,
                padding: "2px 6px",
                borderRadius: 4,
                background: "#dcfce7",
                color: "#15803d",
            }, children: "new" }));
    }
    const palette = {
        exists: { bg: "#fef3c7", fg: "#a16207", label: "exists" },
        type_mismatch: { bg: "#fee2e2", fg: "#b91c1c", label: "type mismatch" },
        dangling_ref: { bg: "#fee2e2", fg: "#b91c1c", label: "dangling ref" },
    };
    const p = palette[conflict.kind.kind];
    return (_jsx("span", { title: conflict.summary, style: {
            fontSize: 11,
            padding: "2px 6px",
            borderRadius: 4,
            background: p.bg,
            color: p.fg,
        }, children: p.label }));
}
export function DecisionPicker({ value, onChange, allowMerge, }) {
    return (_jsxs("select", { value: value, onChange: (e) => onChange(e.target.value), style: { fontSize: 12, padding: "2px 4px" }, children: [_jsx("option", { value: "create_new", children: "Create new" }), allowMerge && _jsx("option", { value: "merge", children: "Merge with existing" }), _jsx("option", { value: "skip", children: "Skip" })] }));
}
export function ConfidenceBar({ value }) {
    const pct = Math.round(Math.max(0, Math.min(1, value ?? 0)) * 100);
    const color = pct >= 70 ? "#16a34a" : pct >= 40 ? "#eab308" : "#dc2626";
    return (_jsx("div", { title: `confidence ${pct}%`, style: { width: 40, height: 4, background: "#e5e7eb", borderRadius: 2 }, children: _jsx("div", { style: { width: `${pct}%`, height: "100%", background: color, borderRadius: 2 } }) }));
}
export function Section({ title, children }) {
    return (_jsxs(Card, { children: [_jsx("h2", { style: { marginTop: 0, fontSize: 16 }, children: title }), children] }));
}
export function Table({ headers, children }) {
    return (_jsx("div", { style: { overflowX: "auto" }, children: _jsxs("table", { style: { width: "100%", borderCollapse: "collapse", fontSize: 13 }, children: [_jsx("thead", { children: _jsx("tr", { style: { textAlign: "left", borderBottom: "1px solid #e5e7eb" }, children: headers.map((h) => (_jsx("th", { style: { padding: "6px 8px", fontWeight: 600, color: "#475569" }, children: h }, h))) }) }), _jsx("tbody", { children: children })] }) }));
}
export function ReviewPanel(props) {
    const { proposal, decisions, onDecision, onEditConcept } = props;
    const counters = useMemo(() => {
        let create = 0;
        let merge = 0;
        let skip = 0;
        for (const v of Object.values(decisions)) {
            if (v === "create_new")
                create++;
            else if (v === "merge")
                merge++;
            else
                skip++;
        }
        return { create, merge, skip };
    }, [decisions]);
    return (_jsxs("div", { style: { display: "grid", gap: 12 }, children: [_jsxs(Card, { children: [_jsxs("div", { style: { display: "flex", justifyContent: "space-between", alignItems: "center", flexWrap: "wrap", gap: 8 }, children: [_jsxs("div", { style: { display: "flex", gap: 16, alignItems: "center", flexWrap: "wrap" }, children: [_jsx("strong", { children: proposal.source?.name ?? "Document" }), proposal.source?.encoding && (_jsxs("span", { style: { fontSize: 12, color: "#475569" }, children: ["encoding: ", _jsx("code", { children: proposal.source.encoding }), proposal.source?.had_bom ? " (BOM)" : ""] })), proposal.language && (_jsxs("span", { style: { fontSize: 12, color: "#475569" }, children: ["language: ", _jsx("code", { children: proposal.language.code }), " ", "(", Math.round(proposal.language.confidence * 100), "%)"] }))] }), _jsxs("div", { style: { display: "flex", gap: 6 }, children: [_jsx("button", { onClick: () => props.onBulkDecision("create_new"), children: "Accept all" }), _jsx("button", { onClick: () => props.onBulkDecision("skip"), children: "Skip all" })] })] }), _jsxs("div", { style: { marginTop: 8, fontSize: 13, color: "#0f172a" }, children: [_jsxs("span", { style: { marginRight: 12 }, children: ["Create: ", _jsx("strong", { children: counters.create })] }), _jsxs("span", { style: { marginRight: 12 }, children: ["Merge: ", _jsx("strong", { children: counters.merge })] }), _jsxs("span", { children: ["Skip: ", _jsx("strong", { children: counters.skip })] })] })] }), proposal.concept_types.length > 0 && (_jsx(Section, { title: `New concept types (${proposal.concept_types.length})`, children: _jsx(Table, { headers: ["Name", "Parent", "Conflict", "Confidence", "Decision"], children: proposal.concept_types.map((ct) => (_jsxs("tr", { children: [_jsx("td", { children: ct.name }), _jsx("td", { style: { color: "#475569" }, children: ct.parent ?? "—" }), _jsx("td", { children: _jsx(ConflictBadge, { conflict: ct.conflict }) }), _jsx("td", { children: _jsx(ConfidenceBar, { value: ct.confidence }) }), _jsx("td", { children: _jsx(DecisionPicker, { value: decisions[ct.client_ref] ?? "skip", onChange: (v) => onDecision(ct.client_ref, v), allowMerge: ct.conflict?.kind.kind === "exists" }) })] }, ct.client_ref))) }) })), proposal.relation_types.length > 0 && (_jsx(Section, { title: `New relation types (${proposal.relation_types.length})`, children: _jsx(Table, { headers: ["Name", "Domain → Range", "Conflict", "Confidence", "Decision"], children: proposal.relation_types.map((rt) => (_jsxs("tr", { children: [_jsx("td", { children: rt.name }), _jsxs("td", { style: { color: "#475569" }, children: [rt.domain, " \u2192 ", rt.range] }), _jsx("td", { children: _jsx(ConflictBadge, { conflict: rt.conflict }) }), _jsx("td", { children: _jsx(ConfidenceBar, { value: rt.confidence }) }), _jsx("td", { children: _jsx(DecisionPicker, { value: decisions[rt.client_ref] ?? "skip", onChange: (v) => onDecision(rt.client_ref, v), allowMerge: rt.conflict?.kind.kind === "exists" }) })] }, rt.client_ref))) }) })), proposal.concepts.length > 0 && (_jsx(Section, { title: `Concepts (${proposal.concepts.length})`, children: _jsx(Table, { headers: ["Type", "Name", "Description", "Conflict", "Confidence", "Decision"], children: proposal.concepts.map((c) => (_jsxs("tr", { children: [_jsx("td", { style: { color: "#475569" }, children: c.concept_type }), _jsx("td", { children: _jsx("input", { value: c.name, onChange: (e) => onEditConcept(c.client_ref, { name: e.target.value }), style: { width: "100%", border: 0, background: "transparent" } }) }), _jsx("td", { children: _jsx("input", { value: c.description ?? "", onChange: (e) => onEditConcept(c.client_ref, { description: e.target.value }), style: { width: "100%", border: 0, background: "transparent", color: "#475569" }, placeholder: "\u2014" }) }), _jsx("td", { children: _jsx(ConflictBadge, { conflict: c.conflict }) }), _jsx("td", { children: _jsx(ConfidenceBar, { value: c.confidence }) }), _jsx("td", { children: _jsx(DecisionPicker, { value: decisions[c.client_ref] ?? "skip", onChange: (v) => onDecision(c.client_ref, v), allowMerge: c.conflict?.kind.kind === "exists" }) })] }, c.client_ref))) }) })), proposal.relations.length > 0 && (_jsx(Section, { title: `Relations (${proposal.relations.length})`, children: _jsx(Table, { headers: ["Type", "Source", "Target", "Conflict", "Confidence", "Decision"], children: proposal.relations.map((r) => (_jsxs("tr", { children: [_jsx("td", { style: { color: "#475569" }, children: r.relation_type }), _jsx("td", { children: _jsx("code", { style: { fontSize: 12 }, children: r.source_ref }) }), _jsx("td", { children: _jsx("code", { style: { fontSize: 12 }, children: r.target_ref }) }), _jsx("td", { children: _jsx(ConflictBadge, { conflict: r.conflict }) }), _jsx("td", { children: _jsx(ConfidenceBar, { value: r.confidence }) }), _jsx("td", { children: _jsx(DecisionPicker, { value: decisions[r.client_ref] ?? "skip", onChange: (v) => onDecision(r.client_ref, v), allowMerge: false }) })] }, r.client_ref))) }) })), proposal.rules.length > 0 && (_jsx(Section, { title: `Rules (${proposal.rules.length})`, children: _jsx(Table, { headers: ["Type", "Name", "When → Then", "Conflict", "Decision"], children: proposal.rules.map((r) => (_jsxs("tr", { children: [_jsx("td", { style: { color: "#475569" }, children: r.rule_type }), _jsx("td", { children: r.name }), _jsxs("td", { style: { color: "#475569", fontSize: 12 }, children: [_jsx("em", { children: "when" }), " ", r.when || "—", " ", _jsx("em", { children: "then" }), " ", r.then || "—"] }), _jsx("td", { children: _jsx(ConflictBadge, { conflict: r.conflict }) }), _jsx("td", { children: _jsx(DecisionPicker, { value: decisions[r.client_ref] ?? "skip", onChange: (v) => onDecision(r.client_ref, v), allowMerge: r.conflict?.kind.kind === "exists" }) })] }, r.client_ref))) }) })), proposal.actions.length > 0 && (_jsx(Section, { title: `Actions (${proposal.actions.length})`, children: _jsx(Table, { headers: ["Type", "Name", "Subject", "Object", "Conflict", "Decision"], children: proposal.actions.map((a) => (_jsxs("tr", { children: [_jsx("td", { style: { color: "#475569" }, children: a.action_type }), _jsx("td", { children: a.name }), _jsx("td", { children: _jsx("code", { style: { fontSize: 12 }, children: a.subject_ref }) }), _jsx("td", { children: _jsx("code", { style: { fontSize: 12 }, children: a.object_ref ?? "—" }) }), _jsx("td", { children: _jsx(ConflictBadge, { conflict: a.conflict }) }), _jsx("td", { children: _jsx(DecisionPicker, { value: decisions[a.client_ref] ?? "skip", onChange: (v) => onDecision(a.client_ref, v), allowMerge: false }) })] }, a.client_ref))) }) })), _jsxs("div", { style: { display: "flex", justifyContent: "space-between", marginTop: 8 }, children: [_jsx("button", { onClick: props.onCancel, children: props.cancelLabel ?? "Cancel" }), _jsx("button", { onClick: props.onApply, disabled: props.applyDisabled, style: {
                            padding: "8px 16px",
                            background: props.applyDisabled ? "#94a3b8" : "#16a34a",
                            color: "white",
                            border: 0,
                            borderRadius: 4,
                            cursor: props.applyDisabled ? "not-allowed" : "pointer",
                        }, children: props.applyLabel ?? "Apply to graph" })] })] }));
}
// ---------- Apply report ----------
export function ApplyReportView({ report, onReset, resetLabel }) {
    return (_jsxs(Card, { children: [_jsx("h2", { style: { marginTop: 0 }, children: "Apply complete" }), _jsxs("div", { style: { display: "flex", gap: 24, marginBottom: 16 }, children: [_jsx(Stat, { label: "Created", value: report.created, color: "#16a34a" }), _jsx(Stat, { label: "Merged", value: report.merged, color: "#2563eb" }), _jsx(Stat, { label: "Skipped", value: report.skipped, color: "#94a3b8" }), _jsx(Stat, { label: "Failed", value: report.failed, color: "#dc2626" })] }), _jsx(OutcomeList, { title: "Concept types", rows: report.concept_types }), _jsx(OutcomeList, { title: "Relation types", rows: report.relation_types }), _jsx(OutcomeList, { title: "Concepts", rows: report.concepts }), _jsx(OutcomeList, { title: "Relations", rows: report.relations }), _jsx(OutcomeList, { title: "Rules", rows: report.rules }), _jsx(OutcomeList, { title: "Actions", rows: report.actions }), _jsx("button", { onClick: onReset, style: { marginTop: 12 }, children: resetLabel ?? "Ingest another document" })] }));
}
function Stat({ label, value, color }) {
    return (_jsxs("div", { children: [_jsx("div", { style: { fontSize: 24, fontWeight: 600, color }, children: value }), _jsx("div", { style: { fontSize: 12, color: "#475569" }, children: label })] }));
}
function OutcomeList({ title, rows }) {
    if (rows.length === 0)
        return null;
    return (_jsxs("details", { style: { marginBottom: 8 }, children: [_jsxs("summary", { style: { cursor: "pointer", fontWeight: 600 }, children: [title, " (", rows.length, ")"] }), _jsx("table", { style: { width: "100%", marginTop: 8, fontSize: 12 }, children: _jsx("tbody", { children: rows.map(([ref, outcome]) => (_jsxs("tr", { children: [_jsx("td", { style: { padding: "2px 4px" }, children: _jsx("code", { children: ref }) }), _jsx("td", { style: { padding: "2px 4px" }, children: outcome.status === "failed" ? (_jsx("span", { style: { color: "#dc2626" }, children: outcome.error })) : outcome.status === "skipped" ? (_jsx("span", { style: { color: "#94a3b8" }, children: "skipped" })) : (_jsxs("span", { style: { color: outcome.status === "merged" ? "#2563eb" : "#16a34a" }, children: [outcome.status, " #", outcome.id] })) })] }, ref))) }) })] }));
}
