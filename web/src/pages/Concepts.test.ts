// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { describe, expect, it } from "vitest";
import type { Concept, ConceptTypeDef } from "../api";
import {
  buildHierarchy,
  conceptDomain,
  conceptIconColor,
  conceptStatus,
  conceptUpdatedAt,
  initials,
  truncate,
} from "./Concepts";

const concept = (over: Partial<Concept> = {}): Concept => ({
  id: 1,
  concept_type: "Person",
  name: "Alice",
  ...over,
});

describe("truncate", () => {
  it("returns strings up to the limit unchanged, including exactly the limit", () => {
    expect(truncate("", 3)).toBe("");
    expect(truncate("abc", 3)).toBe("abc");
    expect(truncate("ab", 3)).toBe("ab");
  });

  it("cuts longer strings to the limit and appends a single ellipsis character", () => {
    expect(truncate("abcdef", 3)).toBe("abc…");
    expect(truncate("abcdef", 3)).toHaveLength(4);
  });

  it("keeps a multi-megabyte blob bounded", () => {
    expect(truncate("x".repeat(2_000_000), 200)).toHaveLength(201);
  });
});

describe("conceptStatus", () => {
  it("maps the status property to a label and badge class, case-insensitively", () => {
    expect(conceptStatus(concept({ properties: { status: "reviewed" } }))).toEqual({ label: "Reviewed", cls: "badge-accent" });
    expect(conceptStatus(concept({ properties: { status: "DRAFT" } }))).toEqual({ label: "Draft", cls: "badge-warn" });
    expect(conceptStatus(concept({ properties: { status: "Archived" } }))).toEqual({ label: "Archived", cls: "badge-danger" });
  });

  it("defaults to Active when the status is missing or unknown", () => {
    expect(conceptStatus(concept())).toEqual({ label: "Active", cls: "badge-success" });
    expect(conceptStatus(concept({ properties: {} }))).toEqual({ label: "Active", cls: "badge-success" });
    expect(conceptStatus(concept({ properties: { status: "whatever" } }))).toEqual({ label: "Active", cls: "badge-success" });
  });
});

describe("conceptUpdatedAt", () => {
  it("returns a numeric updated_at as is (seconds)", () => {
    expect(conceptUpdatedAt(concept({ properties: { updated_at: 1_700_000_000 } }))).toBe(1_700_000_000);
  });

  it("parses an ISO string into whole seconds", () => {
    expect(conceptUpdatedAt(concept({ properties: { updated_at: "2024-01-02T03:04:05.678Z" } }))).toBe(1_704_164_645);
  });

  it("prefers updated_at over created_at, and falls back to created_at", () => {
    expect(conceptUpdatedAt(concept({ properties: { updated_at: 10, created_at: 5 } }))).toBe(10);
    expect(conceptUpdatedAt(concept({ properties: { created_at: 5 } }))).toBe(5);
  });

  it("returns null for an unparsable string, a non-date type, or no properties", () => {
    expect(conceptUpdatedAt(concept({ properties: { updated_at: "yesterday-ish" } }))).toBeNull();
    expect(conceptUpdatedAt(concept({ properties: { updated_at: true } }))).toBeNull();
    expect(conceptUpdatedAt(concept())).toBeNull();
  });
});

describe("conceptDomain", () => {
  const types: Record<string, ConceptTypeDef> = {
    Entity: { name: "Entity" },
    Agent: { name: "Agent", parent: "Entity" },
    Person: { name: "Person", parent: "Agent" },
    Loop1: { name: "Loop1", parent: "Loop2" },
    Loop2: { name: "Loop2", parent: "Loop1" },
    Orphan: { name: "Orphan", parent: "Missing" },
  };

  it("walks up the parent chain to the root type", () => {
    expect(conceptDomain(concept({ concept_type: "Person" }), types)).toBe("Entity");
    expect(conceptDomain(concept({ concept_type: "Agent" }), types)).toBe("Entity");
  });

  it("returns the type itself when it has no parent", () => {
    expect(conceptDomain(concept({ concept_type: "Entity" }), types)).toBe("Entity");
  });

  it("returns the type itself when it is unknown to the ontology", () => {
    expect(conceptDomain(concept({ concept_type: "Ghost" }), types)).toBe("Ghost");
  });

  it("returns the direct type when the parent chain loops", () => {
    expect(conceptDomain(concept({ concept_type: "Loop1" }), types)).toBe("Loop1");
  });

  it("stops at a parent that is not defined and returns that parent name", () => {
    // `Orphan` has parent `Missing`; `Missing` has no definition → treated as root.
    expect(conceptDomain(concept({ concept_type: "Orphan" }), types)).toBe("Missing");
  });
});

describe("conceptIconColor", () => {
  const palette = ["#2563eb", "#7c3aed", "#16a34a", "#d97706", "#dc2626", "#0ea5e9", "#db2777"];

  it("is deterministic and always picks from the palette", () => {
    for (const name of ["Alice", "Bob", "Zürich", "", "a very long concept name indeed"]) {
      const c = conceptIconColor(name);
      expect(palette).toContain(c);
      expect(conceptIconColor(name)).toBe(c);
    }
  });

  it("uses the first palette colour for the empty name and spreads other names over the palette", () => {
    expect(conceptIconColor("")).toBe("#2563eb");
    const distinct = new Set(["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"].map(conceptIconColor));
    expect(distinct.size).toBeGreaterThan(1);
  });
});

describe("initials", () => {
  it("takes the first letter of the first two words, upper-cased", () => {
    expect(initials("alice smith")).toBe("AS");
    expect(initials("Alice Marie Smith")).toBe("AM");
  });

  it("returns one letter for a single word", () => {
    expect(initials("alice")).toBe("A");
  });

  it("ignores leading/trailing whitespace and repeated spaces", () => {
    expect(initials("  alice   smith ")).toBe("AS");
  });

  it("returns the empty string for an empty name", () => {
    expect(initials("")).toBe("");
  });
});

describe("buildHierarchy", () => {
  it("returns no roots for an empty ontology", () => {
    expect(buildHierarchy({})).toEqual([]);
  });

  it("nests children under their parent and sorts roots and children alphabetically", () => {
    const types: Record<string, ConceptTypeDef> = {
      Zebra: { name: "Zebra" },
      Animal: { name: "Animal" },
      Dog: { name: "Dog", parent: "Animal" },
      Cat: { name: "Cat", parent: "Animal" },
      Puppy: { name: "Puppy", parent: "Dog" },
    };
    expect(buildHierarchy(types)).toEqual([
      {
        name: "Animal",
        children: [
          { name: "Cat", children: [] },
          { name: "Dog", children: [{ name: "Puppy", children: [] }] },
        ],
      },
      { name: "Zebra", children: [] },
    ]);
  });

  it("promotes a type whose parent is not defined to a root", () => {
    expect(buildHierarchy({ Orphan: { name: "Orphan", parent: "Missing" } })).toEqual([
      { name: "Orphan", children: [] },
    ]);
  });

  it("treats null and empty-string parents as roots", () => {
    const out = buildHierarchy({ A: { name: "A", parent: null }, B: { name: "B", parent: "" } });
    expect(out.map((n) => n.name)).toEqual(["A", "B"]);
  });
});
