import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import Card from "../components/Card";
import Sparkline from "../components/Sparkline";
import { createConcept, createRelation, deleteConcept, deleteRelation, getOntology, getStats, getStatsHistory, getSubgraph, listConcepts, listRelations, updateConcept, } from "../api";
// ---------------------------------------------------------------------------
// Constants & helpers
// ---------------------------------------------------------------------------
const PAGE_SIZE = 7;
function fmtNum(n) {
    return n.toLocaleString("en-US");
}
function fmtDate(ts) {
    if (!ts)
        return "—";
    return new Date(ts * 1000).toLocaleDateString("en-US", {
        year: "numeric",
        month: "short",
        day: "numeric",
    });
}
function conceptStatus(c) {
    const raw = c.properties?.status?.toLowerCase();
    if (raw === "reviewed")
        return { label: "Reviewed", cls: "badge-accent" };
    if (raw === "draft")
        return { label: "Draft", cls: "badge-warn" };
    if (raw === "archived")
        return { label: "Archived", cls: "badge-danger" };
    return { label: "Active", cls: "badge-success" };
}
function conceptUpdatedAt(c) {
    const v = c.properties?.updated_at ?? c.properties?.created_at;
    if (typeof v === "number")
        return v;
    if (typeof v === "string") {
        const t = Date.parse(v);
        return isNaN(t) ? null : Math.floor(t / 1000);
    }
    return null;
}
function conceptDomain(c, types) {
    // Walk up the parent chain to find the root concept type, fall back to the
    // direct type if there's no parent.
    const direct = c.concept_type;
    let cur = direct;
    const seen = new Set();
    while (cur && !seen.has(cur)) {
        seen.add(cur);
        const def = types[cur];
        if (!def?.parent)
            return cur;
        cur = def.parent;
    }
    return direct;
}
function conceptIconColor(name) {
    // Deterministic pastel based on the name's char codes.
    const palette = ["#2563eb", "#7c3aed", "#16a34a", "#d97706", "#dc2626", "#0ea5e9", "#db2777"];
    let h = 0;
    for (let i = 0; i < name.length; i++)
        h = (h * 31 + name.charCodeAt(i)) >>> 0;
    return palette[h % palette.length];
}
function initials(name) {
    return name
        .split(/\s+/)
        .map((w) => w[0])
        .filter(Boolean)
        .slice(0, 2)
        .join("")
        .toUpperCase();
}
// ---------------------------------------------------------------------------
// Inline SVG icons
// ---------------------------------------------------------------------------
const Icon = {
    layers: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M12 2 2 7l10 5 10-5-10-5z" }), _jsx("path", { d: "m2 17 10 5 10-5" }), _jsx("path", { d: "m2 12 10 5 10-5" })] })),
    group: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2" }), _jsx("circle", { cx: "9", cy: "7", r: "4" }), _jsx("path", { d: "M22 21v-2a4 4 0 0 0-3-3.87" }), _jsx("path", { d: "M16 3.13a4 4 0 0 1 0 7.75" })] })),
    share: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("circle", { cx: "18", cy: "5", r: "3" }), _jsx("circle", { cx: "6", cy: "12", r: "3" }), _jsx("circle", { cx: "18", cy: "19", r: "3" }), _jsx("path", { d: "m8.59 13.51 6.83 3.98" }), _jsx("path", { d: "m15.41 6.51-6.82 3.98" })] })),
    shield: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" }), _jsx("path", { d: "m9 12 2 2 4-4" })] })),
    search: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("circle", { cx: "11", cy: "11", r: "7" }), _jsx("path", { d: "m21 21-4.3-4.3" })] })),
    plus: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M12 5v14" }), _jsx("path", { d: "M5 12h14" })] })),
    expand: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M15 3h6v6" }), _jsx("path", { d: "M9 21H3v-6" }), _jsx("path", { d: "M21 3l-7 7" }), _jsx("path", { d: "M3 21l7-7" })] })),
    edit: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M12 20h9" }), _jsx("path", { d: "M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4z" })] })),
    upload: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" }), _jsx("path", { d: "m17 8-5-5-5 5" }), _jsx("path", { d: "M12 3v12" })] })),
    spark: (_jsx("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: _jsx("path", { d: "m12 3 1.9 5.8H20l-4.9 3.6L17 18.2 12 14.6 7 18.2l1.9-5.8L4 8.8h6.1z" }) })),
    pencil: (_jsx("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: _jsx("path", { d: "M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5z" }) })),
    check: (_jsx("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: _jsx("path", { d: "m20 6-11 11-5-5" }) })),
    chevDown: (_jsx("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: _jsx("path", { d: "m6 9 6 6 6-6" }) })),
    chevRight: (_jsx("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: _jsx("path", { d: "m9 6 6 6-6 6" }) })),
    dots: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("circle", { cx: "12", cy: "5", r: "1.5" }), _jsx("circle", { cx: "12", cy: "12", r: "1.5" }), _jsx("circle", { cx: "12", cy: "19", r: "1.5" })] })),
    download: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" }), _jsx("path", { d: "m7 10 5 5 5-5" }), _jsx("path", { d: "M12 15V3" })] })),
};
function RichStat({ label, value, display, deltaPct, icon, tone, spark, sparkColor }) {
    const cls = deltaPct == null ? "flat" : deltaPct > 0.05 ? "up" : deltaPct < -0.05 ? "down" : "flat";
    const arrow = cls === "up" ? "↑" : cls === "down" ? "↓" : "•";
    return (_jsxs("div", { className: "card stat-rich", children: [_jsx("div", { className: `stat-icon tone-${tone}`, children: icon }), _jsxs("div", { className: "stat-body", children: [_jsx("div", { className: "stat-label", children: label }), _jsx("div", { className: "stat-value", children: display ?? fmtNum(value) }), deltaPct != null && (_jsxs("div", { className: `stat-delta ${cls}`, children: [_jsxs("span", { children: [arrow, " ", Math.abs(deltaPct).toFixed(0), "%"] }), _jsx("span", { className: "muted", children: "vs last month" })] }))] }), _jsx("div", { className: "stat-spark", children: _jsx(Sparkline, { values: spark.length > 1 ? spark : [0, 0], stroke: sparkColor }) })] }));
}
function buildHierarchy(types) {
    const nodes = {};
    const names = Object.keys(types);
    for (const n of names)
        nodes[n] = { name: n, children: [] };
    const roots = [];
    for (const n of names) {
        const parent = types[n].parent;
        if (parent && nodes[parent])
            nodes[parent].children.push(nodes[n]);
        else
            roots.push(nodes[n]);
    }
    const sortRec = (n) => {
        n.children.sort((a, b) => a.name.localeCompare(b.name));
        n.children.forEach(sortRec);
    };
    roots.sort((a, b) => a.name.localeCompare(b.name));
    roots.forEach(sortRec);
    return roots;
}
function HierarchyNode({ node, depth }) {
    const [open, setOpen] = useState(depth < 2);
    const hasChildren = node.children.length > 0;
    return (_jsxs("div", { className: "tree-node", style: { paddingLeft: depth * 14 }, children: [_jsxs("div", { className: "tree-row", children: [hasChildren ? (_jsx("button", { type: "button", className: "tree-toggle", onClick: () => setOpen((v) => !v), "aria-label": open ? "Collapse" : "Expand", children: _jsx("span", { className: "tree-toggle-icon", children: open ? Icon.chevDown : Icon.chevRight }) })) : (_jsx("span", { className: "tree-toggle tree-toggle-empty" })), _jsx("span", { className: "tree-bullet", style: { background: conceptIconColor(node.name) } }), _jsx("span", { className: "tree-label", children: node.name })] }), hasChildren && open && (_jsx("div", { children: node.children.map((c) => (_jsx(HierarchyNode, { node: c, depth: depth + 1 }, c.name))) }))] }));
}
// ---------------------------------------------------------------------------
// Main page
// ---------------------------------------------------------------------------
export default function Concepts() {
    const [stats, setStats] = useState(null);
    const [history, setHistory] = useState(null);
    const [ontology, setOntology] = useState(null);
    const [coverage, setCoverage] = useState(null);
    const [search, setSearch] = useState("");
    const [debouncedSearch, setDebouncedSearch] = useState("");
    const [typeFilter, setTypeFilter] = useState("");
    const [statusFilter, setStatusFilter] = useState("");
    const [sort, setSort] = useState("updated");
    const [concepts, setConcepts] = useState([]);
    const [total, setTotal] = useState(0);
    const [page, setPage] = useState(0);
    const [selected, setSelected] = useState(null);
    const [recent, setRecent] = useState([]);
    const [domainCounts, setDomainCounts] = useState({});
    const [error, setError] = useState(null);
    const [info, setInfo] = useState(null);
    const [busy, setBusy] = useState(false);
    // ---- Debounce search ----
    useEffect(() => {
        const t = setTimeout(() => setDebouncedSearch(search.trim()), 300);
        return () => clearTimeout(t);
    }, [search]);
    // ---- Reset page when filters change ----
    useEffect(() => {
        setPage(0);
    }, [debouncedSearch, typeFilter, statusFilter, sort]);
    // ---- Load stats/history/ontology + coverage ----
    const refreshSidecar = async () => {
        try {
            const [s, h, o, sg] = await Promise.all([
                getStats(),
                getStatsHistory(),
                getOntology(),
                getSubgraph({ limit: 500, expansion_depth: 1 }),
            ]);
            setStats(s);
            setHistory(h);
            setOntology(o);
            // Coverage = fraction of returned concepts that have at least one relation
            const linked = new Set();
            for (const r of sg.subgraph.relations) {
                linked.add(r.source);
                linked.add(r.target);
            }
            const totalNodes = sg.subgraph.concepts.length;
            if (totalNodes > 0) {
                setCoverage(Math.round((linked.size / totalNodes) * 100));
            }
            else {
                setCoverage(null);
            }
        }
        catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
    };
    useEffect(() => {
        refreshSidecar();
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);
    // ---- Recent activity (most-recent ids) ----
    const refreshRecent = async () => {
        try {
            const r = await listConcepts({ limit: 500 });
            const sorted = [...r.concepts].sort((a, b) => b.id - a.id).slice(0, 5);
            setRecent(sorted);
        }
        catch {
            /* non-fatal */
        }
    };
    useEffect(() => {
        refreshRecent();
    }, []);
    // ---- Domain counts from ontology types ----
    useEffect(() => {
        if (!ontology)
            return;
        let cancelled = false;
        (async () => {
            const types = Object.keys(ontology.concept_types);
            const counts = {};
            // Use the root domain (top-level ancestor) as the grouping bucket.
            const rootOf = (t) => {
                const seen = new Set();
                let cur = t;
                while (cur && !seen.has(cur)) {
                    seen.add(cur);
                    const p = ontology.concept_types[cur]?.parent;
                    if (!p)
                        return cur;
                    cur = p;
                }
                return t;
            };
            await Promise.all(types.map(async (t) => {
                try {
                    const r = await listConcepts({ type: t, limit: 1 });
                    const root = rootOf(t);
                    counts[root] = (counts[root] ?? 0) + r.total;
                }
                catch {
                    /* ignore */
                }
            }));
            if (!cancelled)
                setDomainCounts(counts);
        })();
        return () => {
            cancelled = true;
        };
    }, [ontology]);
    // ---- Load concept list whenever filters or page change ----
    useEffect(() => {
        let cancelled = false;
        (async () => {
            try {
                // Server supports type filter natively. Status filter is client-side
                // (the server has no status field). For server-side pagination we
                // fetch the slice and apply client-side status filter on the slice.
                const limit = PAGE_SIZE;
                const offset = page * PAGE_SIZE;
                const r = await listConcepts({
                    type: typeFilter || undefined,
                    q: debouncedSearch || undefined,
                    limit,
                    offset,
                });
                if (cancelled)
                    return;
                let rows = r.concepts;
                if (statusFilter) {
                    rows = rows.filter((c) => conceptStatus(c).label.toLowerCase() === statusFilter);
                }
                if (sort === "name")
                    rows = [...rows].sort((a, b) => a.name.localeCompare(b.name));
                else if (sort === "type")
                    rows = [...rows].sort((a, b) => a.concept_type.localeCompare(b.concept_type));
                else
                    rows = [...rows].sort((a, b) => (conceptUpdatedAt(b) ?? 0) - (conceptUpdatedAt(a) ?? 0));
                setConcepts(rows);
                setTotal(r.total);
                setSelected((prev) => prev ?? rows[0] ?? null);
            }
            catch (e) {
                if (!cancelled)
                    setError(e instanceof Error ? e.message : String(e));
            }
        })();
        return () => {
            cancelled = true;
        };
    }, [debouncedSearch, typeFilter, statusFilter, sort, page]);
    // ---- Actions ----
    const handleCreate = async () => {
        const types = ontology ? Object.keys(ontology.concept_types) : [];
        if (types.length === 0) {
            setError("No concept types are defined yet. Create one in Ontology Builder first.");
            return;
        }
        const name = window.prompt("Concept name?")?.trim();
        if (!name)
            return;
        const conceptType = window.prompt(`Concept type? (${types.slice(0, 6).join(", ")}${types.length > 6 ? ", …" : ""})`, types[0])?.trim();
        if (!conceptType)
            return;
        const description = window.prompt("Definition (optional)?")?.trim() ?? "";
        setBusy(true);
        setError(null);
        try {
            await createConcept({ concept_type: conceptType, name, description });
            setInfo(`Concept "${name}" created.`);
            // Reload list and sidecar.
            setPage(0);
            await Promise.all([refreshSidecar(), refreshRecent()]);
            // Trigger list reload by toggling debouncedSearch (no-op set).
            setDebouncedSearch((s) => s);
        }
        catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
        finally {
            setBusy(false);
        }
    };
    const handleEdit = async (c) => {
        const name = window.prompt("Concept name", c.name)?.trim();
        if (name === undefined)
            return;
        const description = window.prompt("Definition", c.description ?? "")?.trim() ?? "";
        setBusy(true);
        try {
            const updated = await updateConcept(c.id, { name, description });
            setInfo(`Concept "${updated.name}" updated.`);
            setSelected(updated);
            // Reload the visible page.
            setDebouncedSearch((s) => s);
            await refreshRecent();
        }
        catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
        finally {
            setBusy(false);
        }
    };
    const handleDelete = async (c) => {
        if (!window.confirm(`Delete concept "${c.name}"? This cannot be undone.`))
            return;
        setBusy(true);
        try {
            await deleteConcept(c.id);
            setInfo(`Deleted "${c.name}".`);
            if (selected?.id === c.id)
                setSelected(null);
            setDebouncedSearch((s) => s);
            await Promise.all([refreshSidecar(), refreshRecent()]);
        }
        catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
        finally {
            setBusy(false);
        }
    };
    // ---- Derived data ----
    const sparkConcepts = useMemo(() => (history?.samples ?? []).map((s) => s.concepts), [history]);
    const sparkRelations = useMemo(() => (history?.samples ?? []).map((s) => s.relations), [history]);
    const sparkTypes = useMemo(() => (history?.samples ?? []).map((s) => s.concept_types), [history]);
    const sparkCoverage = useMemo(() => {
        // Coverage trend: fraction of relations / concepts at each sample.
        return (history?.samples ?? []).map((s) => s.concepts > 0 ? Math.min(100, Math.round((s.relations / s.concepts) * 100)) : 0);
    }, [history]);
    const types = ontology?.concept_types ?? {};
    const typeNames = Object.keys(types).sort();
    const hierarchy = useMemo(() => buildHierarchy(types), [types]);
    const domainList = useMemo(() => {
        const totalDom = Object.values(domainCounts).reduce((a, b) => a + b, 0);
        return Object.entries(domainCounts)
            .sort((a, b) => b[1] - a[1])
            .slice(0, 6)
            .map(([name, count]) => ({
            name,
            count,
            pct: totalDom > 0 ? Math.round((count / totalDom) * 1000) / 10 : 0,
        }));
    }, [domainCounts]);
    const totalPages = Math.max(1, Math.ceil(total / PAGE_SIZE));
    const showingFrom = total === 0 ? 0 : page * PAGE_SIZE + 1;
    const showingTo = Math.min(total, (page + 1) * PAGE_SIZE);
    // ---------------------------------------------------------------------------
    // Render
    // ---------------------------------------------------------------------------
    return (_jsxs(_Fragment, { children: [_jsx("div", { className: "page-header", children: _jsxs("div", { children: [_jsx("h2", { className: "page-title", children: "Concepts" }), _jsx("p", { className: "page-subtitle", children: "Browse, organize, and inspect ontology concepts and their definitions." })] }) }), error && _jsx("div", { className: "error-banner", children: error }), info && _jsx("div", { className: "success-banner", children: info }), _jsxs("div", { className: "dash-row dash-row-stats", children: [_jsx(RichStat, { label: "Total Concepts", value: stats?.concepts ?? 0, deltaPct: stats ? stats.deltas.concepts_pct * 100 : undefined, icon: Icon.layers, tone: "blue", spark: sparkConcepts, sparkColor: "#2563eb" }), _jsx(RichStat, { label: "Concept Groups", value: stats?.concept_types ?? 0, deltaPct: stats ? stats.deltas.concept_types_pct * 100 : undefined, icon: Icon.group, tone: "violet", spark: sparkTypes, sparkColor: "#7c3aed" }), _jsx(RichStat, { label: "Mapped Relations", value: stats?.relations ?? 0, deltaPct: stats ? stats.deltas.relations_pct * 100 : undefined, icon: Icon.share, tone: "amber", spark: sparkRelations, sparkColor: "#d97706" }), _jsx(RichStat, { label: "Coverage Score", value: coverage ?? 0, display: coverage == null ? "—" : `${coverage}%`, icon: Icon.shield, tone: "green", spark: sparkCoverage, sparkColor: "#16a34a" })] }), _jsxs("div", { className: "concepts-grid", children: [_jsxs(Card, { title: "Concept Library", actions: _jsx("button", { className: "btn-primary", onClick: handleCreate, disabled: busy, children: _jsxs("span", { style: { display: "inline-flex", alignItems: "center", gap: 6 }, children: [_jsx("span", { style: { width: 14, height: 14, display: "inline-flex" }, children: Icon.plus }), "Create Concept"] }) }), children: [_jsxs("div", { className: "concept-toolbar", children: [_jsxs("div", { className: "search-input", children: [_jsx("span", { className: "search-icon", children: Icon.search }), _jsx("input", { placeholder: "Search concepts\u2026", value: search, onChange: (e) => setSearch(e.target.value) })] }), _jsxs("select", { value: typeFilter, onChange: (e) => setTypeFilter(e.target.value), children: [_jsx("option", { value: "", children: "All Domains" }), typeNames.map((t) => (_jsx("option", { value: t, children: t }, t)))] }), _jsxs("select", { value: statusFilter, onChange: (e) => setStatusFilter(e.target.value), children: [_jsx("option", { value: "", children: "All Statuses" }), _jsx("option", { value: "active", children: "Active" }), _jsx("option", { value: "reviewed", children: "Reviewed" }), _jsx("option", { value: "draft", children: "Draft" }), _jsx("option", { value: "archived", children: "Archived" })] }), _jsxs("select", { value: sort, onChange: (e) => setSort(e.target.value), children: [_jsx("option", { value: "updated", children: "Sort: Last Updated" }), _jsx("option", { value: "name", children: "Sort: Name" }), _jsx("option", { value: "type", children: "Sort: Type" })] })] }), _jsxs("table", { className: "table", children: [_jsx("thead", { children: _jsxs("tr", { children: [_jsx("th", { children: "Concept Name" }), _jsx("th", { children: "Domain" }), _jsx("th", { children: "Definition" }), _jsx("th", { children: "Status" }), _jsx("th", { children: "Linked" }), _jsx("th", { children: "Last Updated" }), _jsx("th", { className: "actions", children: "Actions" })] }) }), _jsxs("tbody", { children: [concepts.length === 0 && (_jsx("tr", { children: _jsx("td", { colSpan: 7, className: "empty", children: "No concepts match the current filters." }) })), concepts.map((c) => {
                                                const st = conceptStatus(c);
                                                const dom = conceptDomain(c, types);
                                                const isSel = selected?.id === c.id;
                                                return (_jsxs("tr", { className: isSel ? "row-active" : undefined, onClick: () => setSelected(c), style: { cursor: "pointer" }, children: [_jsx("td", { children: _jsxs("div", { className: "concept-name-cell", children: [_jsx("span", { className: "concept-avatar", style: { background: conceptIconColor(c.name) }, children: initials(c.name) }), _jsx("span", { children: c.name })] }) }), _jsx("td", { children: dom }), _jsx("td", { className: "muted def-cell", children: c.description || "—" }), _jsx("td", { children: _jsx("span", { className: `badge ${st.cls}`, children: st.label }) }), _jsx("td", { children: fmtNum(Number(c.properties?.linked ?? 0)) }), _jsx("td", { children: fmtDate(conceptUpdatedAt(c)) }), _jsxs("td", { className: "actions", children: [_jsx("button", { className: "icon-btn", title: "Edit", onClick: (e) => { e.stopPropagation(); handleEdit(c); }, children: _jsx("span", { style: { width: 16, height: 16, display: "inline-flex" }, children: Icon.pencil }) }), _jsx("button", { className: "icon-btn", title: "Delete", onClick: (e) => { e.stopPropagation(); handleDelete(c); }, children: _jsx("span", { style: { width: 16, height: 16, display: "inline-flex" }, children: Icon.dots }) })] })] }, c.id));
                                            })] })] }), _jsxs("div", { className: "pagination", children: [_jsxs("span", { className: "muted", children: ["Showing ", showingFrom, "\u2013", showingTo, " of ", fmtNum(total), " concepts"] }), _jsxs("div", { className: "pager", children: [_jsx("button", { disabled: page === 0, onClick: () => setPage((p) => Math.max(0, p - 1)), children: "\u2039" }), Array.from({ length: Math.min(5, totalPages) }, (_, i) => i).map((i) => (_jsx("button", { className: i === page ? "pager-active" : "", onClick: () => setPage(i), children: i + 1 }, i))), totalPages > 5 && _jsx("span", { className: "muted", children: "\u2026" }), totalPages > 5 && (_jsx("button", { onClick: () => setPage(totalPages - 1), children: totalPages })), _jsx("button", { disabled: page + 1 >= totalPages, onClick: () => setPage((p) => Math.min(totalPages - 1, p + 1)), children: "\u203A" })] })] })] }), _jsxs("div", { className: "concepts-side", children: [_jsx(Card, { title: "Concept Details", actions: _jsx("button", { className: "icon-btn", title: "Expand", children: _jsx("span", { style: { width: 16, height: 16, display: "inline-flex" }, children: Icon.expand }) }), children: selected ? (_jsxs(_Fragment, { children: [_jsx(ConceptDetails, { concept: selected, domain: conceptDomain(selected, types), onEdit: () => handleEdit(selected) }), _jsx(RelationsPanel, { concept: selected, ontology: ontology, allConcepts: concepts })] })) : (_jsx("div", { className: "empty", children: "Select a concept from the library." })) }), _jsx(Card, { title: "Concept Hierarchy", children: hierarchy.length === 0 ? (_jsx("div", { className: "empty", children: "No concept types defined yet." })) : (_jsx("div", { className: "concept-tree", children: hierarchy.map((n) => (_jsx(HierarchyNode, { node: n, depth: 0 }, n.name))) })) })] })] }), _jsxs("div", { className: "dash-row dash-row-three", children: [_jsx(Card, { title: "Recent Concept Activity", actions: _jsx(Link, { to: "/builder", className: "btn-ghost-link", children: "View All" }), children: recent.length === 0 ? (_jsx("div", { className: "empty", children: "No recent activity." })) : (_jsx("ul", { className: "activity-list", children: recent.map((c) => (_jsxs("li", { children: [_jsx("span", { className: "activity-dot", style: { background: conceptIconColor(c.name) } }), _jsxs("span", { className: "activity-text", children: ["Concept ", _jsxs("strong", { children: ["\u201C", c.name, "\u201D"] }), " added"] }), _jsx("span", { className: "activity-time muted", children: fmtDate(conceptUpdatedAt(c)) })] }, c.id))) })) }), _jsx(Card, { title: "Top Domains", actions: _jsx(Link, { to: "/builder", className: "btn-ghost-link", children: "View All" }), children: domainList.length === 0 ? (_jsx("div", { className: "empty", children: "Loading domains\u2026" })) : (_jsx("ul", { className: "domain-list", children: domainList.map((d) => (_jsxs("li", { children: [_jsx("span", { className: "domain-name", children: d.name }), _jsx("span", { className: "domain-bar", children: _jsx("span", { className: "domain-bar-fill", style: { width: `${Math.min(100, d.pct)}%`, background: conceptIconColor(d.name) } }) }), _jsxs("span", { className: "domain-pct muted", children: [d.count, " (", d.pct.toFixed(1), "%)"] })] }, d.name))) })) }), _jsx(Card, { title: "Quick Actions", children: _jsxs("div", { className: "quick-actions", children: [_jsxs(Link, { to: "/files", className: "quick-action qa-blue", children: [_jsx("span", { className: "qa-icon", children: Icon.upload }), _jsxs("div", { children: [_jsx("div", { className: "qa-title", children: "Import Concepts" }), _jsx("div", { className: "qa-sub muted", children: "Import from files or sources" })] })] }), _jsxs(Link, { to: "/builder", className: "quick-action qa-violet", children: [_jsx("span", { className: "qa-icon", children: Icon.spark }), _jsxs("div", { children: [_jsx("div", { className: "qa-title", children: "Generate with AI" }), _jsx("div", { className: "qa-sub muted", children: "Auto-generate concepts" })] })] }), _jsxs("button", { type: "button", className: "quick-action qa-amber", onClick: () => setInfo("Bulk edit coming soon."), children: [_jsx("span", { className: "qa-icon", children: Icon.edit }), _jsxs("div", { style: { textAlign: "left" }, children: [_jsx("div", { className: "qa-title", children: "Bulk Edit" }), _jsx("div", { className: "qa-sub muted", children: "Edit multiple concepts" })] })] }), _jsxs("button", { type: "button", className: "quick-action qa-green", onClick: () => setInfo("Definition validation coming soon."), children: [_jsx("span", { className: "qa-icon", children: Icon.check }), _jsxs("div", { style: { textAlign: "left" }, children: [_jsx("div", { className: "qa-title", children: "Validate Definitions" }), _jsx("div", { className: "qa-sub muted", children: "Check quality & consistency" })] })] })] }) })] })] }));
}
function ConceptDetails({ concept, domain, onEdit }) {
    const synonyms = (() => {
        const v = concept.properties?.synonyms;
        if (Array.isArray(v))
            return v.map(String);
        if (typeof v === "string")
            return v.split(",").map((s) => s.trim()).filter(Boolean);
        return [];
    })();
    const owner = concept.properties?.owner ?? "—";
    const updated = conceptUpdatedAt(concept);
    return (_jsxs("div", { className: "concept-details", children: [_jsxs("div", { className: "cd-head", children: [_jsx("span", { className: "cd-avatar", style: { background: conceptIconColor(concept.name) }, children: initials(concept.name) }), _jsxs("div", { className: "cd-head-text", children: [_jsxs("div", { className: "cd-name-row", children: [_jsx("h3", { className: "cd-name", children: concept.name }), _jsx("span", { className: "badge badge-accent", children: concept.concept_type })] }), _jsx("p", { className: "cd-desc muted", children: concept.description || "No definition provided." })] })] }), _jsxs("dl", { className: "cd-grid", children: [_jsx("dt", { children: "URI" }), _jsxs("dd", { className: "mono", children: ["urn:concept:", concept.concept_type.toLowerCase(), ":", concept.id] }), _jsx("dt", { children: "Synonyms / Tags" }), _jsx("dd", { children: synonyms.length === 0 ? (_jsx("span", { className: "muted", children: "\u2014" })) : (_jsx("span", { className: "tag-row", children: synonyms.map((s) => _jsx("span", { className: "tag-chip", children: s }, s)) })) }), _jsx("dt", { children: "Domain" }), _jsx("dd", { children: domain }), _jsx("dt", { children: "Owner" }), _jsx("dd", { children: owner }), _jsx("dt", { children: "Last Updated" }), _jsx("dd", { children: fmtDate(updated) })] }), _jsxs("div", { className: "cd-actions", children: [_jsx("button", { onClick: onEdit, children: _jsxs("span", { style: { display: "inline-flex", alignItems: "center", gap: 6 }, children: [_jsx("span", { style: { width: 14, height: 14, display: "inline-flex" }, children: Icon.pencil }), "Edit Concept"] }) }), _jsx(Link, { to: `/graph?seed=${concept.id}`, className: "btn-link-wrap", children: _jsx("button", { children: _jsxs("span", { style: { display: "inline-flex", alignItems: "center", gap: 6 }, children: [_jsx("span", { style: { width: 14, height: 14, display: "inline-flex" }, children: Icon.share }), "View Relations"] }) }) }), _jsx("button", { children: _jsxs("span", { style: { display: "inline-flex", alignItems: "center", gap: 6 }, children: [_jsx("span", { style: { width: 14, height: 14, display: "inline-flex" }, children: Icon.download }), "Export"] }) })] })] }));
}
function RelationsPanel({ concept, ontology, allConcepts }) {
    const [outgoing, setOutgoing] = useState([]);
    const [incoming, setIncoming] = useState([]);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState(null);
    const [adding, setAdding] = useState(false);
    const [relType, setRelType] = useState("");
    const [targetId, setTargetId] = useState("");
    const conceptById = useMemo(() => {
        const m = new Map();
        for (const c of allConcepts)
            m.set(c.id, c);
        return m;
    }, [allConcepts]);
    // Relation types whose domain matches this concept's type.
    const relTypeOptions = useMemo(() => {
        const types = Object.values(ontology?.relation_types ?? {});
        return types.filter((rt) => rt.domain === concept.concept_type);
    }, [ontology, concept.concept_type]);
    // Targets restricted to the chosen relation type's range.
    const targetOptions = useMemo(() => {
        const rt = relTypeOptions.find((t) => t.name === relType);
        if (!rt)
            return allConcepts;
        return allConcepts.filter((c) => c.concept_type === rt.range);
    }, [allConcepts, relTypeOptions, relType]);
    async function refresh() {
        setLoading(true);
        setError(null);
        try {
            const [out, inc] = await Promise.all([
                listRelations({ source: concept.id, limit: 200 }),
                listRelations({ target: concept.id, limit: 200 }),
            ]);
            setOutgoing(out.relations);
            setIncoming(inc.relations);
        }
        catch (e) {
            setError(e.message);
        }
        finally {
            setLoading(false);
        }
    }
    useEffect(() => {
        refresh();
        setAdding(false);
        setRelType("");
        setTargetId("");
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [concept.id]);
    async function onAdd(e) {
        e.preventDefault();
        if (!relType || targetId === "")
            return;
        try {
            await createRelation({
                relation_type: relType,
                source: concept.id,
                target: Number(targetId),
            });
            setAdding(false);
            setRelType("");
            setTargetId("");
            await refresh();
        }
        catch (err) {
            alert("Add failed: " + err.message);
        }
    }
    async function onDelete(id) {
        if (!confirm("Delete this relation?"))
            return;
        try {
            await deleteRelation(id);
            await refresh();
        }
        catch (err) {
            alert("Delete failed: " + err.message);
        }
    }
    const renderRow = (r, otherId, direction) => {
        const other = conceptById.get(otherId);
        return (_jsxs("li", { className: "rel-row", children: [_jsx("span", { className: "rel-arrow", children: direction === "out" ? "→" : "←" }), _jsx("span", { className: "rel-type", children: r.relation_type }), _jsx("span", { className: "rel-other", children: other
                        ? `${other.concept_type}: ${other.name}`
                        : `Concept #${otherId}` }), _jsx("button", { type: "button", className: "rel-del", "aria-label": "Delete relation", onClick: () => onDelete(r.id), children: "\u00D7" })] }, r.id));
    };
    return (_jsxs("div", { className: "rel-panel", children: [_jsxs("div", { className: "rel-head", children: [_jsx("strong", { children: "Relations" }), _jsx("button", { type: "button", className: "btn-ghost", onClick: () => setAdding((v) => !v), disabled: relTypeOptions.length === 0, title: relTypeOptions.length === 0
                            ? "No relation types defined with this concept's type as domain."
                            : "", children: adding ? "Cancel" : "+ Add" })] }), adding && (_jsxs("form", { className: "rel-form", onSubmit: onAdd, children: [_jsxs("select", { value: relType, onChange: (e) => {
                            setRelType(e.target.value);
                            setTargetId("");
                        }, required: true, children: [_jsx("option", { value: "", disabled: true, children: "Relation type\u2026" }), relTypeOptions.map((rt) => (_jsxs("option", { value: rt.name, children: [rt.name, " (", rt.domain, " \u2192 ", rt.range, ")"] }, rt.name)))] }), _jsxs("select", { value: targetId === "" ? "" : String(targetId), onChange: (e) => setTargetId(e.target.value === "" ? "" : Number(e.target.value)), required: true, children: [_jsx("option", { value: "", disabled: true, children: "Target concept\u2026" }), targetOptions
                                .filter((c) => c.id !== concept.id)
                                .map((c) => (_jsxs("option", { value: c.id, children: [c.concept_type, ": ", c.name] }, c.id)))] }), _jsx("button", { type: "submit", className: "btn-primary", children: "Add" })] })), error && _jsx("div", { className: "banner banner-error", children: error }), loading ? (_jsx("div", { className: "muted", children: "Loading relations\u2026" })) : outgoing.length === 0 && incoming.length === 0 ? (_jsx("div", { className: "muted", children: "No relations." })) : (_jsxs(_Fragment, { children: [outgoing.length > 0 && (_jsxs(_Fragment, { children: [_jsx("div", { className: "rel-section-title muted", children: "Outgoing" }), _jsx("ul", { className: "rel-list", children: outgoing.map((r) => renderRow(r, r.target, "out")) })] })), incoming.length > 0 && (_jsxs(_Fragment, { children: [_jsx("div", { className: "rel-section-title muted", children: "Incoming" }), _jsx("ul", { className: "rel-list", children: incoming.map((r) => renderRow(r, r.source, "in")) })] }))] })), _jsx("style", { children: `
        .rel-panel { margin-top: 16px; padding-top: 12px; border-top: 1px solid #e2e8f0;
          display: flex; flex-direction: column; gap: 8px; }
        .rel-head { display: flex; align-items: center; justify-content: space-between; }
        .rel-form { display: flex; flex-direction: column; gap: 6px; padding: 8px;
          background: #f8fafc; border-radius: 8px; }
        .rel-form select { padding: 6px 8px; border: 1px solid #cbd5e1; border-radius: 6px;
          font: inherit; background: #fff; }
        .rel-section-title { font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em;
          margin-top: 4px; }
        .rel-list { list-style: none; margin: 0; padding: 0; display: flex;
          flex-direction: column; gap: 4px; }
        .rel-row { display: flex; align-items: center; gap: 8px; font-size: 13px;
          padding: 4px 6px; border-radius: 6px; }
        .rel-row:hover { background: #f1f5f9; }
        .rel-arrow { color: #64748b; font-weight: 600; }
        .rel-type { color: #2563eb; font-weight: 500; }
        .rel-other { flex: 1; color: #334155; }
        .rel-del { border: none; background: transparent; color: #94a3b8;
          cursor: pointer; font-size: 18px; line-height: 1; padding: 0 4px; }
        .rel-del:hover { color: #dc2626; }
      ` })] }));
}
