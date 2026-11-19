// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// Settings → Sign-in: the provider list is edited as a draft (presets,
// removal, the secret write-only) and saved as a whole patch.

import { describe, expect, it, vi } from "vitest";
import SigninSection, { toPatch } from "./SigninSection";
import { fireEvent, renderPage, screen, waitFor } from "../test/render";
import type { SigninSettings } from "../api";

const stored: SigninSettings = {
  public_url: "https://ontology.example",
  providers: [
    {
      id: "microsoft",
      label: "Entra",
      issuer: "https://login.microsoftonline.com/t/v2.0",
      client_id: "cid",
      client_secret_hint: "sec...3456",
      scopes: "openid email profile",
      allowed_domains: ["example.com"],
      admin_groups: ["admins"],
      enabled: true,
    },
  ],
};

describe("SigninSection", () => {
  it("shows the stored providers, the redirect URI and the secret as a hint only", () => {
    renderPage(<SigninSection signin={stored} onPatch={vi.fn()} />);
    expect(screen.getByDisplayValue("https://ontology.example")).toBeInTheDocument();
    expect(screen.getByDisplayValue("Entra")).toBeInTheDocument();
    expect(screen.getByDisplayValue("example.com")).toBeInTheDocument();
    expect(screen.getByLabelText("Secret client")).toHaveValue("");
    expect(screen.getByLabelText("Secret client")).toHaveAttribute("placeholder", "sec...3456");
    expect(screen.getByText("https://ontology.example/auth/oauth/microsoft/callback")).toBeInTheDocument();
  });

  it("adds presets once, removes a row and saves the whole list", async () => {
    const onPatch = vi.fn().mockResolvedValue(undefined);
    renderPage(<SigninSection signin={{ public_url: "", providers: [] }} onPatch={onPatch} />);
    expect(screen.queryByLabelText("Émetteur (issuer)")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Ajouter Google" }));
    fireEvent.click(screen.getByRole("button", { name: "Ajouter Google" }));
    fireEvent.click(screen.getByRole("button", { name: "Ajouter Microsoft Entra ID" }));
    expect(screen.getAllByLabelText("Émetteur (issuer)")).toHaveLength(2);
    expect(screen.getByDisplayValue("https://accounts.google.com")).toBeInTheDocument();
    // Without a public URL the page's origin is shown as the redirect base.
    expect(screen.getByText(`${window.location.origin}/auth/oauth/google/callback`)).toBeInTheDocument();

    const [googleId] = screen.getAllByLabelText("Identifiant client");
    fireEvent.change(googleId, { target: { value: "g-client" } });
    const [googleSecret] = screen.getAllByLabelText("Secret client");
    fireEvent.change(googleSecret, { target: { value: "g-secret" } });
    const [googleDomains] = screen.getAllByLabelText("Domaines autorisés (création de compte)");
    fireEvent.change(googleDomains, { target: { value: " example.com,, example.ch " } });
    fireEvent.change(screen.getByPlaceholderText("https://ontology.example.com"), {
      target: { value: "https://o.example/" },
    });

    fireEvent.click(screen.getAllByRole("button", { name: "Retirer" })[1]);
    expect(screen.getAllByLabelText("Émetteur (issuer)")).toHaveLength(1);

    fireEvent.click(screen.getByRole("button", { name: "Enregistrer la connexion" }));
    await waitFor(() => expect(onPatch).toHaveBeenCalledTimes(1));
    expect(onPatch).toHaveBeenCalledWith({
      public_url: "https://o.example/",
      providers: [
        {
          id: "google",
          label: "Google",
          issuer: "https://accounts.google.com",
          client_id: "g-client",
          client_secret: "g-secret",
          allowed_domains: ["example.com", "example.ch"],
          admin_groups: [],
          enabled: true,
        },
      ],
    });
  });

  it("adds a custom provider from the prompt and keeps a blank secret out of the patch", () => {
    const prompt = vi.spyOn(window, "prompt").mockReturnValue(" Okta-Prod ");
    renderPage(<SigninSection signin={undefined} onPatch={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Ajouter un autre fournisseur" }));
    expect(screen.getByText("(okta-prod)")).toBeInTheDocument();
    prompt.mockReturnValue(null);
    fireEvent.click(screen.getByRole("button", { name: "Ajouter un autre fournisseur" }));
    expect(screen.getAllByLabelText("Émetteur (issuer)")).toHaveLength(1);
    fireEvent.click(screen.getByLabelText("Actif"));
    prompt.mockRestore();

    expect(
      toPatch({
        id: "x",
        label: "X",
        issuer: "https://x",
        client_id: "c",
        client_secret: "",
        client_secret_hint: "",
        allowed_domains: "",
        admin_groups: "a, b",
        enabled: false,
      }),
    ).toEqual({
      id: "x",
      label: "X",
      issuer: "https://x",
      client_id: "c",
      client_secret: undefined,
      allowed_domains: [],
      admin_groups: ["a", "b"],
      enabled: false,
    });
  });
});
