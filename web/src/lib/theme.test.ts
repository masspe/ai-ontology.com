// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { applyTheme, cssVar, effectiveTheme, getTheme, setTheme, toggleTheme } from "./theme";

beforeEach(() => {
  window.localStorage.clear();
  document.documentElement.removeAttribute("data-theme");
});
afterEach(() => vi.restoreAllMocks());

describe("theme", () => {
  it("follows the system until chosen, then stamps and keeps the choice", () => {
    expect(getTheme()).toBe("system");
    vi.stubGlobal("matchMedia", (q: string) => ({ matches: q.includes("dark") }));
    expect(effectiveTheme()).toBe("dark");
    applyTheme();
    expect(document.documentElement.hasAttribute("data-theme")).toBe(false);

    setTheme("light");
    expect(getTheme()).toBe("light");
    expect(document.documentElement.getAttribute("data-theme")).toBe("light");
    expect(effectiveTheme()).toBe("light");
    expect(window.localStorage.getItem("theme.v1")).toBe("light");

    expect(toggleTheme()).toBe("dark");
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
    expect(toggleTheme()).toBe("light");

    setTheme("system");
    expect(window.localStorage.getItem("theme.v1")).toBeNull();
    expect(document.documentElement.hasAttribute("data-theme")).toBe(false);
  });

  it("ignores garbage in storage and reads a token with a fallback", () => {
    window.localStorage.setItem("theme.v1", "sepia");
    expect(getTheme()).toBe("system");
    expect(cssVar("--surely-not-defined", "#123456")).toBe("#123456");
    document.documentElement.style.setProperty("--probe", "#abcdef");
    expect(cssVar("--probe", "#000000")).toBe("#abcdef");
  });
});
