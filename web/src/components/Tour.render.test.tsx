// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The interactive guide: opens alone on the first visit, walks the parts
// of the application page by page, spotlights the element it explains,
// jumps from the summary, remembers it was seen, reopens from the "?".

import { describe, expect, it } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import Tour, { TOUR_STEPS, openTour } from "./Tour";
import { NAV_ITEMS } from "../layout/nav";

function Page() {
  const loc = useLocation();
  return (
    <div>
      <div data-testid="location">{loc.pathname}</div>
      <button data-tour="help">?</button>
      {NAV_ITEMS.map((i) => (
        <a key={i.to} href={i.to} data-tour={i.tour}>
          {i.label}
        </a>
      ))}
      <form data-tour="search" />
      <Tour />
    </div>
  );
}

function mount(route = "/") {
  const user = userEvent.setup();
  render(
    <MemoryRouter initialEntries={[route]}>
      <Routes>
        <Route path="*" element={<Page />} />
      </Routes>
    </MemoryRouter>,
  );
  return { user };
}

const firstVisit = () => window.localStorage.removeItem("tour.v1");

describe("Tour", () => {
  it("has one step per menu entry plus welcome, search and feedback", () => {
    expect(TOUR_STEPS.length).toBe(NAV_ITEMS.length + 3);
    expect(TOUR_STEPS[0]!.title).toBe("Bienvenue");
    expect(TOUR_STEPS.map((s) => s.target)).toContain("explorer");
  });

  it("stays closed once it was seen, and opens on the ? event", async () => {
    mount();
    expect(screen.queryByTestId("tour")).not.toBeInTheDocument();
    openTour();
    expect(await screen.findByTestId("tour")).toBeInTheDocument();
  });

  it("opens alone on the first visit, walks the steps, navigates and remembers", async () => {
    firstVisit();
    const { user } = mount("/rules");
    expect(await screen.findByRole("heading", { name: "Bienvenue" })).toBeInTheDocument();
    // The welcome step lives on the dashboard: the guide navigates there.
    await waitFor(() => expect(screen.getByTestId("location")).toHaveTextContent("/"));
    expect(screen.getByText(`Étape 1 sur ${TOUR_STEPS.length}`)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Précédent" })).toBeDisabled();
    // The spotlight follows the anchor of the step.
    expect(screen.getByTestId("tour-spot")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Suivant" }));
    expect(screen.getByRole("heading", { name: "Accueil" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Suivant" }));
    expect(screen.getByRole("heading", { name: "Fichiers" })).toBeInTheDocument();
    await waitFor(() => expect(screen.getByTestId("location")).toHaveTextContent("/files"));
    await user.keyboard("{ArrowRight}");
    expect(screen.getByRole("heading", { name: "Relire un document" })).toBeInTheDocument();
    await user.keyboard("{ArrowLeft}");
    expect(screen.getByRole("heading", { name: "Fichiers" })).toBeInTheDocument();

    // The summary jumps anywhere.
    await user.click(screen.getByRole("button", { name: "Sommaire" }));
    await user.click(screen.getByRole("button", { name: "Questions" }));
    expect(screen.getByRole("heading", { name: "Questions" })).toBeInTheDocument();
    await waitFor(() => expect(screen.getByTestId("location")).toHaveTextContent("/queries"));

    // Escape closes and the visit is remembered.
    await user.keyboard("{Escape}");
    expect(screen.queryByTestId("tour")).not.toBeInTheDocument();
    expect(JSON.parse(window.localStorage.getItem("tour.v1")!)).toMatchObject({ done: true });
  });

  it("ends with Terminer on the last step and centres the card when the anchor is missing", async () => {
    firstVisit();
    const { user } = mount();
    await screen.findByTestId("tour");
    await user.click(screen.getByRole("button", { name: "Sommaire" }));
    await user.click(screen.getByRole("button", { name: "Retour d'expérience" }));
    expect(screen.getByText(`Étape ${TOUR_STEPS.length} sur ${TOUR_STEPS.length}`)).toBeInTheDocument();
    // No element carries data-tour="feedback" on this page: no spotlight.
    expect(screen.queryByTestId("tour-spot")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Suivant" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Terminer" }));
    expect(screen.queryByTestId("tour")).not.toBeInTheDocument();
    // Reopening starts again from the beginning.
    openTour();
    expect(await screen.findByRole("heading", { name: "Bienvenue" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Fermer le guide" }));
    expect(screen.queryByTestId("tour")).not.toBeInTheDocument();
  });
});
