// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Rendering tests of the route guard: anonymous → redirect to
// `/login?next=<path+search>`; token present → "Loading…" until `me()`
// settles, then the children (user) or the redirect (no user / error).

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ComponentType, ReactNode } from "react";
import { Route, useLocation } from "react-router-dom";
// @ts-expect-error JSX module
import ProtectedRouteGuard from "./ProtectedRoute.jsx";
// @ts-expect-error JS module
import { msBE } from "../lib/msBE";
import { renderPage, screen, waitFor } from "../test/render";

vi.mock("../lib/msBE", async () => {
  const actual = await vi.importActual<{ msBE: { auth: Record<string, unknown> } }>("../lib/msBE");
  const auth = { ...actual.msBE.auth, me: vi.fn() };
  const mocked = { ...actual.msBE, auth };
  return { ...actual, msBE: mocked, default: mocked };
});

const ProtectedRoute = ProtectedRouteGuard as ComponentType<{ children: ReactNode }>;
const meMock = (msBE as { auth: { me: ReturnType<typeof vi.fn> } }).auth.me;

function ShowLocation() {
  const loc = useLocation();
  return <div data-testid="location">{loc.pathname + loc.search}</div>;
}

function mount(route: string) {
  return renderPage(
    <ProtectedRoute>
      <div data-testid="secret">secret content</div>
    </ProtectedRoute>,
    { route, path: "/files", extraRoutes: <Route path="/login" element={<ShowLocation />} /> },
  );
}

beforeEach(() => {
  meMock.mockResolvedValue({ id: 1, email: "ann@example.com" });
});

describe("ProtectedRoute", () => {
  it("redirects an anonymous visitor to /login?next=<path+search> without calling me()", () => {
    mount("/files?tab=2");
    expect(screen.getByTestId("location")).toHaveTextContent("/login?next=%2Ffiles%3Ftab%3D2");
    expect(screen.queryByTestId("secret")).not.toBeInTheDocument();
    expect(screen.queryByText("Loading…")).not.toBeInTheDocument();
    expect(meMock).not.toHaveBeenCalled();
  });

  it("with a stored token: shows Loading…, then renders the children once me() returns a user", async () => {
    window.localStorage.setItem("msBE.token", "jwt");
    mount("/files");
    expect(screen.getByText("Loading…")).toBeInTheDocument();
    expect(screen.queryByTestId("secret")).not.toBeInTheDocument();
    expect(await screen.findByTestId("secret")).toHaveTextContent("secret content");
    expect(screen.queryByText("Loading…")).not.toBeInTheDocument();
    expect(meMock).toHaveBeenCalledTimes(1);
  });

  it("with a token but no user from me(): redirects to /login", async () => {
    window.localStorage.setItem("msBE.token", "jwt");
    meMock.mockResolvedValue(null);
    mount("/files?tab=2");
    expect(await screen.findByTestId("location")).toHaveTextContent("/login?next=%2Ffiles%3Ftab%3D2");
    expect(screen.queryByTestId("secret")).not.toBeInTheDocument();
  });

  it("with a token that me() rejects (e.g. 401): redirects to /login", async () => {
    window.localStorage.setItem("msBE.token", "stale");
    meMock.mockRejectedValue(Object.assign(new Error("Unauthorized"), { status: 401 }));
    mount("/files");
    expect(await screen.findByTestId("location")).toHaveTextContent("/login?next=%2Ffiles");
  });

  it("ignores a me() result that lands after unmount", async () => {
    window.localStorage.setItem("msBE.token", "jwt");
    let resolve!: (v: unknown) => void;
    meMock.mockImplementation(() => new Promise((r) => (resolve = r)));
    const { unmount } = mount("/files");
    expect(screen.getByText("Loading…")).toBeInTheDocument();
    unmount();
    const errors = vi.spyOn(console, "error").mockImplementation(() => {});
    resolve({ id: 1 });
    await waitFor(() => expect(meMock).toHaveBeenCalledTimes(1));
    await new Promise((r) => setTimeout(r, 0));
    expect(errors).not.toHaveBeenCalled();
  });
});
