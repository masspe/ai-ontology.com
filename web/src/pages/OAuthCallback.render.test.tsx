// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Rendering tests of the OAuth callback page: token taken from the URL
// fragment (preferred) or the query string, `next` sanitising, the error
// parameter, the missing-token case, and the address-bar clean-up.

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ComponentType } from "react";
import { Route, useLocation } from "react-router-dom";
// @ts-expect-error JSX module
import OAuthCallbackPage from "./OAuthCallback.jsx";
// @ts-expect-error JS module
import { msBE } from "../lib/msBE";
import { renderPage, screen } from "../test/render";

vi.mock("../lib/msBE", async () => {
  const actual = await vi.importActual<{ msBE: { auth: Record<string, unknown> } }>("../lib/msBE");
  const auth = { ...actual.msBE.auth, me: vi.fn(), consumeOAuthToken: vi.fn() };
  const mocked = { ...actual.msBE, auth };
  return { ...actual, msBE: mocked, default: mocked };
});

const OAuthCallback = OAuthCallbackPage as ComponentType;
const auth = (msBE as { auth: { me: ReturnType<typeof vi.fn>; consumeOAuthToken: ReturnType<typeof vi.fn> } }).auth;

function ShowLocation() {
  const loc = useLocation();
  return <div data-testid="location">{loc.pathname + loc.search}</div>;
}
const targets = (
  <>
    <Route path="/login" element={<ShowLocation />} />
    <Route path="/files" element={<ShowLocation />} />
    <Route path="/" element={<ShowLocation />} />
  </>
);

function mount(route: string) {
  return renderPage(<OAuthCallback />, { route, path: "/oauth/callback", extraRoutes: targets });
}

beforeEach(() => {
  auth.me.mockResolvedValue({ id: 1 });
  auth.consumeOAuthToken.mockImplementation((t: string | null) => Boolean(t));
});

describe("OAuthCallback page", () => {
  it("shows the finalising message while the session is loaded", () => {
    auth.me.mockImplementation(() => new Promise(() => {}));
    mount("/oauth/callback#token=jwt-f");
    expect(screen.getByText("Finalisation de la connexion…")).toBeInTheDocument();
    expect(auth.consumeOAuthToken).toHaveBeenCalledWith("jwt-f");
    expect(auth.me).toHaveBeenCalledTimes(1);
  });

  it("consumes the fragment token, loads the profile and redirects to next", async () => {
    mount("/oauth/callback#token=jwt-f&next=%2Ffiles%3Ftab%3D2");
    expect(await screen.findByTestId("location")).toHaveTextContent("/files?tab=2");
    expect(auth.consumeOAuthToken).toHaveBeenCalledExactlyOnceWith("jwt-f");
    expect(auth.me).toHaveBeenCalledTimes(1);
  });

  it("falls back to the query string when the fragment is empty", async () => {
    mount("/oauth/callback?token=jwt-q&next=%2Ffiles");
    expect(await screen.findByTestId("location")).toHaveTextContent("/files");
    expect(auth.consumeOAuthToken).toHaveBeenCalledWith("jwt-q");
  });

  it("prefers the fragment over the query string", async () => {
    mount("/oauth/callback?token=jwt-q&next=%2F#token=jwt-f&next=%2Ffiles");
    expect(await screen.findByTestId("location")).toHaveTextContent("/files");
    expect(auth.consumeOAuthToken).toHaveBeenCalledWith("jwt-f");
  });

  it("redirects to next even when loading the profile fails", async () => {
    auth.me.mockRejectedValue(new Error("boom"));
    mount("/oauth/callback#token=jwt-f&next=%2Ffiles");
    expect(await screen.findByTestId("location")).toHaveTextContent("/files");
  });

  it.each([
    ["missing", ""],
    ["protocol-relative", "&next=//evil.example"],
    ["absolute", "&next=https%3A%2F%2Fevil.example"],
  ])("redirects to / when next is %s", async (_label, next) => {
    mount(`/oauth/callback#token=jwt-f${next}`);
    expect(await screen.findByTestId("location")).toHaveTextContent("/");
    expect(screen.getByTestId("location").textContent).toBe("/");
  });

  it("on ?error= toasts the provider error and goes back to /login without touching the token", async () => {
    mount("/oauth/callback#error=access_denied&token=jwt-f");
    expect(await screen.findByRole("status")).toHaveTextContent("OAuth: access_denied");
    expect(await screen.findByTestId("location")).toHaveTextContent("/login");
    expect(auth.consumeOAuthToken).not.toHaveBeenCalled();
    expect(auth.me).not.toHaveBeenCalled();
  });

  it("reads the error from the query string too", async () => {
    mount("/oauth/callback?error=state_mismatch");
    expect(await screen.findByRole("status")).toHaveTextContent("OAuth: state_mismatch");
    expect(await screen.findByTestId("location")).toHaveTextContent("/login");
  });

  it("without a token: toasts 'Jeton OAuth manquant' and goes to /login", async () => {
    mount("/oauth/callback#next=%2Ffiles");
    expect(await screen.findByRole("status")).toHaveTextContent("Jeton OAuth manquant");
    expect(await screen.findByTestId("location")).toHaveTextContent("/login");
    expect(auth.consumeOAuthToken).toHaveBeenCalledWith(null);
    expect(auth.me).not.toHaveBeenCalled();
  });

  it("wipes a real address-bar fragment through history.replaceState", async () => {
    vi.stubGlobal("location", { ...window.location, pathname: "/oauth/callback", hash: "#token=jwt-f" });
    const replaceState = vi.spyOn(window.history, "replaceState").mockImplementation(() => {});
    mount("/oauth/callback#token=jwt-f");
    expect(await screen.findByTestId("location")).toHaveTextContent("/");
    expect(replaceState).toHaveBeenCalledWith(null, "", "/oauth/callback");
  });

  it("ignores a replaceState failure", async () => {
    vi.stubGlobal("location", { ...window.location, pathname: "/oauth/callback", hash: "#token=jwt-f" });
    vi.spyOn(window.history, "replaceState").mockImplementation(() => {
      throw new Error("SecurityError");
    });
    mount("/oauth/callback#token=jwt-f");
    expect(await screen.findByTestId("location")).toHaveTextContent("/");
  });

  it("leaves history alone when the address bar has no fragment", async () => {
    const replaceState = vi.spyOn(window.history, "replaceState");
    mount("/oauth/callback?token=jwt-q");
    expect(await screen.findByTestId("location")).toHaveTextContent("/");
    expect(replaceState).not.toHaveBeenCalledWith(null, "", expect.anything());
  });
});
