import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
import { useEffect, useMemo } from "react";
import ReactFlow, { Background, Controls, useEdgesState, useNodesState, } from "reactflow";
import dagre from "dagre";
import "reactflow/dist/style.css";
const NODE_W = 180;
const NODE_H = 44;
function layout(nodes, edges) {
    const g = new dagre.graphlib.Graph();
    g.setDefaultEdgeLabel(() => ({}));
    g.setGraph({ rankdir: "LR", nodesep: 40, ranksep: 80 });
    nodes.forEach((n) => g.setNode(n.id, { width: NODE_W, height: NODE_H }));
    edges.forEach((e) => g.setEdge(e.source, e.target));
    dagre.layout(g);
    const laidOut = nodes.map((n) => {
        const p = g.node(n.id);
        return { ...n, position: { x: p.x - NODE_W / 2, y: p.y - NODE_H / 2 } };
    });
    return { nodes: laidOut, edges };
}
export default function GraphCanvas({ subgraph }) {
    const { initialNodes, initialEdges } = useMemo(() => {
        if (!subgraph)
            return { initialNodes: [], initialEdges: [] };
        const colorOf = (() => {
            const palette = ["#2563eb", "#7c3aed", "#16a34a", "#dc2626", "#d97706", "#0891b2"];
            const map = {};
            let i = 0;
            return (t) => (map[t] ??= palette[i++ % palette.length]);
        })();
        const ns = subgraph.concepts.map((c) => ({
            id: String(c.id),
            position: { x: 0, y: 0 },
            data: { label: c.name },
            style: {
                background: "#fff",
                border: `2px solid ${colorOf(c.concept_type)}`,
                borderRadius: 10,
                padding: "8px 12px",
                fontSize: 12,
                width: NODE_W,
            },
        }));
        const es = subgraph.relations.map((r) => ({
            id: `r${r.id}`,
            source: String(r.source),
            target: String(r.target),
            label: r.relation_type,
            labelStyle: { fontSize: 10, fill: "#64748b" },
            style: { stroke: "#94a3b8", strokeWidth: 1.2 },
        }));
        const out = layout(ns, es);
        return { initialNodes: out.nodes, initialEdges: out.edges };
    }, [subgraph]);
    const [nodes, setNodes, onNodesChange] = useNodesState(initialNodes);
    const [edges, setEdges, onEdgesChange] = useEdgesState(initialEdges);
    useEffect(() => {
        setNodes(initialNodes);
        setEdges(initialEdges);
    }, [initialNodes, initialEdges, setNodes, setEdges]);
    if (!subgraph || subgraph.concepts.length === 0) {
        return _jsx("div", { className: "empty", children: "No concepts to display. Upload data or generate an ontology to populate the graph." });
    }
    return (_jsxs(ReactFlow, { nodes: nodes, edges: edges, onNodesChange: onNodesChange, onEdgesChange: onEdgesChange, fitView: true, minZoom: 0.1, maxZoom: 2, children: [_jsx(Background, { gap: 20, color: "#e5e7eb" }), _jsx(Controls, { position: "bottom-right" })] }));
}
