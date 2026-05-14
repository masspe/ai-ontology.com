import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
import { useEffect, useMemo, useState } from "react";
import Card from "../components/Card";
import Sparkline from "../components/Sparkline";
import { createAction, deleteAction, getOntology, getStats, listActions, listConcepts, updateAction, } from "../api";
function actionStatus(a) {
    const raw = a.parameters?.status?.toLowerCase();
    if (raw === "reviewed")
        return "Reviewed";
    if (raw === "draft")
        return "Draft";
    if (raw === "paused")
        return "Paused";
    return "Active";
}
function actionUpdatedAt(a) {
    const v = a.parameters?.updated_at ?? a.parameters?.created_at;
    if (typeof v === "number") {
        return new Date(v * 1000).toLocaleDateString("en-US", { year: "numeric", month: "short", day: "numeric" });
    }
    if (typeof v === "string") {
        const t = Date.parse(v);
        if (!isNaN(t))
            return new Date(t).toLocaleDateString("en-US", { year: "numeric", month: "short", day: "numeric" });
    }
    return "—";
}
function actionToRow(a) {
    return {
        id: a.id,
        name: a.name,
        type: a.action_type,
        trigger: a.parameters?.trigger ?? "Manual",
        description: a.description ?? a.effect ?? "",
        status: actionStatus(a),
        lastRun: actionUpdatedAt(a),
    };
}
const PALETTE = ["#2563eb", "#7c3aed", "#16a34a", "#d97706", "#dc2626", "#0891b2", "#db2777"];
function typeColor(name) {
    let h = 0;
    for (let i = 0; i < name.length; i++)
        h = (h * 31 + name.charCodeAt(i)) >>> 0;
    return PALETTE[h % PALETTE.length];
}
// ---------------------------------------------------------------------------
// Inline icons (local — no extra deps)
// ---------------------------------------------------------------------------
const Icon = {
    bolt: (_jsx("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: _jsx("path", { d: "M13 2 3 14h7l-1 8 10-12h-7z" }) })),
    bot: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("rect", { x: "3", y: "8", width: "18", height: "12", rx: "2" }), _jsx("path", { d: "M12 4v4" }), _jsx("circle", { cx: "8.5", cy: "14", r: "1.2", fill: "currentColor" }), _jsx("circle", { cx: "15.5", cy: "14", r: "1.2", fill: "currentColor" }), _jsx("path", { d: "M9 18h6" })] })),
    checkCircle: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("circle", { cx: "12", cy: "12", r: "10" }), _jsx("path", { d: "m9 12 2 2 4-4" })] })),
    alert: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("circle", { cx: "12", cy: "12", r: "10" }), _jsx("path", { d: "M12 8v4" }), _jsx("path", { d: "M12 16h.01" })] })),
    search: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("circle", { cx: "11", cy: "11", r: "7" }), _jsx("path", { d: "m21 21-4.3-4.3" })] })),
    plus: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M12 5v14" }), _jsx("path", { d: "M5 12h14" })] })),
    copy: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("rect", { x: "9", y: "9", width: "13", height: "13", rx: "2" }), _jsx("path", { d: "M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" })] })),
    edit: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M12 20h9" }), _jsx("path", { d: "M16.5 3.5a2.121 2.121 0 1 1 3 3L7 19l-4 1 1-4z" })] })),
    play: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("circle", { cx: "12", cy: "12", r: "10" }), _jsx("path", { d: "m10 8 6 4-6 4z" })] })),
    download: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" }), _jsx("path", { d: "m7 10 5 5 5-5" }), _jsx("path", { d: "M12 15V3" })] })),
    expand: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M3 9V3h6" }), _jsx("path", { d: "M21 15v6h-6" }), _jsx("path", { d: "M3 3l7 7" }), _jsx("path", { d: "m14 14 7 7" })] })),
    more: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "currentColor", children: [_jsx("circle", { cx: "5", cy: "12", r: "1.6" }), _jsx("circle", { cx: "12", cy: "12", r: "1.6" }), _jsx("circle", { cx: "19", cy: "12", r: "1.6" })] })),
    import: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" }), _jsx("path", { d: "m17 8-5-5-5 5" }), _jsx("path", { d: "M12 3v12" })] })),
    spark: (_jsx("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: _jsx("path", { d: "m12 3 1.9 5.8H20l-4.9 3.6L17 18.2 12 14.6 7 18.2l1.9-5.8L4 8.8h6.1z" }) })),
    bulk: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("rect", { x: "3", y: "4", width: "18", height: "4", rx: "1" }), _jsx("rect", { x: "3", y: "10", width: "18", height: "4", rx: "1" }), _jsx("rect", { x: "3", y: "16", width: "18", height: "4", rx: "1" })] })),
    calendar: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("rect", { x: "3", y: "5", width: "18", height: "16", rx: "2" }), _jsx("path", { d: "M16 3v4" }), _jsx("path", { d: "M8 3v4" }), _jsx("path", { d: "M3 11h18" })] })),
    xCircle: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("circle", { cx: "12", cy: "12", r: "10" }), _jsx("path", { d: "m15 9-6 6" }), _jsx("path", { d: "m9 9 6 6" })] })),
};
// ---------------------------------------------------------------------------
// Type → visual mapping
// ---------------------------------------------------------------------------
function typeBadge(type) {
    const known = {
        Automation: { cls: "action-type-automation", dot: "#2563eb" },
        Inference: { cls: "action-type-inference", dot: "#7c3aed" },
        Validation: { cls: "action-type-validation", dot: "#16a34a" },
        Transformation: { cls: "action-type-transformation", dot: "#d97706" },
        Alert: { cls: "action-type-alert", dot: "#dc2626" },
        "AI Action": { cls: "action-type-ai", dot: "#0891b2" },
    };
    return known[type] ?? { cls: "action-type-automation", dot: typeColor(type) };
}
function statusBadge(s) {
    switch (s) {
        case "Active": return "badge-success";
        case "Reviewed": return "badge-accent";
        case "Draft": return "badge-warn";
        case "Paused": return "badge-danger";
    }
}
function RichStat({ label, value, deltaPct, deltaDir, icon, tone, spark, sparkColor }) {
    const showDelta = deltaPct != null;
    const cls = deltaDir ?? (showDelta && deltaPct > 0 ? "up" : showDelta && deltaPct < 0 ? "down" : "flat");
    const arrow = cls === "up" ? "↑" : cls === "down" ? "↓" : "•";
    return (_jsxs("div", { className: "card stat-rich", children: [_jsx("div", { className: `stat-icon tone-${tone}`, children: icon }), _jsxs("div", { className: "stat-body", children: [_jsx("div", { className: "stat-label", children: label }), _jsx("div", { className: "stat-value", children: value }), showDelta && (_jsx("div", { className: `stat-delta ${cls}`, children: _jsxs("span", { children: [arrow, " ", Math.abs(deltaPct).toFixed(0), "%"] }) }))] }), _jsx("div", { className: "stat-spark", children: _jsx(Sparkline, { values: spark, stroke: sparkColor }) })] }));
}
// ---------------------------------------------------------------------------
// Donut chart for Run Status Breakdown
// ---------------------------------------------------------------------------
function Donut({ data }) {
    const total = data.reduce((s, d) => s + d.count, 0);
    const R = 46;
    const r = 30;
    const C = 2 * Math.PI * ((R + r) / 2);
    let offset = 0;
    const segments = data.map((d) => {
        const len = (d.count / total) * C;
        const seg = { color: d.color, len, offset };
        offset += len;
        return seg;
    });
    return (_jsxs("svg", { viewBox: "0 0 120 120", className: "donut", children: [_jsx("circle", { cx: "60", cy: "60", r: (R + r) / 2, fill: "none", stroke: "#f1f5f9", strokeWidth: R - r }), segments.map((s, i) => (_jsx("circle", { cx: "60", cy: "60", r: (R + r) / 2, fill: "none", stroke: s.color, strokeWidth: R - r, strokeDasharray: `${s.len} ${C}`, strokeDashoffset: -s.offset, transform: "rotate(-90 60 60)", strokeLinecap: "butt" }, i)))] }));
}
// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------
export default function Actions() {
    const [actions, setActions] = useState([]);
    const [stats, setStats] = useState(null);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState(null);
    const [selectedId, setSelectedId] = useState(null);
    const [search, setSearch] = useState("");
    const [typeFilter, setTypeFilter] = useState("All Types");
    const [statusFilter, setStatusFilter] = useState("All Statuses");
    const [sort, setSort] = useState("Last Updated");
    const [ontology, setOntology] = useState(null);
    const [allConcepts, setAllConcepts] = useState([]);
    const [editing, setEditing] = useState(null);
    useEffect(() => {
        let cancelled = false;
        (async () => {
            try {
                const [o, cs] = await Promise.all([
                    getOntology(),
                    listConcepts({ limit: 500 }),
                ]);
                if (cancelled)
                    return;
                setOntology(o);
                setAllConcepts(cs.concepts);
            }
            catch {
                /* non-fatal */
            }
        })();
        return () => {
            cancelled = true;
        };
    }, []);
    useEffect(() => {
        let cancelled = false;
        (async () => {
            try {
                const [as_, st] = await Promise.all([listActions(), getStats()]);
                if (cancelled)
                    return;
                setActions(as_);
                setStats(st);
                if (as_.length > 0)
                    setSelectedId(as_[0].id);
            }
            catch (e) {
                if (!cancelled)
                    setError(e.message);
            }
            finally {
                if (!cancelled)
                    setLoading(false);
            }
        })();
        return () => { cancelled = true; };
    }, []);
    const rows = useMemo(() => actions.map(actionToRow), [actions]);
    const typeOptions = useMemo(() => {
        const set = new Set(rows.map((r) => r.type));
        return ["All Types", ...Array.from(set).sort()];
    }, [rows]);
    const filtered = useMemo(() => {
        const q = search.trim().toLowerCase();
        let out = rows.filter((a) => {
            if (q && !(a.name.toLowerCase().includes(q) || a.description.toLowerCase().includes(q)))
                return false;
            if (typeFilter !== "All Types" && a.type !== typeFilter)
                return false;
            if (statusFilter !== "All Statuses" && a.status !== statusFilter)
                return false;
            return true;
        });
        if (sort === "Sort: Name")
            out = [...out].sort((a, b) => a.name.localeCompare(b.name));
        else if (sort === "Sort: Type")
            out = [...out].sort((a, b) => a.type.localeCompare(b.type));
        return out;
    }, [rows, search, typeFilter, statusFilter, sort]);
    const selected = useMemo(() => rows.find((r) => r.id === selectedId) ?? rows[0] ?? null, [rows, selectedId]);
    const selectedAction = useMemo(() => actions.find((a) => a.id === selectedId) ?? actions[0] ?? null, [actions, selectedId]);
    // Status breakdown derived from live rows.
    const runStatus = useMemo(() => {
        const counts = {
            Active: 0, Reviewed: 0, Draft: 0, Paused: 0,
        };
        for (const r of rows)
            counts[r.status] += 1;
        return [
            { name: "Active", count: counts.Active, color: "#2563eb" },
            { name: "Reviewed", count: counts.Reviewed, color: "#7c3aed" },
            { name: "Draft", count: counts.Draft, color: "#d97706" },
            { name: "Paused", count: counts.Paused, color: "#dc2626" },
        ].filter((d) => d.count > 0);
    }, [rows]);
    // Top types as a domain-like breakdown.
    const domains = useMemo(() => {
        const counts = new Map();
        for (const r of rows)
            counts.set(r.type, (counts.get(r.type) ?? 0) + 1);
        const total = rows.length || 1;
        return Array.from(counts.entries())
            .sort((a, b) => b[1] - a[1])
            .slice(0, 5)
            .map(([name, count]) => ({ name, count, pct: (count / total) * 100 }));
    }, [rows]);
    async function onDelete(id) {
        if (!confirm("Delete this action?"))
            return;
        try {
            await deleteAction(id);
            setActions((rs) => rs.filter((r) => r.id !== id));
        }
        catch (e) {
            alert("Delete failed: " + e.message);
        }
    }
    async function onSaveAction(payload, editingId) {
        try {
            if (editingId == null) {
                const { id } = await createAction(payload);
                setActions((rs) => [...rs, { id, ...payload }]);
                setSelectedId(id);
            }
            else {
                const updated = await updateAction(editingId, {
                    name: payload.name,
                    subject: payload.subject,
                    object: payload.object ?? null,
                    parameters: payload.parameters,
                    effect: payload.effect,
                    description: payload.description,
                });
                setActions((rs) => rs.map((a) => (a.id === editingId ? updated : a)));
            }
            setEditing(null);
        }
        catch (e) {
            alert("Save failed: " + e.message);
        }
    }
    const runTotal = runStatus.reduce((s, d) => s + d.count, 0) || 1;
    const totalActions = stats?.actions ?? actions.length;
    const activeActions = rows.filter((r) => r.status === "Active").length;
    return (_jsxs(_Fragment, { children: [_jsx("div", { className: "page-header", children: _jsxs("div", { children: [_jsx("h1", { className: "page-title", children: "Actions" }), _jsx("p", { className: "page-subtitle", children: "Create, schedule, monitor, and manage ontology actions and automations." })] }) }), error && _jsxs("div", { className: "banner banner-error", children: ["Failed to load actions: ", error] }), _jsxs("div", { className: "dash-row dash-row-stats", children: [_jsx(RichStat, { label: "Total Actions", value: String(totalActions), icon: Icon.bolt, tone: "blue", spark: [], sparkColor: "#2563eb" }), _jsx(RichStat, { label: "Active Automations", value: String(activeActions), icon: Icon.bot, tone: "green", spark: [], sparkColor: "#16a34a" }), _jsx(RichStat, { label: "Action Types", value: String(stats?.action_types ?? 0), icon: Icon.checkCircle, tone: "violet", spark: [], sparkColor: "#7c3aed" }), _jsx(RichStat, { label: "Paused/Draft", value: String(rows.filter((r) => r.status === "Paused" || r.status === "Draft").length), icon: Icon.alert, tone: "red", spark: [], sparkColor: "#dc2626" })] }), _jsxs("div", { className: "rules-row", children: [_jsxs(Card, { className: "rule-library", title: "Action Library", actions: _jsxs("button", { className: "btn-primary rule-create-btn", onClick: () => setEditing("new"), children: [_jsx("span", { className: "qa-icon-inline", children: Icon.plus }), "Create Action"] }), children: [_jsxs("div", { className: "rule-toolbar", children: [_jsxs("div", { className: "rule-search", children: [_jsx("span", { className: "rule-search-icon", "aria-hidden": true, children: Icon.search }), _jsx("input", { type: "search", placeholder: "Search actions...", value: search, onChange: (e) => setSearch(e.target.value) })] }), _jsx("select", { value: typeFilter, onChange: (e) => setTypeFilter(e.target.value), className: "rule-select", children: typeOptions.map((t) => _jsx("option", { children: t }, t)) }), _jsxs("select", { value: statusFilter, onChange: (e) => setStatusFilter(e.target.value), className: "rule-select", children: [_jsx("option", { children: "All Statuses" }), _jsx("option", { children: "Active" }), _jsx("option", { children: "Reviewed" }), _jsx("option", { children: "Draft" }), _jsx("option", { children: "Paused" })] }), _jsxs("select", { value: sort, onChange: (e) => setSort(e.target.value), className: "rule-select", children: [_jsx("option", { children: "Sort: Last Updated" }), _jsx("option", { children: "Sort: Name" }), _jsx("option", { children: "Sort: Type" })] })] }), _jsxs("table", { className: "table rule-table", children: [_jsx("thead", { children: _jsxs("tr", { children: [_jsx("th", { children: "Action Name" }), _jsx("th", { children: "Type" }), _jsx("th", { children: "Trigger" }), _jsx("th", { children: "Description" }), _jsx("th", { children: "Status" }), _jsx("th", { children: "Last Run" }), _jsx("th", { children: "Actions" })] }) }), _jsxs("tbody", { children: [loading && _jsx("tr", { children: _jsx("td", { colSpan: 7, className: "muted", children: "Loading actions\u2026" }) }), !loading && filtered.length === 0 && _jsx("tr", { children: _jsx("td", { colSpan: 7, className: "muted", children: "No actions match the current filters." }) }), filtered.map((a) => {
                                                const tb = typeBadge(a.type);
                                                return (_jsxs("tr", { className: selectedId === a.id ? "is-selected" : "", onClick: () => setSelectedId(a.id), children: [_jsxs("td", { children: [_jsx("span", { className: `rule-name-chip ${tb.cls}`, "aria-hidden": true, children: _jsx("i", { style: { background: tb.dot } }) }), _jsx("strong", { className: "rule-name-text", children: a.name })] }), _jsx("td", { className: "muted", children: a.type }), _jsx("td", { className: "muted", children: a.trigger }), _jsx("td", { className: "muted rule-desc", children: a.description }), _jsx("td", { children: _jsx("span", { className: `badge ${statusBadge(a.status)}`, children: a.status }) }), _jsx("td", { className: "muted", children: a.lastRun }), _jsx("td", { children: _jsx("button", { className: "btn-ghost icon-btn", "aria-label": "Delete action", onClick: (e) => { e.stopPropagation(); onDelete(a.id); }, children: Icon.more }) })] }, a.id));
                                            })] })] }), _jsx("div", { className: "rule-pagination", children: _jsxs("span", { className: "muted", children: ["Showing ", filtered.length, " of ", rows.length, " actions"] }) })] }), _jsx(Card, { className: "rule-details", title: "Action Details", actions: _jsx("button", { className: "btn-ghost icon-btn", "aria-label": "Expand", children: Icon.expand }), children: selected && selectedAction ? (_jsxs(_Fragment, { children: [_jsxs("div", { className: "rd-header", children: [_jsx("div", { className: `rd-avatar ${typeBadge(selected.type).cls}`, children: Icon.bolt }), _jsx("div", { className: "rd-heading", children: _jsxs("h3", { children: [selected.name, _jsx("span", { className: "badge badge-accent rd-type-badge", children: selected.type })] }) })] }), _jsxs("dl", { className: "rd-grid", children: [_jsx("dt", { children: "\uD83D\uDD17 ID" }), _jsx("dd", { className: "rd-uri", children: _jsxs("span", { children: ["action:", selected.id] }) }), _jsx("dt", { children: "\uD83D\uDCE6 Action Type" }), _jsx("dd", { children: _jsx("span", { className: "tag-chip tag-core", children: selected.type }) }), _jsx("dt", { children: "\u26A1 Trigger" }), _jsx("dd", { children: _jsx("span", { className: "tag-chip", children: selected.trigger }) }), _jsx("dt", { children: "\uD83C\uDFAF Subject" }), _jsx("dd", { children: _jsxs("span", { className: "tag-chip", children: ["Concept #", selectedAction.subject] }) }), _jsx("dt", { children: "\uD83C\uDFAF Object" }), _jsx("dd", { children: selectedAction.object != null ? _jsxs("span", { className: "tag-chip", children: ["Concept #", selectedAction.object] }) : _jsx("span", { className: "muted", children: "\u2014" }) }), _jsx("dt", { children: "\uD83D\uDD53 Last Updated" }), _jsx("dd", { children: selected.lastRun })] }), _jsxs("div", { className: "rd-logic", children: [_jsx("div", { className: "rd-logic-title", children: "Effect" }), _jsx("pre", { className: "rd-code", children: _jsxs("code", { children: [_jsx("span", { className: "ln", children: "1" }), selectedAction.effect || "(no effect declared)"] }) })] }), _jsxs("div", { className: "rd-actions", children: [_jsxs("button", { className: "btn-ghost", onClick: () => selectedAction && setEditing(selectedAction), children: [_jsx("span", { className: "qa-icon-inline", children: Icon.edit }), " Edit Action"] }), _jsxs("button", { className: "btn-ghost", children: [_jsx("span", { className: "qa-icon-inline", children: Icon.play }), " Run Now"] }), _jsxs("button", { className: "btn-ghost", onClick: () => onDelete(selected.id), children: [_jsx("span", { className: "qa-icon-inline", children: Icon.download }), " Delete"] })] }), runStatus.length > 0 && (_jsxs("div", { className: "rd-categories", children: [_jsx("div", { className: "card-title", children: _jsx("span", { children: "Status Breakdown" }) }), _jsxs("div", { className: "rd-cat-row", children: [_jsx(Donut, { data: runStatus }), _jsx("ul", { className: "rd-cat-legend", children: runStatus.map((c) => (_jsxs("li", { children: [_jsx("i", { style: { background: c.color } }), _jsx("span", { className: "rd-cat-name", children: c.name }), _jsx("span", { className: "rd-cat-count", children: c.count }), _jsxs("span", { className: "muted rd-cat-pct", children: [((c.count / runTotal) * 100).toFixed(0), "%"] })] }, c.name))) })] })] }))] })) : (_jsx("p", { className: "muted", children: loading ? "Loading…" : "No action selected." })) })] }), _jsxs("div", { className: "actions-bottom", children: [_jsx(Card, { title: "Recent Action Activity", children: _jsxs("ul", { className: "rule-activity", children: [actions.length === 0 && _jsx("li", { className: "muted", children: "No activity yet." }), actions.slice(0, 5).map((a) => (_jsxs("li", { children: [_jsx("span", { className: "act-icon act-ok", children: Icon.checkCircle }), _jsxs("span", { className: "ra-text", children: ["Action \"", a.name, "\" present"] }), _jsx("span", { className: "muted ra-time", children: actionUpdatedAt(a) })] }, a.id)))] }) }), _jsx(Card, { title: "Top Action Types", children: _jsxs("ul", { className: "bar-list", children: [domains.length === 0 && _jsx("li", { className: "muted", children: "No types." }), domains.map((d) => (_jsxs("li", { className: "bar-row", children: [_jsx("span", { className: "bar-label", children: d.name }), _jsx("div", { className: "bar-track", children: _jsx("div", { className: "bar-fill", style: { width: `${(d.count / (domains[0]?.count ?? 1)) * 100}%` } }) }), _jsxs("span", { className: "bar-value", children: [d.count, " (", d.pct.toFixed(1), "%)"] })] }, d.name)))] }) }), _jsx(Card, { title: "Quick Actions", children: _jsxs("div", { className: "quick-actions rule-quick", children: [_jsxs("a", { className: "quick-action qa-blue", href: "#", onClick: (e) => e.preventDefault(), children: [_jsx("div", { className: "qa-icon", children: Icon.import }), _jsxs("div", { children: [_jsx("div", { className: "qa-title", children: "Import Actions" }), _jsx("div", { className: "qa-sub muted", children: "Import from files or sources" })] })] }), _jsxs("a", { className: "quick-action qa-violet", href: "#", onClick: (e) => e.preventDefault(), children: [_jsx("div", { className: "qa-icon", children: Icon.spark }), _jsxs("div", { children: [_jsx("div", { className: "qa-title", children: "Generate with AI" }), _jsx("div", { className: "qa-sub muted", children: "Auto-generate actions" })] })] }), _jsxs("a", { className: "quick-action qa-amber", href: "#", onClick: (e) => e.preventDefault(), children: [_jsx("div", { className: "qa-icon", children: Icon.bulk }), _jsxs("div", { children: [_jsx("div", { className: "qa-title", children: "Bulk Edit" }), _jsx("div", { className: "qa-sub muted", children: "Edit multiple actions" })] })] }), _jsxs("a", { className: "quick-action qa-green", href: "#", onClick: (e) => e.preventDefault(), children: [_jsx("div", { className: "qa-icon", children: Icon.play }), _jsxs("div", { children: [_jsx("div", { className: "qa-title", children: "Run All Active" }), _jsx("div", { className: "qa-sub muted", children: "Run all active actions" })] })] })] }) })] }), editing != null && (_jsx(ActionModal, { initial: editing === "new" ? null : editing, ontology: ontology, concepts: allConcepts, onCancel: () => setEditing(null), onSave: (payload) => onSaveAction(payload, editing === "new" ? null : editing.id) }))] }));
}
function ActionModal({ initial, ontology, concepts, onCancel, onSave }) {
    const actionTypes = useMemo(() => Object.keys(ontology?.action_types ?? {}).sort(), [ontology]);
    const [actionType, setActionType] = useState(initial?.action_type ?? actionTypes[0] ?? "");
    const [name, setName] = useState(initial?.name ?? "");
    const [subject, setSubject] = useState(initial?.subject ?? "");
    const [object, setObject] = useState(initial?.object ?? "");
    const [effect, setEffect] = useState(initial?.effect ?? "");
    const [description, setDescription] = useState(initial?.description ?? "");
    const isEdit = initial != null;
    useEffect(() => {
        if (!actionType && actionTypes.length > 0)
            setActionType(actionTypes[0]);
    }, [actionTypes, actionType]);
    function submit(e) {
        e.preventDefault();
        if (!name.trim() || !actionType || subject === "")
            return;
        onSave({
            action_type: actionType,
            name: name.trim(),
            subject: Number(subject),
            object: object === "" ? null : Number(object),
            parameters: initial?.parameters ?? {},
            effect,
            description,
        });
    }
    return (_jsx("div", { className: "modal-backdrop", onClick: onCancel, children: _jsxs("form", { className: "modal-card", onClick: (e) => e.stopPropagation(), onSubmit: submit, children: [_jsx("h3", { className: "modal-title", children: isEdit ? "Edit Action" : "Create Action" }), _jsxs("label", { className: "modal-field", children: [_jsx("span", { children: "Action Type" }), actionTypes.length > 0 ? (_jsx("select", { value: actionType, onChange: (e) => setActionType(e.target.value), disabled: isEdit, required: true, children: actionTypes.map((t) => (_jsx("option", { value: t, children: t }, t))) })) : (_jsx("input", { value: actionType, onChange: (e) => setActionType(e.target.value), disabled: isEdit, required: true })), isEdit && _jsx("small", { className: "muted", children: "Type is immutable." })] }), _jsxs("label", { className: "modal-field", children: [_jsx("span", { children: "Name" }), _jsx("input", { value: name, onChange: (e) => setName(e.target.value), required: true })] }), _jsxs("label", { className: "modal-field", children: [_jsx("span", { children: "Subject (concept)" }), _jsxs("select", { value: subject === "" ? "" : String(subject), onChange: (e) => setSubject(e.target.value === "" ? "" : Number(e.target.value)), required: true, children: [_jsx("option", { value: "", disabled: true, children: "Select a subject concept\u2026" }), concepts.map((c) => (_jsxs("option", { value: c.id, children: [c.concept_type, ": ", c.name] }, c.id)))] })] }), _jsxs("label", { className: "modal-field", children: [_jsx("span", { children: "Object (concept, optional)" }), _jsxs("select", { value: object === "" ? "" : String(object), onChange: (e) => setObject(e.target.value === "" ? "" : Number(e.target.value)), children: [_jsx("option", { value: "", children: "(none)" }), concepts.map((c) => (_jsxs("option", { value: c.id, children: [c.concept_type, ": ", c.name] }, c.id)))] })] }), _jsxs("label", { className: "modal-field", children: [_jsx("span", { children: "Effect" }), _jsx("textarea", { value: effect, onChange: (e) => setEffect(e.target.value), rows: 2 })] }), _jsxs("label", { className: "modal-field", children: [_jsx("span", { children: "Description" }), _jsx("input", { value: description, onChange: (e) => setDescription(e.target.value) })] }), _jsxs("div", { className: "modal-actions", children: [_jsx("button", { type: "button", className: "btn-ghost", onClick: onCancel, children: "Cancel" }), _jsx("button", { type: "submit", className: "btn-primary", children: isEdit ? "Save" : "Create" })] }), _jsx("style", { children: `
          .modal-backdrop { position: fixed; inset: 0; background: rgba(15,23,42,.45);
            display: flex; align-items: center; justify-content: center; z-index: 1000; }
          .modal-card { background: #fff; border-radius: 12px; padding: 24px;
            width: min(520px, 92vw); max-height: 90vh; overflow: auto;
            display: flex; flex-direction: column; gap: 12px;
            box-shadow: 0 20px 60px rgba(0,0,0,.25); }
          .modal-title { margin: 0 0 4px; font-size: 18px; font-weight: 600; }
          .modal-field { display: flex; flex-direction: column; gap: 4px; font-size: 13px; }
          .modal-field > span { font-weight: 500; color: #334155; }
          .modal-field input, .modal-field textarea, .modal-field select {
            padding: 8px 10px; border: 1px solid #cbd5e1; border-radius: 8px;
            font: inherit; background: #fff; }
          .modal-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 8px; }
        ` })] }) }));
}
