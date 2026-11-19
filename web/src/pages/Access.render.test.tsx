// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The Access page: accounts with their role and clearances, edited in
// place; one's own account cannot be demoted or deleted here.

import { beforeEach, describe, expect, it, vi } from "vitest";
import Access from "./Access";
// @ts-expect-error JS module
import { msBE } from "../lib/msBE";
import { fireEvent, renderPage, screen, waitFor } from "../test/render";

vi.mock("../lib/msBE", async () => {
  const actual = await vi.importActual<{ msBE: { auth: Record<string, unknown> } }>("../lib/msBE");
  const mocked = { ...actual.msBE, request: vi.fn() };
  return { ...actual, msBE: mocked, default: mocked };
});

const request = (msBE as { request: ReturnType<typeof vi.fn> }).request;

const users = [
  { id: "u1", email: "alice@example.com", name: "Alice", role: "admin", clearances: [] },
  { id: "u2", email: "bob@example.com", name: "Bob", role: "user", clearances: ["hr"] },
];

beforeEach(() => {
  window.localStorage.setItem("msBE.user", JSON.stringify({ id: "u1", name: "Alice" }));
  request.mockImplementation((method: string, path: string, body?: unknown) => {
    if (method === "GET" && path === "/auth/users") return Promise.resolve({ users });
    if (method === "PATCH") {
      const u = users.find((x) => path.endsWith(x.id))!;
      return Promise.resolve({ user: { ...u, ...(body as object) } });
    }
    return Promise.resolve({});
  });
});

describe("Access page", () => {
  it("lists the accounts; the clearances save on blur, the role on change", async () => {
    renderPage(<Access />);
    expect(await screen.findByText("bob@example.com")).toBeInTheDocument();
    const own = screen.getByRole("combobox", { name: "Rôle alice@example.com" });
    expect(own).toBeDisabled();
    const bobClear = screen.getByRole("textbox", { name: "Habilitations bob@example.com" });
    expect(bobClear).toHaveValue("hr");
    fireEvent.change(bobClear, { target: { value: " HR, Finance ,," } });
    fireEvent.blur(bobClear);
    await waitFor(() =>
      expect(request).toHaveBeenCalledWith("PATCH", "/auth/users/u2", { clearances: ["hr", "finance"] }),
    );
    // An unchanged list is not sent again.
    const calls = request.mock.calls.length;
    fireEvent.blur(bobClear);
    expect(request.mock.calls.length).toBe(calls);
    fireEvent.change(screen.getByRole("combobox", { name: "Rôle bob@example.com" }), { target: { value: "admin" } });
    await waitFor(() => expect(request).toHaveBeenCalledWith("PATCH", "/auth/users/u2", { role: "admin" }));
    expect(screen.getByRole("textbox", { name: "Habilitations bob@example.com" })).toBeDisabled();
  });

  it("creates an account and shows a refused listing as a note, an error as a banner", async () => {
    renderPage(<Access />);
    await screen.findByText("bob@example.com");
    fireEvent.change(screen.getByLabelText("Email"), { target: { value: "carol@example.com" } });
    fireEvent.change(screen.getByLabelText("Mot de passe"), { target: { value: "Passw0rd!x" } });
    fireEvent.click(screen.getByRole("button", { name: "Créer le compte" }));
    await waitFor(() =>
      expect(request).toHaveBeenCalledWith("POST", "/auth/users", { email: "carol@example.com", name: "", password: "Passw0rd!x" }),
    );

    request.mockRejectedValueOnce(new Error("Administrator only"));
    renderPage(<Access />);
    expect((await screen.findAllByText("Administrator only")).length).toBeGreaterThan(0);
    expect(screen.getByText("Aucun compte visible — réservé aux administrateurs.")).toBeInTheDocument();
  });
});
