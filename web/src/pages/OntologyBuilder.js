import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
import { useEffect, useState } from "react";
import Card from "../components/Card";
import Dropzone from "../components/Dropzone";
import OntologyGraph from "../components/OntologyGraph";
import { deleteFile, exportGraphUrl, generateOntology, getFiles, getOntology, getStats, getSubgraph, replaceOntology, upload, } from "../api";
const MAX_DESC = 4000;
const EXAMPLES = [
    "A contract management system tracking parties, agreements, clauses, obligations and renewal dates.",
    "A research knowledge base of papers, authors, institutions, topics and citations.",
    "A clinical ontology covering patients, diagnoses, medications, procedures and outcomes.",
];
const KIND_BY_EXT = {
    json: "ontology",
    jsonl: "jsonl",
    ndjson: "jsonl",
    triples: "triples",
    csv: "csv",
    xlsx: "xlsx",
    txt: "text",
    md: "text",
    pdf: "text",
    docx: "text",
};
const ICON_BY_KIND = {
    ontology: { label: "{ }", bg: "#fef3c7", fg: "#d97706" },
    jsonl: { label: "{ }", bg: "#fef3c7", fg: "#d97706" },
    triples: { label: "△", bg: "#e0e7ff", fg: "#4f46e5" },
    csv: { label: "CSV", bg: "#dcfce7", fg: "#16a34a" },
    xlsx: { label: "XLS", bg: "#dcfce7", fg: "#16a34a" },
    text: { label: "TXT", bg: "#ede9fe", fg: "#7c3aed" },
};
function iconForFile(f) {
    const ext = (f.name.split(".").pop() ?? "").toLowerCase();
    if (ext === "pdf")
        return { label: "PDF", bg: "#fee2e2", fg: "#dc2626" };
    if (ext === "docx" || ext === "doc")
        return { label: "W", bg: "#dbeafe", fg: "#2563eb" };
    return ICON_BY_KIND[f.kind] ?? { label: f.kind.slice(0, 3).toUpperCase(), bg: "#e5e7eb", fg: "#374151" };
}
function fmtBytes(b) {
    if (b < 1024)
        return `${b} B`;
    if (b < 1024 * 1024)
        return `${(b / 1024).toFixed(1)} KB`;
    return `${(b / (1024 * 1024)).toFixed(1)} MB`;
}
function fmtAgo(ts) {
    const diff = Math.max(0, Math.floor(Date.now() / 1000 - ts));
    if (diff < 60)
        return `${diff}s ago`;
    if (diff < 3600)
        return `${Math.floor(diff / 60)} min ago`;
    if (diff < 86400)
        return `${Math.floor(diff / 3600)} h ago`;
    return `${Math.floor(diff / 86400)} d ago`;
}
function buildFilesContext(files) {
    if (files.length === 0)
        return "";
    const lines = files.slice(0, 12).map((f) => {
        const status = f.concepts > 0 || f.relations > 0 ? "ingested" : "uploaded";
        const tag = f.concept_type ? ` as ${f.concept_type}` : "";
        return `- ${f.name} (${f.kind}, ${status}${tag}): ${f.concepts} concepts, ${f.relations} relations`;
    });
    return [
        "",
        "## Source documents already ingested into the graph",
        ...lines,
        "Use the entities, relations and terminology from these documents when proposing the ontology.",
    ].join("\n");
}
export default function OntologyBuilder() {
    const [description, setDescription] = useState("");
    const [showExamples, setShowExamples] = useState(false);
    const [ontology, setOntology] = useState(null);
    const [proposed, setProposed] = useState(null);
    const [stats, setStats] = useState(null);
    const [subgraph, setSubgraph] = useState(null);
    const [files, setFiles] = useState([]);
    const [busy, setBusy] = useState("idle");
    const [error, setError] = useState(null);
    const [info, setInfo] = useState(null);
    const [lastUpdated, setLastUpdated] = useState(null);
    const refresh = async () => {
        try {
            const [o, s, f, sg] = await Promise.all([
                getOntology(),
                getStats(),
                getFiles(),
                getSubgraph({ limit: 150, expansion_depth: 1 }),
            ]);
            setOntology(o);
            setStats(s);
            setFiles(f.files);
            setSubgraph(sg.subgraph);
            setLastUpdated(Math.floor(Date.now() / 1000));
        }
        catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
    };
    useEffect(() => {
        refresh();
    }, []);
    const handleGenerate = async () => {
        if (!description.trim() && files.length === 0)
            return;
        setBusy("generate");
        setError(null);
        setInfo(null);
        setProposed(null);
        try {
            const prompt = description.trim() + buildFilesContext(files);
            const res = await generateOntology(prompt);
            setProposed(res.ontology);
            setInfo(files.length > 0
                ? `Draft generated from your description and ${files.length} file(s). Click Save Ontology to apply.`
                : "Draft ontology generated. Click Save Ontology to apply.");
        }
        catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
        finally {
            setBusy("idle");
        }
    };
    const handleSave = async () => {
        if (!proposed)
            return;
        setBusy("save");
        setError(null);
        try {
            await replaceOntology(proposed);
            setProposed(null);
            setInfo("Ontology saved.");
            await refresh();
        }
        catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
        finally {
            setBusy("idle");
        }
    };
    const handleUpload = async (file) => {
        setBusy("upload");
        setError(null);
        setInfo(null);
        try {
            const ext = (file.name.split(".").pop() ?? "").toLowerCase();
            const kind = KIND_BY_EXT[ext] ?? "text";
            const needsCt = ["csv", "xlsx", "text"].includes(kind);
            const conceptType = needsCt
                ? (window.prompt(`Concept type for ${file.name}?`, "Document") ?? "").trim()
                : undefined;
            if (needsCt && !conceptType) {
                setBusy("idle");
                return;
            }
            const res = await upload(file, { kind, conceptType: conceptType || undefined });
            setInfo(`Processed ${file.name} — ${res.ingested.concepts} concepts, ${res.ingested.relations} relations.`);
            await refresh();
        }
        catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
        finally {
            setBusy("idle");
        }
    };
    const handleDeleteFile = async (id) => {
        try {
            await deleteFile(id);
            await refresh();
        }
        catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
    };
    const clearAll = async () => {
        if (files.length === 0)
            return;
        if (!window.confirm(`Remove ${files.length} file records? (Ingested data stays in the graph.)`))
            return;
        for (const f of files) {
            try {
                await deleteFile(f.id);
            }
            catch {
                /* ignore */
            }
        }
        await refresh();
    };
    const conceptTypeCount = ontology ? Object.keys(ontology.concept_types).length : 0;
    const confidence = stats && stats.concepts > 0 ? Math.min(99, 70 + Math.round((stats.relations / Math.max(1, stats.concepts)) * 10)) : 92;
    const confidenceLabel = confidence >= 85 ? "High" : confidence >= 65 ? "Medium" : "Low";
    return (_jsxs(_Fragment, { children: [_jsx("div", { className: "page-header", children: _jsxs("div", { children: [_jsx("h1", { className: "page-title", children: "Ontology Creation" }), _jsx("p", { className: "page-subtitle", children: "Create an ontology from natural language instructions or from your files." })] }) }), error && _jsx("div", { className: "error-banner", children: error }), info && _jsx("div", { className: "success-banner", children: info }), _jsxs("div", { className: "builder-grid", children: [_jsxs("div", { className: "builder-left", children: [_jsxs(Card, { title: _jsxs("span", { children: ["Describe Your Ontology ", _jsx("span", { className: "info-dot", title: "Plain-English description used by the LLM", children: "\u24D8" })] }), actions: _jsxs("button", { className: "chip-btn", onClick: () => setShowExamples((v) => !v), children: [_jsx("span", { "aria-hidden": true, children: "\u229E" }), " Examples"] }), children: [showExamples && (_jsx("div", { className: "examples-list", children: EXAMPLES.map((ex, i) => (_jsx("button", { className: "example-chip", onClick: () => {
                                                setDescription(ex);
                                                setShowExamples(false);
                                            }, children: ex }, i))) })), _jsx("textarea", { className: "desc-textarea", rows: 6, maxLength: MAX_DESC, placeholder: "Describe the ontology structure, entities, relations, and constraints...", value: description, onChange: (e) => setDescription(e.target.value) }), _jsxs("div", { className: "char-counter", children: [description.length, " / ", MAX_DESC] }), _jsxs("div", { className: "generate-row", children: [_jsxs("button", { className: "btn-generate", onClick: handleGenerate, disabled: busy !== "idle" || (!description.trim() && files.length === 0), children: [_jsx("span", { "aria-hidden": true, children: "\u2726" }), " ", busy === "generate" ? "Generating…" : "Generate Ontology"] }), _jsx("button", { className: "btn-generate-split", "aria-label": "More options", title: "More options", children: "\u25BE" })] })] }), _jsxs(Card, { title: _jsxs("span", { children: ["Upload Files (Optional) ", _jsx("span", { className: "info-dot", title: "Files are ingested into the graph", children: "\u24D8" })] }), style: { marginTop: 16 }, children: [_jsx(Dropzone, { onFile: handleUpload, disabled: busy !== "idle", hint: "Supports PDF, DOCX, CSV, JSON (Max 50MB)" }), files.length > 0 && (_jsxs(_Fragment, { children: [_jsx("div", { className: "upload-section-title", children: "Uploaded Files" }), _jsx("ul", { className: "upload-list", children: files.slice(0, 8).map((f) => {
                                                    const ic = iconForFile(f);
                                                    const processed = f.concepts > 0 || f.relations > 0;
                                                    return (_jsxs("li", { className: "upload-item", children: [_jsx("span", { className: "file-icon", style: { background: ic.bg, color: ic.fg }, children: ic.label }), _jsxs("div", { className: "file-meta", children: [_jsx("div", { className: "file-name", title: f.name, children: f.name }), _jsxs("div", { className: "file-sub", children: [fmtBytes(f.size), " \u00B7 ", f.kind.toUpperCase()] })] }), _jsx("span", { className: `status-pill ${processed ? "ok" : "warn"}`, children: processed ? "Processed" : "Analyzed" }), _jsx("span", { className: `status-check ${processed ? "ok" : "warn"}`, "aria-hidden": true, children: processed ? "✓" : "ⓘ" }), _jsx("button", { className: "kebab-btn", title: "Remove", onClick: () => handleDeleteFile(f.id), children: "\u22EE" })] }, f.id));
                                                }) })] })), _jsxs("div", { className: "upload-actions", children: [_jsxs("button", { className: "action-btn", onClick: refresh, disabled: busy !== "idle", children: [_jsx("span", { "aria-hidden": true, children: "\uD83D\uDD0D" }), " Analyze Files"] }), _jsxs("button", { className: "action-btn", onClick: refresh, disabled: busy !== "idle", children: [_jsx("span", { "aria-hidden": true, children: "\u21C4" }), " Merge Sources"] }), _jsxs("button", { className: "action-btn danger", onClick: clearAll, disabled: busy !== "idle" || files.length === 0, children: [_jsx("span", { "aria-hidden": true, children: "\uD83D\uDDD1" }), " Clear All"] })] })] })] }), _jsxs("div", { className: "builder-right", children: [_jsx(Card, { title: _jsxs("span", { children: ["Ontology Graph ", _jsx("span", { className: "info-dot", title: "Live view of the current ontology", children: "\u24D8" })] }), actions: _jsxs("div", { className: "graph-toolbar", children: [_jsxs("button", { className: "chip-btn", children: [_jsx("span", { "aria-hidden": true, children: "\u21F5" }), " Layout ", _jsx("span", { "aria-hidden": true, children: "\u25BE" })] }), _jsx("button", { className: "icon-square", title: "Zoom in", children: "\uFF0B" }), _jsx("button", { className: "icon-square", title: "Zoom out", children: "\uFF0D" }), _jsx("button", { className: "icon-square", title: "Fit", children: "\u26F6" }), _jsx("button", { className: "icon-square", title: "Fullscreen", children: "\u26F6" })] }), className: "graph-card", children: _jsxs("div", { className: "graph-area", children: [_jsx(OntologyGraph, { subgraph: subgraph, height: 460 }), proposed && (_jsxs("div", { className: "graph-preview-overlay", children: [_jsxs("div", { className: "muted", style: { marginBottom: 6, fontSize: 12 }, children: ["Proposed schema \u00B7 click ", _jsx("strong", { children: "Save Ontology" }), " to apply"] }), _jsx("pre", { className: "json-view", style: { maxHeight: 360 }, children: JSON.stringify(proposed, null, 2) })] }))] }) }), _jsx(Card, { title: _jsxs("span", { children: ["Ontology Insights ", _jsx("span", { className: "info-dot", title: "Counts derived from the live graph", children: "\u24D8" })] }), subtitle: _jsxs("span", { className: "insights-sub", children: ["Last updated: ", lastUpdated ? fmtAgo(lastUpdated) : "—", _jsx("button", { className: "link-btn-sm", onClick: refresh, title: "Refresh", children: "\u21BB" })] }), actions: _jsxs("button", { className: "link-btn", children: ["View Details ", _jsx("span", { "aria-hidden": true, children: "\u203A" })] }), style: { marginTop: 16 }, children: _jsxs("div", { className: "insights-grid", children: [_jsx(InsightTile, { icon: "\uD83D\uDC65", label: "Entities", value: stats?.concepts ?? 0, delta: stats?.deltas.concepts_pct, color: "#dbeafe", iconColor: "#2563eb" }), _jsx(InsightTile, { icon: "\u232C", label: "Relations", value: stats?.relations ?? 0, delta: stats?.deltas.relations_pct, color: "#ede9fe", iconColor: "#7c3aed" }), _jsx(InsightTile, { icon: "\u25A6", label: "Classes", value: conceptTypeCount, delta: stats?.deltas.concept_types_pct, color: "#dcfce7", iconColor: "#16a34a" }), _jsx(InsightTile, { icon: "\u2713", label: "Confidence", value: `${confidence}%`, hint: confidenceLabel, color: "#fef3c7", iconColor: "#d97706" })] }) })] })] }), _jsxs("div", { className: "builder-footer", children: [_jsxs("a", { className: "footer-btn", href: exportGraphUrl("jsonl"), children: [_jsx("span", { "aria-hidden": true, children: "\u2913" }), " Export ", _jsx("span", { "aria-hidden": true, children: "\u25BE" })] }), _jsxs("button", { className: "footer-btn primary", onClick: handleSave, disabled: !proposed || busy !== "idle", children: [_jsx("span", { "aria-hidden": true, children: "\uD83D\uDCBE" }), " Save Ontology"] })] }), _jsx("style", { children: `
        .builder-grid {
          display: grid;
          grid-template-columns: minmax(0, 460px) minmax(0, 1fr);
          gap: 18px;
          align-items: start;
        }
        @media (max-width: 1100px) { .builder-grid { grid-template-columns: 1fr; } }

        .info-dot { color: var(--muted); margin-left: 4px; font-size: 13px; cursor: help; }

        /* ---------- chip / pill buttons ---------- */
        .chip-btn {
          display: inline-flex; align-items: center; gap: 6px;
          padding: 6px 12px; font-size: 12px;
          background: var(--panel); border: 1px solid var(--border);
          border-radius: 8px; cursor: pointer; color: var(--text);
        }
        .chip-btn:hover { background: var(--panel-2); }

        .icon-square {
          width: 32px; height: 32px;
          display: inline-grid; place-items: center;
          background: var(--panel); border: 1px solid var(--border);
          border-radius: 8px; cursor: pointer; color: var(--text);
          font-size: 13px;
        }
        .icon-square:hover { background: var(--panel-2); }

        .link-btn { background: none; border: none; color: var(--accent); font-size: 12px; cursor: pointer; padding: 4px 6px; }
        .link-btn:hover { text-decoration: underline; }
        .link-btn-sm { background: none; border: none; color: var(--muted); cursor: pointer; font-size: 14px; margin-left: 4px; }
        .link-btn-sm:hover { color: var(--accent); }

        /* ---------- Describe section ---------- */
        .desc-textarea {
          width: 100%;
          resize: vertical;
          min-height: 130px;
          padding: 12px;
          font: inherit;
          background: var(--panel);
          border: 1px solid var(--border);
          border-radius: var(--radius-sm);
        }
        .desc-textarea:focus { outline: none; border-color: var(--accent); box-shadow: 0 0 0 3px var(--accent-soft); }
        .char-counter { font-size: 11px; color: var(--muted); text-align: right; margin-top: 6px; }

        .examples-list { display: flex; flex-direction: column; gap: 6px; margin-bottom: 10px; }
        .example-chip {
          text-align: left; padding: 8px 10px; font-size: 12px;
          background: var(--panel-2); border: 1px solid var(--border);
          border-radius: 8px; cursor: pointer; color: var(--text);
        }
        .example-chip:hover { background: var(--accent-soft); border-color: var(--accent); color: var(--accent); }

        .generate-row {
          margin-top: 12px;
          display: grid;
          grid-template-columns: 1fr 40px;
          gap: 1px;
          background: var(--accent);
          border-radius: 10px;
          overflow: hidden;
        }
        .btn-generate, .btn-generate-split {
          background: var(--accent); color: #fff;
          border: none; cursor: pointer; padding: 12px 16px;
          font-weight: 600; font-size: 14px;
          display: inline-flex; align-items: center; justify-content: center; gap: 8px;
        }
        .btn-generate:hover, .btn-generate-split:hover { background: #1d4fd1; }
        .btn-generate:disabled, .btn-generate-split:disabled { background: #93b4f5; cursor: not-allowed; }
        .btn-generate-split { padding: 12px 0; font-size: 12px; }

        /* ---------- Upload Files ---------- */
        .upload-section-title { margin: 16px 0 8px; font-size: 12px; font-weight: 600; color: var(--text); }
        .upload-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
        .upload-item {
          display: grid;
          grid-template-columns: 36px 1fr auto auto auto;
          align-items: center; gap: 10px;
          padding: 10px 12px;
          background: var(--panel);
          border: 1px solid var(--border);
          border-radius: 10px;
        }
        .file-icon {
          width: 36px; height: 36px; border-radius: 8px;
          display: grid; place-items: center;
          font-size: 11px; font-weight: 700;
        }
        .file-meta { min-width: 0; }
        .file-name { font-size: 13px; font-weight: 600; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
        .file-sub { font-size: 11px; color: var(--muted); margin-top: 2px; }
        .status-pill {
          font-size: 11px; padding: 3px 10px; border-radius: 999px; font-weight: 500;
        }
        .status-pill.ok { background: #dcfce7; color: #166534; }
        .status-pill.warn { background: #fef3c7; color: #92400e; }
        .status-check { font-size: 14px; }
        .status-check.ok { color: #16a34a; }
        .status-check.warn { color: #d97706; }
        .kebab-btn {
          background: none; border: none; cursor: pointer; color: var(--muted);
          padding: 2px 6px; font-size: 16px; line-height: 1;
        }
        .kebab-btn:hover { color: var(--text); }

        .upload-actions {
          margin-top: 16px; display: grid;
          grid-template-columns: repeat(3, 1fr); gap: 8px;
        }
        .action-btn {
          display: inline-flex; align-items: center; justify-content: center; gap: 6px;
          padding: 9px 10px; font-size: 12px;
          background: var(--panel); border: 1px solid var(--border);
          border-radius: 8px; cursor: pointer; color: var(--text);
          white-space: nowrap;
        }
        .action-btn:hover { background: var(--panel-2); }
        .action-btn.danger { color: #dc2626; }
        .action-btn.danger:hover { background: #fef2f2; border-color: #fecaca; }
        .action-btn:disabled { opacity: 0.5; cursor: not-allowed; }

        /* ---------- Graph ---------- */
        .graph-toolbar { display: inline-flex; gap: 6px; align-items: center; }
        .graph-card .graph-with-legend {
          position: relative;
          display: grid;
          grid-template-columns: 1fr auto;
          gap: 12px;
          min-height: 440px;
        }
        .graph-area {
          position: relative; min-height: 440px;
          border: 1px solid var(--border);
          border-radius: var(--radius-sm);
          overflow: hidden; background: var(--panel);
        }
        .graph-preview-overlay {
          position: absolute; inset: 0; background: rgba(247,248,251,0.97);
          padding: 16px; overflow: auto;
        }
        .graph-legend {
          list-style: none; padding: 12px 14px; margin: 0;
          display: flex; flex-direction: column; gap: 10px;
          font-size: 12px; color: var(--text);
          background: var(--panel); border: 1px solid var(--border); border-radius: var(--radius-sm);
          min-width: 110px;
        }
        .legend-dot { display: inline-block; width: 10px; height: 10px; border-radius: 50%; margin-right: 8px; vertical-align: middle; }

        /* ---------- Insights ---------- */
        .insights-sub { display: inline-flex; align-items: center; gap: 4px; font-size: 12px; color: var(--muted); }
        .insights-grid { display: grid; grid-template-columns: repeat(4, 1fr); gap: 12px; }
        @media (max-width: 900px) { .insights-grid { grid-template-columns: repeat(2, 1fr); } }
        .insight-tile {
          display: flex; gap: 12px; align-items: center;
          padding: 16px; border: 1px solid var(--border);
          border-radius: var(--radius-sm); background: var(--panel);
        }
        .insight-icon {
          width: 44px; height: 44px; border-radius: 12px;
          display: grid; place-items: center;
          font-size: 20px; flex-shrink: 0;
        }
        .insight-meta .value { font-size: 26px; font-weight: 700; line-height: 1.1; color: var(--text); }
        .insight-meta .label { font-size: 12px; color: var(--muted); margin-top: 2px; }
        .insight-meta .delta { font-size: 11px; margin-top: 4px; font-weight: 500; }

        /* ---------- Footer ---------- */
        .builder-footer {
          display: flex; justify-content: flex-end; gap: 10px;
          margin-top: 20px; padding-top: 16px;
        }
        .footer-btn {
          display: inline-flex; align-items: center; gap: 6px;
          padding: 10px 18px; font-size: 13px; font-weight: 500;
          border-radius: 10px; cursor: pointer;
          background: var(--panel); border: 1px solid var(--border);
          color: var(--text); text-decoration: none;
        }
        .footer-btn:hover { background: var(--panel-2); text-decoration: none; }
        .footer-btn.primary {
          background: var(--accent); color: #fff; border-color: var(--accent);
        }
        .footer-btn.primary:hover { background: #1d4fd1; }
        .footer-btn.primary:disabled { background: #93b4f5; border-color: #93b4f5; cursor: not-allowed; }
      ` })] }));
}
function InsightTile({ icon, label, value, delta, hint, color, iconColor }) {
    const cls = delta == null ? "flat" : delta > 0.5 ? "up" : delta < -0.5 ? "down" : "flat";
    const arrow = cls === "up" ? "↑" : cls === "down" ? "↓" : "•";
    return (_jsxs("div", { className: "insight-tile", children: [_jsx("div", { className: "insight-icon", style: { background: color, color: iconColor }, children: icon }), _jsxs("div", { className: "insight-meta", children: [_jsx("div", { className: "value", children: typeof value === "number" && value >= 1000 ? `${(value / 1000).toFixed(1)}k` : value }), _jsx("div", { className: "label", children: label }), delta != null && (_jsxs("div", { className: `delta ${cls}`, style: { color: cls === "up" ? "#16a34a" : cls === "down" ? "#dc2626" : "var(--muted)" }, children: [arrow, " ", Math.abs(delta).toFixed(0), "% vs last run"] })), delta == null && hint && (_jsxs("div", { className: "delta", style: { color: iconColor }, children: [hint, " ", _jsx("span", { style: { display: "inline-block", width: 6, height: 6, borderRadius: 999, background: iconColor, marginLeft: 4, verticalAlign: "middle" } })] }))] })] }));
}
