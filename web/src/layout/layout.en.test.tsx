// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The navigation in English: labels, titles and guide sentences are read at
// render time, in the active language.

import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import TopNav from "./TopNav";
import { NAV_GROUPS, NAV_ITEMS } from "./nav";
import { setLang } from "../lib/i18n";

describe("navigation in English", () => {
  it("shows the four entries, the items and the guide sentences in English", () => {
    setLang("en", false);
    render(
      <MemoryRouter>
        <TopNav />
      </MemoryRouter>,
    );
    expect(screen.getByRole("navigation", { name: "Navigation" })).toBeInTheDocument();
    expect(NAV_GROUPS.map((g) => g.title)).toEqual(["Home", "Import", "Explore", "Settings"]);
    expect(screen.getByRole("link", { name: /Home/ })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Settings/ })).toBeInTheDocument();
    expect(NAV_ITEMS.map((i) => i.label)).toContain("Data model");
    expect(NAV_ITEMS.find((i) => i.to === "/rules")?.guide).toMatch(/^Consistency rules/);
  });
});
