// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Rendering tests of the OntologyGraph SVG: empty state, concentric layout
// (hub at the centre, inner/outer rings), label ellipsis and pill widths,
// the `limit` cut, the legend, and the proposed-concept overlay (dashed
// nodes/edges, ref resolution by client_ref and `Type:Name`).

import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import OntologyGraph from "./OntologyGraph";
import type { Concept, Relation, Subgraph } from "../api";
import type { OntologyProposal } from "../lib/proposalTypes";

const VBW = 960;

function concept(id: number, name: string, type = "Thing"): Concept {
  return { id, concept_type: type, name };
}
function relation(id: number, source: number, target: number, relation_type = "links"): Relation {
  return { id, relation_type, source, target };
}

/** Text nodes rendered inside the SVG, in document order. */
function svgTexts(container: HTMLElement): string[] {
  return Array.from(container.querySelectorAll("svg text")).map((t) => t.textContent ?? "");
}
function nodeRects(container: HTMLElement): SVGRectElement[] {
  // Node pills have rx = h/2 = 16; edge-label boxes have rx = 9.
  return Array.from(container.querySelectorAll("svg rect")).filter(
    (r) => r.getAttribute("rx") === "16",
  ) as SVGRectElement[];
}

const emptyProposal: OntologyProposal = {
  concept_types: [],
  relation_types: [],
  concepts: [],
  relations: [],
  rules: [],
  actions: [],
};

