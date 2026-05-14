import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
import { useEffect, useState } from "react";
import Card from "../components/Card";
import StatCard from "../components/StatCard";
import Sparkline from "../components/Sparkline";
import { getFiles, getStats, getStatsHistory } from "../api";
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
        return `${Math.floor(diff / 60)}m ago`;
    if (diff < 86400)
        return `${Math.floor(diff / 3600)}h ago`;
    return `${Math.floor(diff / 86400)}d ago`;
}
export default function Dashboard() {
    const [stats, setStats] = useState(null);
    const [history, setHistory] = useState(null);
    const [files, setFiles] = useState([]);
    const [error, setError] = useState(null);
    useEffect(() => {
        let cancelled = false;
        const tick = async () => {
            try {
                const [s, h, f] = await Promise.all([getStats(), getStatsHistory(), getFiles()]);
                if (cancelled)
                    return;
                setStats(s);
                setHistory(h);
                setFiles(f.files);
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
    return (_jsxs(_Fragment, { children: [_jsx("div", { className: "page-header", children: _jsxs("div", { children: [_jsx("h1", { className: "page-title", children: "Dashboard" }), _jsx("p", { className: "page-subtitle", children: "Overview of your knowledge graph and recent activity." })] }) }), error && _jsx("div", { className: "error-banner", children: error }), _jsxs("div", { className: "grid grid-4", style: { marginBottom: 16 }, children: [_jsx(StatCard, { label: "Concepts", value: stats?.concepts ?? 0, deltaPct: stats?.deltas.concepts_pct }), _jsx(StatCard, { label: "Relations", value: stats?.relations ?? 0, deltaPct: stats?.deltas.relations_pct }), _jsx(StatCard, { label: "Concept types", value: stats?.concept_types ?? 0, deltaPct: stats?.deltas.concept_types_pct }), _jsx(StatCard, { label: "Relation types", value: stats?.relation_types ?? 0, deltaPct: stats?.deltas.relation_types_pct })] }), _jsxs("div", { className: "grid grid-2", style: { marginBottom: 16 }, children: [_jsx(Card, { title: "Concepts over time", children: history && history.samples.length > 0 ? (_jsxs(_Fragment, { children: [_jsx(Sparkline, { values: history.samples.map((s) => s.concepts) }), _jsxs("div", { className: "muted", style: { fontSize: 12, marginTop: 8 }, children: [history.samples.length, " samples \u00B7 last update ", fmtAgo(history.samples[history.samples.length - 1].ts)] })] })) : (_jsx("div", { className: "empty", children: "No samples yet. The dashboard auto-refreshes every 15 s." })) }), _jsx(Card, { title: "Relations over time", children: history && history.samples.length > 0 ? (_jsx(Sparkline, { values: history.samples.map((s) => s.relations), stroke: "#7c3aed" })) : (_jsx("div", { className: "empty", children: "No samples yet." })) })] }), _jsx(Card, { title: "Recent files", subtitle: "Latest uploads ingested into the graph", children: files.length === 0 ? (_jsxs("div", { className: "empty", children: ["Nothing here yet. Head to ", _jsx("strong", { children: "Files" }), " to upload your first dataset."] })) : (_jsxs("table", { className: "table", children: [_jsx("thead", { children: _jsxs("tr", { children: [_jsx("th", { children: "File" }), _jsx("th", { children: "Kind" }), _jsx("th", { children: "Size" }), _jsx("th", { children: "Ingested" }), _jsx("th", { children: "Uploaded" })] }) }), _jsx("tbody", { children: files.slice(0, 6).map((f) => (_jsxs("tr", { children: [_jsx("td", { children: f.name }), _jsx("td", { children: _jsx("span", { className: "badge", children: f.kind }) }), _jsx("td", { children: fmtBytes(f.size) }), _jsx("td", { children: _jsxs("span", { className: "muted", children: [f.concepts, " c \u00B7 ", f.relations, " r"] }) }), _jsx("td", { className: "muted", children: fmtAgo(f.uploaded_at) })] }, f.id))) })] })) })] }));
}
