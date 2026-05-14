// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { forwardRef, useEffect, useImperativeHandle, useMemo, useRef } from "react";
import ReactFlow, {
  Background,
  Edge,
  Node,
  ReactFlowProvider,
  useEdgesState,
  useNodesState,
  useReactFlow,
} from "reactflow";
import dagre from "dagre";
import "reactflow/dist/style.css";
import type { Subgraph } from "../api";

export type LayoutDir = "LR" | "TB" | "RL" | "BT";

export interface GraphCanvasHandle {
  fit: () => void;
  zoomIn: () => void;
  zoomOut: () => void;
  focusNode: (id: string) => void;
}

interface Props {
  subgraph: Subgraph | null;
  layoutDir?: LayoutDir;
  showLabels?: boolean;
  highlightPaths?: boolean;
  selectedNodeId?: string | null;
  conceptTypeColors?: Record<string, string>;
  onNodeClick?: (id: string) => void;
}

const NODE_W = 160;
const NODE_H = 40;
const DEFAULT_PALETTE = ["#2563eb", "#7c3aed", "#16a34a", "#dc2626", "#d97706", "#0891b2", "#db2777", "#0d9488"];

function layoutGraph(nodes: Node[], edges: Edge[], dir: LayoutDir): Node[] {
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

function softFill(hex: string): string {
  // Lighten by mixing with white at ~85%
  const m = /^#?([a-f\d]{6})$/i.exec(hex);
  if (!m) return "#dbeafe";
  const v = parseInt(m[1]!, 16);
  const r = (v >> 16) & 255;
  const g = (v >> 8) & 255;
  const b = v & 255;
  const mix = (c: number) => Math.round(c + (255 - c) * 0.82);
  return `rgb(${mix(r)}, ${mix(g)}, ${mix(b)})`;
}

function CanvasInner({
  subgraph,
  layoutDir = "LR",
  showLabels = true,
  highlightPaths = false,
  selectedNodeId = null,
  conceptTypeColors,
  onNodeClick,
  apiRef,
}: Props & { apiRef: React.MutableRefObject<GraphCanvasHandle | null> }) {
  const rf = useReactFlow();

  const { initialNodes, initialEdges } = useMemo<{ initialNodes: Node[]; initialEdges: Edge[] }>(() => {
    if (!subgraph) return { initialNodes: [], initialEdges: [] };
    const fallback: Record<string, string> = {};
    let idx = 0;
    const colorOf = (t: string): string => {
      if (conceptTypeColors && conceptTypeColors[t]) return conceptTypeColors[t]!;
      return (fallback[t] ??= DEFAULT_PALETTE[idx++ % DEFAULT_PALETTE.length]!);
    };

    const neighbors = new Set<string>();
    if (selectedNodeId) {
      for (const r of subgraph.relations) {
        if (String(r.source) === selectedNodeId) neighbors.add(String(r.target));
        if (String(r.target) === selectedNodeId) neighbors.add(String(r.source));
      }
      neighbors.add(selectedNodeId);
    }

    const ns: Node[] = subgraph.concepts.map((c) => {
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
          textAlign: "center" as const,
          boxShadow: isSelected ? `0 0 0 4px ${fill}` : "none",
          opacity: dim ? 0.25 : 1,
          transition: "opacity 0.15s",
        },
      };
    });

    const es: Edge[] = subgraph.relations.map((r) => {
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
        labelBgPadding: [4, 2] as [number, number],
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

  useImperativeHandle(
    apiRef,
    () => ({
      fit: () => rf.fitView({ padding: 0.15, duration: 250 }),
      zoomIn: () => rf.zoomIn({ duration: 200 }),
      zoomOut: () => rf.zoomOut({ duration: 200 }),
      focusNode: (id: string) => {
        const n = rf.getNode(id);
        if (n) rf.setCenter(n.position.x + NODE_W / 2, n.position.y + NODE_H / 2, { zoom: 1.4, duration: 300 });
      },
    }),
    [apiRef, rf]
  );

  if (!subgraph || subgraph.concepts.length === 0) {
    return <div className="empty">No concepts to display. Upload data or generate an ontology to populate the graph.</div>;
  }

  return (
    <ReactFlow
      nodes={nodes}
      edges={edges}
      onNodesChange={onNodesChange}
      onEdgesChange={onEdgesChange}
      onNodeClick={(_, n) => onNodeClick?.(n.id)}
      fitView
      minZoom={0.1}
      maxZoom={2}
      proOptions={{ hideAttribution: true }}
    >
      <Background gap={20} color="#e5e7eb" />
    </ReactFlow>
  );
}

const GraphCanvas = forwardRef<GraphCanvasHandle, Props>(function GraphCanvas(props, ref) {
  const apiRef = useRef<GraphCanvasHandle | null>(null);
  useImperativeHandle(ref, () => apiRef.current ?? { fit: () => {}, zoomIn: () => {}, zoomOut: () => {}, focusNode: () => {} }, []);
  return (
    <ReactFlowProvider>
      <CanvasInner {...props} apiRef={apiRef} />
    </ReactFlowProvider>
  );
});

export default GraphCanvas;
