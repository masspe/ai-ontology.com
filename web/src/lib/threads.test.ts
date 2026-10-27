// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.

import { beforeEach, describe, expect, it } from "vitest";
import { loadThreads, newThreadId, relatedTurns, saveThread, type Thread } from "./threads";

const thread = (id: string, cited: number[]): Thread => ({
  id,
  started: 1,
  turns: [{ question: `q-${id}`, answer: "a", cited, concepts: [] }],
});

beforeEach(() => window.localStorage.clear());

describe("threads", () => {
  it("saves newest first, replaces by id, drops the oldest past fifty, ignores empty and garbage", () => {
    saveThread(thread("a", [1]));
    saveThread(thread("b", [2]));
    saveThread({ ...thread("a", [1, 3]) });
    expect(loadThreads().map((t) => t.id)).toEqual(["a", "b"]);
    expect(loadThreads()[0]!.turns[0]!.cited).toEqual([1, 3]);
    saveThread({ id: "empty", started: 1, turns: [] });
    expect(loadThreads()).toHaveLength(2);
    for (let i = 0; i < 60; i++) saveThread(thread(`t${i}`, []));
    expect(loadThreads()).toHaveLength(50);
    expect(loadThreads()[0]!.id).toBe("t59");
    window.localStorage.setItem("conversations.v1", "{not json");
    expect(loadThreads()).toEqual([]);
    window.localStorage.setItem("conversations.v1", JSON.stringify([{ nope: 1 }, thread("ok", [])]));
    expect(loadThreads().map((t) => t.id)).toEqual(["ok"]);
  });

  it("finds the turns of other threads that cited the same sheets", () => {
    saveThread(thread("a", [1]));
    saveThread(thread("b", [2, 3]));
    saveThread(thread("c", [3]));
    expect(relatedTurns([3], "c").map((r) => r.thread.id)).toEqual(["b"]);
    expect(relatedTurns([1, 3], "none").map((r) => r.thread.id)).toEqual(["c", "b", "a"]);
    expect(relatedTurns([1, 3], "none", 1)).toHaveLength(1);
    expect(relatedTurns([], "none")).toEqual([]);
    expect(relatedTurns([9], "none")).toEqual([]);
  });

  it("makes distinct ids that start with the time", () => {
    const a = newThreadId();
    const b = newThreadId();
    expect(a).not.toBe(b);
    expect(parseInt(a.split("-")[0]!, 36)).toBeGreaterThan(0);
  });
});
