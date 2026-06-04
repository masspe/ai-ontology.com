// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
const CAPACITY = 500;
const buffer = [];
let installed = false;
function fmt(args) {
    return args
        .map((a) => {
        if (a instanceof Error)
            return `${a.name}: ${a.message}\n${a.stack ?? ""}`;
        if (typeof a === "string")
            return a;
        try {
            return JSON.stringify(a);
        }
        catch {
            return String(a);
        }
    })
        .join(" ");
}
function record(level, args) {
    buffer.push({ ts: Date.now(), level, msg: fmt(args) });
    if (buffer.length > CAPACITY)
        buffer.splice(0, buffer.length - CAPACITY);
}
export function installLogCapture() {
    if (installed || typeof window === "undefined")
        return;
    installed = true;
    const levels = ["log", "info", "warn", "error", "debug"];
    for (const lvl of levels) {
        const original = console[lvl];
        console[lvl] = (...args) => {
            record(lvl, args);
            original.apply(console, args);
        };
    }
    window.addEventListener("error", (e) => {
        record("error", [`window.error: ${e.message}`, e.error]);
    });
    window.addEventListener("unhandledrejection", (e) => {
        record("error", [`unhandledrejection:`, e.reason]);
    });
}
export function getLogTail(limit = 200) {
    const slice = buffer.slice(-limit);
    return slice
        .map((e) => `${new Date(e.ts).toISOString()} [${e.level}] ${e.msg}`)
        .join("\n");
}
export function clearLogs() {
    buffer.length = 0;
}
