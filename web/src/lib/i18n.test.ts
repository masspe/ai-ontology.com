// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.

import { describe, expect, it, vi } from "vitest";
import { en } from "../i18n/en";
import { getLang, setLang, t } from "./i18n";

describe("t", () => {
  it("returns the French key in French and substitutes placeholders", () => {
    setLang("fr", false);
    expect(t("Aucun fichier.")).toBe("Aucun fichier.");
    expect(t("{n} fiches et {m} liens", { n: "1,234", m: 5 })).toBe("1,234 fiches et 5 liens");
    expect(t("{n} fiches", {})).toBe("{n} fiches");
    expect(document.documentElement.lang).toBe("fr");
  });

  it("looks the sentence up in the English dictionary and falls back to French", () => {
    setLang("en", false);
    expect(t("Accueil")).toBe(en["Accueil"] ?? "Accueil");
    expect(en["Accueil"]).toBeDefined();
    expect(t("phrase sans traduction {x}", { x: 1 })).toBe("phrase sans traduction 1");
    expect(document.documentElement.lang).toBe("en");
  });

  it("detects the language from the stored choice, else from the browser", () => {
    setLang("en", false);
    expect(getLang()).toBe("en");
    expect(localStorage.getItem("lang")).toBe("en");
    setLang("fr", false);
    expect(getLang()).toBe("fr");
  });

  it("reloads the page when the choice changes", () => {
    const reload = vi.fn();
    vi.stubGlobal("location", { ...window.location, reload });
    setLang("en");
    expect(reload).toHaveBeenCalledOnce();
    setLang("fr", false);
  });
});

/** Every `t("…")` key in the sources, by file. Keys are plain double-quoted
 * strings on one line: that is the convention that keeps this check exact.
 * The sources are read as text through Vite's glob import. */
const SOURCES = import.meta.glob<string>("../**/*.{ts,tsx,js,jsx}", { query: "?raw", import: "default", eager: true });

function keysInSources(): Map<string, string[]> {
  const out = new Map<string, string[]>();
  for (const [path, src] of Object.entries(SOURCES)) {
    if (/\/(i18n|test)\//.test(path) || /\.test\.[jt]sx?$/.test(path)) continue;
    for (const m of src.matchAll(/\bt\(\s*"((?:[^"\\]|\\.)*)"/g)) {
      const key = JSON.parse(`"${m[1]}"`) as string;
      const files = out.get(key) ?? [];
      if (!files.includes(path)) files.push(path);
      out.set(key, files);
    }
  }
  return out;
}

describe("English dictionary", () => {
  it("has an entry for every t() key of the sources, and no entry without a key", () => {
    const keys = keysInSources();
    const missing = [...keys.entries()].filter(([k]) => !(k in en)).map(([k, f]) => `${k}  (${f.join(", ")})`);
    expect(missing, "French sentences without an English entry").toEqual([]);
    const unused = Object.keys(en).filter((k) => !keys.has(k));
    expect(unused, "English entries whose French key is no longer used").toEqual([]);
    expect(keys.size).toBeGreaterThan(0);
  });

  it("keeps the placeholders of each key in its translation", () => {
    const broken = Object.entries(en).filter(([fr, e]) => {
      const a = [...fr.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
      const b = [...e.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
      return a.join() !== b.join();
    });
    expect(broken).toEqual([]);
  });
});
