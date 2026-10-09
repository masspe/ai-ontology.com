// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";
import TopBar from "./TopBar";
import { getLang, setLang } from "../lib/i18n";

function mount() {
  return render(
    <MemoryRouter>
      <TopBar />
    </MemoryRouter>,
  );
}

describe("language toggle", () => {
  it("shows the other language and switches after confirmation", async () => {
    const reload = vi.fn();
    vi.stubGlobal("location", { ...window.location, reload });
    vi.spyOn(window, "confirm").mockReturnValue(true);
    mount();
    const button = screen.getByRole("button", { name: "Changer de langue" });
    expect(button).toHaveTextContent("EN");
    await userEvent.click(button);
    expect(getLang()).toBe("en");
    expect(localStorage.getItem("lang")).toBe("en");
    expect(reload).toHaveBeenCalledOnce();
  });

  it("does nothing when the confirmation is refused", async () => {
    const reload = vi.fn();
    vi.stubGlobal("location", { ...window.location, reload });
    vi.spyOn(window, "confirm").mockReturnValue(false);
    mount();
    await userEvent.click(screen.getByRole("button", { name: "Changer de langue" }));
    expect(getLang()).toBe("fr");
    expect(reload).not.toHaveBeenCalled();
  });

  it("offers French when the interface is in English", () => {
    setLang("en", false);
    mount();
    expect(screen.getByRole("button", { name: "Switch language" })).toHaveTextContent("FR");
  });
});
