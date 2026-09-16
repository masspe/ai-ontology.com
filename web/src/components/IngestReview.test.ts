// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { describe, expect, it } from "vitest";
import { defaultDecisionFor } from "./IngestReview";

describe("defaultDecisionFor", () => {
  it("proposes create_new when there is no conflict (undefined or null)", () => {
    expect(defaultDecisionFor()).toBe("create_new");
    expect(defaultDecisionFor(undefined)).toBe("create_new");
    expect(defaultDecisionFor(null)).toBe("create_new");
  });

  it("proposes merge when an identical item already exists", () => {
    expect(
      defaultDecisionFor({
        kind: { kind: "exists", existing_id: "42", existing_display: "Person:Alice" },
        summary: "already there",
      }),
    ).toBe("merge");
  });

  it("proposes create_new on a type mismatch with the existing item", () => {
    expect(
      defaultDecisionFor({
        kind: { kind: "type_mismatch", existing_type: "Company", existing_id: "7" },
        summary: "type differs",
      }),
    ).toBe("create_new");
  });

  it("proposes skip when the item points at a missing reference", () => {
    expect(
      defaultDecisionFor({ kind: { kind: "dangling_ref", missing_ref: "c9" }, summary: "missing" }),
    ).toBe("skip");
  });
});
