// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import Card from "./Card";

describe("Card", () => {
  it("renders title, actions, subtitle and children with the extra class and style", () => {
    render(
      <Card title="Stats" subtitle="Last 7 days" actions={<button>Reload</button>} className="wide" style={{ minHeight: 10 }}>
        <p>body</p>
      </Card>,
    );
    const section = screen.getByText("body").closest("section")!;
    expect(section).toHaveClass("card", "wide");
    expect(section).toHaveStyle({ minHeight: "10px" });
    const header = section.querySelector("header.card-title")!;
    expect(header).toHaveTextContent("Stats");
    expect(header).toContainElement(screen.getByRole("button", { name: "Reload" }));
    expect(section.querySelector(".card-subtitle")).toHaveTextContent("Last 7 days");
  });

  it("renders a header for actions alone, with an empty title span", () => {
    render(
      <Card actions={<span>act</span>}>
        <p>body</p>
      </Card>,
    );
    const header = document.querySelector("header.card-title")!;
    expect(header).not.toBeNull();
    expect(header.children[0]).toBeEmptyDOMElement();
    expect(header).toHaveTextContent("act");
  });

  it("renders neither header nor subtitle when only children are given", () => {
    render(
      <Card>
        <p>only</p>
      </Card>,
    );
    const section = screen.getByText("only").closest("section")!;
    expect(section.className).toBe("card");
    expect(section.querySelector("header")).toBeNull();
    expect(section.querySelector(".card-subtitle")).toBeNull();
  });
});
