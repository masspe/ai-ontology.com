// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// Light or dark, the reader's choice: "system" follows the OS, an explicit
// choice is stamped on <html data-theme> and kept in the browser.

export type Theme = "system" | "light" | "dark";

const KEY = "theme.v1";

export function getTheme(): Theme {
  try {
    const v = window.localStorage.getItem(KEY);
    return v === "light" || v === "dark" ? v : "system";
  } catch {
    return "system";
  }
}

/** What is on screen now, after the system preference when not chosen. */
export function effectiveTheme(): "light" | "dark" {
  const t = getTheme();
  if (t !== "system") return t;
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

export function applyTheme(t: Theme = getTheme()): void {
  const root = document.documentElement;
  if (t === "system") root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", t);
}

export function setTheme(t: Theme): void {
  try {
    if (t === "system") window.localStorage.removeItem(KEY);
    else window.localStorage.setItem(KEY, t);
  } catch {
    /* private mode: lasts for the page */
  }
  applyTheme(t);
}

/** The next choice on the toggle: what is shown, inverted. */
export function toggleTheme(): Theme {
  const next: Theme = effectiveTheme() === "dark" ? "light" : "dark";
  setTheme(next);
  return next;
}

/** A design token's current value, for the few places that need a colour as a string (SVG, canvas). */
export function cssVar(name: string, fallback: string): string {
  try {
    const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
    return v || fallback;
  } catch {
    return fallback;
  }
}
