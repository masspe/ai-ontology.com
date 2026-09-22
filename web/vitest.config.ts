// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Unit-test runner configuration (Vitest). Kept separate from
// `vite.config.ts` so the dev-server proxy table stays untouched. Tests live
// next to the code as `src/**/*.test.ts(x)`; they are type-checked by
// `tsc --noEmit` (tsconfig includes `src`) but never reach the production
// bundle, which `vite build` assembles from `index.html` only.

import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
    // Explicit `import { describe, it, expect, vi } from "vitest"` in every
    // test file — no ambient globals, so `tsc` needs no extra `types` entry.
    globals: false,
    // Every test starts from a clean slate: spies restored, every `vi.fn()`
    // reset to its factory implementation (call history AND the queued
    // `mockResolvedValueOnce` answers — a failing test must not leak its
    // primed answers into the next one; each file re-primes in
    // `beforeEach`), `vi.stubEnv` / `vi.stubGlobal` undone, localStorage
    // emptied (see setup file).
    restoreMocks: true,
    mockReset: true,
    unstubEnvs: true,
    unstubGlobals: true,
    // Dates are formatted with toLocale*: pin the zone so a CI runner or a
    // developer east of UTC+11 sees the same day as the fixtures.
    env: { TZ: "UTC" },
    setupFiles: ["src/test/setup.ts"],
    // The rendering tests drive 1 500-line pages through user-event in
    // jsdom: a scenario takes 2-4 s alone and several times that when every
    // core runs a worker. The timeout is headroom, never a wait (tests use
    // findBy/waitFor), and capping the workers keeps each one responsive on
    // a laptop as well as on the 4-core CI runner.
    testTimeout: 30_000,
    hookTimeout: 30_000,
    maxWorkers: "50%",
    coverage: {
      provider: "v8",
      // Everything under src, the JavaScript auth modules included: what is
      // not in the denominator is not measured.
      include: ["src/**/*.{js,jsx,ts,tsx}"],
      exclude: ["src/**/*.test.{ts,tsx}", "src/test/**", "src/main.tsx", "src/vite-env.d.ts"],
      // The bar the project holds itself to (README "Test coverage"): a
      // `vitest run --coverage` — the CI `web` job — fails below it. Lines
      // are the figure on the badges; statements and functions follow.
      thresholds: {
        lines: 90,
        statements: 90,
        functions: 90,
        branches: 90,
      },
    },
  },
});
