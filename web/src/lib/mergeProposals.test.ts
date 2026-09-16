// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { describe, expect, it } from "vitest";
import { mergeProposals, type ProposalPart } from "./mergeProposals";
import { iterRefs, type OntologyProposal } from "./proposalTypes";

/** Empty proposal with optional overrides — keeps fixtures short. */
function proposal(over: Partial<OntologyProposal> = {}): OntologyProposal {
  return {
    concept_types: [],
    relation_types: [],
    concepts: [],
    relations: [],
    rules: [],
    actions: [],
    ...over,
  };
}

function part(file: string, over: Partial<OntologyProposal> = {}): ProposalPart {
  return { file, proposal: proposal(over) };
}

describe("mergeProposals — empty inputs", () => {
  it("returns an empty proposal with null language for zero parts", () => {
    const out = mergeProposals([]);
    expect(out.concept_types).toEqual([]);
    expect(out.relation_types).toEqual([]);
    expect(out.concepts).toEqual([]);
    expect(out.relations).toEqual([]);
    expect(out.rules).toEqual([]);
    expect(out.actions).toEqual([]);
    expect(out.language).toBeNull();
    expect(out.source?.encoding).toBeUndefined();
  });

  it("labels the source of zero parts as '0 files ()' (current behaviour, documented)", () => {
    expect(mergeProposals([]).source?.name).toBe("0 files ()");
  });

  it("merges a single empty proposal into an empty proposal named after its file", () => {
    const out = mergeProposals([part("a.txt")]);
    expect(iterRefs(out)).toEqual([]);
    expect(out.source?.name).toBe("a.txt");
  });
});

describe("mergeProposals — single part", () => {
  const single = part("notes.md", {
    source: { name: "notes.md", encoding: "utf-8" },
    language: { code: "fr", script: "Latn", confidence: 0.9 },
    concept_types: [{ client_ref: "ct1", name: "Person" }],
    relation_types: [{ client_ref: "rt1", name: "knows", domain: "Person", range: "Person" }],
    concepts: [
      { client_ref: "c1", concept_type: "Person", name: "Alice" },
      { client_ref: "c2", concept_type: "Person", name: "Bob" },
    ],
    relations: [{ client_ref: "r1", relation_type: "knows", source_ref: "c1", target_ref: "c2" }],
    rules: [{ client_ref: "ru1", rule_type: "constraint", name: "unique-name", applies_to: ["ct1", "c1"] }],
    actions: [
      { client_ref: "a1", action_type: "notify", name: "ping", subject_ref: "c1", object_ref: "c2" },
      { client_ref: "a2", action_type: "notify", name: "solo", subject_ref: "c1", object_ref: null },
      { client_ref: "a3", action_type: "notify", name: "bare", subject_ref: "c1" },
    ],
  });

  it("prefixes every client_ref with f0/", () => {
    const out = mergeProposals([single]);
    expect(iterRefs(out)).toEqual([
      "f0/ct1",
      "f0/rt1",
      "f0/c1",
      "f0/c2",
      "f0/r1",
      "f0/ru1",
      "f0/a1",
      "f0/a2",
      "f0/a3",
    ]);
  });

  it("rewrites relation source_ref/target_ref to the prefixed refs", () => {
    const [r] = mergeProposals([single]).relations;
    expect(r.source_ref).toBe("f0/c1");
    expect(r.target_ref).toBe("f0/c2");
  });

  it("rewrites rule applies_to entries to the prefixed refs", () => {
    const [ru] = mergeProposals([single]).rules;
    expect(ru.applies_to).toEqual(["f0/ct1", "f0/c1"]);
  });

  it("rewrites action subject_ref and object_ref, keeping null/undefined object_ref as-is", () => {
    const [a1, a2, a3] = mergeProposals([single]).actions;
    expect(a1.subject_ref).toBe("f0/c1");
    expect(a1.object_ref).toBe("f0/c2");
    expect(a2.object_ref).toBeNull();
    expect(a3.object_ref).toBeUndefined();
  });

  it("keeps a rule without applies_to undefined", () => {
    const out = mergeProposals([part("x", { rules: [{ client_ref: "r", rule_type: "t", name: "n" }] })]);
    expect(out.rules[0].applies_to).toBeUndefined();
  });

  it("uses the file name as source.name and passes encoding and language through", () => {
    const out = mergeProposals([single]);
    expect(out.source).toEqual({ name: "notes.md", encoding: "utf-8" });
    expect(out.language).toEqual({ code: "fr", script: "Latn", confidence: 0.9 });
  });

  it("preserves every other field of each item", () => {
    const out = mergeProposals([
      part("x", {
        concepts: [
          {
            client_ref: "c1",
            concept_type: "Person",
            name: "Alice",
            description: "d",
            properties: [["age", "30"]],
            evidence: "e",
            confidence: 0.5,
            conflict: null,
          },
        ],
      }),
    ]);
    expect(out.concepts[0]).toEqual({
      client_ref: "f0/c1",
      concept_type: "Person",
      name: "Alice",
      description: "d",
      properties: [["age", "30"]],
      evidence: "e",
      confidence: 0.5,
      conflict: null,
    });
  });

  it("does not mutate the input proposals", () => {
    const parts = [single];
    const before = JSON.stringify(parts);
    mergeProposals(parts);
    expect(JSON.stringify(parts)).toBe(before);
  });
});

