import { jsx as _jsx } from "react/jsx-runtime";
// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
import { forwardRef, useEffect, useImperativeHandle, useMemo, useRef } from "react";
import ReactFlow, { Background, ReactFlowProvider, useEdgesState, useNodesState, useReactFlow, } from "reactflow";
import dagre from "dagre";
import "reactflow/dist/style.css";
const NODE_W = 160;
const NODE_H = 40;
const DEFAULT_PALETTE = ["#2563eb", "#7c3aed", "#16a34a", "#dc2626", "#d97706", "#0891b2", "#db2777", "#0d9488"];
function layoutGraph(nodes, edges, dir) {
    const g = new dagre.graphlib.Graph();
    g.setDefaultEdgeLabel(() => ({}));
    g.setGraph({ rankdir: dir, nodesep: 50, ranksep: 90 });
    nodes.forEach((n) => g.setNode(n.id, { width: NODE_W, height: NODE_H }));
    edges.forEach((e) => g.setEdge(e.source, e.target));
    dagre.layout(g);
    return nodes.map((n) => {
        const p = g.node(n.id);
        return { ...n, position: { x: p.x - NODE_W / 2, y: p.y - NODE_H / 2 } };
    });
}
function softFill(hex) {
    // Lighten by mixing with white at ~85%
    const m = /^#?([a-f\d]{6})$/i.exec(hex);
    if (!m)
        return "#dbeafe";
    const v = parseInt(m[1], 16);
    const r = (v >> 16) & 255;
    const g = (v >> 8) & 255;
    const b = v & 255;
    const mix = (c) => Math.round(c + (255 - c) * 0.82);
    return `rgb(${mix(r)}, ${mix(g)}, ${mix(b)})`;
}
function CanvasInner({ subgraph, layoutDir = "LR", showLabels = true, highlightPaths = false, selectedNodeId = null, conceptTypeColors, onNodeClick, apiRef, }) {
    const rf = useReactFlow();
    const { initialNodes, initialEdges } = useMemo(() => {
        if (!subgraph)
            return { initialNodes: [], initialEdges: [] };
        const fallback = {};
        let idx = 0;
        const colorOf = (t) => {
            if (conceptTypeColors && conceptTypeColors[t])
                return conceptTypeColors[t];
            return (fallback[t] ??= DEFAULT_PALETTE[idx++ % DEFAULT_PALETTE.length]);
        };
        const neighbors = new Set();
        if (selectedNodeId) {
            for (const r of subgraph.relations) {
                if (String(r.source) === selectedNodeId)
                    neighbors.add(String(r.target));
                if (String(r.target) === selectedNodeId)
                    neighbors.add(String(r.source));
            }
            neighbors.add(selectedNodeId);
        }
        const ns = subgraph.concepts.map((c) => {
            const id = String(c.id);
            const stroke = colorOf(c.concept_type);
            const fill = softFill(stroke);
            const isSelected = selectedNodeId === id;
            const dim = highlightPaths && selectedNodeId != null && !neighbors.has(id);
            return {
                id,
                position: { x: 0, y: 0 },
                data: { label: showLabels ? c.name : "" },
                style: {
                    background: fill,
                    border: `${isSelected ? 2.5 : 1.5}px solid ${stroke}`,
                    borderRadius: 999,
                    padding: "6px 14px",
                    fontSize: 12,
                    fontWeight: 600,
                    color: stroke,
                    width: NODE_W,
                    height: NODE_H,
                    textAlign: "center",
                    boxShadow: isSelected ? `0 0 0 4px ${fill}` : "none",
                    opacity: dim ? 0.25 : 1,
                    transition: "opacity 0.15s",
                },
            };
        });
        const es = subgraph.relations.map((r) => {
            const s = String(r.source);
            const t = String(r.target);
            const dim = highlightPaths && selectedNodeId != null && !(neighbors.has(s) && neighbors.has(t));
            return {
                id: `r${r.id}`,
                source: s,
                target: t,
                label: showLabels ? r.relation_type : undefined,
                labelStyle: { fontSize: 10, fill: "#7c3aed", fontWeight: 600 },
                labelBgStyle: { fill: "#fff" },
                labelBgPadding: [4, 2],
                labelBgBorderRadius: 4,
                style: {
                    stroke: dim ? "#cbd5e1" : "#94a3b8",
                    strokeWidth: 1.4,
                    opacity: dim ? 0.4 : 1,
                },
            };
        });
        return { initialNodes: layoutGraph(ns, es, layoutDir), initialEdges: es };
    }, [subgraph, layoutDir, showLabels, highlightPaths, selectedNodeId, conceptTypeColors]);
    const [nodes, setNodes, onNodesChange] = useNodesState(initialNodes);
    const [edges, setEdges, onEdgesChange] = useEdgesState(initialEdges);
    useEffect(() => {
        setNodes(initialNodes);
        setEdges(initialEdges);
        // Fit after layout settles
        const id = window.setTimeout(() => rf.fitView({ padding: 0.15, duration: 250 }), 30);
        return () => window.clearTimeout(id);
    }, [initialNodes, initialEdges, setNodes, setEdges, rf]);
    useImperativeHandle(apiRef, () => ({
        fit: () => rf.fitView({ padding: 0.15, duration: 250 }),
        zoomIn: () => rf.zoomIn({ duration: 200 }),
        zoomOut: () => rf.zoomOut({ duration: 200 }),
        focusNode: (id) => {
            const n = rf.getNode(id);
            if (n)
                rf.setCenter(n.position.x + NODE_W / 2, n.position.y + NODE_H / 2, { zoom: 1.4, duration: 300 });
        },
    }), [apiRef, rf]);
    if (!subgraph || subgraph.concepts.length === 0) {
        return _jsx("div", { className: "empty", children: "No concepts to display. Upload data or generate an ontology to populate the graph." });
    }
    return (_jsx(ReactFlow, { nodes: nodes, edges: edges, onNodesChange: onNodesChange, onEdgesChange: onEdgesChange, onNodeClick: (_, n) => onNodeClick?.(n.id), fitView: true, minZoom: 0.1, maxZoom: 2, proOptions: { hideAttribution: true }, children: _jsx(Background, { gap: 20, color: "#e5e7eb" }) }));
}
const GraphCanvas = forwardRef(function GraphCanvas(props, ref) {
    const apiRef = useRef(null);
    useImperativeHandle(ref, () => apiRef.current ?? { fit: () => { }, zoomIn: () => { }, zoomOut: () => { }, focusNode: () => { } }, []);
    return (_jsx(ReactFlowProvider, { children: _jsx(CanvasInner, { ...props, apiRef: apiRef }) }));
});
export default GraphCanvas;
