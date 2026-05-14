// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { useEffect, useMemo } from "react";
import ReactFlow, {
  Background,
  Controls,
  Edge,
  Node,
  useEdgesState,
  useNodesState,
} from "reactflow";
import dagre from "dagre";
import "reactflow/dist/style.css";
import type { Subgraph } from "../api";

interface Props {
  subgraph: Subgraph | null;
}

const NODE_W = 180;
const NODE_H = 44;

function layout(nodes: Node[], edges: Edge[]): { nodes: Node[]; edges: Edge[] } {
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

export default function GraphCanvas({ subgraph }: Props) {
  const { initialNodes, initialEdges } = useMemo<{ initialNodes: Node[]; initialEdges: Edge[] }>(() => {
    if (!subgraph) return { initialNodes: [], initialEdges: [] };
    const colorOf = (() => {
      const palette = ["#2563eb", "#7c3aed", "#16a34a", "#dc2626", "#d97706", "#0891b2"];
      const map: Record<string, string> = {};
      let i = 0;
      return (t: string) => (map[t] ??= palette[i++ % palette.length]);
    })();
    const ns: Node[] = subgraph.concepts.map((c) => ({
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
    const es: Edge[] = subgraph.relations.map((r) => ({
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
    return <div className="empty">No concepts to display. Upload data or generate an ontology to populate the graph.</div>;
  }

  return (
    <ReactFlow
      nodes={nodes}
      edges={edges}
      onNodesChange={onNodesChange}
      onEdgesChange={onEdgesChange}
      fitView
      minZoom={0.1}
      maxZoom={2}
    >
      <Background gap={20} color="#e5e7eb" />
      <Controls position="bottom-right" />
    </ReactFlow>
  );
}
