// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The Access page: accounts with their role and clearances, edited in
// place; one's own account cannot be demoted or deleted here. The API
// keys: their scope, expiry and last use, a secret shown once.

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

const keys = [
  { id: "k1", name: "etl", prefix: "ok_abc", createdBy: "alice@example.com", clearances: null, readOnly: false, expiresAt: null, lastUsedAt: null },
  { id: "k2", name: "report", prefix: "ok_def", createdBy: "alice@example.com", clearances: ["hr"], readOnly: true, expiresAt: 4102444800, lastUsedAt: "2026-11-20T10:30:00Z" },
  { id: "k3", name: "old", prefix: "ok_ghi", createdBy: "dave@example.com", clearances: [], readOnly: false, expiresAt: 1, lastUsedAt: null },
];

beforeEach(() => {
  window.localStorage.setItem("msBE.user", JSON.stringify({ id: "u1", name: "Alice" }));
  request.mockReset();
  request.mockImplementation((method: string, path: string, body?: unknown) => {
    if (method === "GET" && path === "/auth/users") return Promise.resolve({ users });
    if (method === "GET" && path === "/auth/keys") return Promise.resolve({ keys });
    if (method === "POST" && path === "/auth/keys") return Promise.resolve({ key: "ok_secret_new", name: (body as { name: string }).name });
    if (method === "POST" && path.endsWith("/rotate")) return Promise.resolve({ key: "ok_secret_rotated", name: "etl" });
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
    // Whoever may not list the accounts is not shown the keys either.
    expect(screen.queryAllByText("Clés d'API")).toHaveLength(1);
  });

  it("deletes an account once confirmed and shows what the server refuses", async () => {
    renderPage(<Access />);
    await screen.findByText("bob@example.com");
    const remove = screen.getAllByRole("button", { name: "Supprimer" });
    expect(remove[0]).toBeDisabled();
    fireEvent.click(remove[1]);
    fireEvent.click(await screen.findByRole("button", { name: "Annuler" }));
    await waitFor(() => expect(screen.queryByText("Supprimer ce compte ?")).toBeNull());
    expect(request).not.toHaveBeenCalledWith("DELETE", "/auth/users/u2");

    // Refused: the account stays and the reason is shown, for a delete, a change and a creation.
    request.mockImplementation((method: string, path: string) =>
      method === "GET" ? Promise.resolve(path === "/auth/users" ? { users } : { keys }) : Promise.reject(new Error(`${method} refused`)),
    );
    fireEvent.click(screen.getAllByRole("button", { name: "Supprimer" })[1]);
    fireEvent.click(await screen.findByRole("button", { name: "Confirmer" }));
    expect(await screen.findByText("DELETE refused")).toBeInTheDocument();
    expect(screen.getByText("bob@example.com")).toBeInTheDocument();
    fireEvent.change(screen.getByRole("combobox", { name: "Rôle bob@example.com" }), { target: { value: "admin" } });
    expect(await screen.findByText("PATCH refused")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Email"), { target: { value: "carol@example.com" } });
    fireEvent.change(screen.getByLabelText("Nom"), { target: { value: "Carol" } });
    fireEvent.change(screen.getByLabelText("Mot de passe"), { target: { value: "Passw0rd!x" } });
    fireEvent.click(screen.getByRole("button", { name: "Créer le compte" }));
    expect(await screen.findByText("POST refused")).toBeInTheDocument();
    expect(request).toHaveBeenCalledWith("POST", "/auth/users", { email: "carol@example.com", name: "Carol", password: "Passw0rd!x" });

    request.mockImplementation((method: string, path: string) => Promise.resolve(method === "GET" && path === "/auth/keys" ? { keys } : {}));
    fireEvent.click(screen.getAllByRole("button", { name: "Supprimer" })[1]);
    fireEvent.click(await screen.findByRole("button", { name: "Confirmer" }));
    await waitFor(() => expect(request).toHaveBeenCalledWith("DELETE", "/auth/users/u2"));
    await waitFor(() => expect(screen.queryByText("bob@example.com")).toBeNull());
  });

  it("lists the API keys with their scope, expiry and last use", async () => {
    renderPage(<Access />);
    expect(await screen.findByText("etl")).toBeInTheDocument();
    expect(screen.getByText("voit tout")).toBeInTheDocument();
    expect(screen.getByText("hr")).toBeInTheDocument();
    expect(screen.getByText("lecture seule")).toBeInTheDocument();
    expect(screen.getByText("2100-01-01")).toBeInTheDocument();
    expect(screen.getByText("2026-11-20 10:30")).toBeInTheDocument();
    expect(screen.getByText("expirée")).toBeInTheDocument();
    expect(screen.getByText("fiches sans label seulement")).toBeInTheDocument();
    expect(screen.getAllByText("jamais")).toHaveLength(3);
  });

  it("creates a key with its scope and shows the secret once", async () => {
    renderPage(<Access />);
    await screen.findByText("etl");
    fireEvent.change(screen.getByLabelText("Nom de la clé"), { target: { value: "ci" } });
    fireEvent.change(screen.getByLabelText("Habilitations de la clé (vide : voit tout)"), { target: { value: " HR, ops" } });
    fireEvent.change(screen.getByLabelText("Expire dans (jours, vide : jamais)"), { target: { value: "30" } });
    fireEvent.click(screen.getByLabelText("Lecture seule"));
    fireEvent.click(screen.getByRole("button", { name: "Créer la clé" }));
    await waitFor(() =>
      expect(request).toHaveBeenCalledWith("POST", "/auth/keys", { name: "ci", clearances: ["hr", "ops"], read_only: true, expires_in_days: 30 }),
    );
    expect(await screen.findByText("ok_secret_new")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("Clé ci — copiez-la maintenant");
    expect(screen.getByLabelText("Nom de la clé")).toHaveValue("");
    fireEvent.click(screen.getByRole("button", { name: "Fermer" }));
    expect(screen.queryByText("ok_secret_new")).toBeNull();

    // Nothing asked: no clearances and no expiry are sent, the server's defaults apply.
    fireEvent.change(screen.getByLabelText("Nom de la clé"), { target: { value: "all" } });
    fireEvent.click(screen.getByRole("button", { name: "Créer la clé" }));
    await waitFor(() => expect(request).toHaveBeenCalledWith("POST", "/auth/keys", { name: "all", read_only: false }));
  });

  it("rotates and revokes a key after a confirmation, and shows a refusal", async () => {
    renderPage(<Access />);
    await screen.findByText("etl");
    fireEvent.click(screen.getAllByRole("button", { name: "Renouveler" })[0]);
    expect(await screen.findByText("etl : l'ancien secret cesse de fonctionner.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Annuler" }));
    await waitFor(() => expect(screen.queryByText("etl : l'ancien secret cesse de fonctionner.")).toBeNull());
    expect(request).not.toHaveBeenCalledWith("POST", "/auth/keys/k1/rotate");

    fireEvent.click(screen.getAllByRole("button", { name: "Renouveler" })[0]);
    fireEvent.click(await screen.findByRole("button", { name: "Confirmer" }));
    await waitFor(() => expect(request).toHaveBeenCalledWith("POST", "/auth/keys/k1/rotate"));
    expect(await screen.findByText("ok_secret_rotated")).toBeInTheDocument();

    fireEvent.click(screen.getAllByRole("button", { name: "Révoquer" })[1]);
    fireEvent.click(screen.getByRole("button", { name: "Annuler" }));
    fireEvent.click(screen.getAllByRole("button", { name: "Révoquer" })[1]);
    fireEvent.click(await screen.findByRole("button", { name: "Confirmer" }));
    await waitFor(() => expect(request).toHaveBeenCalledWith("DELETE", "/auth/keys/k2"));

    request.mockImplementationOnce(() => Promise.reject(new Error("unknown key")));
    fireEvent.click(screen.getAllByRole("button", { name: "Révoquer" })[2]);
    fireEvent.click(await screen.findByRole("button", { name: "Confirmer" }));
    expect(await screen.findByText("unknown key")).toBeInTheDocument();
    // The keys already listed stay.
    expect(screen.getByText("etl")).toBeInTheDocument();
  });

  it("says so when there is no key, or when the keys cannot be listed", async () => {
    request.mockImplementation((method: string, path: string) =>
      path === "/auth/users" ? Promise.resolve({ users }) : method === "GET" ? Promise.resolve({ keys: [] }) : Promise.reject("refused"),
    );
    const first = renderPage(<Access />);
    expect(await screen.findByText("Aucune clé.")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Nom de la clé"), { target: { value: "x" } });
    fireEvent.click(screen.getByRole("button", { name: "Créer la clé" }));
    expect(await screen.findByText("refused")).toBeInTheDocument();
    first.unmount();

    request.mockImplementation((_method: string, path: string) => (path === "/auth/users" ? Promise.resolve({ users }) : Promise.reject(new Error("keys are for administrators"))));
    renderPage(<Access />);
    expect(await screen.findByText("keys are for administrators")).toBeInTheDocument();
    expect(screen.getByText("Aucune clé.")).toBeInTheDocument();
  });
});
