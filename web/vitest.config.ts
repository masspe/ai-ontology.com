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
    // Every test starts from a clean slate: spies restored, `vi.stubEnv` /
    // `vi.stubGlobal` undone, localStorage emptied (see setup file).
    restoreMocks: true,
    unstubEnvs: true,
    unstubGlobals: true,
    setupFiles: ["src/test/setup.ts"],
    coverage: {
      provider: "v8",
      include: ["src/**/*.{ts,tsx}"],
      exclude: ["src/**/*.test.{ts,tsx}", "src/test/**", "src/main.tsx", "src/vite-env.d.ts"],
    },
  },
});
