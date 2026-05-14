import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// OntologyGraph — pill-style SVG visualization matching the Dashboard
// `NetworkPreview` look. Renders a Subgraph (concepts + relations) with
// rounded rectangle nodes colored by concept type, light-grey edges, and
// optional relation labels.
import { useMemo } from "react";
// --- palette (kept in sync with Dashboard.NetworkPreview) -------------------
const PALETTE = [
    { fill: "#dbeafe", stroke: "#2563eb", text: "#1d4ed8" }, // blue
    { fill: "#dcfce7", stroke: "#16a34a", text: "#15803d" }, // green
    { fill: "#fef3c7", stroke: "#d97706", text: "#b45309" }, // amber
    { fill: "#fee2e2", stroke: "#dc2626", text: "#b91c1c" }, // red
    { fill: "#ede9fe", stroke: "#7c3aed", text: "#6d28d9" }, // violet
    { fill: "#cffafe", stroke: "#0891b2", text: "#0e7490" }, // cyan
    { fill: "#fce7f3", stroke: "#db2777", text: "#be185d" }, // pink
    { fill: "#e5e7eb", stroke: "#475569", text: "#334155" }, // slate
];
function ellipsize(s, max) {
    if (s.length <= max)
        return s;
    return s.slice(0, Math.max(0, max - 1)) + "…";
}
function nodeWidth(label) {
    return Math.min(160, Math.max(70, label.length * 7 + 24));
}
function layout(subgraph, w, h, limit) {
    // assign a color slot per concept type
    const typeIndex = new Map();
    for (const c of subgraph.concepts) {
        if (!typeIndex.has(c.concept_type))
            typeIndex.set(c.concept_type, typeIndex.size % PALETTE.length);
    }
    // degree count
    const deg = new Map();
    for (const r of subgraph.relations) {
        deg.set(r.source, (deg.get(r.source) ?? 0) + 1);
        deg.set(r.target, (deg.get(r.target) ?? 0) + 1);
    }
    const concepts = [...subgraph.concepts]
        .sort((a, b) => (deg.get(b.id) ?? 0) - (deg.get(a.id) ?? 0))
        .slice(0, limit);
    const visible = new Set(concepts.map((c) => c.id));
    // ----------------------------------------------------------------------
    // Concentric layout: hub at center, top neighbors in inner ring, the rest
    // distributed across an outer ring. Tuned to look like the mockup.
    // ----------------------------------------------------------------------
    const cx = w / 2;
    const cy = h / 2;
    const positions = new Map();
    if (concepts.length === 0) {
        return { nodes: [], edges: [], width: w, height: h, types: [] };
    }
    // hub
    positions.set(concepts[0].id, { x: cx, y: cy });
    // neighbors of hub for inner ring
    const hubNeighbors = new Set();
    for (const r of subgraph.relations) {
        if (r.source === concepts[0].id)
            hubNeighbors.add(r.target);
        if (r.target === concepts[0].id)
            hubNeighbors.add(r.source);
    }
    const inner = [];
    const outer = [];
    for (const c of concepts.slice(1)) {
        if (hubNeighbors.has(c.id) && inner.length < 6)
            inner.push(c);
        else
            outer.push(c);
    }
    // make sure inner has something even if hub has no neighbors
    while (inner.length < Math.min(6, concepts.length - 1) && outer.length > 0) {
        inner.push(outer.shift());
    }
    const rInner = Math.min(w, h) * 0.28;
    const rOuter = Math.min(w, h) * 0.46;
    inner.forEach((c, i) => {
        const a = (i / inner.length) * Math.PI * 2 - Math.PI / 2;
        positions.set(c.id, { x: cx + Math.cos(a) * rInner, y: cy + Math.sin(a) * rInner * 0.72 });
    });
    outer.forEach((c, i) => {
        const a = (i / Math.max(1, outer.length)) * Math.PI * 2 - Math.PI / 2 + 0.2;
        positions.set(c.id, { x: cx + Math.cos(a) * rOuter, y: cy + Math.sin(a) * rOuter * 0.78 });
    });
    const nodes = concepts.map((c) => {
        const p = positions.get(c.id);
        const label = ellipsize(c.name, 22);
        const wd = nodeWidth(label);
        return {
            id: c.id,
            label,
            type: c.concept_type,
            x: p.x,
            y: p.y,
            w: wd,
            h: 32,
            color: typeIndex.get(c.concept_type) ?? 0,
        };
    });
    const edges = subgraph.relations
        .filter((r) => visible.has(r.source) && visible.has(r.target))
        .map((r) => ({ from: r.source, to: r.target, label: r.relation_type }));
    const types = [...typeIndex.entries()].map(([name, color]) => ({ name, color }));
    return { nodes, edges, width: w, height: h, types };
}
export default function OntologyGraph({ subgraph, height = 460, limit = 24, showLegend = true, className, }) {
    // viewBox is fixed; SVG scales to container width.
    const VBW = 960;
    const VBH = Math.max(280, Math.round((height / 460) * 520));
    const lay = useMemo(() => {
        if (!subgraph || subgraph.concepts.length === 0) {
            return { nodes: [], edges: [], width: VBW, height: VBH, types: [] };
        }
        return layout(subgraph, VBW, VBH, limit);
    }, [subgraph, VBW, VBH, limit]);
    const byId = useMemo(() => {
        const m = new Map();
        for (const n of lay.nodes)
            m.set(n.id, n);
        return m;
    }, [lay.nodes]);
    return (_jsxs("div", { className: `og-wrap${className ? " " + className : ""}`, style: { minHeight: height }, children: [lay.nodes.length === 0 ? (_jsx("div", { className: "og-empty", children: "Generate or upload sources to populate the ontology graph." })) : (_jsxs("svg", { viewBox: `0 0 ${VBW} ${VBH}`, preserveAspectRatio: "xMidYMid meet", className: "og-svg", children: [_jsx("g", { children: lay.edges.map((e, i) => {
                            const a = byId.get(e.from);
                            const b = byId.get(e.to);
                            if (!a || !b)
                                return null;
                            const mx = (a.x + b.x) / 2;
                            const my = (a.y + b.y) / 2;
                            return (_jsxs("g", { children: [_jsx("line", { x1: a.x, y1: a.y, x2: b.x, y2: b.y, stroke: "#cbd5e1", strokeWidth: 1.2 }), e.label && (_jsxs("g", { transform: `translate(${mx}, ${my})`, children: [_jsx("rect", { x: -e.label.length * 3 - 6, y: -9, width: e.label.length * 6 + 12, height: 18, rx: 9, fill: "#ede9fe", stroke: "#c4b5fd" }), _jsx("text", { x: 0, y: 4, fontSize: "10", fontWeight: "500", fill: "#6d28d9", textAnchor: "middle", children: ellipsize(e.label, 18) })] }))] }, `e-${i}`));
                        }) }), _jsx("g", { children: lay.nodes.map((n) => {
                            const p = PALETTE[n.color % PALETTE.length];
                            return (_jsxs("g", { children: [_jsx("rect", { x: n.x - n.w / 2, y: n.y - n.h / 2, width: n.w, height: n.h, rx: n.h / 2, fill: p.fill, stroke: p.stroke, strokeWidth: 1.4 }), _jsx("text", { x: n.x, y: n.y + 4, fontSize: "12", fontWeight: "600", fill: p.text, textAnchor: "middle", children: n.label })] }, `n-${n.id}`));
                        }) })] })), showLegend && lay.types.length > 0 && (_jsx("ul", { className: "og-legend", children: lay.types.slice(0, 8).map((t) => {
                    const p = PALETTE[t.color % PALETTE.length];
                    return (_jsxs("li", { children: [_jsx("span", { className: "og-dot", style: { background: p.stroke } }), " ", t.name] }, t.name));
                }) })), _jsx("style", { children: `
        .og-wrap {
          position: relative;
          width: 100%;
          background: var(--panel);
          border-radius: var(--radius-sm);
          padding: 12px;
        }
        .og-svg { width: 100%; height: 100%; min-height: ${height - 24}px; display: block; }
        .og-empty {
          display: grid; place-items: center;
          min-height: ${height - 24}px;
          color: var(--muted); font-size: 13px;
        }
        .og-legend {
          position: absolute;
          top: 12px; right: 12px;
          list-style: none; margin: 0; padding: 8px 12px;
          background: var(--panel); border: 1px solid var(--border);
          border-radius: 8px;
          display: flex; flex-direction: column; gap: 6px;
          font-size: 11px; color: var(--text);
        }
        .og-dot { display: inline-block; width: 8px; height: 8px; border-radius: 50%; margin-right: 6px; vertical-align: middle; }
      ` })] }));
}
