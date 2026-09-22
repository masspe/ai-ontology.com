// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Shared Vitest setup: runs before every test file. Guarantees that browser
// storage never leaks between tests and that no test can hit the network by
// accident — a real `fetch` call fails loudly instead of dialling out.
//
// Rendering tests (Testing Library) also need a few browser APIs jsdom does
// not implement: they are stubbed here once so every page can mount.

import "@testing-library/jest-dom/vitest";
import { cleanup, configure } from "@testing-library/react";
import { afterEach, beforeEach, vi } from "vitest";

// `findBy*` / `waitFor` poll for this long before failing. The default of
// one second is tight for the 1 500-line pages when several jsdom workers
// share the CPU; three seconds is headroom, not a wait (they resolve as
// soon as the condition holds).
configure({ asyncUtilTimeout: 3_000 });

beforeEach(() => {
  window.localStorage.clear();
  window.sessionStorage.clear();
  vi.stubGlobal(
    "fetch",
    vi.fn(() => Promise.reject(new Error("network access is disabled in unit tests"))),
  );
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

// ---- jsdom gaps ------------------------------------------------------------

class ResizeObserverStub {
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
}
if (typeof window.ResizeObserver === "undefined") {
  (window as unknown as { ResizeObserver: typeof ResizeObserverStub }).ResizeObserver =
    ResizeObserverStub;
}

class IntersectionObserverStub {
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
  takeRecords(): never[] {
    return [];
  }
}
if (typeof window.IntersectionObserver === "undefined") {
  (window as unknown as { IntersectionObserver: typeof IntersectionObserverStub }).IntersectionObserver =
    IntersectionObserverStub;
}

if (typeof window.matchMedia === "undefined") {
  window.matchMedia = (query: string) =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    }) as MediaQueryList;
}

if (typeof Element.prototype.scrollIntoView === "undefined") {
  Element.prototype.scrollIntoView = () => {};
}

if (typeof URL.createObjectURL === "undefined") {
  URL.createObjectURL = () => "blob:jsdom/stub";
  URL.revokeObjectURL = () => {};
}

// A 2D canvas context that records nothing but never throws: the graph
// canvas, sparklines and the OCR pipeline draw into it during tests.
const canvasContextStub = () =>
  new Proxy(
    {},
    {
      get(_target, prop) {
        if (prop === "measureText") return () => ({ width: 0 });
        if (prop === "getImageData") return () => ({ data: new Uint8ClampedArray(4), width: 1, height: 1 });
        if (prop === "createLinearGradient" || prop === "createRadialGradient") {
          return () => ({ addColorStop: () => {} });
        }
        if (prop === "canvas") return { width: 0, height: 0 };
        return () => {};
      },
      set() {
        return true;
      },
    },
  );
HTMLCanvasElement.prototype.getContext = (() =>
  canvasContextStub() as unknown as CanvasRenderingContext2D) as unknown as typeof HTMLCanvasElement.prototype.getContext;
HTMLCanvasElement.prototype.toDataURL = () => "data:image/png;base64,";
