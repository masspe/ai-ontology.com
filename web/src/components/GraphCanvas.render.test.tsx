// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Rendering tests of the graph canvas: the React Flow scene is mounted for
// real (jsdom, no layout engine) and driven through the node / pane DOM
// events React Flow wires: click, double-click, hover, pane click, and the
// imperative handle (fit / zoom / focusNode).

import { createRef } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import GraphCanvas, { type GraphCanvasHandle } from "./GraphCanvas";
import type { Subgraph } from "../api";

const subgraph: Subgraph = {
  concepts: [
    { id: 1, concept_type: "Person", name: "Alice" },
    { id: 2, concept_type: "Company", name: "ACME" },
    { id: 3, concept_type: "Person", name: "Bob" },
    { id: 4, concept_type: "City", name: "Geneva" },
  ],
  relations: [
    { id: 10, relation_type: "worksAt", source: 1, target: 2 },
    { id: 11, relation_type: "knows", source: 1, target: 3 },
    { id: 12, relation_type: "basedIn", source: 2, target: 4 },
  ],
};

const node = (id: number) => screen.getByTestId(`rf__node-${id}`);
const edge = (id: number) => screen.getByTestId(`rf__edge-r${id}`);
const edgePath = (id: number) => edge(id).querySelector("path") as SVGPathElement;
const pane = () => document.querySelector(".react-flow__pane") as HTMLElement;

// The palette colours as jsdom serialises them in inline styles.
const BLUE = "rgb(37, 99, 235)"; // #2563eb
const VIOLET = "rgb(124, 58, 237)"; // #7c3aed
const GREEN = "rgb(22, 163, 74)"; // #16a34a
const RED = "rgb(255, 0, 0)"; // #ff0000
const GREY = "rgb(148, 163, 184)"; // #94a3b8, unfocused edge
const DIM = "rgb(203, 213, 225)"; // #cbd5e1, dimmed edge

/**
 * React Flow only sizes a node (and therefore draws its edges) when its
 * ResizeObserver reports the node element. jsdom never lays anything out, so
 * this stub reports every observed element straight away.
 */
class FiringResizeObserver {
  constructor(private readonly cb: ResizeObserverCallback) {}
  observe(target: Element): void {
    const entry = { target, contentRect: target.getBoundingClientRect() } as unknown as ResizeObserverEntry;
    queueMicrotask(() => this.cb([entry], this as unknown as ResizeObserver));
  }
  unobserve(): void {}
  disconnect(): void {}
}

/** Enough of DOMMatrixReadOnly for React Flow to read the viewport zoom. */
class DOMMatrixStub {
  m22 = 1;
  constructor(transform?: string) {
    const m = /scale\(([\d.]+)\)/.exec(transform ?? "");
    if (m) this.m22 = Number(m[1]);
  }
}

beforeEach(() => {
  // React Flow measures its container and its nodes; jsdom has no layout, so
  // every element reports a fixed box.
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
    x: 0, y: 0, top: 0, left: 0, right: 800, bottom: 600, width: 800, height: 600, toJSON: () => ({}),
  } as DOMRect);
  Object.defineProperty(HTMLElement.prototype, "offsetWidth", { configurable: true, get: () => 800 });
  Object.defineProperty(HTMLElement.prototype, "offsetHeight", { configurable: true, get: () => 600 });
  vi.stubGlobal("ResizeObserver", FiringResizeObserver);
  vi.stubGlobal("DOMMatrixReadOnly", DOMMatrixStub);
  // Edge labels measure their text box; jsdom has no SVG geometry.
  Object.defineProperty(SVGElement.prototype, "getBBox", {
    configurable: true,
    value: () => ({ x: 0, y: 0, width: 40, height: 12 }),
  });
});

afterEach(() => {
  vi.restoreAllMocks();
  delete (SVGElement.prototype as unknown as { getBBox?: unknown }).getBBox;
});

