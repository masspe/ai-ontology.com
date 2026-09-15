// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { forwardRef, useEffect, useImperativeHandle, useMemo, useRef, useState } from "react";
import ReactFlow, {
  Background,
  Controls,
  Edge,
  MarkerType,
  MiniMap,
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
  /** Click on a node. The parent decides whether that selects or toggles. */
  onNodeClick?: (id: string) => void;
  /** Click on the empty canvas — used to clear the selection. */
  onPaneClick?: () => void;
  /** Double-click on a node — the parent typically centres on it. */
  onNodeDoubleClick?: (id: string) => void;
}

const NODE_W = 160;
const NODE_H = 40;
const DEFAULT_PALETTE = ["#2563eb", "#7c3aed", "#16a34a", "#dc2626", "#d97706", "#0891b2", "#db2777", "#0d9488"];

/** Node styling inputs that do not move nodes around. */
interface NodeMeta {
  id: string;
  name: string;
  type: string;
  stroke: string;
  fill: string;
}

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
  onPaneClick,
  onNodeDoubleClick,
  apiRef,
}: Props & { apiRef: React.MutableRefObject<GraphCanvasHandle | null> }) {
  const rf = useReactFlow();
  const [hoveredId, setHoveredId] = useState<string | null>(null);

  // 1. Layout — only when the data or the direction changes. Selection and
  //    hover must never re-run dagre, or every click would reshuffle the
  //    canvas and reset the viewport.
  const { layoutNodes, baseEdges, metaById } = useMemo(() => {
    if (!subgraph) {
      return { layoutNodes: [] as Node[], baseEdges: [] as Edge[], metaById: new Map<string, NodeMeta>() };
    }
    const fallback: Record<string, string> = {};
    let idx = 0;
    const colorOf = (t: string): string => {
      if (conceptTypeColors && conceptTypeColors[t]) return conceptTypeColors[t]!;
      return (fallback[t] ??= DEFAULT_PALETTE[idx++ % DEFAULT_PALETTE.length]!);
    };
    const metaById = new Map<string, NodeMeta>();
    const ns: Node[] = subgraph.concepts.map((c) => {
      const id = String(c.id);
      const stroke = colorOf(c.concept_type);
      metaById.set(id, { id, name: c.name, type: c.concept_type, stroke, fill: softFill(stroke) });
      return { id, position: { x: 0, y: 0 }, data: { label: c.name } };
    });
    const es: Edge[] = subgraph.relations.map((r) => ({
      id: `r${r.id}`,
      source: String(r.source),
      target: String(r.target),
      label: r.relation_type,
    }));
    return { layoutNodes: layoutGraph(ns, es, layoutDir), baseEdges: es, metaById };
  }, [subgraph, layoutDir, conceptTypeColors]);

  // 2. Focus set: the selected node and its neighbours, or — with nothing
  //    selected — the hovered node and its neighbours.
  const focusId = selectedNodeId ?? hoveredId;
  const neighbors = useMemo(() => {
    const set = new Set<string>();
    if (!focusId) return set;
    for (const e of baseEdges) {
      if (e.source === focusId) set.add(e.target);
      if (e.target === focusId) set.add(e.source);
    }
    set.add(focusId);
    return set;
  }, [baseEdges, focusId]);

  // 3. Styling — cheap, runs on every selection/hover change.
  const styledNodes = useMemo<Node[]>(
    () =>
      layoutNodes.map((n) => {
        const m = metaById.get(n.id)!;
        const isSelected = selectedNodeId === n.id;
        const isHovered = hoveredId === n.id;
        const dim = highlightPaths && focusId != null && !neighbors.has(n.id);
        return {
          ...n,
          data: { label: showLabels ? m.name : "" },
          className: "gv-node",
          style: {
            background: m.fill,
            border: `${isSelected ? 2.5 : isHovered ? 2 : 1.5}px solid ${m.stroke}`,
            borderRadius: 999,
            padding: "6px 14px",
            fontSize: 12,
            fontWeight: 600,
            color: m.stroke,
            width: NODE_W,
            height: NODE_H,
            textAlign: "center" as const,
            boxShadow: isSelected ? `0 0 0 4px ${m.fill}, 0 6px 16px rgba(15, 23, 42, 0.18)` : isHovered ? `0 0 0 3px ${m.fill}` : "none",
            opacity: dim ? 0.2 : 1,
            transition: "opacity 0.15s, box-shadow 0.15s",
            cursor: "pointer",
          },
        };
      }),
    [layoutNodes, metaById, selectedNodeId, hoveredId, focusId, neighbors, highlightPaths, showLabels]
  );

  const styledEdges = useMemo<Edge[]>(
    () =>
      baseEdges.map((e) => {
        const touchesFocus = focusId != null && (e.source === focusId || e.target === focusId);
        const dim = highlightPaths && focusId != null && !touchesFocus;
        const accent = touchesFocus ? metaById.get(focusId!)?.stroke ?? "#7c3aed" : "#94a3b8";
        return {
          ...e,
          type: "smoothstep",
          animated: touchesFocus && selectedNodeId != null,
          label: showLabels ? e.label : undefined,
          labelStyle: { fontSize: 10, fill: touchesFocus ? accent : "#7c3aed", fontWeight: 600 },
          labelBgStyle: { fill: "#fff", fillOpacity: dim ? 0.3 : 0.95 },
          labelBgPadding: [4, 2] as [number, number],
          labelBgBorderRadius: 4,
          markerEnd: { type: MarkerType.ArrowClosed, width: 14, height: 14, color: dim ? "#cbd5e1" : accent },
          style: {
            stroke: dim ? "#cbd5e1" : accent,
            strokeWidth: touchesFocus ? 2.2 : 1.4,
            opacity: dim ? 0.35 : 1,
            transition: "opacity 0.15s, stroke 0.15s",
          },
        };
      }),
    [baseEdges, focusId, selectedNodeId, metaById, highlightPaths, showLabels]
  );

  const [nodes, setNodes, onNodesChange] = useNodesState(styledNodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState(styledEdges);

  // New layout: take the computed positions and fit the viewport.
  useEffect(() => {
    setNodes(styledNodes);
    const id = window.setTimeout(() => rf.fitView({ padding: 0.15, duration: 250 }), 30);
    return () => window.clearTimeout(id);
    // Only the layout is a trigger here; styling is applied below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [layoutNodes, setNodes, rf]);

  // Style change only: keep whatever positions the user dragged nodes to.
  useEffect(() => {
    setNodes((prev) => {
      const pos = new Map(prev.map((p) => [p.id, p.position] as const));
      return styledNodes.map((n) => ({ ...n, position: pos.get(n.id) ?? n.position }));
    });
  }, [styledNodes, setNodes]);

  useEffect(() => {
    setEdges(styledEdges);
  }, [styledEdges, setEdges]);

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
      onNodeDoubleClick={(_, n) => onNodeDoubleClick?.(n.id)}
      onNodeMouseEnter={(_, n) => setHoveredId(n.id)}
      onNodeMouseLeave={() => setHoveredId(null)}
      onPaneClick={() => onPaneClick?.()}
      nodesConnectable={false}
      elementsSelectable
      fitView
      minZoom={0.1}
      maxZoom={2.5}
      proOptions={{ hideAttribution: true }}
    >
      <Background gap={20} color="#e5e7eb" />
      <Controls showInteractive={false} position="bottom-left" />
      <MiniMap
        pannable
        zoomable
        position="bottom-right"
        nodeColor={(n) => metaById.get(n.id)?.stroke ?? "#94a3b8"}
        nodeStrokeWidth={2}
        maskColor="rgba(241, 245, 249, 0.7)"
        style={{ width: 140, height: 90 }}
      />
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
