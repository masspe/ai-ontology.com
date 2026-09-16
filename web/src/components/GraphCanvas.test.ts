// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import type { Edge, Node } from "reactflow";
import { describe, expect, it } from "vitest";
import { layoutGraph, softFill, type LayoutDir } from "./GraphCanvas";

describe("softFill", () => {
  it("mixes a 6-digit hex colour with 82% white", () => {
    // #2563eb → r 37, g 99, b 235 → each c + (255 - c) * 0.82, rounded
    expect(softFill("#2563eb")).toBe("rgb(216, 227, 251)");
  });

  it("accepts the hex without a leading '#' and in upper case", () => {
    expect(softFill("2563eb")).toBe("rgb(216, 227, 251)");
    expect(softFill("#2563EB")).toBe("rgb(216, 227, 251)");
  });

  it("maps black to a light grey and leaves white unchanged", () => {
    expect(softFill("#000000")).toBe("rgb(209, 209, 209)");
    expect(softFill("#ffffff")).toBe("rgb(255, 255, 255)");
  });

  it("falls back to #dbeafe for anything that is not a 6-digit hex colour", () => {
    for (const bad of ["#fff", "red", "", "#12345", "#1234567", "rgb(1,2,3)", "#gggggg"]) {
      expect(softFill(bad), bad).toBe("#dbeafe");
    }
  });
});

describe("layoutGraph", () => {
  const node = (id: string): Node => ({ id, position: { x: 0, y: 0 }, data: { label: id } });
  const edge = (source: string, target: string): Edge => ({ id: `${source}-${target}`, source, target });
  const chain = () => ({
    nodes: [node("a"), node("b"), node("c")],
    edges: [edge("a", "b"), edge("b", "c")],
  });

  it("returns one positioned node per input node, in the same order, keeping every other field", () => {
    const { nodes, edges } = chain();
    const out = layoutGraph(nodes, edges, "LR");
    expect(out.map((n) => n.id)).toEqual(["a", "b", "c"]);
    for (const n of out) {
      expect(n.data).toEqual({ label: n.id });
      expect(Number.isFinite(n.position.x)).toBe(true);
      expect(Number.isFinite(n.position.y)).toBe(true);
    }
  });

  it("does not mutate the input nodes", () => {
    const { nodes, edges } = chain();
    layoutGraph(nodes, edges, "TB");
    expect(nodes.every((n) => n.position.x === 0 && n.position.y === 0)).toBe(true);
  });

  it("places a single node so that its 160x40 box is centred on the dagre point", () => {
    const [n] = layoutGraph([node("solo")], [], "LR");
    // dagre puts a lone node at (w/2, h/2); subtracting half the box gives (0, 0)
    expect(n.position).toEqual({ x: 0, y: 0 });
  });

  it("lays a chain out along the requested direction", () => {
    const expectOrder = (dir: LayoutDir, pick: (n: Node) => number, ascending: boolean) => {
      const { nodes, edges } = chain();
      const [a, b, c] = layoutGraph(nodes, edges, dir);
      const cmp = ascending ? (x: number, y: number) => x < y : (x: number, y: number) => x > y;
      expect(cmp(pick(a), pick(b)), `${dir}: a→b`).toBe(true);
      expect(cmp(pick(b), pick(c)), `${dir}: b→c`).toBe(true);
    };
    expectOrder("LR", (n) => n.position.x, true);
    expectOrder("RL", (n) => n.position.x, false);
    expectOrder("TB", (n) => n.position.y, true);
    expectOrder("BT", (n) => n.position.y, false);
  });

  it("keeps ranks aligned: nodes of a LR chain share one y, of a TB chain one x", () => {
    const lr = layoutGraph(chain().nodes, chain().edges, "LR");
    expect(new Set(lr.map((n) => n.position.y)).size).toBe(1);
    const tb = layoutGraph(chain().nodes, chain().edges, "TB");
    expect(new Set(tb.map((n) => n.position.x)).size).toBe(1);
  });

  it("separates unconnected nodes so their boxes never overlap", () => {
    const out = layoutGraph([node("a"), node("b"), node("c")], [], "LR");
    const boxes = out.map((n) => ({ x1: n.position.x, x2: n.position.x + 160, y1: n.position.y, y2: n.position.y + 40 }));
    for (let i = 0; i < boxes.length; i++) {
      for (let j = i + 1; j < boxes.length; j++) {
        const a = boxes[i];
        const b = boxes[j];
        const overlap = a.x1 < b.x2 && b.x1 < a.x2 && a.y1 < b.y2 && b.y1 < a.y2;
        expect(overlap, `${out[i].id} vs ${out[j].id}`).toBe(false);
      }
    }
  });

  it("is deterministic for identical input", () => {
    const first = layoutGraph(chain().nodes, chain().edges, "TB");
    const second = layoutGraph(chain().nodes, chain().edges, "TB");
    expect(second).toEqual(first);
  });
});