describe("GraphCanvas", () => {
  it("shows the empty state without a subgraph or without concepts", () => {
    const { rerender } = render(<GraphCanvas subgraph={null} />);
    expect(screen.getByText(/No concepts to display/)).toBeInTheDocument();
    rerender(<GraphCanvas subgraph={{ concepts: [], relations: [] }} />);
    expect(screen.getByText(/No concepts to display/)).toBeInTheDocument();
  });

  it("renders one node per concept with its label, and one labelled edge per relation", async () => {
    render(<GraphCanvas subgraph={subgraph} />);
    expect(await screen.findByText("Alice")).toBeInTheDocument();
    for (const c of subgraph.concepts) expect(node(c.id)).toBeInTheDocument();
    expect(screen.getByText("Geneva")).toBeInTheDocument();
    expect(document.querySelectorAll(".react-flow__node")).toHaveLength(4);
    // Edges appear once the nodes are measured.
    expect(await screen.findByTestId("rf__edge-r10")).toBeInTheDocument();
    expect(document.querySelectorAll(".react-flow__edge")).toHaveLength(3);
    expect(screen.getByText("worksAt")).toBeInTheDocument();
  });

  it("hides node and edge labels when asked", async () => {
    render(<GraphCanvas subgraph={subgraph} showLabels={false} />);
    await screen.findByTestId("rf__edge-r10");
    expect(screen.queryByText("Alice")).toBeNull();
    expect(screen.queryByText("worksAt")).toBeNull();
  });

  it("colours nodes by concept type: explicit colours first, then a stable fallback palette", async () => {
    const { rerender } = render(<GraphCanvas subgraph={subgraph} conceptTypeColors={{ Person: "#ff0000" }} />);
    await screen.findByTestId("rf__node-1");
    // Person → explicit red; Company / City → first two palette entries.
    expect(node(1).style.border).toContain(RED);
    expect(node(3).style.border).toContain(RED);
    expect(node(2).style.border).toContain(BLUE);
    expect(node(4).style.border).toContain(VIOLET);
    // Without a map every type comes from the palette, in first-seen order.
    rerender(<GraphCanvas subgraph={subgraph} />);
    expect(node(1).style.border).toContain(BLUE);
    expect(node(2).style.border).toContain(VIOLET);
    expect(node(4).style.border).toContain(GREEN);
  });

  it("reports node clicks, double-clicks and pane clicks to the parent", async () => {
    const onNodeClick = vi.fn();
    const onNodeDoubleClick = vi.fn();
    const onPaneClick = vi.fn();
    render(
      <GraphCanvas subgraph={subgraph} onNodeClick={onNodeClick} onNodeDoubleClick={onNodeDoubleClick} onPaneClick={onPaneClick} />,
    );
    await screen.findByTestId("rf__node-1");
    fireEvent.click(node(2));
    expect(onNodeClick).toHaveBeenCalledWith("2");
    fireEvent.doubleClick(node(3));
    expect(onNodeDoubleClick).toHaveBeenCalledWith("3");
    fireEvent.click(pane());
    expect(onPaneClick).toHaveBeenCalledTimes(1);
  });

  it("tolerates missing callbacks", async () => {
    render(<GraphCanvas subgraph={subgraph} />);
    await screen.findByTestId("rf__node-1");
    expect(() => {
      fireEvent.click(node(1));
      fireEvent.doubleClick(node(1));
      fireEvent.click(pane());
    }).not.toThrow();
  });

  it("emphasises the selected node and its edges", async () => {
    const { rerender } = render(<GraphCanvas subgraph={subgraph} selectedNodeId={null} />);
    await screen.findByTestId("rf__edge-r10");
    expect(node(1).style.border.startsWith("1.5px")).toBe(true);
    expect(node(1).style.boxShadow).toBe("none");
    // No focus: every edge is grey and static.
    expect(edge(10)).not.toHaveClass("animated");
    expect(edgePath(10).style.stroke).toBe(GREY);
    rerender(<GraphCanvas subgraph={subgraph} selectedNodeId="1" />);
    expect(node(1).style.border.startsWith("2.5px")).toBe(true);
    expect(node(1).style.boxShadow).not.toBe("none");
    // The nodes are re-measured on a microtask; the edges come back after it.
    // The selected node's edges take its colour and animate; others do not.
    expect(await screen.findByTestId("rf__edge-r10")).toHaveClass("animated");
    expect(edgePath(10).style.stroke).toBe(BLUE);
    expect(edge(12)).not.toHaveClass("animated");
    expect(edgePath(12).style.stroke).toBe(GREY);
  });

  it("dims everything outside the focus set when highlightPaths is on", async () => {
    render(<GraphCanvas subgraph={subgraph} highlightPaths selectedNodeId="4" />);
    await screen.findByTestId("rf__edge-r10");
    // 4 (Geneva) is linked to 2 (ACME) only: 1 and 3 are dimmed.
    expect(node(4).style.opacity).toBe("1");
    expect(node(2).style.opacity).toBe("1");
    expect(node(1).style.opacity).toBe("0.2");
    expect(node(3).style.opacity).toBe("0.2");
    // Same for edges: 2→4 stays sharp, 1→2 and 1→3 fade to the dim grey.
    expect(edgePath(12).style.opacity).toBe("1");
    expect(edgePath(10).style.opacity).toBe("0.35");
    expect(edgePath(10).style.stroke).toBe(DIM);
  });

  it("uses the hovered node as the focus when nothing is selected", async () => {
    render(<GraphCanvas subgraph={subgraph} highlightPaths />);
    await screen.findByTestId("rf__node-1");
    // No focus yet: nothing dimmed.
    expect(node(4).style.opacity).toBe("1");
    fireEvent.mouseEnter(node(1));
    // Hovering 1 (Alice) keeps 2 and 3, dims 4.
    expect(node(1).style.border.startsWith("2px")).toBe(true);
    expect(node(1).style.boxShadow).not.toBe("none");
    expect(node(4).style.opacity).toBe("0.2");
    expect(node(2).style.opacity).toBe("1");
    fireEvent.mouseLeave(node(1));
    expect(node(4).style.opacity).toBe("1");
    expect(node(1).style.border.startsWith("1.5px")).toBe(true);
  });

  it("does not dim anything when highlightPaths is off, even with a selection", async () => {
    render(<GraphCanvas subgraph={subgraph} highlightPaths={false} selectedNodeId="4" />);
    await screen.findByTestId("rf__node-1");
    for (const c of subgraph.concepts) expect(node(c.id).style.opacity).toBe("1");
  });

  it("re-lays the graph out when the direction changes", async () => {
    const { rerender } = render(<GraphCanvas subgraph={subgraph} layoutDir="LR" />);
    await screen.findByTestId("rf__node-1");
    const before = node(1).style.transform;
    rerender(<GraphCanvas subgraph={subgraph} layoutDir="TB" />);
    await waitFor(() => expect(node(1).style.transform).not.toBe(before));
  });

  it("exposes fit / zoomIn / zoomOut / focusNode through the ref", async () => {
    const ref = createRef<GraphCanvasHandle>();
    render(<GraphCanvas ref={ref} subgraph={subgraph} />);
    await screen.findByTestId("rf__node-1");
    expect(ref.current).not.toBeNull();
    expect(() => {
      act(() => {
        ref.current!.fit();
        ref.current!.zoomIn();
        ref.current!.zoomOut();
        ref.current!.focusNode("2");
        ref.current!.focusNode("does-not-exist");
      });
    }).not.toThrow();
  });

  it("falls back to no-op handle methods before the scene mounts", () => {
    const ref = createRef<GraphCanvasHandle>();
    render(<GraphCanvas ref={ref} subgraph={null} />);
    expect(ref.current).not.toBeNull();
    expect(() => {
      ref.current!.fit();
      ref.current!.zoomIn();
      ref.current!.zoomOut();
      ref.current!.focusNode("1");
    }).not.toThrow();
  });
});
