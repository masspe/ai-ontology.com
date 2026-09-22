// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Route-table tests of `App`: every page module is replaced by a stub so the
// test exercises the router wiring (public auth routes, protected shell with
// its index and section routes, catch-all redirect) and nothing else.
// `App` owns a `BrowserRouter`, so the location is set through `history`.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import App from "./App";

// `vi.mock` calls are hoisted above imports, so the factory helper must be too.
const { stub } = vi.hoisted(() => ({
  stub: (label: string) => () => ({ default: () => <div data-testid="page">{label}</div> }),
}));

vi.mock("./pages/Dashboard", stub("Dashboard page"));
vi.mock("./pages/OntologyBuilder", stub("Builder page"));
vi.mock("./pages/Files", stub("Files page"));
vi.mock("./pages/IngestWizard", stub("Ingest page"));
vi.mock("./pages/GraphView", stub("Graph page"));
vi.mock("./pages/Concepts", stub("Concepts page"));
vi.mock("./pages/Rules", stub("Rules page"));
vi.mock("./pages/Queries", stub("Queries page"));
vi.mock("./pages/Actions", stub("Actions page"));
vi.mock("./pages/Settings", stub("Settings page"));
vi.mock("./pages/Login.jsx", stub("Login page"));
vi.mock("./pages/Signup.jsx", stub("Signup page"));
vi.mock("./pages/OAuthCallback.jsx", stub("OAuth callback page"));
vi.mock("./routes/ProtectedRoute.jsx", () => ({
  default: ({ children }: { children: ReactNode }) => <div data-testid="protected">{children}</div>,
}));
vi.mock("./api", async () => {
  const actual = await vi.importActual<typeof import("./api")>("./api");
  return { ...actual, createFeedback: vi.fn() };
});

function visit(path: string) {
  window.history.pushState({}, "", path);
  return render(<App />);
}

beforeEach(() => {
  window.history.replaceState({}, "", "/");
});
afterEach(() => {
  window.history.replaceState({}, "", "/");
});

describe("App routes", () => {
  it.each([
    ["/", "Dashboard page"],
    ["/builder", "Builder page"],
    ["/files", "Files page"],
    ["/ingest", "Ingest page"],
    ["/graph", "Graph page"],
    ["/concepts", "Concepts page"],
    ["/rules", "Rules page"],
    ["/queries", "Queries page"],
    ["/actions", "Actions page"],
    ["/settings", "Settings page"],
  ])("%s renders inside the protected shell", (path, label) => {
    visit(path);
    expect(screen.getByTestId("page")).toHaveTextContent(label);
    // Protected: the page is wrapped by ProtectedRoute and the Layout shell.
    expect(screen.getByTestId("protected")).toContainElement(screen.getByTestId("page"));
    expect(document.querySelector("main.content")).toContainElement(screen.getByTestId("page"));
    expect(screen.getByText("AI Ontology Studio")).toBeInTheDocument();
    expect(window.location.pathname).toBe(path);
  });

  it("highlights the sidebar entry of the current section", () => {
    visit("/rules");
    expect(screen.getByRole("link", { name: "Rules" })).toHaveClass("active");
    expect(screen.getByRole("link", { name: "Dashboard" })).not.toHaveClass("active");
  });

  it.each([
    ["/login", "Login page"],
    ["/signup", "Signup page"],
    ["/oauth/callback", "OAuth callback page"],
  ])("%s is public: no shell, no guard", (path, label) => {
    visit(path);
    expect(screen.getByTestId("page")).toHaveTextContent(label);
    expect(screen.queryByTestId("protected")).not.toBeInTheDocument();
    expect(screen.queryByText("AI Ontology Studio")).not.toBeInTheDocument();
  });

  it("redirects an unknown path to the dashboard, replacing the history entry", async () => {
    const before = window.history.length;
    visit("/does/not/exist?x=1");
    await waitFor(() => expect(window.location.pathname).toBe("/"));
    expect(screen.getByTestId("page")).toHaveTextContent("Dashboard page");
    expect(screen.getByTestId("protected")).toBeInTheDocument();
    expect(window.history.length).toBe(before + 1);
  });

  it("mounts the toast provider around the routes", () => {
    visit("/");
    // The toast provider renders its fixed-position (empty) container so
    // pages can call useToast(); the confirm provider renders nothing until asked.
    const containers = Array.from(document.querySelectorAll<HTMLElement>("div")).filter(
      (d) => d.style.position === "fixed" && d.style.zIndex === "9999",
    );
    expect(containers).toHaveLength(1);
    expect(containers[0]).toBeEmptyDOMElement();
  });
});
