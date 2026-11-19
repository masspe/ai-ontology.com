// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// Settings → Sign-in: the OpenID Connect providers (Microsoft Entra ID,
// Google, any provider with a discovery document). Edited as a draft and
// saved as a whole; a secret is write-only (the server shows a hint).
import { useEffect, useState } from "react";
import Card from "./Card";
import type { SigninProvider, SigninProviderPatch, SigninSettings } from "../api";
import { t } from "../lib/i18n";

interface Props {
  signin: SigninSettings | undefined;
  onPatch: (patch: { public_url?: string; providers?: SigninProviderPatch[] }) => Promise<void> | void;
}

/** A provider as edited: the lists as comma-separated text, the secret as typed. */
interface Draft {
  id: string;
  label: string;
  issuer: string;
  client_id: string;
  client_secret: string;
  client_secret_hint: string;
  allowed_domains: string;
  admin_groups: string;
  enabled: boolean;
}

const PRESETS: Record<string, Partial<Draft>> = {
  microsoft: {
    label: "Microsoft",
    issuer: "https://login.microsoftonline.com/<tenant-id>/v2.0",
  },
  google: { label: "Google", issuer: "https://accounts.google.com" },
};

function toDraft(p: SigninProvider): Draft {
  return {
    id: p.id,
    label: p.label ?? "",
    issuer: p.issuer ?? "",
    client_id: p.client_id ?? "",
    client_secret: "",
    client_secret_hint: p.client_secret_hint ?? "",
    allowed_domains: (p.allowed_domains ?? []).join(", "),
    admin_groups: (p.admin_groups ?? []).join(", "),
    enabled: p.enabled ?? true,
  };
}

const list = (s: string) => s.split(",").map((x) => x.trim()).filter(Boolean);

export function toPatch(d: Draft): SigninProviderPatch {
  return {
    id: d.id,
    label: d.label,
    issuer: d.issuer,
    client_id: d.client_id,
    client_secret: d.client_secret || undefined,
    allowed_domains: list(d.allowed_domains),
    admin_groups: list(d.admin_groups),
    enabled: d.enabled,
  };
}

