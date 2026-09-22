// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Rendering tests of the two small stat widgets: the StatCard tile (value
// formatting, delta arrow and hint) and the Sparkline SVG it is paired with.

import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import StatCard from "./StatCard";
import Sparkline from "./Sparkline";

describe("StatCard", () => {
  it("renders the label and a string value untouched", () => {
    const { container } = render(<StatCard label="Health" value="98%" />);
    expect(container.querySelector(".label")).toHaveTextContent("Health");
    expect(container.querySelector(".value")).toHaveTextContent("98%");
    // No delta and no hint: no delta line at all.
    expect(container.querySelector(".delta")).toBeNull();
  });

  it("abbreviates large numbers and leaves small ones as is", () => {
    const value = (v: number) => render(<StatCard label="n" value={v} />).container.querySelector(".value")!.textContent;
    expect(value(999)).toBe("999");
    expect(value(9_999)).toBe("9999");
    expect(value(12_345)).toBe("12.3k");
    expect(value(2_500_000)).toBe("2.5M");
  });

  it("shows an upward delta with the default hint", () => {
    const { container } = render(<StatCard label="n" value={1} deltaPct={12.34} />);
    const delta = container.querySelector(".delta")!;
    expect(delta).toHaveClass("up");
    expect(delta).toHaveTextContent("▲ 12.3% vs first sample");
  });

  it("shows a downward delta with a custom hint", () => {
    const { container } = render(<StatCard label="n" value={1} deltaPct={-3} hint="vs yesterday" />);
    const delta = container.querySelector(".delta")!;
    expect(delta).toHaveClass("down");
    expect(delta).toHaveTextContent("▼ 3.0% vs yesterday");
  });

  it("treats a delta within ±0.5 as flat", () => {
    const { container } = render(<StatCard label="n" value={1} deltaPct={0.2} />);
    const delta = container.querySelector(".delta")!;
    expect(delta).toHaveClass("flat");
    expect(delta).toHaveTextContent("• 0.2%");
  });

  it("shows the hint alone when there is no delta", () => {
    const { container } = render(<StatCard label="n" value={1} hint="since boot" />);
    const delta = container.querySelector(".delta")!;
    expect(delta).toHaveClass("flat");
    expect(delta).toHaveTextContent("since boot");
  });
});

describe("Sparkline", () => {
  it("renders an empty SVG when there are no values", () => {
    const { container } = render(<Sparkline values={[]} />);
    const svg = container.querySelector("svg.sparkline")!;
    expect(svg).toBeInTheDocument();
    expect(svg.children).toHaveLength(0);
  });

  it("scales the points across the viewBox: min at the bottom, max at the top", () => {
    const { container } = render(<Sparkline values={[0, 10, 5]} stroke="#123456" />);
    const line = container.querySelector("polyline")!;
    // x: 0, 50, 100; y: min → 34, max → 6, mid → 20.
    expect(line.getAttribute("points")).toBe("0.00,34.00 50.00,6.00 100.00,20.00");
    expect(line.getAttribute("stroke")).toBe("#123456");
    const area = container.querySelector("polygon")!;
    expect(area.getAttribute("points")).toBe("0,36 0.00,34.00 50.00,6.00 100.00,20.00 100,36");
  });

  it("handles a flat series (zero span) and a single value without NaN", () => {
    const flat = render(<Sparkline values={[4, 4]} />).container.querySelector("polyline")!;
    expect(flat.getAttribute("points")).toBe("0.00,34.00 100.00,34.00");
    const single = render(<Sparkline values={[7]} />).container.querySelector("polyline")!;
    expect(single.getAttribute("points")).toBe("0.00,34.00");
    expect(single.getAttribute("stroke")).toBe("var(--accent)");
  });
});