describe("OntologyGraph", () => {
  it("shows the empty hint when there is nothing to draw (null or empty subgraph)", () => {
    const { container, rerender } = render(<OntologyGraph subgraph={null} />);
    expect(container.textContent).toContain("Generate or upload sources to populate the ontology graph.");
    expect(container.querySelector("svg")).toBeNull();
    expect(container.querySelector(".og-legend")).toBeNull();

    rerender(<OntologyGraph subgraph={{ concepts: [], relations: [] }} className="extra" />);
    expect(container.querySelector("svg")).toBeNull();
    expect(container.querySelector(".og-wrap")).toHaveClass("extra");
  });

  it("puts the most connected concept at the centre and its neighbours on the inner ring", () => {
    // Node 3 has degree 3, the rest degree 1: 3 is the hub.
    const sg: Subgraph = {
      concepts: [concept(1, "A", "Person"), concept(2, "B", "Person"), concept(3, "Hub", "Org"), concept(4, "D", "Place")],
      relations: [relation(10, 3, 1, "employs"), relation(11, 3, 2, "employs"), relation(12, 4, 3, "hosts")],
    };
    const { container } = render(<OntologyGraph subgraph={sg} height={460} />);
    const svg = container.querySelector("svg")!;
    expect(svg.getAttribute("viewBox")).toBe(`0 0 ${VBW} 520`);

    const rects = nodeRects(container);
    expect(rects).toHaveLength(4);
    // Nodes are emitted in degree order: the hub first, centred horizontally.
    const hub = rects[0];
    const hubW = Number(hub.getAttribute("width"));
    expect(Number(hub.getAttribute("x")) + hubW / 2).toBeCloseTo(VBW / 2);
    expect(Number(hub.getAttribute("y")) + 16).toBeCloseTo(520 / 2);
    // Pill width: max(70, 3*7+24 = 45) -> 70.
    expect(hubW).toBe(70);

    // Three edges, each with a label pill.
    expect(container.querySelectorAll("svg line")).toHaveLength(3);
    const texts = svgTexts(container);
    expect(texts).toEqual(expect.arrayContaining(["employs", "hosts", "Hub", "A", "B", "D"]));
    // Nothing is dashed in a plain subgraph.
    for (const line of Array.from(container.querySelectorAll("svg line"))) {
      expect(line.getAttribute("stroke")).toBe("#cbd5e1");
      expect(line.hasAttribute("stroke-dasharray")).toBe(false);
    }

    // Legend: one entry per type, coloured by palette slot.
    const legend = Array.from(container.querySelectorAll(".og-legend li")).map((li) => li.textContent?.trim());
    expect(legend).toEqual(["Person", "Org", "Place"]);
    const dots = container.querySelectorAll(".og-legend .og-dot");
    expect((dots[0] as HTMLElement).style.background).toBe("rgb(37, 99, 235)"); // blue
    expect((dots[1] as HTMLElement).style.background).toBe("rgb(22, 163, 74)"); // green
  });

  it("ellipsizes long labels, caps the pill width and hides the legend on request", () => {
    const longName = "An extraordinarily long concept name indeed";
    const sg: Subgraph = {
      concepts: [concept(1, longName), concept(2, "Short")],
      relations: [relation(1, 1, 2, "a relation label that is far too long")],
    };
    const { container } = render(<OntologyGraph subgraph={sg} showLegend={false} height={100} />);
    // height 100 -> VBH = max(280, round(100/460*520)=113) = 280
    expect(container.querySelector("svg")!.getAttribute("viewBox")).toBe(`0 0 ${VBW} 280`);
    const texts = svgTexts(container);
    expect(texts).toContain(`${longName.slice(0, 21)}…`);
    expect(texts).toContain("a relation label …");
    const rects = nodeRects(container);
    expect(Number(rects[0].getAttribute("width"))).toBe(160);
    expect(container.querySelector(".og-legend")).toBeNull();
    // Inline style follows the height prop.
    expect(container.querySelector("style")!.textContent).toContain("min-height: 76px");
  });

  it("fills the inner ring from the outer ring when the hub has no neighbours", () => {
    const sg: Subgraph = {
      concepts: [concept(1, "Solo"), concept(2, "B"), concept(3, "C")],
      relations: [],
    };
    const { container } = render(<OntologyGraph subgraph={sg} />);
    const rects = nodeRects(container);
    expect(rects).toHaveLength(3);
    expect(container.querySelectorAll("svg line")).toHaveLength(0);
    // The two inner-ring pills sit above and below the hub at distinct heights.
    const ys = new Set(rects.map((r) => Math.round(Number(r.getAttribute("y")))));
    expect(ys.size).toBe(3);
  });

  it("applies `limit`, drops edges to hidden concepts and overflows the hub's neighbours to the outer ring", () => {
    const concepts: Concept[] = [concept(0, "Hub", "T0")];
    const relations: Relation[] = [];
    for (let i = 1; i <= 12; i++) {
      concepts.push(concept(i, `N${i}`, `T${i}`)); // 13 types > 8 palette slots
      relations.push(relation(100 + i, 0, i, ""));
    }
    const { container } = render(<OntologyGraph subgraph={{ concepts, relations }} limit={10} />);
    const rects = nodeRects(container);
    expect(rects).toHaveLength(10);
    // Only edges whose both ends are visible survive: hub + 9 neighbours.
    expect(container.querySelectorAll("svg line")).toHaveLength(9);
    // Empty relation_type -> no label pill.
    expect(container.querySelectorAll("svg rect")).toHaveLength(10);
    // Legend is capped at 8 entries; palette cycles past 8 types.
    expect(container.querySelectorAll(".og-legend li")).toHaveLength(8);
    const hub = rects[0];
    expect(Number(hub.getAttribute("x")) + Number(hub.getAttribute("width")) / 2).toBeCloseTo(VBW / 2);
  });

  it("overlays proposed concepts and relations with dashed strokes, resolving refs by client_ref and Type:Name", () => {
    const sg: Subgraph = {
      concepts: [concept(1, "ACME", "Org"), concept(2, "Alice", "Person")],
      relations: [relation(1, 2, 1, "works_at")],
    };
    const proposed: OntologyProposal = {
      ...emptyProposal,
      concepts: [
        // Matches an existing node (same type, case/space-insensitive name): no new pill.
        { client_ref: "f0/c0", concept_type: "Org", name: "  acme " },
        // Brand-new proposed concept.
        { client_ref: "f0/c1", concept_type: "Product", name: "Widget", description: "d" },
      ],
      relations: [
        // client_ref -> client_ref (existing ACME, new Widget)
        { client_ref: "f0/r0", relation_type: "sells", source_ref: "f0/c0", target_ref: "f0/c1" },
        // graph-ref `Type:Name` form
        { client_ref: "f0/r1", relation_type: "buys", source_ref: "Person:alice", target_ref: "f0/c1" },
        // unresolvable refs are skipped
        { client_ref: "f0/r2", relation_type: "x", source_ref: "Nope:Missing", target_ref: "f0/c1" },
        { client_ref: "f0/r3", relation_type: "y", source_ref: "f0/c1", target_ref: "noColonRef" },
      ],
    };
    const { container } = render(<OntologyGraph subgraph={sg} proposed={proposed} />);
    const texts = svgTexts(container);
    expect(texts).toContain("+ Widget");
    expect(texts).toContain("ACME");
    expect(texts.filter((t) => t.toLowerCase().includes("acme"))).toHaveLength(1);
    expect(texts).toEqual(expect.arrayContaining(["works_at", "sells", "buys"]));
    expect(texts).not.toContain("x");
    expect(texts).not.toContain("y");

    const lines = Array.from(container.querySelectorAll("svg line"));
    expect(lines).toHaveLength(3);
    const dashed = lines.filter((l) => l.getAttribute("stroke-dasharray") === "4 3");
    expect(dashed).toHaveLength(2);
    expect(dashed[0].getAttribute("stroke")).toBe("#7c3aed");

    const proposedRect = nodeRects(container).find((r) => r.getAttribute("stroke-dasharray") === "5 3")!;
    expect(proposedRect).toBeDefined();
    expect(proposedRect.getAttribute("stroke-width")).toBe("1.8");
    // The legend lists the proposed type too.
    expect(Array.from(container.querySelectorAll(".og-legend li")).map((li) => li.textContent?.trim())).toEqual([
      "Org",
      "Person",
      "Product",
    ]);
  });

  it("draws proposed concepts alone when there is no live subgraph", () => {
    const proposed: OntologyProposal = {
      ...emptyProposal,
      concepts: [
        { client_ref: "a", concept_type: "T", name: "One" },
        { client_ref: "b", concept_type: "T", name: "Two" },
      ],
      relations: [{ client_ref: "r", relation_type: "rel", source_ref: "a", target_ref: "b" }],
    };
    const { container } = render(<OntologyGraph subgraph={undefined} proposed={proposed} />);
    expect(svgTexts(container)).toEqual(expect.arrayContaining(["+ One", "+ Two", "rel"]));
    expect(container.querySelectorAll("svg line")).toHaveLength(1);
  });
});
