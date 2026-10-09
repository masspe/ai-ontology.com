// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// Two languages, no dependency. The French sentence is the key:
// `t('Aucun fichier.')` returns it as is in French and looks it up in the
// English dictionary (src/i18n/en) otherwise; a missing entry falls back to
// French, visibly, and src/lib/i18n.test.ts fails on it. Placeholders are
// `{name}` in both languages: `t('{n} fiches', { n: 3 })`.
//
// The language is the one stored under `lang`, else the browser's (French
// stays French, everything else is English). Switching reloads the page:
// no subscription machinery, every component reads `t()` at render time.

import { en } from "../i18n/en";

export type Lang = "fr" | "en";

const STORAGE_KEY = "lang";
const DICT: Record<Lang, Record<string, string>> = { fr: {}, en };

function detect(): Lang {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored === "fr" || stored === "en") return stored;
  } catch {
    /* storage unavailable: fall through to the browser language */
  }
  const browser = typeof navigator !== "undefined" ? navigator.language : "fr";
  return browser.toLowerCase().startsWith("fr") ? "fr" : "en";
}

let current: Lang | null = null;

function apply(lang: Lang): void {
  current = lang;
  if (typeof document !== "undefined") document.documentElement.lang = lang;
}

/** The active language, detected once and cached until `setLang`. */
export function getLang(): Lang {
  if (current === null) apply(detect());
  return current as Lang;
}

/** Store the choice; reload so every page renders in the new language. */
export function setLang(lang: Lang, reload = true): void {
  try {
    localStorage.setItem(STORAGE_KEY, lang);
  } catch {
    /* the choice lasts for this page only */
  }
  apply(lang);
  if (reload && typeof location !== "undefined") location.reload();
}

/** Translate a French sentence, substituting `{name}` placeholders. */
export function t(fr: string, vars?: Record<string, string | number>): string {
  const lang = getLang();
  const s = lang === "fr" ? fr : (DICT.en[fr] ?? fr);
  return vars ? s.replace(/\{(\w+)\}/g, (m, k: string) => (k in vars ? String(vars[k]) : m)) : s;
}