describe("mergeProposals — graph references (Type:Name)", () => {
  it("leaves Type:Name references untouched in relations, rules and actions", () => {
    const out = mergeProposals([
      part("x", {
        concepts: [{ client_ref: "c1", concept_type: "Person", name: "Alice" }],
        relations: [{ client_ref: "r1", relation_type: "knows", source_ref: "c1", target_ref: "Person:Bob" }],
        rules: [{ client_ref: "ru1", rule_type: "t", name: "n", applies_to: ["Person:Bob"] }],
        actions: [
          { client_ref: "a1", action_type: "t", name: "n", subject_ref: "Person:Bob", object_ref: "Person:Carol" },
        ],
      }),
    ]);
    expect(out.relations[0].source_ref).toBe("f0/c1");
    expect(out.relations[0].target_ref).toBe("Person:Bob");
    expect(out.rules[0].applies_to).toEqual(["Person:Bob"]);
    expect(out.actions[0].subject_ref).toBe("Person:Bob");
    expect(out.actions[0].object_ref).toBe("Person:Carol");
  });

  it("dedupes two files' relations that both target the same Type:Name graph ref", () => {
    const out = mergeProposals([
      part("a", {
        concepts: [{ client_ref: "c1", concept_type: "Person", name: "Alice" }],
        relations: [{ client_ref: "r1", relation_type: "knows", source_ref: "c1", target_ref: "Person:Bob" }],
      }),
      part("b", {
        concepts: [{ client_ref: "c1", concept_type: "Person", name: "Alice" }],
        relations: [{ client_ref: "r1", relation_type: "knows", source_ref: "c1", target_ref: "Person:Bob" }],
      }),
    ]);
    expect(out.relations).toHaveLength(1);
    expect(out.relations[0].client_ref).toBe("f0/r1");
  });
});

