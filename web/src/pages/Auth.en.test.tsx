// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The sign-in, sign-up and OAuth callback pages in English.

import { describe, expect, it, vi } from "vitest";
import type { ComponentType } from "react";
// @ts-expect-error JSX module
import LoginPage from "./Login.jsx";
// @ts-expect-error JSX module
import SignupPage from "./Signup.jsx";
// @ts-expect-error JSX module
import OAuthCallbackPage from "./OAuthCallback.jsx";
import { setLang } from "../lib/i18n";
import { renderPage, screen } from "../test/render";

vi.mock("../lib/msBE", async () => {
  const actual = await vi.importActual<{ msBE: { auth: Record<string, unknown> } }>("../lib/msBE");
  const auth = { ...actual.msBE.auth, me: vi.fn(() => new Promise(() => {})), consumeOAuthToken: vi.fn(() => true) };
  const mocked = { ...actual.msBE, auth };
  return { ...actual, msBE: mocked, default: mocked };
});

const Login = LoginPage as ComponentType;
const Signup = SignupPage as ComponentType;
const OAuthCallback = OAuthCallbackPage as ComponentType;

describe("authentication pages in English", () => {
  it("Login: heading, fields, validation message and links", async () => {
    setLang("en", false);
    const { user } = renderPage(<Login />, { route: "/login", path: "/login" });
    expect(screen.getByRole("heading", { name: "Sign in" })).toBeInTheDocument();
    expect(screen.getByLabelText(/^Password/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Continue with Google" })).toBeInTheDocument();
    expect(screen.getByText(/No account yet\?/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    expect(screen.getByText("Invalid email")).toBeInTheDocument();
    expect(screen.getByText("Password required")).toBeInTheDocument();
  });

  it("Signup: heading, labels and validation message", async () => {
    setLang("en", false);
    const { user } = renderPage(<Signup />, { route: "/signup", path: "/signup" });
    expect(screen.getByRole("heading", { name: "Create an account" })).toBeInTheDocument();
    expect(screen.getByLabelText(/^Confirm/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Sign up with Microsoft" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Create my account" }));
    expect(screen.getByText("Name required")).toBeInTheDocument();
  });

  it("OAuthCallback: the finishing message", () => {
    setLang("en", false);
    renderPage(<OAuthCallback />, { route: "/oauth/callback", path: "/oauth/callback" });
    expect(screen.getByText("Finishing sign-in…")).toBeInTheDocument();
  });
});
