// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// Questions pinned to the home page (ROADMAP §3.9 lot B, point 5): ids of
// saved queries, per browser.

const KEY = "queries.pinned.v1";

export function pinnedIds(): number[] {
  try {
    const v = JSON.parse(window.localStorage.getItem(KEY) ?? "[]");
    return Array.isArray(v) ? v.filter((x) => Number.isInteger(x)) : [];
  } catch {
    return [];
  }
}

export function togglePin(id: number): number[] {
  const cur = pinnedIds();
  const next = cur.includes(id) ? cur.filter((x) => x !== id) : [...cur, id];
  try {
    window.localStorage.setItem(KEY, JSON.stringify(next));
  } catch {
    /* private mode: the pin lasts for the page */
  }
  return next;
}
