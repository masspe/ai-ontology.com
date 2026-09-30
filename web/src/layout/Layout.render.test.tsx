// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// Rendering tests of the app shell: the four top entries (a page each for
// Accueil and Réglages, a menu for Importer and Explorer, the active one
// marked), the top bar (global search → /queries?q=, the guide button, the
// account menu with logout, the feedback modal) and the outlet that hosts
// the page.

import { describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import Layout from "./Layout";
import { NAV_GROUPS, NAV_ITEMS } from "./nav";

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
          <Route path="login" element={<ShowLocation />} />
          <Route path="queries" element={<ShowLocation />} />
          <Route path="*" element={<ShowLocation />} />
        </Route>
      </Routes>
    </MemoryRouter>,
  );
  return { user, ...utils };
}

const SEARCH = "Rechercher une fiche, poser une question…";
const summary = (title: string) => screen.getByText(title, { selector: "summary" });

describe("Layout shell", () => {
  it("renders the four entries, every page in its menu, the top bar and the routed page", () => {
    mount("/rules");
    expect(screen.getByRole("link", { name: "AI Ontology Studio" })).toHaveAttribute("href", "/");
    expect(NAV_GROUPS.map((g) => g.title)).toEqual(["Accueil", "Importer", "Explorer", "Réglages"]);
    expect(screen.getByRole("link", { name: "Accueil" })).toHaveAttribute("href", "/");
    expect(screen.getByRole("link", { name: "Réglages" })).toHaveAttribute("href", "/settings");
    for (const group of NAV_GROUPS.filter((g) => g.items.length > 1)) {
      const menu = screen.getByRole("menu", { name: group.title });
      for (const item of group.items) {
        expect(within(menu).getByRole("menuitem", { name: item.label })).toHaveAttribute("href", item.to);
      }
    }
    expect(NAV_ITEMS).toHaveLength(10);
    expect(screen.getByPlaceholderText(SEARCH)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Guide" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Compte" })).toHaveTextContent("U");
    expect(document.querySelector("aside.sidebar")).toBeNull();
    expect(document.querySelector("main.content")).toHaveTextContent("Rules page");
  });

  it("marks the entry of the current section: the menu that holds the page, or the page itself", () => {
    mount("/rules");
    expect(summary("Explorer")).toHaveClass("active");
    expect(summary("Importer")).not.toHaveClass("active");
    expect(screen.getByRole("menuitem", { name: "Règles" })).toHaveClass("active");
    expect(screen.getByRole("menuitem", { name: "Questions" })).not.toHaveClass("active");
    expect(screen.getByRole("link", { name: "Accueil" })).not.toHaveClass("active");
  });

  it("activates Accueil on the index route only", () => {
    mount("/");
    expect(screen.getByRole("link", { name: "Accueil" })).toHaveClass("active");
    expect(summary("Explorer")).not.toHaveClass("active");
    expect(document.querySelector("main.content")).toHaveTextContent("Dashboard page");
  });

  it("opens a menu, navigates through it and closes it", async () => {
    const { user } = mount("/");
    const details = summary("Explorer").closest("details")!;
    expect(details).not.toHaveAttribute("open");
    await user.click(summary("Explorer"));
    expect(details).toHaveAttribute("open");
    await user.click(screen.getByRole("menuitem", { name: "Règles" }));
    expect(document.querySelector("main.content")).toHaveTextContent("Rules page");
    expect(details).not.toHaveAttribute("open");
    expect(summary("Explorer")).toHaveClass("active");
  });
});

describe("TopBar", () => {
  it("submits the global search to /queries?q=<encoded>", async () => {
    const { user } = mount("/rules");
    await user.type(screen.getByPlaceholderText(SEARCH), "late invoices & fees{Enter}");
    expect(screen.getByTestId("location")).toHaveTextContent("/queries?q=late%20invoices%20%26%20fees");
    expect(screen.getByRole("menuitem", { name: "Questions" })).toHaveClass("active");
  });

  it("ignores a blank search", async () => {
    const { user } = mount("/rules");
    await user.type(screen.getByPlaceholderText(SEARCH), "   {Enter}");
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

  it("shows the user's initials, a menu with their name, and logs out to /login", async () => {
    window.localStorage.setItem("msBE.user", JSON.stringify({ name: "Ada Lovelace", email: "ada@example.com" }));
    window.localStorage.setItem("msBE.token", "t");
    const { user } = mount("/rules");
    const account = screen.getByRole("button", { name: "Compte" });
    expect(account).toHaveTextContent("AL");
    expect(screen.queryByRole("menu", { name: "" })).not.toBeInTheDocument();
    await user.click(account);
    const menu = account.parentElement!.querySelector(".account-menu") as HTMLElement;
    expect(menu).toHaveTextContent("Ada Lovelace");
    expect(menu).toHaveTextContent("ada@example.com");
    // Escape closes, a second click reopens.
    await user.keyboard("{Escape}");
    expect(account.parentElement!.querySelector(".account-menu")).toBeNull();
    await user.click(account);
    await user.click(screen.getByRole("menuitem", { name: "Se déconnecter" }));
    expect(await screen.findByTestId("location")).toHaveTextContent("/login");
    expect(window.localStorage.getItem("msBE.token")).toBeNull();
  });

  it("opens the guide from the ? button", async () => {
    const { user } = mount("/");
    expect(screen.queryByTestId("tour")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Guide" }));
    expect(screen.getByTestId("tour")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Bienvenue" })).toBeInTheDocument();
  });
});
