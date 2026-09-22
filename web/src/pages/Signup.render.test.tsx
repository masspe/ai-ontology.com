// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Rendering tests of the Signup page: `?next=` sanitising, the four field
// validations, the password-strength meter, the loading state, the success
// toast + redirect and the error toast.

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ComponentType } from "react";
import { Route, useLocation } from "react-router-dom";
// @ts-expect-error JSX module
import SignupPage from "./Signup.jsx";
// @ts-expect-error JS module
import { msBE } from "../lib/msBE";
import { renderPage, screen, waitFor } from "../test/render";

vi.mock("../lib/msBE", async () => {
  const actual = await vi.importActual<{ msBE: { auth: Record<string, unknown> } }>("../lib/msBE");
  const auth = { ...actual.msBE.auth, signup: vi.fn() };
  const mocked = { ...actual.msBE, auth };
  return { ...actual, msBE: mocked, default: mocked };
});

const Signup = SignupPage as ComponentType;
const signupMock = (msBE as { auth: { signup: ReturnType<typeof vi.fn> } }).auth.signup;

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

function mount(route = "/signup") {
  return renderPage(<Signup />, { route, path: "/signup", extraRoutes: targets });
}

const nameBox = () => screen.getByLabelText(/^Nom/);
const emailBox = () => screen.getByLabelText(/^Email/);
const passwordBox = () => screen.getByLabelText(/^Mot de passe/);
const confirmBox = () => screen.getByLabelText(/^Confirmer/);
const submit = () => screen.getByRole("button", { name: "Créer mon compte" });

/** Background colours of the five strength bars, in order. */
function bars(container: HTMLElement): string[] {
  const meter = passwordBox().parentElement!.querySelector("div")!;
  expect(container.contains(meter)).toBe(true);
  return Array.from(meter.children).map((el) => (el as HTMLElement).style.background);
}

async function fillValid(user: ReturnType<typeof renderPage>["user"]) {
  await user.type(nameBox(), "  Ann Lee ");
  await user.type(emailBox(), "  Ann@Example.COM ");
  await user.type(passwordBox(), "Str0ngpass");
  await user.type(confirmBox(), "Str0ngpass");
}

beforeEach(() => {
  signupMock.mockResolvedValue({ token: "jwt", user: { id: 1 } });
});

