import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import Card from "../components/Card";
import Sparkline from "../components/Sparkline";
import GraphCanvas from "../components/GraphCanvas";
import { getFiles, getOntology, getQueries, getStatsHistory, getSubgraph, } from "../api";
// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------
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
function xsdFor(value) {
    if (value == null)
        return "xsd:string";
    if (typeof value === "boolean")
        return "xsd:boolean";
    if (typeof value === "number")
        return Number.isInteger(value) ? "xsd:integer" : "xsd:decimal";
    if (typeof value === "string") {
        if (/^\d{4}-\d{2}-\d{2}/.test(value))
            return "xsd:date";
        return "xsd:string";
    }
    return "xsd:any";
}
const TYPE_PALETTE = ["#2563eb", "#7c3aed", "#16a34a", "#dc2626", "#d97706", "#0891b2", "#db2777", "#0d9488"];
// ---------------------------------------------------------------------------
// Icons
// ---------------------------------------------------------------------------
const Icon = {
    layers: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M12 2 2 7l10 5 10-5-10-5z" }), _jsx("path", { d: "m2 17 10 5 10-5" }), _jsx("path", { d: "m2 12 10 5 10-5" })] })),
    share: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("circle", { cx: "18", cy: "5", r: "3" }), _jsx("circle", { cx: "6", cy: "12", r: "3" }), _jsx("circle", { cx: "18", cy: "19", r: "3" }), _jsx("path", { d: "m8.59 13.51 6.83 3.98" }), _jsx("path", { d: "m15.41 6.51-6.82 3.98" })] })),
    funnel: (_jsx("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: _jsx("path", { d: "M22 3H2l8 9.46V19l4 2v-8.54z" }) })),
    shield: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" }), _jsx("path", { d: "m9 12 2 2 4-4" })] })),
    search: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("circle", { cx: "11", cy: "11", r: "7" }), _jsx("path", { d: "m21 21-4.3-4.3" })] })),
    layout: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("rect", { x: "3", y: "3", width: "7", height: "7", rx: "1" }), _jsx("rect", { x: "14", y: "3", width: "7", height: "7", rx: "1" }), _jsx("rect", { x: "3", y: "14", width: "7", height: "7", rx: "1" }), _jsx("rect", { x: "14", y: "14", width: "7", height: "7", rx: "1" })] })),
    plus: (_jsx("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: _jsx("path", { d: "M12 5v14M5 12h14" }) })),
    minus: (_jsx("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: _jsx("path", { d: "M5 12h14" }) })),
    fit: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M3 8V5a2 2 0 0 1 2-2h3" }), _jsx("path", { d: "M16 3h3a2 2 0 0 1 2 2v3" }), _jsx("path", { d: "M21 16v3a2 2 0 0 1-2 2h-3" }), _jsx("path", { d: "M8 21H5a2 2 0 0 1-2-2v-3" })] })),
    expand: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M15 3h6v6" }), _jsx("path", { d: "M9 21H3v-6" }), _jsx("path", { d: "m21 3-7 7" }), _jsx("path", { d: "m3 21 7-7" })] })),
    refresh: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M3 12a9 9 0 0 1 15-6.7L21 8" }), _jsx("path", { d: "M21 3v5h-5" }), _jsx("path", { d: "M21 12a9 9 0 0 1-15 6.7L3 16" }), _jsx("path", { d: "M3 21v-5h5" })] })),
    chevR: (_jsx("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: _jsx("path", { d: "m9 18 6-6-6-6" }) })),
    chevL: (_jsx("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: _jsx("path", { d: "m15 18-6-6 6-6" }) })),
    person: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2" }), _jsx("circle", { cx: "12", cy: "7", r: "4" })] })),
    focus: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("circle", { cx: "12", cy: "12", r: "3" }), _jsx("circle", { cx: "12", cy: "12", r: "9" })] })),
    play: (_jsx("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: _jsx("polygon", { points: "5 3 19 12 5 21 5 3" }) })),
    pathIcon: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("circle", { cx: "5", cy: "6", r: "2" }), _jsx("circle", { cx: "19", cy: "6", r: "2" }), _jsx("circle", { cx: "12", cy: "18", r: "2" }), _jsx("path", { d: "M7 6h10M6.5 7.5l4 8.5M17.5 7.5l-4 8.5" })] })),
    activity: (_jsx("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: _jsx("path", { d: "M22 12h-4l-3 9L9 3l-3 9H2" }) })),
};
function Kpi({ label, value, deltaPct, deltaLabel, icon, tone, spark, sparkColor }) {
    const cls = deltaPct == null ? "flat" : deltaPct > 0.05 ? "up" : deltaPct < -0.05 ? "down" : "flat";
    const arrow = cls === "up" ? "↑" : cls === "down" ? "↓" : "—";
    return (_jsxs("div", { className: "card stat-rich", children: [_jsx("div", { className: `stat-icon tone-${tone}`, children: icon }), _jsxs("div", { className: "stat-body", children: [_jsx("div", { className: "stat-label", children: label }), _jsx("div", { className: "stat-value", children: value }), deltaPct != null && (_jsxs("div", { className: `stat-delta ${cls}`, children: [_jsxs("span", { children: [arrow, " ", Math.abs(deltaPct).toFixed(0), "%"] }), _jsx("span", { className: "muted", children: deltaLabel ?? "vs last month" })] }))] }), _jsx("div", { className: "stat-spark", children: _jsx(Sparkline, { values: spark.length > 1 ? spark : [0, 0], stroke: sparkColor }) })] }));
}
// ---------------------------------------------------------------------------
// Toggle switch
// ---------------------------------------------------------------------------
function Toggle({ checked, onChange, label, icon }) {
    return (_jsxs("label", { className: "gv-toggle", children: [_jsxs("span", { className: "gv-toggle-label", children: [icon && _jsx("span", { className: "gv-toggle-icon", children: icon }), label] }), _jsx("span", { className: `gv-switch${checked ? " on" : ""}`, onClick: () => onChange(!checked), role: "switch", "aria-checked": checked, children: _jsx("span", { className: "gv-switch-knob" }) })] }));
}
// ---------------------------------------------------------------------------
// Graph View page
// ---------------------------------------------------------------------------
const LAYOUTS = [
    { value: "LR", label: "Left → Right" },
    { value: "TB", label: "Top → Bottom" },
    { value: "RL", label: "Right → Left" },
    { value: "BT", label: "Bottom → Top" },
];
export default function GraphView() {
    const navigate = useNavigate();
    const canvasRef = useRef(null);
    // Data
    const [ontology, setOntology] = useState(null);
    const [subgraph, setSubgraph] = useState(null);
    const [history, setHistory] = useState(null);
    const [queries, setQueries] = useState([]);
    const [files, setFiles] = useState([]);
    // Filters
    const [search, setSearch] = useState("");
    const [nodeType, setNodeType] = useState("All Types");
    const [relType, setRelType] = useState("All Relations");
    const [depth, setDepth] = useState(3);
    const [showLabels, setShowLabels] = useState(true);
    const [clusterView, setClusterView] = useState(false);
    const [highlightPaths, setHighlightPaths] = useState(true);
    const [showConstraints, setShowConstraints] = useState(false);
    // Canvas controls
    const [layoutDir, setLayoutDir] = useState("LR");
    const [layoutOpen, setLayoutOpen] = useState(false);
    const [collapseInspector, setCollapseInspector] = useState(false);
    // Selection / inspector tab
    const [selectedId, setSelectedId] = useState(null);
    const [tab, setTab] = useState("inspector");
    // State
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState(null);
    // ---------- Data loading ----------
    const loadSubgraph = async (d = depth) => {
        setBusy(true);
        setError(null);
        try {
            const res = await getSubgraph({
                seed_query: search.trim() || undefined,
                seed_concept_types: nodeType !== "All Types" ? [nodeType] : [],
                expansion_depth: d,
                limit: 250,
            });
            setSubgraph(res.subgraph);
        }
        catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
        finally {
            setBusy(false);
        }
    };
    useEffect(() => {
        getOntology().then(setOntology).catch(() => undefined);
        getStatsHistory().then(setHistory).catch(() => undefined);
        getQueries().then((q) => setQueries(q.queries)).catch(() => undefined);
        getFiles().then((f) => setFiles(f.files)).catch(() => undefined);
        loadSubgraph(depth);
        const t = window.setInterval(() => loadSubgraph(depth), 30_000);
        return () => window.clearInterval(t);
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);
    // Refetch when depth changes (debounced via slider release would be nicer; effect is fine)
    useEffect(() => {
        loadSubgraph(depth);
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [depth]);
    // ---------- Derived ----------
    const conceptTypes = useMemo(() => (ontology ? Object.keys(ontology.concept_types) : []), [ontology]);
    const relationTypes = useMemo(() => (ontology ? Object.keys(ontology.relation_types) : []), [ontology]);
    const rules = useMemo(() => (ontology?.rule_types ? Object.values(ontology.rule_types) : []), [ontology]);
    const actions = useMemo(() => (ontology?.action_types ? Object.values(ontology.action_types) : []), [ontology]);
    // Stable color map for concept types (shared with canvas + legend)
    const conceptTypeColors = useMemo(() => {
        const out = {};
        conceptTypes.forEach((t, i) => {
            out[t] = TYPE_PALETTE[i % TYPE_PALETTE.length];
        });
        return out;
    }, [conceptTypes]);
    // Client-side filtered subgraph
    const filteredSubgraph = useMemo(() => {
        if (!subgraph)
            return null;
        const q = search.trim().toLowerCase();
        const keepConcept = (c) => {
            if (nodeType !== "All Types" && c.concept_type !== nodeType)
                return false;
            if (q && !c.name.toLowerCase().includes(q) && !c.concept_type.toLowerCase().includes(q))
                return false;
            return true;
        };
        const concepts = subgraph.concepts.filter(keepConcept);
        const keepIds = new Set(concepts.map((c) => c.id));
        const relations = subgraph.relations.filter((r) => {
            if (relType !== "All Relations" && r.relation_type !== relType)
                return false;
            return keepIds.has(r.source) && keepIds.has(r.target);
        });
        return { concepts, relations };
    }, [subgraph, search, nodeType, relType]);
    // Active filters count for KPI
    const activeFiltersCount = useMemo(() => {
        let n = 0;
        if (search.trim())
            n++;
        if (nodeType !== "All Types")
            n++;
        if (relType !== "All Relations")
            n++;
        if (depth !== 3)
            n++;
        return n;
    }, [search, nodeType, relType, depth]);
    // KPI values
    const nodesCount = filteredSubgraph?.concepts.length ?? 0;
    const relsCount = filteredSubgraph?.relations.length ?? 0;
    const samples = history?.samples ?? [];
    const graphHealth = useMemo(() => {
        const c = nodesCount;
        const r = relsCount;
        if (c === 0)
            return 0;
        return Math.min(100, Math.round((r / Math.max(1, c)) * 50));
    }, [nodesCount, relsCount]);
    // Selected node / concept
    const selectedConcept = useMemo(() => {
        if (!selectedId || !filteredSubgraph)
            return null;
        return filteredSubgraph.concepts.find((c) => String(c.id) === selectedId) ?? null;
    }, [selectedId, filteredSubgraph]);
    const selectedTypeDef = useMemo(() => {
        if (!selectedConcept || !ontology)
            return null;
        return ontology.concept_types[selectedConcept.concept_type] ?? null;
    }, [selectedConcept, ontology]);
    const selectedStats = useMemo(() => {
        if (!selectedConcept || !filteredSubgraph)
            return { total: 0, incoming: 0, outgoing: 0 };
        let inc = 0;
        let out = 0;
        for (const r of filteredSubgraph.relations) {
            if (r.target === selectedConcept.id)
                inc++;
            if (r.source === selectedConcept.id)
                out++;
        }
        return { total: inc + out, incoming: inc, outgoing: out };
    }, [selectedConcept, filteredSubgraph]);
    // Properties: prefer concept's own values, else type schema
    const selectedProps = useMemo(() => {
        if (selectedConcept?.properties && Object.keys(selectedConcept.properties).length > 0) {
            return Object.entries(selectedConcept.properties).map(([k, v]) => ({ name: k, type: xsdFor(v) }));
        }
        const schema = selectedTypeDef?.properties;
        if (schema && typeof schema === "object") {
            return Object.entries(schema).map(([k, v]) => ({ name: k, type: typeof v === "string" ? `xsd:${v}` : xsdFor(v) }));
        }
        return [];
    }, [selectedConcept, selectedTypeDef]);
    // Query suggestions from ontology
    const querySuggestions = useMemo(() => {
        if (!ontology)
            return [];
        const out = [];
        const cts = Object.keys(ontology.concept_types);
        const rts = Object.values(ontology.relation_types);
        for (const r of rts.slice(0, 3)) {
            out.push(`Find all ${r.domain} ${r.name.replace(/([A-Z])/g, " $1").trim().toLowerCase()} a specific ${r.range}`);
        }
        if (cts.length >= 2)
            out.push(`Show all ${cts[0]} created by an ${cts[1]}`);
        if (cts.length >= 2)
            out.push(`List all ${cts[0]} related to ${cts[1]}`);
        if (cts.length >= 1)
            out.push(`Find recent ${cts[cts.length - 1]}s`);
        return out.slice(0, 5);
    }, [ontology]);
    // Graph activity (from recent files)
    const activity = useMemo(() => {
        return [...files].sort((a, b) => b.uploaded_at - a.uploaded_at).slice(0, 4);
    }, [files]);
    // Recent paths from saved queries
    const recentPaths = useMemo(() => queries.slice(0, 4), [queries]);
    // ---------- Handlers ----------
    const onNodeClick = (id) => {
        setSelectedId(id);
        setTab("inspector");
        setCollapseInspector(false);
    };
    const onFocusNode = () => {
        if (selectedId)
            canvasRef.current?.focusNode(selectedId);
    };
    const onExpandNeighbors = async () => {
        if (!selectedConcept)
            return;
        setNodeType(selectedConcept.concept_type);
        setDepth(Math.min(5, depth + 1));
    };
    const onRunQuery = () => {
        if (selectedConcept) {
            navigate(`/queries?q=${encodeURIComponent(selectedConcept.name)}`);
        }
        else {
            navigate("/queries");
        }
    };
    const onFullscreen = () => {
        const el = document.getElementById("gv-canvas-wrap");
        if (!el)
            return;
        if (document.fullscreenElement)
            document.exitFullscreen();
        else
            el.requestFullscreen?.();
    };
    // ---------- Render ----------
    const headerSampleConcepts = samples.map((s) => s.concepts);
    const headerSampleRelations = samples.map((s) => s.relations);
    return (_jsxs(_Fragment, { children: [_jsx("div", { className: "page-header", children: _jsxs("div", { children: [_jsx("h1", { className: "page-title", children: "Graph View" }), _jsx("p", { className: "page-subtitle", children: "Explore ontology entities, classes, and relationships visually." })] }) }), error && _jsx("div", { className: "error-banner", children: error }), _jsxs("div", { className: "dash-row dash-row-stats", children: [_jsx(Kpi, { label: "Nodes", value: fmtNum(nodesCount), icon: Icon.layers, tone: "blue", spark: headerSampleConcepts, sparkColor: "#2563eb" }), _jsx(Kpi, { label: "Relations", value: fmtNum(relsCount), icon: Icon.share, tone: "violet", spark: headerSampleRelations, sparkColor: "#7c3aed" }), _jsx(Kpi, { label: "Active Filters", value: fmtNum(activeFiltersCount), icon: Icon.funnel, tone: "amber", spark: [], sparkColor: "#d97706" }), _jsx(Kpi, { label: "Graph Health", value: `${graphHealth}%`, icon: Icon.shield, tone: "green", spark: [], sparkColor: "#16a34a" })] }), _jsxs("div", { className: `dash-row gv-row-main${collapseInspector ? " inspector-collapsed" : ""}`, children: [_jsxs(Card, { title: "Filters & Controls", children: [_jsx("div", { className: "gv-filter-group", children: _jsxs("div", { className: "files-search", children: [_jsx("span", { className: "files-search-icon", children: Icon.search }), _jsx("input", { placeholder: "Search nodes\u2026", value: search, onChange: (e) => setSearch(e.target.value) })] }) }), _jsxs("div", { className: "gv-filter-group", children: [_jsx("label", { className: "gv-filter-label", children: "Node Type" }), _jsxs("select", { value: nodeType, onChange: (e) => setNodeType(e.target.value), children: [_jsx("option", { children: "All Types" }), conceptTypes.map((t) => _jsx("option", { children: t }, t))] })] }), _jsxs("div", { className: "gv-filter-group", children: [_jsx("label", { className: "gv-filter-label", children: "Relation Type" }), _jsxs("select", { value: relType, onChange: (e) => setRelType(e.target.value), children: [_jsx("option", { children: "All Relations" }), relationTypes.map((t) => _jsx("option", { children: t }, t))] })] }), _jsxs("div", { className: "gv-filter-group", children: [_jsxs("div", { className: "gv-slider-head", children: [_jsx("label", { className: "gv-filter-label", children: "Traversal Depth" }), _jsxs("span", { className: "muted gv-depth-value", children: [depth, " levels"] })] }), _jsx("input", { type: "range", min: 1, max: 5, step: 1, value: depth, onChange: (e) => setDepth(Number(e.target.value)), className: "gv-slider" }), _jsx("div", { className: "gv-slider-ticks", children: [1, 2, 3, 4, 5].map((n) => _jsx("span", { children: n }, n)) })] }), _jsxs("div", { className: "gv-toggles", children: [_jsx(Toggle, { checked: showLabels, onChange: setShowLabels, label: "Show labels", icon: Icon.funnel }), _jsx(Toggle, { checked: clusterView, onChange: setClusterView, label: "Cluster view", icon: Icon.layers }), _jsx(Toggle, { checked: highlightPaths, onChange: setHighlightPaths, label: "Highlight paths", icon: Icon.share }), _jsx(Toggle, { checked: showConstraints, onChange: setShowConstraints, label: "Show constraints", icon: Icon.shield })] })] }), _jsxs(Card, { title: _jsxs("span", { className: "gv-canvas-title", children: [_jsx("span", { children: "Ontology Graph" }), _jsxs("span", { className: "badge-live", children: [_jsx("span", { className: "dot" }), " Live"] })] }), actions: _jsxs("div", { className: "gv-toolbar", children: [_jsxs("div", { className: "gv-toolbar-group gv-layout-picker", children: [_jsxs("button", { className: "gv-tool-btn", onClick: () => setLayoutOpen((v) => !v), children: [_jsx("span", { className: "gv-tool-icon", children: Icon.layout }), _jsx("span", { children: "Layout" }), _jsx("span", { className: "gv-caret", children: "\u25BE" })] }), layoutOpen && (_jsx("ul", { className: "gv-layout-menu", onMouseLeave: () => setLayoutOpen(false), children: LAYOUTS.map((l) => (_jsx("li", { children: _jsx("button", { className: `gv-layout-item${layoutDir === l.value ? " active" : ""}`, onClick: () => { setLayoutDir(l.value); setLayoutOpen(false); }, children: l.label }) }, l.value))) }))] }), _jsx("button", { className: "gv-tool-btn icon", onClick: () => canvasRef.current?.zoomIn(), "aria-label": "Zoom in", children: Icon.plus }), _jsx("button", { className: "gv-tool-btn icon", onClick: () => canvasRef.current?.zoomOut(), "aria-label": "Zoom out", children: Icon.minus }), _jsx("button", { className: "gv-tool-btn icon", onClick: () => canvasRef.current?.fit(), "aria-label": "Fit view", children: Icon.fit }), _jsx("button", { className: "gv-tool-btn icon", onClick: onFullscreen, "aria-label": "Fullscreen", children: Icon.expand }), _jsx("button", { className: "gv-tool-btn icon", onClick: () => setHighlightPaths((v) => !v), "aria-label": "Toggle filters", children: Icon.funnel }), _jsx("button", { className: "gv-tool-btn icon", onClick: () => loadSubgraph(), "aria-label": "Refresh", disabled: busy, children: Icon.refresh })] }), children: [_jsx("div", { id: "gv-canvas-wrap", className: "gv-canvas", children: _jsx(GraphCanvas, { ref: canvasRef, subgraph: filteredSubgraph, layoutDir: layoutDir, showLabels: showLabels, highlightPaths: highlightPaths, selectedNodeId: selectedId, conceptTypeColors: conceptTypeColors, onNodeClick: onNodeClick }) }), _jsxs("div", { className: "gv-legend", children: [_jsxs("span", { className: "gv-legend-item", children: [_jsx("span", { className: "dot", style: { background: "#2563eb" } }), " Class"] }), _jsxs("span", { className: "gv-legend-item", children: [_jsx("span", { className: "dot", style: { background: "#16a34a" } }), " Entity"] }), _jsxs("span", { className: "gv-legend-item", children: [_jsx("span", { className: "dot", style: { background: "#7c3aed" } }), " Relation"] }), _jsxs("span", { className: "gv-legend-item", children: [_jsx("span", { className: "dot", style: { background: "#d97706" } }), " Data Type"] }), _jsxs("span", { className: "gv-legend-item", children: [_jsx("span", { className: "dot", style: { background: "#dc2626" } }), " Constraint"] })] })] }), _jsxs(Card, { className: "gv-inspector-card", title: _jsx("span", { className: "gv-inspector-head", children: _jsx("span", { children: "Node Inspector" }) }), actions: _jsxs("span", { className: "gv-inspector-nav", children: [_jsx("button", { className: "icon-btn", "aria-label": "Previous", onClick: () => setCollapseInspector(true), children: Icon.chevL }), _jsx("button", { className: "icon-btn", "aria-label": "Next", onClick: () => setCollapseInspector(false), children: Icon.chevR })] }), children: [_jsxs("div", { className: "gv-tabs", children: [_jsx("button", { className: `gv-tab${tab === "inspector" ? " active" : ""}`, onClick: () => setTab("inspector"), children: "Inspector" }), _jsxs("button", { className: `gv-tab${tab === "rules" ? " active" : ""}`, onClick: () => setTab("rules"), children: ["Rules ", _jsx("span", { className: "gv-tab-count", children: rules.length })] }), _jsxs("button", { className: `gv-tab${tab === "actions" ? " active" : ""}`, onClick: () => setTab("actions"), children: ["Actions ", _jsx("span", { className: "gv-tab-count", children: actions.length })] })] }), tab === "inspector" && (selectedConcept ? (_jsxs("div", { className: "gv-inspector", children: [_jsxs("div", { className: "gv-inspector-title", children: [_jsx("span", { className: "gv-inspector-avatar", style: { background: `${conceptTypeColors[selectedConcept.concept_type] ?? "#2563eb"}22`, color: conceptTypeColors[selectedConcept.concept_type] ?? "#2563eb" }, children: Icon.person }), _jsx("span", { className: "gv-inspector-name", style: { color: conceptTypeColors[selectedConcept.concept_type] ?? "var(--accent)" }, children: selectedConcept.name }), _jsx("span", { className: "badge badge-accent", children: "Class" })] }), _jsxs("div", { className: "gv-inspector-uri", children: [_jsx("span", { className: "muted", children: "URI:" }), " ", _jsxs("span", { className: "mono", children: ["ex:", selectedConcept.name] })] }), (selectedConcept.description || selectedTypeDef?.description) && (_jsx("p", { className: "gv-inspector-desc", children: selectedConcept.description || selectedTypeDef?.description })), _jsx("h4", { className: "gv-section-h", children: "Overview" }), _jsxs("ul", { className: "gv-overview", children: [_jsxs("li", { children: [_jsx("span", { className: "muted", children: "Node Type" }), _jsxs("span", { children: [selectedConcept.concept_type, _jsx("span", { className: "gv-chev", children: Icon.chevR })] })] }), _jsxs("li", { children: [_jsx("span", { className: "muted", children: "Total Connections" }), _jsxs("span", { children: [selectedStats.total, _jsx("span", { className: "gv-chev", children: Icon.chevR })] })] }), _jsxs("li", { children: [_jsx("span", { className: "muted", children: "Incoming Relations" }), _jsxs("span", { children: [selectedStats.incoming, _jsx("span", { className: "gv-chev", children: Icon.chevR })] })] }), _jsxs("li", { children: [_jsx("span", { className: "muted", children: "Outgoing Relations" }), _jsxs("span", { children: [selectedStats.outgoing, _jsx("span", { className: "gv-chev", children: Icon.chevR })] })] })] }), _jsxs("h4", { className: "gv-section-h", children: ["Properties (", selectedProps.length, ")"] }), selectedProps.length === 0 ? (_jsx("div", { className: "muted gv-empty-mini", children: "No declared properties." })) : (_jsx("ul", { className: "gv-props", children: selectedProps.map((p) => (_jsxs("li", { children: [_jsx("span", { className: "gv-prop-name", children: p.name }), _jsx("span", { className: "gv-prop-type mono muted", children: p.type })] }, p.name))) })), _jsxs("div", { className: "gv-inspector-actions", children: [_jsxs("button", { className: "btn-ghost-outline", onClick: onFocusNode, children: [_jsx("span", { children: Icon.focus }), " Focus Node"] }), _jsxs("button", { className: "btn-ghost-outline", onClick: onExpandNeighbors, children: [_jsx("span", { children: Icon.share }), " Expand Neighbors"] }), _jsxs("button", { className: "btn-primary gv-run", onClick: onRunQuery, children: [_jsx("span", { children: Icon.play }), " Run Query"] })] })] })) : (_jsx("div", { className: "empty gv-empty", children: "Click a node in the graph to inspect its concept type, properties, and connections." }))), tab === "rules" && (rules.length === 0 ? (_jsx("div", { className: "empty gv-empty", children: "No rules declared in this ontology." })) : (_jsx("ul", { className: "gv-rule-list", children: rules.map((r) => (_jsxs("li", { className: "gv-rule-card", children: [_jsxs("div", { className: "gv-rule-head", children: [_jsx("span", { className: "gv-rule-name", children: r.name }), r.strict ? _jsx("span", { className: "badge badge-danger", children: "strict" }) : _jsx("span", { className: "badge badge-accent", children: "advisory" })] }), r.description && _jsx("p", { className: "muted gv-rule-desc", children: r.description }), r.when && _jsxs("div", { className: "gv-rule-row", children: [_jsx("span", { className: "gv-rule-k", children: "WHEN" }), _jsx("span", { className: "gv-rule-v", children: r.when })] }), r.then && _jsxs("div", { className: "gv-rule-row", children: [_jsx("span", { className: "gv-rule-k", children: "THEN" }), _jsx("span", { className: "gv-rule-v", children: r.then })] }), r.applies_to && r.applies_to.length > 0 && (_jsx("div", { className: "gv-rule-tags", children: r.applies_to.map((t) => _jsx("span", { className: "badge", children: t }, t)) }))] }, r.name))) }))), tab === "actions" && (actions.length === 0 ? (_jsx("div", { className: "empty gv-empty", children: "No actions declared in this ontology." })) : (_jsx("ul", { className: "gv-rule-list", children: actions.map((a) => (_jsxs("li", { className: "gv-rule-card", children: [_jsxs("div", { className: "gv-rule-head", children: [_jsx("span", { className: "gv-rule-name", children: a.name }), _jsx("span", { className: "badge badge-success", children: "action" })] }), a.description && _jsx("p", { className: "muted gv-rule-desc", children: a.description }), _jsxs("div", { className: "gv-rule-row", children: [_jsx("span", { className: "gv-rule-k", children: "SUBJECT" }), _jsxs("span", { className: "gv-rule-v", children: [a.subject, a.object ? _jsxs(_Fragment, { children: [" \u2192 ", _jsx("strong", { children: a.object })] }) : null] })] }), a.parameters && a.parameters.length > 0 && (_jsxs("div", { className: "gv-rule-row", children: [_jsx("span", { className: "gv-rule-k", children: "PARAMS" }), _jsx("span", { className: "gv-rule-tags", children: a.parameters.map((p) => _jsx("span", { className: "badge", children: p }, p)) })] })), a.effect && _jsxs("div", { className: "gv-rule-row", children: [_jsx("span", { className: "gv-rule-k", children: "EFFECT" }), _jsx("span", { className: "gv-rule-v", children: a.effect })] })] }, a.name))) })))] })] }), _jsxs("div", { className: "dash-row dash-row-three", children: [_jsx(Card, { title: "Recent Paths / Saved Views", actions: _jsx("button", { className: "btn-ghost-link", onClick: () => navigate("/queries"), children: "View All" }), children: recentPaths.length === 0 ? (_jsx("div", { className: "empty gv-empty", children: "No saved views yet." })) : (_jsx("ul", { className: "gv-path-list", children: recentPaths.map((p) => (_jsxs("li", { className: "gv-path-item", children: [_jsx("span", { className: "gv-path-ic", children: Icon.pathIcon }), _jsx("span", { className: "gv-path-text", title: p.query, children: p.name }), _jsx("span", { className: "muted gv-path-time", children: p.last_run_at ? fmtAgo(p.last_run_at) : "—" })] }, p.id))) })) }), _jsx(Card, { title: "Graph Activity", actions: _jsx("button", { className: "btn-ghost-link", onClick: () => navigate("/files"), children: "View All" }), children: activity.length === 0 ? (_jsx("div", { className: "empty gv-empty", children: "No recent activity." })) : (_jsx("ul", { className: "gv-activity", children: activity.map((f) => {
                                const s = (f.status || "").toLowerCase();
                                const tone = s === "processed" || s === "ingested" || s === "done" ? "ok"
                                    : s === "failed" || s === "error" ? "err"
                                        : s === "analyzed" ? "info"
                                            : "warn";
                                const verb = tone === "ok" ? "Bulk import completed for"
                                    : tone === "err" ? "Validation failed on"
                                        : tone === "info" ? "Node updated from"
                                            : "New ingestion in progress for";
                                return (_jsxs("li", { className: "gv-activity-item", children: [_jsx("span", { className: `gv-act-ic gv-act-${tone}`, children: Icon.activity }), _jsxs("span", { className: "gv-act-text", children: [verb, " ", _jsx("em", { children: f.name })] }), _jsx("span", { className: "muted gv-act-time", children: fmtAgo(f.uploaded_at) })] }, f.id));
                            }) })) }), _jsx(Card, { title: "Query Suggestions", actions: _jsx("button", { className: "btn-ghost-link", onClick: () => navigate("/queries"), children: "View All" }), children: querySuggestions.length === 0 ? (_jsx("div", { className: "empty gv-empty", children: "No suggestions available." })) : (_jsx("ul", { className: "gv-suggestions", children: querySuggestions.map((q, i) => (_jsxs("li", { className: "gv-suggestion", children: [_jsx("span", { className: "gv-sug-ic", children: Icon.search }), _jsx("button", { className: "gv-sug-text", onClick: () => navigate(`/queries?q=${encodeURIComponent(q)}`), children: q })] }, i))) })) })] })] }));
}
