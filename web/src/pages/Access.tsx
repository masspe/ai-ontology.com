// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// Access: the accounts, their role and their clearances — the sheet
// labels each one may see. Administrators only; everyone else is told so.
import { useEffect, useState } from "react";
import Card from "../components/Card";
// @ts-expect-error JSX module
import { useConfirm } from "../components/ConfirmDialog.jsx";
// @ts-expect-error JS module
import { msBE } from "../lib/msBE";
import { t } from "../lib/i18n";

export interface AccountRow {
  id: string;
  email: string;
  name: string;
  role: string;
  clearances: string[];
  createdAt?: string;
}

const list = (s: string) =>
  s
    .split(",")
    .map((x) => x.trim().toLowerCase())
    .filter(Boolean);

export default function Access() {
  const confirm = useConfirm();
  const [rows, setRows] = useState<AccountRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [newUser, setNewUser] = useState({ email: "", name: "", password: "" });
  const me = msBE.auth.currentUser() as { id?: string } | null;

  const load = async () => {
    try {
      const data = (await msBE.request("GET", "/auth/users")) as { users: AccountRow[] };
      setRows(data.users);
      setDrafts(Object.fromEntries(data.users.map((u) => [u.id, u.clearances.join(", ")])));
      setError(null);
    } catch (e: unknown) {
      setRows([]);
      setError(e instanceof Error ? e.message : String(e));
    }
  };
  useEffect(() => {
    void load();
  }, []);

  const patch = async (id: string, body: { role?: string; clearances?: string[] }) => {
    try {
      const data = (await msBE.request("PATCH", `/auth/users/${id}`, body)) as { user: AccountRow };
      setRows((rs) => (rs ?? []).map((r) => (r.id === id ? data.user : r)));
      setDrafts((d) => ({ ...d, [id]: data.user.clearances.join(", ") }));
      setError(null);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const remove = async (u: AccountRow) => {
    if (!(await confirm({ title: t("Supprimer ce compte ?"), message: u.email }))) return;
    try {
      await msBE.request("DELETE", `/auth/users/${u.id}`);
      setRows((rs) => (rs ?? []).filter((r) => r.id !== u.id));
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const create = async (ev: React.FormEvent) => {
    ev.preventDefault();
    try {
      await msBE.request("POST", "/auth/users", newUser);
      setNewUser({ email: "", name: "", password: "" });
      await load();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <>
      <div className="page-header">
        <div>
          <h1 className="page-title">{t("Accès")}</h1>
          <p className="page-subtitle">
            {t("Qui voit quoi : chaque fiche peut porter des labels ; un compte voit une fiche quand ses habilitations couvrent tous ses labels. Les administrateurs voient tout.")}
          </p>
        </div>
      </div>
      {error && <div className="error-banner">{error}</div>}

      <Card title={t("Comptes")}>
        {rows === null ? (
          <p className="muted">{t("Chargement…")}</p>
        ) : rows.length === 0 ? (
          <p className="muted">{t("Aucun compte visible — réservé aux administrateurs.")}</p>
        ) : (
          <table className="table">
            <thead>
              <tr>
                <th>{t("Compte")}</th>
                <th>{t("Rôle")}</th>
                <th>{t("Habilitations (labels, séparés par des virgules)")}</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {rows.map((u) => (
                <tr key={u.id}>
                  <td>
                    <strong>{u.name}</strong>
                    <br />
                    <span className="muted">{u.email}</span>
                  </td>
                  <td>
                    <select
                      aria-label={`${t("Rôle")} ${u.email}`}
                      value={u.role}
                      disabled={u.id === me?.id}
                      onChange={(e) => void patch(u.id, { role: e.target.value })}
                    >
                      <option value="user">{t("utilisateur")}</option>
                      <option value="admin">{t("administrateur")}</option>
                    </select>
                  </td>
                  <td>
                    <input
                      aria-label={`${t("Habilitations")} ${u.email}`}
                      value={drafts[u.id] ?? ""}
                      placeholder={u.role === "admin" ? t("voit tout") : "hr, finance"}
                      disabled={u.role === "admin"}
                      onChange={(e) => setDrafts((d) => ({ ...d, [u.id]: e.target.value }))}
                      onBlur={() => {
                        const next = list(drafts[u.id] ?? "");
                        if (next.join(",") !== u.clearances.join(",")) void patch(u.id, { clearances: next });
                      }}
                    />
                  </td>
                  <td>
                    <button
                      className="btn-ghost"
                      type="button"
                      disabled={u.id === me?.id}
                      onClick={() => void remove(u)}
                    >
                      {t("Supprimer")}
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>

      <Card title={t("Nouveau compte")} subtitle={t("Un compte local ; les comptes d'entreprise arrivent seuls à la première connexion (Réglages → Connexion).")}>
        <form onSubmit={(e) => void create(e)} className="grid grid-3" style={{ gap: 12 }}>
          <label className="field">
            <span>{t("Email")}</span>
            <input type="email" required value={newUser.email} onChange={(e) => setNewUser((n) => ({ ...n, email: e.target.value }))} />
          </label>
          <label className="field">
            <span>{t("Nom")}</span>
            <input type="text" value={newUser.name} onChange={(e) => setNewUser((n) => ({ ...n, name: e.target.value }))} />
          </label>
          <label className="field">
            <span>{t("Mot de passe")}</span>
            <input type="password" required autoComplete="new-password" value={newUser.password} onChange={(e) => setNewUser((n) => ({ ...n, password: e.target.value }))} />
          </label>
          <div>
            <button className="btn-primary" type="submit">
              {t("Créer le compte")}
            </button>
          </div>
        </form>
      </Card>
    </>
  );
}