describe("Signup page", () => {
  it("renders the form, the OAuth links and the login link with next=/", () => {
    mount();
    expect(screen.getByRole("heading", { name: "Créer un compte" })).toBeInTheDocument();
    expect(nameBox()).toHaveAttribute("autocomplete", "name");
    expect(passwordBox()).toHaveAttribute("type", "password");
    expect(confirmBox()).toHaveAttribute("type", "password");
    expect(screen.getByRole("link", { name: "S'inscrire avec Google" })).toHaveAttribute(
      "href",
      "/auth/oauth/google/start?next=%2F",
    );
    expect(screen.getByRole("link", { name: "S'inscrire avec Microsoft" })).toHaveAttribute(
      "href",
      "/auth/oauth/microsoft/start?next=%2F",
    );
    expect(screen.getByRole("link", { name: "Se connecter" })).toHaveAttribute("href", "/login?next=%2F");
  });

  it("threads a relative ?next= through the links and rejects unsafe ones", () => {
    const first = mount("/signup?next=%2Ffiles%3Ftab%3D2");
    expect(screen.getByRole("link", { name: "Se connecter" })).toHaveAttribute("href", "/login?next=%2Ffiles%3Ftab%3D2");
    expect(screen.getByRole("link", { name: "S'inscrire avec Microsoft" })).toHaveAttribute(
      "href",
      "/auth/oauth/microsoft/start?next=%2Ffiles%3Ftab%3D2",
    );
    first.unmount();
    mount("/signup?next=//evil.example");
    expect(screen.getByRole("link", { name: "Se connecter" })).toHaveAttribute("href", "/login?next=%2F");
  });

  it("rejects an empty form with every field error and no request", async () => {
    const { user } = mount();
    await user.click(submit());
    expect(screen.getByText("Nom requis")).toBeInTheDocument();
    expect(screen.getByText("Email invalide")).toBeInTheDocument();
    expect(screen.getByText("Mot de passe trop faible (8+ car., majuscule, chiffre)")).toBeInTheDocument();
    expect(nameBox()).toHaveAttribute("aria-invalid", "true");
    expect(emailBox()).toHaveAttribute("aria-invalid", "true");
    expect(passwordBox()).toHaveAttribute("aria-invalid", "true");
    // Empty password and empty confirm match, so no mismatch error yet.
    expect(screen.queryByText("Les mots de passe ne correspondent pas")).not.toBeInTheDocument();
    expect(confirmBox()).toHaveAttribute("aria-invalid", "false");
    expect(signupMock).not.toHaveBeenCalled();
  });

  it("flags a whitespace-only name and a password mismatch", async () => {
    const { user } = mount();
    await user.type(nameBox(), "   ");
    await user.type(emailBox(), "ann@example.com");
    await user.type(passwordBox(), "Str0ngpass");
    await user.type(confirmBox(), "Str0ngpas");
    await user.click(submit());
    expect(screen.getByText("Nom requis")).toBeInTheDocument();
    expect(screen.getByText("Les mots de passe ne correspondent pas")).toBeInTheDocument();
    expect(screen.queryByText("Email invalide")).not.toBeInTheDocument();
    expect(screen.queryByText(/trop faible/)).not.toBeInTheDocument();
    expect(confirmBox()).toHaveAttribute("aria-invalid", "true");
    expect(signupMock).not.toHaveBeenCalled();
  });

  it("colours the strength meter: red below 3 criteria, amber at 3, green at 4+", async () => {
    const { user, container } = mount();
    const grey = "rgb(229, 231, 235)";
    expect(bars(container)).toEqual([grey, grey, grey, grey, grey]);

    await user.type(passwordBox(), "abc"); // lowercase only → 1
    expect(bars(container)).toEqual(["rgb(220, 38, 38)", grey, grey, grey, grey]);

    await user.type(passwordBox(), "defghA"); // 8+ chars, upper, lower → 3
    expect(bars(container)).toEqual(["rgb(245, 158, 11)", "rgb(245, 158, 11)", "rgb(245, 158, 11)", grey, grey]);

    await user.type(passwordBox(), "1"); // + digit → 4
    expect(bars(container).filter((c) => c === "rgb(22, 163, 74)")).toHaveLength(4);

    await user.type(passwordBox(), "!"); // + symbol → 5
    expect(bars(container)).toEqual(Array(5).fill("rgb(22, 163, 74)"));
  });

  it("signs up with trimmed name, lower-cased email; toasts and redirects to next", async () => {
    const { user } = mount("/signup?next=%2Ffiles%3Ftab%3D2");
    await fillValid(user);
    await user.click(submit());
    await waitFor(() => expect(signupMock).toHaveBeenCalledTimes(1));
    expect(signupMock).toHaveBeenCalledWith({
      email: "ann@example.com",
      password: "Str0ngpass",
      name: "Ann Lee",
    });
    expect(await screen.findByRole("status")).toHaveTextContent("Compte créé");
    expect(await screen.findByTestId("location")).toHaveTextContent("/files?tab=2");
  });

  it("disables the button and shows 'Création…' while pending, then redirects to /", async () => {
    let resolve!: (v: unknown) => void;
    signupMock.mockImplementation(() => new Promise((r) => (resolve = r)));
    const { user } = mount();
    await fillValid(user);
    await user.click(submit());
    const pending = await screen.findByRole("button", { name: "Création…" });
    expect(pending).toBeDisabled();
    await user.click(pending);
    expect(signupMock).toHaveBeenCalledTimes(1);
    resolve({ token: "jwt" });
    expect(await screen.findByTestId("location")).toHaveTextContent("/");
  });

  it("shows the server error in a toast and re-enables the form", async () => {
    signupMock.mockRejectedValue(new Error("email already used"));
    const { user } = mount();
    await fillValid(user);
    await user.click(submit());
    expect(await screen.findByRole("status")).toHaveTextContent("email already used");
    expect(submit()).toBeEnabled();
    expect(screen.queryByTestId("location")).not.toBeInTheDocument();
  });

  it("falls back to a generic error toast when the error has no message", async () => {
    signupMock.mockRejectedValue({});
    const { user } = mount();
    await fillValid(user);
    await user.click(submit());
    expect(await screen.findByRole("status")).toHaveTextContent("Échec de l'inscription");
  });
});