export default function SigninSection({ signin, onPatch }: Props) {
  const [publicUrl, setPublicUrl] = useState(signin?.public_url ?? "");
  const [rows, setRows] = useState<Draft[]>((signin?.providers ?? []).map(toDraft));
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    setPublicUrl(signin?.public_url ?? "");
    setRows((signin?.providers ?? []).map(toDraft));
  }, [signin]);

  const update = (i: number, patch: Partial<Draft>) =>
    setRows((rs) => rs.map((r, j) => (j === i ? { ...r, ...patch } : r)));

  const add = (id: string) => {
    const base = id.replace(/[^a-z0-9-]/g, "");
    if (!base || rows.some((r) => r.id === base)) return;
    setRows((rs) => [
      ...rs,
      {
        id: base,
        label: base,
        issuer: "",
        client_id: "",
        client_secret: "",
        client_secret_hint: "",
        allowed_domains: "",
        admin_groups: "",
        enabled: true,
        ...PRESETS[base],
      },
    ]);
  };

  const save = async () => {
    setSaving(true);
    try {
      await onPatch({ public_url: publicUrl.trim(), providers: rows.map(toPatch) });
    } finally {
      setSaving(false);
    }
  };

  const origin = publicUrl.trim().replace(/\/+$/, "") || (typeof window !== "undefined" ? window.location.origin : "");

  return (
    <Card
      title={
        <span style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
          <span style={{ color: "var(--accent)" }}>⚿</span>
          {t("Connexion par identité d'entreprise (OpenID Connect)")}
        </span>
      }
      subtitle={t("Microsoft Entra ID, Google ou tout fournisseur OpenID Connect. Les comptes sont créés à la première connexion pour les domaines autorisés ; les groupes peuvent donner le rôle administrateur.")}
    >
      <label className="field">
        <span>{t("URL publique de ce serveur")}</span>
        <input
          type="url"
          placeholder="https://ontology.example.com"
          value={publicUrl}
          onChange={(e) => setPublicUrl(e.target.value)}
        />
        <small className="muted">{t("Le fournisseur renvoie l'utilisateur à cette adresse ; elle doit correspondre à l'URI de redirection enregistrée chez lui.")}</small>
      </label>

      {rows.map((r, i) => (
        <fieldset key={r.id} className="provider-row" style={{ border: "1px solid var(--border)", borderRadius: "var(--radius-sm)", padding: 12, marginTop: 12 }}>
          <legend style={{ fontWeight: 600 }}>
            {r.label || r.id} <code className="muted">({r.id})</code>
          </legend>
          <div className="grid grid-2" style={{ gap: 12 }}>
            <label className="field">
              <span>{t("Libellé du bouton")}</span>
              <input type="text" value={r.label} onChange={(e) => update(i, { label: e.target.value })} />
            </label>
            <label className="field">
              <span>{t("Émetteur (issuer)")}</span>
              <input type="text" value={r.issuer} onChange={(e) => update(i, { issuer: e.target.value })} />
            </label>
            <label className="field">
              <span>{t("Identifiant client")}</span>
              <input type="text" value={r.client_id} onChange={(e) => update(i, { client_id: e.target.value })} />
            </label>
            <label className="field">
              <span>{t("Secret client")}</span>
              <input
                type="password"
                autoComplete="new-password"
                placeholder={r.client_secret_hint || t("non défini")}
                value={r.client_secret}
                onChange={(e) => update(i, { client_secret: e.target.value })}
              />
            </label>
            <label className="field">
              <span>{t("Domaines autorisés (création de compte)")}</span>
              <input type="text" placeholder="example.com, example.ch" value={r.allowed_domains} onChange={(e) => update(i, { allowed_domains: e.target.value })} />
            </label>
            <label className="field">
              <span>{t("Groupes administrateurs")}</span>
              <input type="text" value={r.admin_groups} onChange={(e) => update(i, { admin_groups: e.target.value })} />
            </label>
          </div>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginTop: 8, gap: 8, flexWrap: "wrap" }}>
            <small className="muted">
              {t("URI de redirection à enregistrer :")} <code>{`${origin}/auth/oauth/${r.id}/callback`}</code>
            </small>
            <span style={{ display: "inline-flex", gap: 12, alignItems: "center" }}>
              <label style={{ display: "inline-flex", gap: 6, alignItems: "center" }}>
                <input type="checkbox" checked={r.enabled} onChange={(e) => update(i, { enabled: e.target.checked })} />
                {t("Actif")}
              </label>
              <button type="button" className="btn btn-ghost" onClick={() => setRows((rs) => rs.filter((_, j) => j !== i))}>
                {t("Retirer")}
              </button>
            </span>
          </div>
        </fieldset>
      ))}

      <div style={{ display: "flex", gap: 8, marginTop: 12, flexWrap: "wrap" }}>
        <button type="button" className="btn btn-ghost" onClick={() => add("microsoft")}>
          {t("Ajouter Microsoft Entra ID")}
        </button>
        <button type="button" className="btn btn-ghost" onClick={() => add("google")}>
          {t("Ajouter Google")}
        </button>
        <button
          type="button"
          className="btn btn-ghost"
          onClick={() => {
            const id = window.prompt(t("Identifiant du fournisseur (lettres minuscules, chiffres, tirets)"), "");
            if (id) add(id.trim().toLowerCase());
          }}
        >
          {t("Ajouter un autre fournisseur")}
        </button>
        <span style={{ flex: 1 }} />
        <button type="button" className="btn btn-primary" disabled={saving} onClick={save}>
          {saving ? t("Enregistrement…") : t("Enregistrer la connexion")}
        </button>
      </div>
    </Card>
  );
}
