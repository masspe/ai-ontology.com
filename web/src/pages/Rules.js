import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
import { useEffect, useMemo, useState } from "react";
import Card from "../components/Card";
import Sparkline from "../components/Sparkline";
import { createRule, deleteRule, getOntology, getStats, listConcepts, listRules, updateRule, } from "../api";
function ruleStatus(r) {
    const raw = r.properties?.status?.toLowerCase();
    if (raw === "reviewed")
        return "Reviewed";
    if (raw === "draft")
        return "Draft";
    if (raw === "disabled")
        return "Disabled";
    return r.strict ? "Active" : "Draft";
}
function ruleUpdatedAt(r) {
    const v = r.properties?.updated_at ?? r.properties?.created_at;
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
function ruleToRow(r) {
    return {
        id: r.id,
        name: r.name,
        type: r.rule_type,
        scope: (r.applies_to?.length ? `${r.applies_to.length} concept(s)` : "—"),
        description: r.description ?? r.when ?? "",
        status: ruleStatus(r),
        updated: ruleUpdatedAt(r),
    };
}
const PALETTE = ["#2563eb", "#7c3aed", "#d97706", "#16a34a", "#dc2626", "#0ea5e9", "#db2777"];
function typeColor(name) {
    let h = 0;
    for (let i = 0; i < name.length; i++)
        h = (h * 31 + name.charCodeAt(i)) >>> 0;
    return PALETTE[h % PALETTE.length];
}
// ---------------------------------------------------------------------------
// Inline icons (kept local — no extra deps)
// ---------------------------------------------------------------------------
const Icon = {
    total: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" }), _jsx("path", { d: "M14 2v6h6" }), _jsx("path", { d: "M9 13h6" }), _jsx("path", { d: "M9 17h4" })] })),
    shield: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" }), _jsx("path", { d: "m9 12 2 2 4-4" })] })),
    check: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("circle", { cx: "12", cy: "12", r: "10" }), _jsx("path", { d: "m9 12 2 2 4-4" })] })),
    bolt: (_jsx("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: _jsx("path", { d: "M13 2 3 14h7l-1 8 10-12h-7z" }) })),
    search: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("circle", { cx: "11", cy: "11", r: "7" }), _jsx("path", { d: "m21 21-4.3-4.3" })] })),
    plus: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M12 5v14" }), _jsx("path", { d: "M5 12h14" })] })),
    copy: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("rect", { x: "9", y: "9", width: "13", height: "13", rx: "2" }), _jsx("path", { d: "M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" })] })),
    edit: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M12 20h9" }), _jsx("path", { d: "M16.5 3.5a2.121 2.121 0 1 1 3 3L7 19l-4 1 1-4z" })] })),
    flask: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M9 2h6" }), _jsx("path", { d: "M10 2v6L4 20a2 2 0 0 0 2 3h12a2 2 0 0 0 2-3l-6-12V2" })] })),
    download: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" }), _jsx("path", { d: "m7 10 5 5 5-5" }), _jsx("path", { d: "M12 15V3" })] })),
    expand: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M3 9V3h6" }), _jsx("path", { d: "M21 15v6h-6" }), _jsx("path", { d: "M3 3l7 7" }), _jsx("path", { d: "m14 14 7 7" })] })),
    more: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "currentColor", children: [_jsx("circle", { cx: "5", cy: "12", r: "1.6" }), _jsx("circle", { cx: "12", cy: "12", r: "1.6" }), _jsx("circle", { cx: "19", cy: "12", r: "1.6" })] })),
    import: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("path", { d: "M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" }), _jsx("path", { d: "m17 8-5-5-5 5" }), _jsx("path", { d: "M12 3v12" })] })),
    spark: (_jsx("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: _jsx("path", { d: "m12 3 1.9 5.8H20l-4.9 3.6L17 18.2 12 14.6 7 18.2l1.9-5.8L4 8.8h6.1z" }) })),
    bulk: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("rect", { x: "3", y: "4", width: "18", height: "4", rx: "1" }), _jsx("rect", { x: "3", y: "10", width: "18", height: "4", rx: "1" }), _jsx("rect", { x: "3", y: "16", width: "18", height: "4", rx: "1" })] })),
    play: (_jsxs("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", children: [_jsx("circle", { cx: "12", cy: "12", r: "10" }), _jsx("path", { d: "m10 8 6 4-6 4z" })] })),
};
// ---------------------------------------------------------------------------
// Type → visual mapping
// ---------------------------------------------------------------------------
function typeBadge(type) {
    // Built-in categories get distinct CSS classes; fall back to a deterministic
    // color from the palette so user-defined rule types stay visually stable.
    const known = {
        Validation: { cls: "rule-type-validation", dot: "#2563eb" },
        Inference: { cls: "rule-type-inference", dot: "#7c3aed" },
        Constraint: { cls: "rule-type-constraint", dot: "#d97706" },
        Transformation: { cls: "rule-type-transformation", dot: "#16a34a" },
    };
    return known[type] ?? { cls: "rule-type-validation", dot: typeColor(type) };
}
function statusBadge(s) {
    switch (s) {
        case "Active": return "badge-success";
        case "Reviewed": return "badge-accent";
        case "Draft": return "badge-warn";
        case "Disabled": return "badge-danger";
    }
}
function RichStat({ label, value, deltaPct, icon, tone, spark, sparkColor }) {
    const showDelta = deltaPct != null;
    const cls = showDelta && deltaPct > 0 ? "up" : showDelta && deltaPct < 0 ? "down" : "flat";
    const arrow = cls === "up" ? "↑" : cls === "down" ? "↓" : "•";
    return (_jsxs("div", { className: "card stat-rich", children: [_jsx("div", { className: `stat-icon tone-${tone}`, children: icon }), _jsxs("div", { className: "stat-body", children: [_jsx("div", { className: "stat-label", children: label }), _jsx("div", { className: "stat-value", children: value }), showDelta && (_jsx("div", { className: `stat-delta ${cls}`, children: _jsxs("span", { children: [arrow, " ", Math.abs(deltaPct).toFixed(0), "%"] }) }))] }), _jsx("div", { className: "stat-spark", children: _jsx(Sparkline, { values: spark, stroke: sparkColor }) })] }));
}
// ---------------------------------------------------------------------------
// Donut chart for Rule Categories
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
export default function Rules() {
    const [rules, setRules] = useState([]);
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
                /* non-fatal: form picks degrade to free text */
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
                const [rs, st] = await Promise.all([listRules(), getStats()]);
                if (cancelled)
                    return;
                setRules(rs);
                setStats(st);
                if (rs.length > 0)
                    setSelectedId(rs[0].id);
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
        return () => {
            cancelled = true;
        };
    }, []);
    const rows = useMemo(() => rules.map(ruleToRow), [rules]);
    const typeOptions = useMemo(() => {
        const set = new Set(rows.map((r) => r.type));
        return ["All Types", ...Array.from(set).sort()];
    }, [rows]);
    const filtered = useMemo(() => {
        const q = search.trim().toLowerCase();
        let out = rows.filter((r) => {
            if (q && !(r.name.toLowerCase().includes(q) || r.description.toLowerCase().includes(q)))
                return false;
            if (typeFilter !== "All Types" && r.type !== typeFilter)
                return false;
            if (statusFilter !== "All Statuses" && r.status !== statusFilter)
                return false;
            return true;
        });
        if (sort === "Sort: Name" || sort === "Name") {
            out = [...out].sort((a, b) => a.name.localeCompare(b.name));
        }
        else if (sort === "Sort: Type" || sort === "Type") {
            out = [...out].sort((a, b) => a.type.localeCompare(b.type));
        }
        return out;
    }, [rows, search, typeFilter, statusFilter, sort]);
    const categories = useMemo(() => {
        const counts = new Map();
        for (const r of rows)
            counts.set(r.type, (counts.get(r.type) ?? 0) + 1);
        const total = rows.length || 1;
        return Array.from(counts.entries()).map(([name, count]) => ({
            name,
            count,
            pct: (count / total) * 100,
            color: typeBadge(name).dot,
        }));
    }, [rows]);
    const domains = useMemo(() => {
        // Group by first concept-id in `applies_to` (or "Unscoped").
        const counts = new Map();
        for (const r of rules) {
            const key = r.applies_to?.[0] != null ? `Concept #${r.applies_to[0]}` : "Unscoped";
            counts.set(key, (counts.get(key) ?? 0) + 1);
        }
        const total = rules.length || 1;
        return Array.from(counts.entries())
            .sort((a, b) => b[1] - a[1])
            .slice(0, 5)
            .map(([name, count]) => ({ name, count, pct: (count / total) * 100 }));
    }, [rules]);
    const selected = useMemo(() => rows.find((r) => r.id === selectedId) ?? rows[0] ?? null, [rows, selectedId]);
    const selectedRule = useMemo(() => rules.find((r) => r.id === selectedId) ?? rules[0] ?? null, [rules, selectedId]);
    async function onDelete(id) {
        if (!confirm("Delete this rule?"))
            return;
        try {
            await deleteRule(id);
            setRules((rs) => rs.filter((r) => r.id !== id));
        }
        catch (e) {
            alert("Delete failed: " + e.message);
        }
    }
    async function onSaveRule(payload, editingId) {
        try {
            if (editingId == null) {
                const { id } = await createRule(payload);
                setRules((rs) => [...rs, { id, ...payload }]);
                setSelectedId(id);
            }
            else {
                const updated = await updateRule(editingId, {
                    name: payload.name,
                    when: payload.when,
                    then: payload.then,
                    applies_to: payload.applies_to,
                    strict: payload.strict,
                    description: payload.description,
                    properties: payload.properties,
                });
                setRules((rs) => rs.map((r) => (r.id === editingId ? updated : r)));
            }
            setEditing(null);
        }
        catch (e) {
            alert("Save failed: " + e.message);
        }
    }
    const totalRules = stats?.rules ?? rules.length;
    const activeRules = rows.filter((r) => r.status === "Active").length;
    const totalRuleTypes = stats?.rule_types ?? new Set(rows.map((r) => r.type)).size;
    return (_jsxs(_Fragment, { children: [_jsx("div", { className: "page-header", children: _jsxs("div", { children: [_jsx("h1", { className: "page-title", children: "Rules" }), _jsx("p", { className: "page-subtitle", children: "Create, validate, and manage ontology rules, constraints, and inference logic." })] }) }), error && _jsxs("div", { className: "banner banner-error", children: ["Failed to load rules: ", error] }), _jsxs("div", { className: "dash-row dash-row-stats", children: [_jsx(RichStat, { label: "Total Rules", value: String(totalRules), icon: Icon.total, tone: "blue", spark: [], sparkColor: "#2563eb" }), _jsx(RichStat, { label: "Active Rules", value: String(activeRules), icon: Icon.shield, tone: "green", spark: [], sparkColor: "#16a34a" }), _jsx(RichStat, { label: "Rule Types", value: String(totalRuleTypes), icon: Icon.check, tone: "violet", spark: [], sparkColor: "#7c3aed" }), _jsx(RichStat, { label: "Applied Concepts", value: String(rules.reduce((s, r) => s + (r.applies_to?.length ?? 0), 0)), icon: Icon.bolt, tone: "amber", spark: [], sparkColor: "#d97706" })] }), _jsxs("div", { className: "rules-row", children: [_jsxs(Card, { className: "rule-library", title: "Rule Library", actions: _jsxs("button", { className: "btn-primary rule-create-btn", onClick: () => setEditing("new"), children: [_jsx("span", { className: "qa-icon-inline", children: Icon.plus }), "Create Rule"] }), children: [_jsxs("div", { className: "rule-toolbar", children: [_jsxs("div", { className: "rule-search", children: [_jsx("span", { className: "rule-search-icon", "aria-hidden": true, children: Icon.search }), _jsx("input", { type: "search", placeholder: "Search rules...", value: search, onChange: (e) => setSearch(e.target.value) })] }), _jsx("select", { value: typeFilter, onChange: (e) => setTypeFilter(e.target.value), className: "rule-select", children: typeOptions.map((t) => _jsx("option", { children: t }, t)) }), _jsxs("select", { value: statusFilter, onChange: (e) => setStatusFilter(e.target.value), className: "rule-select", children: [_jsx("option", { children: "All Statuses" }), _jsx("option", { children: "Active" }), _jsx("option", { children: "Reviewed" }), _jsx("option", { children: "Draft" }), _jsx("option", { children: "Disabled" })] }), _jsxs("select", { value: sort, onChange: (e) => setSort(e.target.value), className: "rule-select", children: [_jsx("option", { children: "Sort: Last Updated" }), _jsx("option", { children: "Sort: Name" }), _jsx("option", { children: "Sort: Type" })] })] }), _jsxs("table", { className: "table rule-table", children: [_jsx("thead", { children: _jsxs("tr", { children: [_jsx("th", { children: "Rule Name" }), _jsx("th", { children: "Type" }), _jsx("th", { children: "Scope" }), _jsx("th", { children: "Description" }), _jsx("th", { children: "Status" }), _jsx("th", { children: "Last Updated" }), _jsx("th", { children: "Actions" })] }) }), _jsxs("tbody", { children: [loading && (_jsx("tr", { children: _jsx("td", { colSpan: 7, className: "muted", children: "Loading rules\u2026" }) })), !loading && filtered.length === 0 && (_jsx("tr", { children: _jsx("td", { colSpan: 7, className: "muted", children: "No rules match the current filters." }) })), filtered.map((r) => {
                                                const tb = typeBadge(r.type);
                                                return (_jsxs("tr", { className: selectedId === r.id ? "is-selected" : "", onClick: () => setSelectedId(r.id), children: [_jsxs("td", { children: [_jsx("span", { className: `rule-name-chip ${tb.cls}`, "aria-hidden": true, children: _jsx("i", { style: { background: tb.dot } }) }), _jsx("strong", { className: "rule-name-text", children: r.name })] }), _jsx("td", { className: "muted", children: r.type }), _jsx("td", { className: "muted", children: r.scope }), _jsx("td", { className: "muted rule-desc", children: r.description }), _jsx("td", { children: _jsx("span", { className: `badge ${statusBadge(r.status)}`, children: r.status }) }), _jsx("td", { className: "muted", children: r.updated }), _jsx("td", { children: _jsx("button", { className: "btn-ghost icon-btn", "aria-label": "Delete rule", onClick: (e) => { e.stopPropagation(); onDelete(r.id); }, children: Icon.more }) })] }, r.id));
                                            })] })] }), _jsx("div", { className: "rule-pagination", children: _jsxs("span", { className: "muted", children: ["Showing ", filtered.length, " of ", rows.length, " rules"] }) })] }), _jsx(Card, { className: "rule-details", title: "Rule Details", actions: _jsx("button", { className: "btn-ghost icon-btn", "aria-label": "Expand", children: Icon.expand }), children: selected && selectedRule ? (_jsxs(_Fragment, { children: [_jsxs("div", { className: "rd-header", children: [_jsx("div", { className: `rd-avatar ${typeBadge(selected.type).cls}`, children: Icon.shield }), _jsxs("div", { className: "rd-heading", children: [_jsxs("h3", { children: [selected.name, _jsx("span", { className: "badge badge-accent rd-type-badge", children: selected.type })] }), _jsx("p", { className: "muted", children: selected.description || "—" })] })] }), _jsxs("dl", { className: "rd-grid", children: [_jsx("dt", { children: "\uD83D\uDD17 ID" }), _jsx("dd", { className: "rd-uri", children: _jsxs("span", { children: ["rule:", selected.id] }) }), _jsx("dt", { children: "\uD83D\uDCE6 Rule Type" }), _jsx("dd", { children: _jsx("span", { className: "tag-chip tag-core", children: selected.type }) }), _jsx("dt", { children: "\u26A0 Strict" }), _jsx("dd", { children: _jsx("span", { className: "tag-chip tag-high", children: selectedRule.strict ? "Yes" : "No" }) }), _jsx("dt", { children: "\uD83D\uDD53 Last Updated" }), _jsx("dd", { children: selected.updated }), _jsx("dt", { children: "\uD83D\uDD16 Applies To" }), _jsx("dd", { children: selectedRule.applies_to?.length
                                                ? selectedRule.applies_to.map((cid) => (_jsxs("span", { className: "tag-chip", children: ["Concept #", cid] }, cid)))
                                                : _jsx("span", { className: "muted", children: "\u2014" }) })] }), _jsxs("div", { className: "rd-logic", children: [_jsx("div", { className: "rd-logic-title", children: "Rule Logic" }), _jsx("pre", { className: "rd-code", children: _jsxs("code", { children: [_jsx("span", { className: "ln", children: "1" }), _jsx("span", { className: "kw", children: "WHEN" }), " ", selectedRule.when || "(no condition)", "\n", _jsx("span", { className: "ln", children: "2" }), _jsx("span", { className: "kw", children: "THEN" }), " ", selectedRule.then || "(no action)"] }) })] }), _jsxs("div", { className: "rd-actions", children: [_jsxs("button", { className: "btn-ghost", onClick: () => selectedRule && setEditing(selectedRule), children: [_jsx("span", { className: "qa-icon-inline", children: Icon.edit }), " Edit Rule"] }), _jsxs("button", { className: "btn-ghost", onClick: () => onDelete(selected.id), children: [_jsx("span", { className: "qa-icon-inline", children: Icon.flask }), " Delete"] })] }), categories.length > 0 && (_jsxs("div", { className: "rd-categories", children: [_jsx("div", { className: "card-title", children: _jsx("span", { children: "Rule Categories" }) }), _jsxs("div", { className: "rd-cat-row", children: [_jsx(Donut, { data: categories }), _jsx("ul", { className: "rd-cat-legend", children: categories.map((c) => (_jsxs("li", { children: [_jsx("i", { style: { background: c.color } }), _jsx("span", { className: "rd-cat-name", children: c.name }), _jsx("span", { className: "rd-cat-count", children: c.count }), _jsxs("span", { className: "muted rd-cat-pct", children: [c.pct.toFixed(1), "%"] })] }, c.name))) })] })] }))] })) : (_jsx("p", { className: "muted", children: loading ? "Loading…" : "No rule selected." })) })] }), _jsxs("div", { className: "rules-bottom", children: [_jsx(Card, { title: "Recent Rule Activity", children: _jsxs("ul", { className: "rule-activity", children: [rules.length === 0 && _jsx("li", { className: "muted", children: "No activity yet." }), rules.slice(0, 5).map((r) => (_jsxs("li", { children: [_jsx("span", { className: "act-icon act-ok", children: Icon.check }), _jsxs("span", { className: "ra-text", children: ["Rule \"", r.name, "\" present"] }), _jsx("span", { className: "muted ra-time", children: ruleUpdatedAt(r) })] }, r.id)))] }) }), _jsx(Card, { title: "Top Scopes", children: _jsxs("ul", { className: "bar-list", children: [domains.length === 0 && _jsx("li", { className: "muted", children: "No scopes." }), domains.map((d) => (_jsxs("li", { className: "bar-row", children: [_jsx("span", { className: "bar-label", children: d.name }), _jsx("div", { className: "bar-track", children: _jsx("div", { className: "bar-fill", style: { width: `${(d.count / (domains[0]?.count ?? 1)) * 100}%` } }) }), _jsxs("span", { className: "bar-value", children: [d.count, " (", d.pct.toFixed(1), "%)"] })] }, d.name)))] }) }), _jsx(Card, { title: "Quick Actions", children: _jsxs("div", { className: "quick-actions rule-quick", children: [_jsxs("a", { className: "quick-action qa-blue", href: "#", onClick: (e) => e.preventDefault(), children: [_jsx("div", { className: "qa-icon", children: Icon.import }), _jsxs("div", { children: [_jsx("div", { className: "qa-title", children: "Import Rules" }), _jsx("div", { className: "qa-sub muted", children: "Import rules from files or sources" })] })] }), _jsxs("a", { className: "quick-action qa-violet", href: "#", onClick: (e) => e.preventDefault(), children: [_jsx("div", { className: "qa-icon", children: Icon.spark }), _jsxs("div", { children: [_jsx("div", { className: "qa-title", children: "Generate with AI" }), _jsx("div", { className: "qa-sub muted", children: "Auto-generate rules from data" })] })] }), _jsxs("a", { className: "quick-action qa-amber", href: "#", onClick: (e) => e.preventDefault(), children: [_jsx("div", { className: "qa-icon", children: Icon.bulk }), _jsxs("div", { children: [_jsx("div", { className: "qa-title", children: "Bulk Edit" }), _jsx("div", { className: "qa-sub muted", children: "Edit multiple rules" })] })] }), _jsxs("a", { className: "quick-action qa-green", href: "#", onClick: (e) => e.preventDefault(), children: [_jsx("div", { className: "qa-icon", children: Icon.play }), _jsxs("div", { children: [_jsx("div", { className: "qa-title", children: "Run Validation" }), _jsx("div", { className: "qa-sub muted", children: "Validate all rules and constraints" })] })] })] }) })] }), editing != null && (_jsx(RuleModal, { initial: editing === "new" ? null : editing, ontology: ontology, concepts: allConcepts, onCancel: () => setEditing(null), onSave: (payload) => onSaveRule(payload, editing === "new" ? null : editing.id) }))] }));
}
function RuleModal({ initial, ontology, concepts, onCancel, onSave }) {
    const ruleTypes = useMemo(() => Object.keys(ontology?.rule_types ?? {}).sort(), [ontology]);
    const [ruleType, setRuleType] = useState(initial?.rule_type ?? ruleTypes[0] ?? "");
    const [name, setName] = useState(initial?.name ?? "");
    const [when, setWhen] = useState(initial?.when ?? "");
    const [then, setThen] = useState(initial?.then ?? "");
    const [strict, setStrict] = useState(initial?.strict ?? false);
    const [description, setDescription] = useState(initial?.description ?? "");
    const [appliesTo, setAppliesTo] = useState(initial?.applies_to ?? []);
    const isEdit = initial != null;
    // Default rule_type once ontology loads.
    useEffect(() => {
        if (!ruleType && ruleTypes.length > 0)
            setRuleType(ruleTypes[0]);
    }, [ruleTypes, ruleType]);
    function submit(e) {
        e.preventDefault();
        if (!name.trim() || !ruleType)
            return;
        onSave({
            rule_type: ruleType,
            name: name.trim(),
            when,
            then,
            applies_to: appliesTo,
            strict,
            description,
            properties: initial?.properties ?? {},
        });
    }
    return (_jsx("div", { className: "modal-backdrop", onClick: onCancel, children: _jsxs("form", { className: "modal-card", onClick: (e) => e.stopPropagation(), onSubmit: submit, children: [_jsx("h3", { className: "modal-title", children: isEdit ? "Edit Rule" : "Create Rule" }), _jsxs("label", { className: "modal-field", children: [_jsx("span", { children: "Rule Type" }), ruleTypes.length > 0 ? (_jsx("select", { value: ruleType, onChange: (e) => setRuleType(e.target.value), disabled: isEdit, required: true, children: ruleTypes.map((t) => (_jsx("option", { value: t, children: t }, t))) })) : (_jsx("input", { value: ruleType, onChange: (e) => setRuleType(e.target.value), disabled: isEdit, required: true })), isEdit && _jsx("small", { className: "muted", children: "Type is immutable." })] }), _jsxs("label", { className: "modal-field", children: [_jsx("span", { children: "Name" }), _jsx("input", { value: name, onChange: (e) => setName(e.target.value), required: true })] }), _jsxs("label", { className: "modal-field", children: [_jsx("span", { children: "Description" }), _jsx("input", { value: description, onChange: (e) => setDescription(e.target.value) })] }), _jsxs("label", { className: "modal-field", children: [_jsx("span", { children: "When" }), _jsx("textarea", { value: when, onChange: (e) => setWhen(e.target.value), rows: 2 })] }), _jsxs("label", { className: "modal-field", children: [_jsx("span", { children: "Then" }), _jsx("textarea", { value: then, onChange: (e) => setThen(e.target.value), rows: 2 })] }), _jsxs("label", { className: "modal-field", children: [_jsx("span", { children: "Applies To (concepts)" }), _jsx("select", { multiple: true, value: appliesTo.map(String), onChange: (e) => setAppliesTo(Array.from(e.target.selectedOptions).map((o) => Number(o.value))), size: Math.min(6, Math.max(3, concepts.length)), children: concepts.map((c) => (_jsxs("option", { value: c.id, children: [c.concept_type, ": ", c.name] }, c.id))) }), _jsx("small", { className: "muted", children: "Hold Ctrl/Cmd to multi-select. Empty = global." })] }), _jsxs("label", { className: "modal-check", children: [_jsx("input", { type: "checkbox", checked: strict, onChange: (e) => setStrict(e.target.checked) }), _jsx("span", { children: "Strict (treated as active)" })] }), _jsxs("div", { className: "modal-actions", children: [_jsx("button", { type: "button", className: "btn-ghost", onClick: onCancel, children: "Cancel" }), _jsx("button", { type: "submit", className: "btn-primary", children: isEdit ? "Save" : "Create" })] }), _jsx("style", { children: `
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
          .modal-field select[multiple] { padding: 4px; }
          .modal-check { display: flex; align-items: center; gap: 8px; font-size: 13px; }
          .modal-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 8px; }
        ` })] }) }));
}
