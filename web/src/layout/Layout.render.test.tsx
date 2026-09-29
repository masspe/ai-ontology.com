// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// Rendering tests of the app shell: the menu by flow (groups, active
// link, collapse toggle), the top bar (global search → /queries?q=, the
// guide button, the account menu with logout, the feedback modal) and the
// outlet that hosts the page.

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

describe("Layout shell", () => {
  it("renders the brand, every nav entry in its group, the top bar and the routed page", () => {
    mount("/rules");
    expect(screen.getByText("AI Ontology Studio")).toBeInTheDocument();
    for (const group of NAV_GROUPS) {
      const nav = screen.getByRole("navigation", { name: group.title });
      for (const item of group.items) {
        expect(within(nav).getByRole("link", { name: item.label })).toHaveAttribute("href", item.to);
      }
    }
    expect(NAV_ITEMS.map((i) => i.label)).toEqual([
      "Tableau de bord",
      "Modèle de données",
      "Fichiers",
      "Importer des documents",
      "Graphe",
      "Fiches",
      "Questions",
      "Règles",
      "Actions",
      "Paramètres",
    ]);
    expect(screen.getByPlaceholderText(SEARCH)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Guide" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Compte" })).toHaveTextContent("U");
    expect(screen.queryByTitle("Notifications")).not.toBeInTheDocument();
    expect(document.querySelector("main.content")).toHaveTextContent("Rules page");
  });

  it("marks only the current section active; the dashboard is an exact match", () => {
    mount("/rules");
    expect(screen.getByRole("link", { name: "Règles" })).toHaveClass("active");
    expect(screen.getByRole("link", { name: "Tableau de bord" })).not.toHaveClass("active");
    expect(screen.getByRole("link", { name: "Questions" })).not.toHaveClass("active");
  });

  it("activates the dashboard on the index route only", () => {
    mount("/");
    expect(screen.getByRole("link", { name: "Tableau de bord" })).toHaveClass("active");
    expect(screen.getByRole("link", { name: "Règles" })).not.toHaveClass("active");
    expect(document.querySelector("main.content")).toHaveTextContent("Dashboard page");
  });

  it("navigates through the sidebar links", async () => {
    const { user } = mount("/");
    await user.click(screen.getByRole("link", { name: "Règles" }));
    expect(document.querySelector("main.content")).toHaveTextContent("Rules page");
    expect(screen.getByRole("link", { name: "Règles" })).toHaveClass("active");
  });

  it("collapses and expands the sidebar, exposing labels as tooltips when collapsed", async () => {
    const { user } = mount("/");
    const shell = document.querySelector(".app-shell")!;
    const aside = document.querySelector("aside.sidebar")!;
    expect(shell).not.toHaveClass("collapsed");
    expect(screen.getByRole("link", { name: "Règles" })).not.toHaveAttribute("title");

    await user.click(screen.getByRole("button", { name: "Réduire le menu" }));
    expect(shell).toHaveClass("collapsed");
    expect(aside).toHaveClass("collapsed");
    expect(screen.queryByText("Réduire")).not.toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Règles" })).toHaveAttribute("title", "Règles");

    await user.click(screen.getByRole("button", { name: "Déployer le menu" }));
    expect(shell).not.toHaveClass("collapsed");
    expect(screen.getByText("Réduire")).toBeInTheDocument();
  });
});

describe("TopBar", () => {
  it("submits the global search to /queries?q=<encoded>", async () => {
    const { user } = mount("/rules");
    await user.type(screen.getByPlaceholderText(SEARCH), "late invoices & fees{Enter}");
    expect(screen.getByTestId("location")).toHaveTextContent("/queries?q=late%20invoices%20%26%20fees");
    expect(screen.getByRole("link", { name: "Questions" })).toHaveClass("active");
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
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    await user.click(account);
    expect(screen.getByRole("menu")).toHaveTextContent("Ada Lovelace");
    expect(screen.getByRole("menu")).toHaveTextContent("ada@example.com");
    // Escape closes, a second click reopens.
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
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
