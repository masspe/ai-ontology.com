import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import Card from "../components/Card";
import Sparkline from "../components/Sparkline";
import { getFiles, getOntology, getQueries, getStats, getStatsHistory, listConcepts, } from "../api";
// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------
function fmtBytes(b) {
    if (b < 1024)
        return `${b} B`;
    if (b < 1024 * 1024)
        return `${(b / 1024).toFixed(1)} KB`;
    return `${(b / (1024 * 1024)).toFixed(1)} MB`;
}
function fmtNum(n) {
    return n.toLocaleString("en-US");
}
function fmtAgo(ts) {
    const diff = Math.max(0, Math.floor(Date.now() / 1000 - ts));
    if (diff < 60)
        return `${diff}s ago`;
    if (diff < 3600)
        return `${Math.floor(diff / 60)}m ago`;
    if (diff < 86400)
        return `${Math.floor(diff / 3600)}h ago`;
    return `${Math.floor(diff / 86400)}d ago`;
}
function fileKindClass(kind) {
    const k = kind.toLowerCase();
    if (k.includes("pdf"))
        return "file-icon pdf";
    if (k.includes("csv"))
        return "file-icon csv";
    if (k.includes("doc"))
        return "file-icon doc";
    if (k.includes("xls") || k.includes("sheet"))
        return "file-icon xls";
    if (k.includes("json"))
        return "file-icon json";
    return "file-icon generic";
}
function ingestStatus(f) {
    const s = (f.status || "").toLowerCase();
    if (s === "processed" || s === "ingested" || s === "done")
        return { label: "Processed", cls: "badge-success" };
    if (s === "pending" || s === "queued")
        return { label: "Pending", cls: "badge-warn" };
    if (s === "failed" || s === "error")
        return { label: "Failed", cls: "badge-danger" };
    if (s === "analyzed")
        return { label: "Analyzed", cls: "badge-accent" };
    return { label: f.status || "—", cls: "badge" };
}
// ---------------------------------------------------------------------------
// Inline SVG icons
// ---------------------------------------------------------------------------
const Icon = {
    layers: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M12 2 2 7l10 5 10-5-10-5z" }), _jsx("path", { d: "m2 17 10 5 10-5" }), _jsx("path", { d: "m2 12 10 5 10-5" })] })),
    users: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2" }), _jsx("circle", { cx: "9", cy: "7", r: "4" }), _jsx("path", { d: "M22 21v-2a4 4 0 0 0-3-3.87" }), _jsx("path", { d: "M16 3.13a4 4 0 0 1 0 7.75" })] })),
    share: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("circle", { cx: "18", cy: "5", r: "3" }), _jsx("circle", { cx: "6", cy: "12", r: "3" }), _jsx("circle", { cx: "18", cy: "19", r: "3" }), _jsx("path", { d: "m8.59 13.51 6.83 3.98" }), _jsx("path", { d: "m15.41 6.51-6.82 3.98" })] })),
    shield: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" }), _jsx("path", { d: "m9 12 2 2 4-4" })] })),
    upload: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" }), _jsx("path", { d: "m17 8-5-5-5 5" }), _jsx("path", { d: "M12 3v12" })] })),
    search: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("circle", { cx: "11", cy: "11", r: "7" }), _jsx("path", { d: "m21 21-4.3-4.3" })] })),
    graph: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("circle", { cx: "5", cy: "12", r: "2.5" }), _jsx("circle", { cx: "19", cy: "5", r: "2.5" }), _jsx("circle", { cx: "19", cy: "19", r: "2.5" }), _jsx("path", { d: "m7 11 10-5" }), _jsx("path", { d: "m7 13 10 5" })] })),
    plus: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M12 5v14" }), _jsx("path", { d: "M5 12h14" })] })),
    check: (_jsx("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: _jsx("path", { d: "m20 6-11 11-5-5" }) })),
    spark: (_jsx("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: _jsx("path", { d: "m12 3 1.9 5.8H20l-4.9 3.6L17 18.2 12 14.6 7 18.2l1.9-5.8L4 8.8h6.1z" }) })),
};
function RichStat({ label, value, deltaPct, icon, tone, spark, sparkColor }) {
    const cls = deltaPct == null ? "flat" : deltaPct > 0.05 ? "up" : deltaPct < -0.05 ? "down" : "flat";
    const arrow = cls === "up" ? "↑" : cls === "down" ? "↓" : "•";
    return (_jsxs("div", { className: "card stat-rich", children: [_jsx("div", { className: `stat-icon tone-${tone}`, children: icon }), _jsxs("div", { className: "stat-body", children: [_jsx("div", { className: "stat-label", children: label }), _jsx("div", { className: "stat-value", children: fmtNum(value) }), deltaPct != null && (_jsxs("div", { className: `stat-delta ${cls}`, children: [_jsxs("span", { children: [arrow, " ", Math.abs(deltaPct).toFixed(0), "%"] }), _jsx("span", { className: "muted", children: "vs last period" })] }))] }), _jsx("div", { className: "stat-spark", children: _jsx(Sparkline, { values: spark.length > 1 ? spark : [0, 0], stroke: sparkColor }) })] }));
}
function GrowthChart({ samples }) {
    if (samples.length === 0) {
        return _jsx("div", { className: "empty", children: "No samples yet. The dashboard auto-refreshes every 15s." });
    }
    const W = 600;
    const H = 220;
    const padL = 44;
    const padR = 12;
    const padT = 12;
    const padB = 28;
    const innerW = W - padL - padR;
    const innerH = H - padT - padB;
    const xs = samples.map((s) => s.ts);
    const ys = samples.map((s) => s.value);
    const xMin = Math.min(...xs);
    const xMax = Math.max(...xs);
    const yMin = 0;
    const yMax = Math.max(1, Math.max(...ys));
    const sx = (t) => padL + ((t - xMin) / Math.max(1, xMax - xMin)) * innerW;
    const sy = (v) => padT + innerH - ((v - yMin) / (yMax - yMin)) * innerH;
    const pts = samples.map((s) => `${sx(s.ts).toFixed(1)},${sy(s.value).toFixed(1)}`).join(" ");
    const areaPts = `${padL},${padT + innerH} ${pts} ${padL + innerW},${padT + innerH}`;
    const ticks = [0, 0.25, 0.5, 0.75, 1].map((p) => {
        const v = yMin + (yMax - yMin) * (1 - p);
        return { y: padT + innerH * p, v };
    });
    const xLabelIdx = samples.length <= 7
        ? samples.map((_, i) => i)
        : [0, Math.floor(samples.length / 4), Math.floor(samples.length / 2), Math.floor((3 * samples.length) / 4), samples.length - 1];
    return (_jsxs("svg", { viewBox: `0 0 ${W} ${H}`, className: "growth-chart", preserveAspectRatio: "none", children: [ticks.map((t, i) => (_jsxs("g", { children: [_jsx("line", { x1: padL, x2: padL + innerW, y1: t.y, y2: t.y, stroke: "var(--border)", strokeDasharray: "3 3" }), _jsx("text", { x: padL - 8, y: t.y + 4, fontSize: "10", fill: "var(--muted)", textAnchor: "end", children: t.v >= 1000 ? `${(t.v / 1000).toFixed(1)}k` : Math.round(t.v) })] }, i))), _jsx("polygon", { points: areaPts, fill: "var(--accent-soft)", opacity: 0.55 }), _jsx("polyline", { points: pts, fill: "none", stroke: "var(--accent)", strokeWidth: 2 }), samples.map((s, i) => (_jsx("circle", { cx: sx(s.ts), cy: sy(s.value), r: 2, fill: "var(--accent)" }, i))), xLabelIdx.map((i) => {
                const s = samples[i];
                const d = new Date(s.ts * 1000);
                const label = d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
                return (_jsx("text", { x: sx(s.ts), y: H - 8, fontSize: "10", fill: "var(--muted)", textAnchor: "middle", children: label }, i));
            })] }));
}
// ---------------------------------------------------------------------------
// Decorative mini network preview
// ---------------------------------------------------------------------------
function NetworkPreview({ ontology }) {
    const names = ontology ? Object.keys(ontology.concept_types).slice(0, 6) : [];
    if (names.length === 0) {
        return _jsx("div", { className: "empty", children: "No ontology defined yet." });
    }
    const center = { x: 240, y: 130, label: names[0] };
    const radius = 100;
    const around = names.slice(1).map((label, i, arr) => {
        const angle = (i / Math.max(1, arr.length)) * Math.PI * 2 - Math.PI / 2;
        return {
            label,
            x: center.x + Math.cos(angle) * radius * (1 + (i % 2) * 0.1),
            y: center.y + Math.sin(angle) * (radius - 30),
        };
    });
    const palette = ["#dbeafe", "#dcfce7", "#fef3c7", "#fee2e2", "#ede9fe", "#cffafe"];
    const stroke = ["#2563eb", "#16a34a", "#d97706", "#dc2626", "#7c3aed", "#0891b2"];
    return (_jsxs("div", { className: "network-preview", children: [_jsxs("svg", { viewBox: "0 0 480 260", preserveAspectRatio: "xMidYMid meet", children: [around.map((n, i) => (_jsx("line", { x1: center.x, y1: center.y, x2: n.x, y2: n.y, stroke: "#cbd5e1", strokeWidth: 1.2 }, `l-${i}`))), _jsxs("g", { children: [_jsx("rect", { x: center.x - 50, y: center.y - 16, width: 100, height: 32, rx: 16, fill: "#dbeafe", stroke: "#2563eb" }), _jsx("text", { x: center.x, y: center.y + 4, fontSize: "12", fontWeight: "600", fill: "#1d4ed8", textAnchor: "middle", children: center.label })] }), around.map((n, i) => (_jsxs("g", { children: [_jsx("rect", { x: n.x - 44, y: n.y - 14, width: 88, height: 28, rx: 14, fill: palette[i % palette.length], stroke: stroke[i % stroke.length] }), _jsx("text", { x: n.x, y: n.y + 4, fontSize: "11", fontWeight: "600", fill: stroke[i % stroke.length], textAnchor: "middle", children: n.label })] }, `n-${i}`)))] }), _jsxs("div", { className: "network-legend", children: [_jsxs("span", { children: [_jsx("i", { style: { background: "#2563eb" } }), " Class"] }), _jsxs("span", { children: [_jsx("i", { style: { background: "#16a34a" } }), " Entity"] }), _jsxs("span", { children: [_jsx("i", { style: { background: "#d97706" } }), " Relation"] }), _jsxs("span", { children: [_jsx("i", { style: { background: "#dc2626" } }), " Constraint"] })] })] }));
}
// ---------------------------------------------------------------------------
// Dashboard page
// ---------------------------------------------------------------------------
export default function Dashboard() {
    const [stats, setStats] = useState(null);
    const [history, setHistory] = useState(null);
    const [files, setFiles] = useState([]);
    const [queries, setQueries] = useState([]);
    const [ontology, setOntology] = useState(null);
    const [concepts, setConcepts] = useState([]);
    const [error, setError] = useState(null);
    useEffect(() => {
        let cancelled = false;
        const tick = async () => {
            try {
                const [s, h, f, q, o, c] = await Promise.all([
                    getStats(),
                    getStatsHistory(),
                    getFiles(),
                    getQueries().catch(() => ({ queries: [] })),
                    getOntology().catch(() => null),
                    listConcepts({ limit: 500 }).catch(() => ({ total: 0, concepts: [] })),
                ]);
                if (cancelled)
                    return;
                setStats(s);
                setHistory(h);
                setFiles(f.files);
                setQueries(q.queries);
                setOntology(o);
                setConcepts(c.concepts);
                setError(null);
            }
            catch (e) {
                if (!cancelled)
                    setError(e instanceof Error ? e.message : String(e));
            }
        };
        tick();
        const id = window.setInterval(tick, 15_000);
        return () => {
            cancelled = true;
            window.clearInterval(id);
        };
    }, []);
    const samples = history?.samples ?? [];
    const sparkConcepts = samples.map((s) => s.concepts);
    const sparkRelations = samples.map((s) => s.relations);
    const sparkConceptTypes = samples.map((s) => s.concept_types);
    const sparkRelationTypes = samples.map((s) => s.relation_types);
    const topTypes = useMemo(() => {
        const counts = new Map();
        for (const c of concepts)
            counts.set(c.concept_type, (counts.get(c.concept_type) ?? 0) + 1);
        return Array.from(counts.entries())
            .sort((a, b) => b[1] - a[1])
            .slice(0, 5);
    }, [concepts]);
    const topMax = topTypes.length > 0 ? topTypes[0][1] : 1;
    const conceptTypes = ontology ? Object.entries(ontology.concept_types).slice(0, 5) : [];
    return (_jsxs(_Fragment, { children: [_jsx("div", { className: "page-header", children: _jsxs("div", { children: [_jsx("h1", { className: "page-title", children: "Dashboard" }), _jsx("p", { className: "page-subtitle", children: "Monitor your ontology projects, files, and graph activity." })] }) }), error && _jsx("div", { className: "error-banner", children: error }), _jsxs("div", { className: "dash-row dash-row-stats", children: [_jsx(RichStat, { label: "Concept Types", value: stats?.concept_types ?? 0, deltaPct: stats?.deltas.concept_types_pct, icon: Icon.layers, tone: "blue", spark: sparkConceptTypes, sparkColor: "#2563eb" }), _jsx(RichStat, { label: "Entities", value: stats?.concepts ?? 0, deltaPct: stats?.deltas.concepts_pct, icon: Icon.users, tone: "violet", spark: sparkConcepts, sparkColor: "#7c3aed" }), _jsx(RichStat, { label: "Relations", value: stats?.relations ?? 0, deltaPct: stats?.deltas.relations_pct, icon: Icon.share, tone: "amber", spark: sparkRelations, sparkColor: "#d97706" }), _jsx(RichStat, { label: "Relation Types", value: stats?.relation_types ?? 0, deltaPct: stats?.deltas.relation_types_pct, icon: Icon.shield, tone: "green", spark: sparkRelationTypes, sparkColor: "#16a34a" })] }), _jsxs("div", { className: "dash-row dash-row-chart", children: [_jsxs(Card, { title: "Ontology Growth (Entities)", actions: _jsx(Link, { to: "/graph", className: "btn-ghost-link", children: "View Analytics" }), children: [_jsx(GrowthChart, { samples: samples.map((s) => ({ ts: s.ts, value: s.concepts })) }), _jsxs("div", { className: "chart-legend", children: [_jsxs("span", { children: [_jsx("i", { style: { background: "var(--accent)" } }), " Entities Added"] }), _jsxs("span", { children: [_jsx("i", { style: { background: "#94a3b8" } }), " Trend"] })] })] }), _jsx(Card, { title: "Ontology Network Preview", actions: _jsx(Link, { to: "/graph", className: "btn-ghost-link", children: "Open Graph \u2197" }), children: _jsx(NetworkPreview, { ontology: ontology }) })] }), _jsxs("div", { className: "dash-row dash-row-three", children: [_jsx(Card, { title: "Recent Concept Types", actions: _jsx(Link, { to: "/builder", className: "btn-ghost-link", children: "View All" }), children: conceptTypes.length === 0 ? (_jsx("div", { className: "empty", children: "No ontology defined yet." })) : (_jsxs("table", { className: "table compact-table", children: [_jsx("thead", { children: _jsxs("tr", { children: [_jsx("th", { children: "Name" }), _jsx("th", { children: "Parent" }), _jsx("th", { children: "Properties" }), _jsx("th", { children: "Status" })] }) }), _jsx("tbody", { children: conceptTypes.map(([name, def]) => (_jsxs("tr", { children: [_jsx("td", { children: _jsx("strong", { children: name }) }), _jsx("td", { className: "muted", children: def.parent ?? "—" }), _jsx("td", { className: "muted", children: def.properties ? Object.keys(def.properties).length : 0 }), _jsx("td", { children: _jsx("span", { className: "badge badge-success", children: "Active" }) })] }, name))) })] })) }), _jsxs(Card, { title: "Files / Ingestion Status", actions: _jsx(Link, { to: "/files", className: "btn-ghost-link", children: "View All" }), children: [files.length === 0 ? (_jsx("div", { className: "empty", children: "No files uploaded yet." })) : (_jsx("ul", { className: "file-list", children: files.slice(0, 5).map((f) => {
                                    const st = ingestStatus(f);
                                    return (_jsxs("li", { className: "file-row", children: [_jsx("div", { className: fileKindClass(f.kind), children: f.kind.toUpperCase().slice(0, 4) }), _jsxs("div", { className: "file-meta", children: [_jsx("div", { className: "file-name", children: f.name }), _jsxs("div", { className: "muted file-sub", children: [fmtBytes(f.size), " \u00B7 ", f.kind] })] }), _jsx("span", { className: `badge ${st.cls}`, children: st.label }), _jsx("span", { className: "muted file-time", children: fmtAgo(f.uploaded_at) })] }, f.id));
                                }) })), _jsxs(Link, { to: "/files", className: "dropzone-mini", children: [_jsx("span", { className: "muted", children: "Drag & drop files anywhere to upload" }), _jsxs("span", { className: "upload-link", children: [Icon.upload, " Upload Files"] })] })] }), _jsxs("div", { className: "dash-stack", children: [_jsx(Card, { title: "Quick Actions", children: _jsxs("div", { className: "quick-actions", children: [_jsxs(Link, { to: "/builder", className: "quick-action qa-blue", children: [_jsx("div", { className: "qa-icon", children: Icon.plus }), _jsxs("div", { children: [_jsx("div", { className: "qa-title", children: "Create Ontology" }), _jsx("div", { className: "qa-sub muted", children: "Start a new ontology" })] })] }), _jsxs(Link, { to: "/files", className: "quick-action qa-green", children: [_jsx("div", { className: "qa-icon", children: Icon.upload }), _jsxs("div", { children: [_jsx("div", { className: "qa-title", children: "Upload Files" }), _jsx("div", { className: "qa-sub muted", children: "Import and process data" })] })] }), _jsxs(Link, { to: "/queries", className: "quick-action qa-violet", children: [_jsx("div", { className: "qa-icon", children: Icon.search }), _jsxs("div", { children: [_jsx("div", { className: "qa-title", children: "Run Query" }), _jsx("div", { className: "qa-sub muted", children: "Search your graph" })] })] }), _jsxs(Link, { to: "/graph", className: "quick-action qa-amber", children: [_jsx("div", { className: "qa-icon", children: Icon.graph }), _jsxs("div", { children: [_jsx("div", { className: "qa-title", children: "Open Graph" }), _jsx("div", { className: "qa-sub muted", children: "Explore relationships" })] })] })] }) }), _jsx(Card, { title: "Recent Queries", actions: _jsx(Link, { to: "/queries", className: "btn-ghost-link", children: "View All" }), children: queries.length === 0 ? (_jsx("div", { className: "empty", children: "No saved queries yet." })) : (_jsx("ul", { className: "query-list", children: queries.slice(0, 5).map((q) => (_jsxs("li", { children: [_jsx("span", { className: "query-dot" }), _jsx("span", { className: "query-text", title: q.query, children: q.name }), _jsx("span", { className: "muted query-time", children: q.last_run_at ? fmtAgo(q.last_run_at) : "—" }), _jsx("span", { className: "query-check", children: Icon.check })] }, q.id))) })) })] })] }), _jsxs("div", { className: "dash-row dash-row-three", children: [_jsx(Card, { title: "Team Activity", actions: _jsx(Link, { to: "/files", className: "btn-ghost-link", children: "View All" }), children: files.length === 0 ? (_jsx("div", { className: "empty", children: "No activity yet." })) : (_jsx("ul", { className: "activity-list", children: files.slice(0, 4).map((f) => (_jsxs("li", { className: "activity-item", children: [_jsx("div", { className: "activity-avatar", children: (f.name[0] ?? "?").toUpperCase() }), _jsxs("div", { className: "activity-body", children: [_jsxs("div", { className: "activity-text", children: [_jsx("strong", { children: "System" }), " ingested ", _jsx("em", { children: f.name })] }), _jsx("div", { className: "muted activity-time", children: fmtAgo(f.uploaded_at) })] })] }, f.id))) })) }), _jsx(Card, { title: "Top Entity Types", subtitle: `Across ${fmtNum(concepts.length)} entities`, children: topTypes.length === 0 ? (_jsx("div", { className: "empty", children: "No entities yet." })) : (_jsx("ul", { className: "bar-list", children: topTypes.map(([name, count]) => (_jsxs("li", { className: "bar-row", children: [_jsx("span", { className: "bar-label", children: name }), _jsx("div", { className: "bar-track", children: _jsx("div", { className: "bar-fill", style: { width: `${Math.max(4, (count / topMax) * 100)}%` } }) }), _jsx("span", { className: "bar-value", children: fmtNum(count) })] }, name))) })) }), _jsx(Card, { title: "Insights", children: _jsxs("div", { className: "insights-card", children: [_jsx("div", { className: "insights-icon", children: Icon.spark }), _jsxs("div", { children: [_jsx("div", { className: "insights-title", children: stats && stats.deltas.concepts_pct > 0
                                                ? `Entity growth is up ${stats.deltas.concepts_pct.toFixed(0)}%`
                                                : "Graph activity overview" }), _jsx("div", { className: "muted insights-body", children: stats
                                                ? `You have ${fmtNum(stats.concepts)} entities across ${fmtNum(stats.concept_types)} concept types and ${fmtNum(stats.relations)} relations.`
                                                : "Awaiting stats from the server." })] })] }) })] })] }));
}
