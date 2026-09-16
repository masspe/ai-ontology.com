// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { describe, expect, it } from "vitest";
import { iterRefs, type OntologyProposal } from "./proposalTypes";

describe("iterRefs", () => {
  it("returns an empty list for an empty proposal", () => {
    const p: OntologyProposal = {
      concept_types: [],
      relation_types: [],
      concepts: [],
      relations: [],
      rules: [],
      actions: [],
    };
    expect(iterRefs(p)).toEqual([]);
  });

  it("lists every client_ref by family in declaration order (concept types, relation types, concepts, relations, rules, actions)", () => {
    const p: OntologyProposal = {
      concept_types: [
        { client_ref: "ct2", name: "B" },
        { client_ref: "ct1", name: "A" },
      ],
      relation_types: [{ client_ref: "rt1", name: "r", domain: "A", range: "B" }],
      concepts: [{ client_ref: "c1", concept_type: "A", name: "a" }],
      relations: [{ client_ref: "r1", relation_type: "r", source_ref: "c1", target_ref: "c1" }],
      rules: [{ client_ref: "ru1", rule_type: "t", name: "n" }],
      actions: [{ client_ref: "a1", action_type: "t", name: "n", subject_ref: "c1" }],
    };
    expect(iterRefs(p)).toEqual(["ct2", "ct1", "rt1", "c1", "r1", "ru1", "a1"]);
  });

  it("does not dedupe: a ref present in two families is returned twice", () => {
    const p: OntologyProposal = {
      concept_types: [{ client_ref: "dup", name: "A" }],
      relation_types: [],
      concepts: [{ client_ref: "dup", concept_type: "A", name: "a" }],
      relations: [],
      rules: [],
      actions: [],
    };
    expect(iterRefs(p)).toEqual(["dup", "dup"]);
  });
});
