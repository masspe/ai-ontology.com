// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.

import { useState } from "react";
import { act, render, screen, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import MultiSelect, { type MultiSelectOption } from "./MultiSelect";

const OPTIONS: MultiSelectOption[] = [
  { value: "Entreprise", label: "Entreprise" },
  { value: "Équipe", label: "Équipe", hint: "3 fiches" },
  { value: "Facture", label: "Facture" },
  { value: "Employé", label: "Employé" },
];

function Harness({ spy, initial = [], note }: { spy: (v: string[]) => void; initial?: string[]; note?: string }) {
  const [sel, setSel] = useState<string[]>(initial);
  return (
    <>
      <MultiSelect
        label="Types de fiche"
        options={OPTIONS}
        selected={sel}
        onChange={(v) => { spy(v); setSel(v); }}
        allLabel="Tous les types"
        note={note}
      />
      <button>ailleurs</button>
    </>
  );
}

const trigger = () => screen.getByRole("button", { name: /Types de fiche/ });
const all = () => screen.getByRole("checkbox", { name: "Tout sélectionner" }) as HTMLInputElement;

describe("MultiSelect", () => {
  it("opens and closes from the button, and shows the state on it", async () => {
    const user = userEvent.setup();
    render(<Harness spy={vi.fn()} initial={["Facture"]} note="Une note" />);
    expect(trigger()).toHaveAttribute("aria-haspopup", "dialog");
    expect(trigger()).toHaveAttribute("aria-expanded", "false");
    expect(trigger()).toHaveTextContent("Facture");
    expect(screen.queryByRole("dialog")).toBeNull();
    await user.click(trigger());
    expect(trigger()).toHaveAttribute("aria-expanded", "true");
    const panel = screen.getByRole("dialog", { name: "Types de fiche" });
    expect(panel).toHaveTextContent("Une note");
    expect(screen.getByRole("searchbox")).toHaveFocus();
    expect(screen.getByRole("checkbox", { name: /Équipe/ })).toHaveAccessibleName(/^Équipe\s*3 fiches$/);
    await user.click(trigger());
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("toggles options and emits the values in the options' order", async () => {
    const user = userEvent.setup();
    const spy = vi.fn();
    render(<Harness spy={spy} />);
    expect(trigger()).toHaveTextContent("Tous les types");
    await user.click(trigger());
    await user.click(screen.getByRole("checkbox", { name: "Facture" }));
    expect(spy).toHaveBeenLastCalledWith(["Facture"]);
    await user.click(screen.getByRole("checkbox", { name: "Entreprise" }));
    expect(spy).toHaveBeenLastCalledWith(["Entreprise", "Facture"]);
    expect(trigger()).toHaveTextContent("2 sur 4");
    expect(all().indeterminate).toBe(true);
    expect(all().checked).toBe(false);
    // Space on a focused checkbox toggles it (native behaviour).
    screen.getByRole("checkbox", { name: "Entreprise" }).focus();
    await user.keyboard(" ");
    expect(spy).toHaveBeenLastCalledWith(["Facture"]);
    await user.click(screen.getByRole("checkbox", { name: "Facture" }));
    expect(spy).toHaveBeenLastCalledWith([]);
    expect(all().indeterminate).toBe(false);
  });

  it("searches without case or accents, and 'Tout sélectionner' acts on the visible options", async () => {
    const user = userEvent.setup();
    const spy = vi.fn();
    render(<Harness spy={spy} initial={["Facture"]} />);
    await user.click(trigger());
    await user.type(screen.getByRole("searchbox"), "EQUI");
    expect(screen.getAllByRole("checkbox").map((c) => c.parentElement!.textContent)).toEqual([
      "Tout sélectionner",
      "Équipe3 fiches",
    ]);
    await user.clear(screen.getByRole("searchbox"));
    await user.type(screen.getByRole("searchbox"), "e");
    // Entreprise, Équipe, Facture, Employé all contain an "e".
    expect(screen.getAllByRole("checkbox")).toHaveLength(5);
    await user.clear(screen.getByRole("searchbox"));
    await user.type(screen.getByRole("searchbox"), "empl");
    expect(screen.getAllByRole("checkbox")).toHaveLength(2);
    expect(all().checked).toBe(false);
    await user.click(all());
    expect(spy).toHaveBeenLastCalledWith(["Facture", "Employé"]);
    expect(all().checked).toBe(true);
    // Checked when all visible are selected: a second click unselects them only.
    await user.click(all());
    expect(spy).toHaveBeenLastCalledWith(["Facture"]);
    await user.clear(screen.getByRole("searchbox"));
    await user.type(screen.getByRole("searchbox"), "zzz");
    expect(screen.getByText("Aucun résultat.")).toBeInTheDocument();
    expect(all()).toBeDisabled();
    // Everything selected reads as "all" on the button.
    await user.clear(screen.getByRole("searchbox"));
    await user.click(all());
    expect(spy).toHaveBeenLastCalledWith(["Entreprise", "Équipe", "Facture", "Employé"]);
    expect(trigger()).toHaveTextContent("Tous les types");
  });

  it("'Effacer' clears the selection", async () => {
    const user = userEvent.setup();
    const spy = vi.fn();
    render(<Harness spy={spy} initial={["Facture", "Équipe"]} />);
    await user.click(trigger());
    await user.click(screen.getByRole("button", { name: "Effacer" }));
    expect(spy).toHaveBeenLastCalledWith([]);
    expect(screen.getByRole("button", { name: "Effacer" })).toBeDisabled();
    expect(trigger()).toHaveTextContent("Tous les types");
  });

  it("closes on Escape (focus back on the button), on an outside click and when focus leaves", async () => {
    const user = userEvent.setup();
    render(<Harness spy={vi.fn()} />);
    await user.click(trigger());
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(trigger()).toHaveFocus();

    await user.click(trigger());
    // A click inside (on an option's text) keeps it open.
    await user.click(screen.getByText("Facture"));
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    fireEvent.mouseDown(document.body);
    expect(screen.queryByRole("dialog")).toBeNull();

    await user.click(trigger());
    act(() => screen.getByRole("button", { name: "ailleurs" }).focus());
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
