// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

type Level = "log" | "info" | "warn" | "error" | "debug";
const LEVELS: Level[] = ["log", "info", "warn", "error", "debug"];

// The module keeps an `installed` flag and the ring buffer as module state,
// and `installLogCapture` monkey-patches `console`. Each test therefore gets
// a fresh module instance and the original console methods back afterwards.
const originals = Object.fromEntries(LEVELS.map((l) => [l, console[l]])) as Record<Level, (...a: unknown[]) => void>;

async function load() {
  vi.resetModules();
  return await import("./logBuffer");
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-01-02T03:04:05.000Z"));
  // Silence the pass-through to the real console during these tests.
  for (const l of LEVELS) console[l] = vi.fn();
});

afterEach(() => {
  for (const l of LEVELS) console[l] = originals[l];
});

describe("getLogTail / clearLogs", () => {
  it("is empty before anything was captured", async () => {
    const { getLogTail } = await load();
    expect(getLogTail()).toBe("");
  });

  it("formats each entry as '<ISO timestamp> [<level>] <message>' on its own line", async () => {
    const { installLogCapture, getLogTail } = await load();
    installLogCapture();
    console.log("hello", "world");
    vi.setSystemTime(new Date("2026-01-02T03:04:06.000Z"));
    console.warn("careful");
    expect(getLogTail()).toBe(
      "2026-01-02T03:04:05.000Z [log] hello world\n2026-01-02T03:04:06.000Z [warn] careful",
    );
  });

  it("serialises objects as JSON, Errors as name/message/stack, and strings verbatim", async () => {
    const { installLogCapture, getLogTail } = await load();
    installLogCapture();
    const err = new Error("boom");
    err.stack = "Error: boom\n    at here";
    console.error("failed:", { a: 1 }, err);
    expect(getLogTail()).toBe('2026-01-02T03:04:05.000Z [error] failed: {"a":1} Error: boom\nError: boom\n    at here');
  });

  it("falls back to String() for values JSON.stringify cannot handle", async () => {
    const { installLogCapture, getLogTail } = await load();
    installLogCapture();
    console.log(10n);
    expect(getLogTail()).toBe("2026-01-02T03:04:05.000Z [log] 10");
  });

  it("returns only the last `limit` entries (default 200)", async () => {
    const { installLogCapture, getLogTail } = await load();
    installLogCapture();
    for (let i = 0; i < 250; i++) console.info(`m${i}`);
    const lines = getLogTail().split("\n");
    expect(lines).toHaveLength(200);
    expect(lines[0]).toMatch(/\[info\] m50$/);
    expect(lines[199]).toMatch(/\[info\] m249$/);
    expect(getLogTail(2).split("\n")).toEqual([
      "2026-01-02T03:04:05.000Z [info] m248",
      "2026-01-02T03:04:05.000Z [info] m249",
    ]);
  });

  it("keeps at most 500 entries, dropping the oldest", async () => {
    const { installLogCapture, getLogTail } = await load();
    installLogCapture();
    for (let i = 0; i < 600; i++) console.debug(`d${i}`);
    const lines = getLogTail(1000).split("\n");
    expect(lines).toHaveLength(500);
    expect(lines[0]).toMatch(/\[debug\] d100$/);
    expect(lines[499]).toMatch(/\[debug\] d599$/);
  });

  it("clearLogs empties the buffer", async () => {
    const { installLogCapture, getLogTail, clearLogs } = await load();
    installLogCapture();
    console.log("x");
    clearLogs();
    expect(getLogTail()).toBe("");
  });
});

describe("installLogCapture", () => {
  it("wraps all five console levels and still forwards the call to the original method", async () => {
    const { installLogCapture, getLogTail } = await load();
    const spies = Object.fromEntries(LEVELS.map((l) => [l, console[l] as ReturnType<typeof vi.fn>])) as Record<Level, ReturnType<typeof vi.fn>>;
    installLogCapture();
    for (const l of LEVELS) console[l](`via ${l}`, 1);
    for (const l of LEVELS) expect(spies[l]).toHaveBeenCalledExactlyOnceWith(`via ${l}`, 1);
    expect(getLogTail().split("\n").map((s) => s.replace(/^\S+ /, ""))).toEqual(
      LEVELS.map((l) => `[${l}] via ${l} 1`),
    );
  });

  it("installs only once: a second call does not double-wrap console", async () => {
    const { installLogCapture, getLogTail } = await load();
    installLogCapture();
    const wrapped = console.log;
    installLogCapture();
    expect(console.log).toBe(wrapped);
    console.log("once");
    expect(getLogTail().split("\n")).toHaveLength(1);
  });

  it("records window 'error' events as error entries", async () => {
    const { installLogCapture, getLogTail } = await load();
    installLogCapture();
    window.dispatchEvent(new ErrorEvent("error", { message: "kaboom", error: new Error("kaboom") }));
    const tail = getLogTail();
    expect(tail).toContain("[error] window.error: kaboom");
  });

  it("records 'unhandledrejection' events with their reason", async () => {
    const { installLogCapture, getLogTail } = await load();
    installLogCapture();
    const ev = new Event("unhandledrejection") as Event & { reason?: unknown };
    ev.reason = { why: "no" };
    window.dispatchEvent(ev);
    expect(getLogTail()).toBe('2026-01-02T03:04:05.000Z [error] unhandledrejection: {"why":"no"}');
  });
});
