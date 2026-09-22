// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Rendering tests of the app shell: sidebar navigation (active link,
// collapse toggle), top bar (global search → /queries?q=, feedback modal)
// and the outlet that hosts the page.

import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import Layout from "./Layout";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return { ...actual, createFeedback: vi.fn() };
});

function ShowLocation() {
  const loc = useLocation();
  return <div data-testid="location">{loc.pathname + loc.search}</div>;
}

function mount(route = "/") {
  const user = userEvent.setup();
  const utils = render(
    <MemoryRouter initialEntries={[route]}>
      <Routes>
        <Route element={<Layout />}>
          <Route index element={<div>Dashboard page</div>} />
          <Route path="rules" element={<div>Rules page</div>} />
          <Route path="queries" element={<ShowLocation />} />
          <Route path="*" element={<ShowLocation />} />
        </Route>
      </Routes>
    </MemoryRouter>,
  );
  return { user, ...utils };
}

const NAV_LABELS = [
  "Dashboard",
  "Ontology Builder",
  "Files",
  "Ingest (LLM)",
  "Graph View",
  "Concepts",
  "Rules",
  "Queries",
  "Actions",
  "Settings",
];

describe("Layout shell", () => {
  it("renders the brand, every nav entry, the top bar and the routed page in the outlet", () => {
    mount("/rules");
    expect(screen.getByText("AI Ontology Studio")).toBeInTheDocument();
    for (const label of NAV_LABELS) expect(screen.getByRole("link", { name: label })).toBeInTheDocument();
    expect(screen.getByText("Default workspace")).toBeInTheDocument();
    expect(screen.getByPlaceholderText("Search concepts, queries, files…")).toBeInTheDocument();
    expect(screen.getByTitle("Account")).toHaveTextContent("U");
    expect(screen.getByTitle("Help")).toBeInTheDocument();
    expect(screen.getByTitle("Notifications")).toBeInTheDocument();
    expect(document.querySelector("main.content")).toHaveTextContent("Rules page");
  });

  it("marks only the current section active; Dashboard is an exact match", () => {
    mount("/rules");
    expect(screen.getByRole("link", { name: "Rules" })).toHaveClass("active");
    expect(screen.getByRole("link", { name: "Dashboard" })).not.toHaveClass("active");
    expect(screen.getByRole("link", { name: "Queries" })).not.toHaveClass("active");
    expect(screen.getByRole("link", { name: "Rules" })).toHaveAttribute("href", "/rules");
  });

  it("activates Dashboard on the index route only", () => {
    mount("/");
    expect(screen.getByRole("link", { name: "Dashboard" })).toHaveClass("active");
    expect(screen.getByRole("link", { name: "Rules" })).not.toHaveClass("active");
    expect(document.querySelector("main.content")).toHaveTextContent("Dashboard page");
  });

  it("navigates through the sidebar links", async () => {
    const { user } = mount("/");
    await user.click(screen.getByRole("link", { name: "Rules" }));
    expect(document.querySelector("main.content")).toHaveTextContent("Rules page");
    expect(screen.getByRole("link", { name: "Rules" })).toHaveClass("active");
    expect(screen.getByRole("link", { name: "Dashboard" })).not.toHaveClass("active");
  });

  it("collapses and expands the sidebar, exposing labels as tooltips when collapsed", async () => {
    const { user } = mount("/");
    const shell = document.querySelector(".app-shell")!;
    const aside = document.querySelector("aside.sidebar")!;
    expect(shell).not.toHaveClass("collapsed");
    expect(aside).not.toHaveClass("collapsed");
    expect(screen.getByRole("link", { name: "Rules" })).not.toHaveAttribute("title");

    await user.click(screen.getByRole("button", { name: /Collapse/ }));
    expect(shell).toHaveClass("collapsed");
    expect(aside).toHaveClass("collapsed");
    expect(screen.queryByText("Collapse")).not.toBeInTheDocument();
    expect(screen.getByText("›")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Rules" })).toHaveAttribute("title", "Rules");

    await user.click(screen.getByText("›").closest("button")!);
    expect(shell).not.toHaveClass("collapsed");
    expect(screen.getByText("Collapse")).toBeInTheDocument();
    expect(screen.getByText("‹")).toBeInTheDocument();
  });
});

describe("TopBar", () => {
  it("submits the global search to /queries?q=<encoded>", async () => {
    const { user } = mount("/rules");
    await user.type(screen.getByPlaceholderText("Search concepts, queries, files…"), "late invoices & fees{Enter}");
    expect(screen.getByTestId("location")).toHaveTextContent("/queries?q=late%20invoices%20%26%20fees");
    expect(screen.getByRole("link", { name: "Queries" })).toHaveClass("active");
  });

  it("ignores a blank search", async () => {
    const { user } = mount("/rules");
    await user.type(screen.getByPlaceholderText("Search concepts, queries, files…"), "   {Enter}");
    expect(document.querySelector("main.content")).toHaveTextContent("Rules page");
    expect(screen.queryByTestId("location")).not.toBeInTheDocument();
  });

  it("opens and closes the feedback modal", async () => {
    const { user } = mount("/");
    expect(screen.queryByRole("heading", { name: /Envoyer un feedback/ })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /Feedback/ }));
    expect(screen.getByRole("heading", { name: /Envoyer un feedback/ })).toBeInTheDocument();
    await user.click(screen.getByTitle("Fermer"));
    expect(screen.queryByRole("heading", { name: /Envoyer un feedback/ })).not.toBeInTheDocument();
  });
});