describe("mergeProposals — deduplication across files", () => {
  it("dedupes concept types by exact name, keeping the first occurrence's data", () => {
    const out = mergeProposals([
      part("a", { concept_types: [{ client_ref: "ct1", name: "Person", description: "from a" }] }),
      part("b", { concept_types: [{ client_ref: "ct9", name: "Person", description: "from b" }] }),
    ]);
    expect(out.concept_types).toEqual([{ client_ref: "f0/ct1", name: "Person", description: "from a" }]);
  });

  it("dedupes relation types by exact name", () => {
    const out = mergeProposals([
      part("a", { relation_types: [{ client_ref: "rt1", name: "knows", domain: "Person", range: "Person" }] }),
      part("b", { relation_types: [{ client_ref: "rt1", name: "knows", domain: "Person", range: "Person" }] }),
      part("c", { relation_types: [{ client_ref: "rt1", name: "owns", domain: "Person", range: "Thing" }] }),
    ]);
    expect(out.relation_types.map((r) => r.client_ref)).toEqual(["f0/rt1", "f2/rt1"]);
  });

  it("dedupes concepts by type + case-insensitive trimmed name", () => {
    const out = mergeProposals([
      part("a", { concepts: [{ client_ref: "c1", concept_type: "Person", name: "Alice" }] }),
      part("b", { concepts: [{ client_ref: "c1", concept_type: "Person", name: "  alice " }] }),
      part("c", { concepts: [{ client_ref: "c1", concept_type: "Person", name: "ALICE" }] }),
    ]);
    expect(out.concepts).toHaveLength(1);
    expect(out.concepts[0]).toMatchObject({ client_ref: "f0/c1", name: "Alice" });
  });

  it("keeps concepts with the same name but different types", () => {
    const out = mergeProposals([
      part("a", { concepts: [{ client_ref: "c1", concept_type: "Person", name: "Mercury" }] }),
      part("b", { concepts: [{ client_ref: "c1", concept_type: "Planet", name: "Mercury" }] }),
    ]);
    expect(out.concepts.map((c) => c.client_ref)).toEqual(["f0/c1", "f1/c1"]);
  });

  it("remaps a later file's relation onto the canonical concept refs of an earlier file", () => {
    const out = mergeProposals([
      part("a", {
        concepts: [
          { client_ref: "c1", concept_type: "Person", name: "Alice" },
          { client_ref: "c2", concept_type: "Person", name: "Bob" },
        ],
      }),
      part("b", {
        concepts: [
          { client_ref: "x", concept_type: "Person", name: "alice" },
          { client_ref: "y", concept_type: "Person", name: "Carol" },
        ],
        relations: [{ client_ref: "r1", relation_type: "knows", source_ref: "x", target_ref: "y" }],
      }),
    ]);
    expect(out.concepts.map((c) => c.client_ref)).toEqual(["f0/c1", "f0/c2", "f1/y"]);
    expect(out.relations).toEqual([
      { client_ref: "f1/r1", relation_type: "knows", source_ref: "f0/c1", target_ref: "f1/y" },
    ]);
  });

  it("dedupes relations by type + canonical source + canonical target", () => {
    const rel = { client_ref: "r1", relation_type: "knows", source_ref: "c1", target_ref: "c2" };
    const concepts = [
      { client_ref: "c1", concept_type: "Person", name: "Alice" },
      { client_ref: "c2", concept_type: "Person", name: "Bob" },
    ];
    const out = mergeProposals([
      part("a", { concepts, relations: [rel] }),
      part("b", { concepts, relations: [rel] }),
    ]);
    expect(out.relations).toHaveLength(1);
    expect(out.relations[0]).toMatchObject({ client_ref: "f0/r1", source_ref: "f0/c1", target_ref: "f0/c2" });
  });

  it("keeps relations that share a type but differ in direction", () => {
    const concepts = [
      { client_ref: "c1", concept_type: "Person", name: "Alice" },
      { client_ref: "c2", concept_type: "Person", name: "Bob" },
    ];
    const out = mergeProposals([
      part("a", {
        concepts,
        relations: [{ client_ref: "r1", relation_type: "knows", source_ref: "c1", target_ref: "c2" }],
      }),
      part("b", {
        concepts,
        relations: [{ client_ref: "r1", relation_type: "knows", source_ref: "c2", target_ref: "c1" }],
      }),
    ]);
    expect(out.relations.map((r) => [r.client_ref, r.source_ref, r.target_ref])).toEqual([
      ["f0/r1", "f0/c1", "f0/c2"],
      ["f1/r1", "f0/c2", "f0/c1"],
    ]);
  });

  it("dedupes rules by rule_type + name and canonicalizes applies_to of the kept rules", () => {
    const out = mergeProposals([
      part("a", {
        concept_types: [{ client_ref: "ct1", name: "Person" }],
        rules: [{ client_ref: "ru1", rule_type: "constraint", name: "n", applies_to: ["ct1"] }],
      }),
      part("b", {
        concept_types: [{ client_ref: "T", name: "Person" }],
        rules: [
          { client_ref: "ru1", rule_type: "constraint", name: "n", applies_to: ["T"] },
          { client_ref: "ru2", rule_type: "inference", name: "n", applies_to: ["T"] },
        ],
      }),
    ]);
    expect(out.rules.map((r) => [r.client_ref, r.applies_to])).toEqual([
      ["f0/ru1", ["f0/ct1"]],
      ["f1/ru2", ["f0/ct1"]],
    ]);
  });

  it("dedupes actions by action_type + name and canonicalizes subject/object refs", () => {
    const out = mergeProposals([
      part("a", {
        concepts: [{ client_ref: "c1", concept_type: "Person", name: "Alice" }],
        actions: [{ client_ref: "a1", action_type: "notify", name: "ping", subject_ref: "c1" }],
      }),
      part("b", {
        concepts: [
          { client_ref: "p", concept_type: "Person", name: "alice" },
          { client_ref: "q", concept_type: "Person", name: "Bob" },
        ],
        actions: [
          { client_ref: "a1", action_type: "notify", name: "ping", subject_ref: "p" },
          { client_ref: "a2", action_type: "notify", name: "pong", subject_ref: "q", object_ref: "p" },
        ],
      }),
    ]);
    expect(out.actions).toEqual([
      { client_ref: "f0/a1", action_type: "notify", name: "ping", subject_ref: "f0/c1" },
      { client_ref: "f1/a2", action_type: "notify", name: "pong", subject_ref: "f1/q", object_ref: "f0/c1" },
    ]);
  });

  it("resolves a chain of canonical refs (file 2 → file 1 → file 0) to the file 0 ref", () => {
    const alice = (ref: string, name: string) => ({ client_ref: ref, concept_type: "Person", name });
    const out = mergeProposals([
      part("a", { concepts: [alice("c1", "Alice")] }),
      part("b", { concepts: [alice("c1", "alice")] }),
      part("c", {
        concepts: [alice("c1", "ALICE"), alice("c2", "Zed")],
        relations: [{ client_ref: "r", relation_type: "knows", source_ref: "c1", target_ref: "c2" }],
      }),
    ]);
    expect(out.relations[0].source_ref).toBe("f0/c1");
    expect(out.relations[0].target_ref).toBe("f2/c2");
  });
});

