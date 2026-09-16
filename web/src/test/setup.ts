// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Shared Vitest setup: runs before every test file. Guarantees that browser
// storage never leaks between tests and that no test can hit the network by
// accident — a real `fetch` call fails loudly instead of dialling out.

import { afterEach, beforeEach, vi } from "vitest";

beforeEach(() => {
  window.localStorage.clear();
  window.sessionStorage.clear();
  vi.stubGlobal(
    "fetch",
    vi.fn(() => Promise.reject(new Error("network access is disabled in unit tests"))),
  );
});

afterEach(() => {
  vi.useRealTimers();
});
