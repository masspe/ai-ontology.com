// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Rendering tests of the Login page: `?next=` sanitising (form redirect,
// OAuth links, signup link), client-side validation, the loading state,
// the success toast + redirect and the error toast.

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ComponentType } from "react";
import { Route, useLocation } from "react-router-dom";
// @ts-expect-error JSX module
import LoginPage from "./Login.jsx";
// @ts-expect-error JS module
import { msBE } from "../lib/msBE";
import { renderPage, screen, waitFor } from "../test/render";

vi.mock("../lib/msBE", async () => {
  const actual = await vi.importActual<{ msBE: { auth: Record<string, unknown> } }>("../lib/msBE");
  const auth = { ...actual.msBE.auth, login: vi.fn() };
  const mocked = { ...actual.msBE, auth };
  return { ...actual, msBE: mocked, default: mocked };
});

const Login = LoginPage as ComponentType;
const loginMock = (msBE as { auth: { login: ReturnType<typeof vi.fn> } }).auth.login;

function ShowLocation() {
  const loc = useLocation();
  return <div data-testid="location">{loc.pathname + loc.search}</div>;
}
const targets = (
  <>
    <Route path="/files" element={<ShowLocation />} />
    <Route path="/" element={<ShowLocation />} />
  </>
);

function mount(route = "/login") {
  return renderPage(<Login />, { route, path: "/login", extraRoutes: targets });
}

const emailBox = () => screen.getByLabelText(/^Email/);
const passwordBox = () => screen.getByLabelText(/^Mot de passe/);
const submit = () => screen.getByRole("button", { name: "Se connecter" });

beforeEach(() => {
  loginMock.mockResolvedValue({ token: "jwt", user: { id: 1 } });
});

describe("Login page", () => {
  it("renders the form, the OAuth links and the signup link with a sanitised next=/", () => {
    mount();
    expect(screen.getByRole("heading", { name: "Se connecter" })).toBeInTheDocument();
    expect(emailBox()).toHaveAttribute("type", "email");
    expect(passwordBox()).toHaveAttribute("type", "password");
    expect(screen.getByRole("link", { name: "Continuer avec Google" })).toHaveAttribute(
      "href",
      "/auth/oauth/google/start?next=%2F",
    );
    expect(screen.getByRole("link", { name: "Continuer avec Microsoft" })).toHaveAttribute(
      "href",
      "/auth/oauth/microsoft/start?next=%2F",
    );
    expect(screen.getByRole("link", { name: "Créer un compte" })).toHaveAttribute("href", "/signup?next=%2F");
  });

  it("threads a relative ?next= through the OAuth and signup links", () => {
    mount("/login?next=%2Ffiles%3Ftab%3D2");
    expect(screen.getByRole("link", { name: "Continuer avec Google" })).toHaveAttribute(
      "href",
      "/auth/oauth/google/start?next=%2Ffiles%3Ftab%3D2",
    );
    expect(screen.getByRole("link", { name: "Créer un compte" })).toHaveAttribute(
      "href",
      "/signup?next=%2Ffiles%3Ftab%3D2",
    );
  });

  it.each([
    ["protocol-relative", "//evil.example"],
    ["absolute", "https%3A%2F%2Fevil.example"],
    ["relative without slash", "files"],
  ])("falls back to next=/ for an unsafe %s target", (_label, next) => {
    mount(`/login?next=${next}`);
    expect(screen.getByRole("link", { name: "Créer un compte" })).toHaveAttribute("href", "/signup?next=%2F");
  });

  it("rejects an empty form: both field errors, aria-invalid, no request", async () => {
    const { user } = mount();
    await user.click(submit());
    expect(screen.getByText("Email invalide")).toBeInTheDocument();
    expect(screen.getByText("Mot de passe requis")).toBeInTheDocument();
    expect(emailBox()).toHaveAttribute("aria-invalid", "true");
    expect(passwordBox()).toHaveAttribute("aria-invalid", "true");
    expect(loginMock).not.toHaveBeenCalled();
  });

  it("flags a malformed email alone and clears the error once the form is valid", async () => {
    const { user } = mount();
    await user.type(emailBox(), "not-an-email");
    await user.type(passwordBox(), "secret");
    await user.click(submit());
    expect(screen.getByText("Email invalide")).toBeInTheDocument();
    expect(screen.queryByText("Mot de passe requis")).not.toBeInTheDocument();
    expect(passwordBox()).toHaveAttribute("aria-invalid", "false");
    expect(loginMock).not.toHaveBeenCalled();

    await user.clear(emailBox());
    await user.type(emailBox(), "ann@example.com");
    await user.click(submit());
    await waitFor(() => expect(loginMock).toHaveBeenCalledTimes(1));
    expect(screen.queryByText("Email invalide")).not.toBeInTheDocument();
  });

  it("logs in with the trimmed email, toasts and redirects to next (replace)", async () => {
    const { user } = mount("/login?next=%2Ffiles%3Ftab%3D2");
    await user.type(emailBox(), "  ann@example.com  ");
    await user.type(passwordBox(), "secret");
    await user.click(submit());
    await waitFor(() => expect(loginMock).toHaveBeenCalledTimes(1));
    expect(loginMock).toHaveBeenCalledWith({ email: "ann@example.com", password: "secret" });
    expect(await screen.findByRole("status")).toHaveTextContent("Connexion réussie");
    expect(await screen.findByTestId("location")).toHaveTextContent("/files?tab=2");
    expect(screen.queryByRole("heading", { name: "Se connecter" })).not.toBeInTheDocument();
  });

  it("redirects to / when no next is given", async () => {
    const { user } = mount();
    await user.type(emailBox(), "ann@example.com");
    await user.type(passwordBox(), "secret");
    await user.click(submit());
    expect(await screen.findByTestId("location")).toHaveTextContent("/");
  });

  it("disables the button and shows 'Connexion…' while the request is pending", async () => {
    let resolve!: (v: unknown) => void;
    loginMock.mockImplementation(() => new Promise((r) => (resolve = r)));
    const { user } = mount();
    await user.type(emailBox(), "ann@example.com");
    await user.type(passwordBox(), "secret");
    await user.click(submit());
    const pending = await screen.findByRole("button", { name: "Connexion…" });
    expect(pending).toBeDisabled();
    // Clicking the disabled button never issues a second request.
    await user.click(pending);
    expect(loginMock).toHaveBeenCalledTimes(1);
    resolve({ token: "jwt" });
    expect(await screen.findByTestId("location")).toHaveTextContent("/");
  });

  it("shows the server error message in a toast and stays on the page", async () => {
    loginMock.mockRejectedValue(new Error("bad credentials"));
    const { user } = mount();
    await user.type(emailBox(), "ann@example.com");
    await user.type(passwordBox(), "wrong");
    await user.click(submit());
    expect(await screen.findByRole("status")).toHaveTextContent("bad credentials");
    expect(screen.getByRole("button", { name: "Se connecter" })).toBeEnabled();
    expect(screen.queryByTestId("location")).not.toBeInTheDocument();
  });

  it("falls back to a generic error toast when the error has no message", async () => {
    loginMock.mockRejectedValue({});
    const { user } = mount();
    await user.type(emailBox(), "ann@example.com");
    await user.type(passwordBox(), "wrong");
    await user.click(submit());
    expect(await screen.findByRole("status")).toHaveTextContent("Échec de connexion");
  });
});