describe("mergeProposals — client_ref uniqueness", () => {
  it("produces globally unique client_refs when every file reuses the same local refs", () => {
    const mk = (n: string) =>
      part(n, {
        concept_types: [{ client_ref: "ct1", name: `Type-${n}` }],
        relation_types: [{ client_ref: "rt1", name: `rel-${n}`, domain: "x", range: "y" }],
        concepts: [{ client_ref: "c1", concept_type: "T", name: `Concept-${n}` }],
        relations: [{ client_ref: "r1", relation_type: `rel-${n}`, source_ref: "c1", target_ref: "c1" }],
        rules: [{ client_ref: "ru1", rule_type: "t", name: `rule-${n}` }],
        actions: [{ client_ref: "a1", action_type: "t", name: `act-${n}`, subject_ref: "c1" }],
      });
    const out = mergeProposals([mk("a"), mk("b"), mk("c")]);
    const refs = iterRefs(out);
    expect(refs).toHaveLength(18);
    expect(new Set(refs).size).toBe(18);
    expect(refs.filter((r) => r.startsWith("f1/"))).toHaveLength(6);
  });
});

describe("mergeProposals — source and language summary", () => {
  it("names the source '<n> files (a, b)' for two or three files", () => {
    expect(mergeProposals([part("a.txt"), part("b.txt")]).source?.name).toBe("2 files (a.txt, b.txt)");
    expect(mergeProposals([part("a"), part("b"), part("c")]).source?.name).toBe("3 files (a, b, c)");
  });

  it("lists only the first three file names followed by an ellipsis beyond three files", () => {
    const out = mergeProposals([part("a"), part("b"), part("c"), part("d"), part("e")]);
    expect(out.source?.name).toBe("5 files (a, b, c, …)");
  });

  it("reports a single shared encoding, 'mixed' for several, undefined for none", () => {
    const enc = (e: string) => ({ source: { name: "x", encoding: e } });
    expect(mergeProposals([part("a", enc("utf-8")), part("b", enc("utf-8"))]).source?.encoding).toBe("utf-8");
    expect(mergeProposals([part("a", enc("utf-8")), part("b", enc("latin1"))]).source?.encoding).toBe("mixed");
    expect(mergeProposals([part("a"), part("b", { source: { name: "b" } })]).source?.encoding).toBeUndefined();
  });

  it("keeps the first non-null language even when earlier files have none", () => {
    const out = mergeProposals([
      part("a", { language: null }),
      part("b", { language: { code: "de", script: "Latn", confidence: 0.7 } }),
      part("c", { language: { code: "fr", script: "Latn", confidence: 0.99 } }),
    ]);
    expect(out.language?.code).toBe("de");
  });
});
