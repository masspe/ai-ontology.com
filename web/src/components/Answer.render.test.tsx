// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The answer a reader sees: `#id` tokens become linked sheets, the `Cited:`
// line becomes the row of sheets with the cited ones first, and nothing the
// subgraph does not know is invented.

import { describe, expect, it } from "vitest";
import Answer, { splitCited } from "./Answer";
import { renderPage, screen } from "../test/render";
import type { Concept } from "../api";

const concepts: Concept[] = [
  { id: 1, concept_type: "Table", name: "appdb.emp", description: "Salariés" },
  { id: 2, concept_type: "Screen", name: "gl100" },
  { id: 3, concept_type: "Program", name: "pr/w-pr100.w" },
];

describe("splitCited", () => {
  it("strips the Cited line and reads its ids", () => {
    expect(splitCited("Two sheets.\nCited: [#1, #3]")).toEqual({ body: "Two sheets.", cited: [1, 3] });
    expect(splitCited("No line")).toEqual({ body: "No line", cited: [] });
  });

  it("strips a Cited line that is still arriving", () => {
    expect(splitCited("Prose.\nCited: [#1, #")).toEqual({ body: "Prose.", cited: [1] });
    expect(splitCited("Prose.\nCited:")).toEqual({ body: "Prose.", cited: [] });
  });
});

describe("Answer", () => {
  it("turns #id into the sheet, linked, and leaves unknown ids as text", () => {
    renderPage(<Answer text="The table #1 is read by #3 and by #99." concepts={concepts} />);
    const box = document.querySelector(".answer-box")!;
    expect(box.textContent).toBe("The table Table · appdb.emp is read by Program · pr/w-pr100.w and by #99.");
    const links = box.querySelectorAll("a.sheet-chip");
    expect(links).toHaveLength(2);
    expect(links[0]).toHaveAttribute("href", "/concepts/1");
    expect(links[0]).toHaveAttribute("title", "Salariés");
  });

  it("puts the cited sheets first and marks them, then the other retrieved sheets", () => {
    renderPage(<Answer text={"See #2.\nCited: [#2, #2]"} concepts={concepts} />);
    expect(document.querySelector(".answer-box")!.textContent).toBe("See Screen · gl100.");
    const row = document.querySelector(".citations")!;
    const chips = Array.from(row.querySelectorAll("a.sheet-chip"));
    expect(chips.map((c) => c.textContent)).toEqual(["Screen · gl100", "Table · appdb.emp", "Program · pr/w-pr100.w"]);
    expect(chips[0]).toHaveClass("cited");
    expect(chips[0]).toHaveAttribute("title", "Citée dans la réponse");
    expect(chips[1]).not.toHaveClass("cited");
  });

  it("prefers the ids the server parsed over the text", () => {
    renderPage(<Answer text={"Prose.\nCited: [#2]"} concepts={concepts} cited={[3]} />);
    const chips = Array.from(document.querySelectorAll(".citations a.sheet-chip"));
    expect(chips[0]).toHaveTextContent("Program · pr/w-pr100.w");
    expect(chips[0]).toHaveClass("cited");
  });

  it("shows at most twelve sheets, the cited ones counted first", () => {
    const many: Concept[] = Array.from({ length: 20 }, (_, i) => ({ id: i + 1, concept_type: "T", name: `n${i + 1}` }));
    renderPage(<Answer text={"x\nCited: [#20]"} concepts={many} />);
    const chips = Array.from(document.querySelectorAll(".citations a.sheet-chip"));
    expect(chips).toHaveLength(12);
    expect(chips[0]).toHaveTextContent("T · n20");
    expect(chips[11]).toHaveTextContent("T · n11");
  });

  it("renders no sheet row without sheets, and the cursor while streaming", () => {
    renderPage(<Answer text="…" cursor />);
    expect(document.querySelector(".citations")).toBeNull();
    expect(screen.getByText("▍")).toBeInTheDocument();
  });
});
