// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// Conversations kept in the browser: each thread is the questions asked
// in a row, their answers and the sheets each answer cited. They are what
// « déjà demandé » reads: an earlier question that cited the same sheets.

import type { Concept } from "../api";

export interface ThreadTurn {
  question: string;
  answer: string;
  cited: number[];
  /** The retrieved sheets, trimmed to what a chip needs. */
  concepts: Pick<Concept, "id" | "concept_type" | "name">[];
}

export interface Thread {
  id: string;
  started: number;
  turns: ThreadTurn[];
}

const KEY = "conversations.v1";
const MAX_THREADS = 50;

export function loadThreads(): Thread[] {
  try {
    const v = JSON.parse(window.localStorage.getItem(KEY) ?? "[]");
    return Array.isArray(v) ? v.filter((t) => t && typeof t.id === "string" && Array.isArray(t.turns)) : [];
  } catch {
    return [];
  }
}

/** Insert or replace `thread`, newest first, oldest dropped past the cap. */
export function saveThread(thread: Thread): void {
  if (thread.turns.length === 0) return;
  const rest = loadThreads().filter((t) => t.id !== thread.id);
  const next = [thread, ...rest].slice(0, MAX_THREADS);
  try {
    window.localStorage.setItem(KEY, JSON.stringify(next));
  } catch {
    /* private mode: the thread lasts for the page */
  }
}

export function newThreadId(): string {
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

/** Turns of other threads that cited any of `cited`, newest thread first. */
export function relatedTurns(cited: number[], exceptThread: string, limit = 3): Array<{ thread: Thread; turn: ThreadTurn }> {
  if (cited.length === 0) return [];
  const wanted = new Set(cited);
  const out: Array<{ thread: Thread; turn: ThreadTurn }> = [];
  for (const thread of loadThreads()) {
    if (thread.id === exceptThread) continue;
    for (const turn of thread.turns) {
      if (turn.cited.some((id) => wanted.has(id))) {
        out.push({ thread, turn });
        if (out.length >= limit) return out;
      }
    }
  }
  return out;
}
