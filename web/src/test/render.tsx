// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Rendering helpers for page tests. A page is mounted the way `App.tsx`
// mounts it: inside a router (in memory, at the requested route) and inside
// the toast and confirm providers, so `useToast()` / `useConfirm()` /
// `useSearchParams()` work without the real shell.
//
// The API module is NOT mocked here: each test file declares
// `vi.mock("../api", ...)` for the functions its page calls, so the mock is
// visible next to the assertions that depend on it.

import type { ReactElement, ReactNode } from "react";
import { render, type RenderOptions, type RenderResult } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
// @ts-expect-error JSX module
import { ToastProvider } from "../components/Toast.jsx";
// @ts-expect-error JSX module
import { ConfirmProvider } from "../components/ConfirmDialog.jsx";

export interface RenderPageOptions extends Omit<RenderOptions, "wrapper"> {
  /** Initial location, e.g. `"/queries?q=hello"`. Default `"/"`. */
  route?: string;
  /** Route pattern the element is mounted on. Default `"*"` (matches any). */
  path?: string;
  /** Extra routes to register, e.g. a target for `navigate()` assertions. */
  extraRoutes?: ReactNode;
}

export interface RenderedPage extends RenderResult {
  user: ReturnType<typeof userEvent.setup>;
}

/** Mount `ui` inside the providers and a memory router. */
export function renderPage(ui: ReactElement, options: RenderPageOptions = {}): RenderedPage {
  const { route = "/", path = "*", extraRoutes, ...rest } = options;
  const user = userEvent.setup();
  const result = render(
    <MemoryRouter initialEntries={[route]}>
      <ToastProvider>
        <ConfirmProvider>
          <Routes>
            <Route path={path} element={ui} />
            {extraRoutes}
          </Routes>
        </ConfirmProvider>
      </ToastProvider>
    </MemoryRouter>,
    rest,
  );
  return { ...result, user };
}

/** Resolve on the next macrotask: lets pending `useEffect` fetches settle. */
export function flushPromises(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

/**
 * A `File` with a stable `text()` / `arrayBuffer()`: jsdom's `File` lacks
 * both, and the ingest pages read uploads through them.
 */
export function makeFile(name: string, content: string, type = "text/plain"): File {
  const file = new File([content], name, { type });
  Object.defineProperty(file, "text", { value: () => Promise.resolve(content) });
  Object.defineProperty(file, "arrayBuffer", {
    value: () => Promise.resolve(new TextEncoder().encode(content).buffer),
  });
  return file;
}

export { screen, within, waitFor, fireEvent, act } from "@testing-library/react";
