// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// Access: the accounts, their role and their clearances — the sheet
// labels each one may see — and the API keys with their scope.
// Administrators only; everyone else is told so.
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

export interface KeyRow {
  id: string;
  name: string;
  prefix: string;
  createdBy: string;
  /** `null`: sees everything. */
  clearances: string[] | null;
  readOnly: boolean;
  /** Unix seconds; `null`: never. */
  expiresAt: number | null;
  lastUsedAt: string | null;
}

/** The API keys: what each may see and do, until when, last used; a new or
 * rotated secret is shown once. */
function ApiKeys() {
  const confirm = useConfirm();
  const [keys, setKeys] = useState<KeyRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [draft, setDraft] = useState({ name: "", clearances: "", readOnly: false, days: "" });
  const [secret, setSecret] = useState<{ name: string; key: string } | null>(null);

  const run = async (f: () => Promise<unknown>) => {
    try {
      await f();
      setKeys(((await msBE.request("GET", "/auth/keys")) as { keys: KeyRow[] }).keys);
      setError(null);
    } catch (e: unknown) {
      setKeys((k) => k ?? []);
      setError(e instanceof Error ? e.message : String(e));
    }
  };
  useEffect(() => {
    void run(async () => undefined);
  }, []);

  const create = (ev: React.FormEvent) => {
    ev.preventDefault();
    void run(async () => {
      const made = (await msBE.request("POST", "/auth/keys", {
        name: draft.name,
        // Empty: the key sees everything, as an administrator does.
        ...(draft.clearances.trim() ? { clearances: list(draft.clearances) } : {}),
        read_only: draft.readOnly,
        ...(draft.days ? { expires_in_days: Number(draft.days) } : {}),
      })) as { name: string; key: string };
      setSecret({ name: made.name, key: made.key });
      setDraft({ name: "", clearances: "", readOnly: false, days: "" });
    });
  };

  const rotate = async (k: KeyRow) => {
    if (!(await confirm({ title: t("Remplacer le secret de cette clé ?"), message: t("{name} : l'ancien secret cesse de fonctionner.", { name: k.name }) }))) return;
    void run(async () => {
      const made = (await msBE.request("POST", `/auth/keys/${k.id}/rotate`)) as { name: string; key: string };
      setSecret({ name: made.name, key: made.key });
    });
  };

  const revoke = async (k: KeyRow) => {
    if (!(await confirm({ title: t("Révoquer cette clé ?"), message: k.name, danger: true }))) return;
    void run(() => msBE.request("DELETE", `/auth/keys/${k.id}`));
  };

  const now = Date.now() / 1000;
  return (
    <Card title={t("Clés d'API")} subtitle={t("Pour les programmes : une clé voit ce que ses habilitations couvrent, écrit ou non, jusqu'à son expiration.")}>
      {error && <div className="error-banner">{error}</div>}
      {secret && (
        <div className="success-banner" role="status">
          {t("Clé {name} — copiez-la maintenant, elle ne sera plus affichée :", { name: secret.name })} <code>{secret.key}</code>{" "}
          <button className="btn-ghost" type="button" onClick={() => setSecret(null)}>
            {t("Fermer")}
          </button>
        </div>
      )}
      {keys === null ? (
        <p className="muted">{t("Chargement…")}</p>
      ) : keys.length === 0 ? (
        <p className="muted">{t("Aucune clé.")}</p>
      ) : (
        <table className="table">
          <thead>
            <tr>
              <th>{t("Clé")}</th>
              <th>{t("Habilitations")}</th>
              <th>{t("Expiration")}</th>
              <th>{t("Dernière utilisation")}</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {keys.map((k) => (
              <tr key={k.id}>
                <td>
                  <strong>{k.name}</strong> {k.readOnly && <span className="badge">{t("lecture seule")}</span>}
                  <br />
                  <code>{k.prefix}…</code> <span className="muted">{k.createdBy}</span>
                </td>
                <td>{k.clearances === null ? t("voit tout") : k.clearances.join(", ") || t("fiches sans label seulement")}</td>
                <td>
                  {k.expiresAt === null ? (
                    <span className="muted">{t("jamais")}</span>
                  ) : k.expiresAt <= now ? (
                    <span className="badge badge-danger">{t("expirée")}</span>
                  ) : (
                    new Date(k.expiresAt * 1000).toISOString().slice(0, 10)
                  )}
                </td>
                <td>{k.lastUsedAt ? k.lastUsedAt.slice(0, 16).replace("T", " ") : <span className="muted">{t("jamais")}</span>}</td>
                <td style={{ whiteSpace: "nowrap" }}>
                  <button className="btn-ghost" type="button" onClick={() => void rotate(k)}>
                    {t("Renouveler")}
                  </button>
                  <button className="btn-ghost" type="button" onClick={() => void revoke(k)}>
                    {t("Révoquer")}
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <form onSubmit={create} className="grid grid-3" style={{ gap: 12, marginTop: 12 }}>
        <label className="field">
          <span>{t("Nom de la clé")}</span>
          <input required maxLength={80} value={draft.name} onChange={(e) => setDraft((d) => ({ ...d, name: e.target.value }))} />
        </label>
        <label className="field">
          <span>{t("Habilitations de la clé (vide : voit tout)")}</span>
          <input value={draft.clearances} placeholder="hr, finance" onChange={(e) => setDraft((d) => ({ ...d, clearances: e.target.value }))} />
        </label>
        <label className="field">
          <span>{t("Expire dans (jours, vide : jamais)")}</span>
          <input type="number" min={1} max={3650} value={draft.days} onChange={(e) => setDraft((d) => ({ ...d, days: e.target.value }))} />
        </label>
        <label style={{ display: "flex", gap: 6, alignItems: "center" }}>
          <input type="checkbox" checked={draft.readOnly} onChange={(e) => setDraft((d) => ({ ...d, readOnly: e.target.checked }))} /> {t("Lecture seule")}
        </label>
        <div>
          <button className="btn-primary" type="submit">
            {t("Créer la clé")}
          </button>
        </div>
      </form>
    </Card>
  );
}

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

      {/* Administrators only, as the listing above. */}
      {rows && rows.length > 0 && <ApiKeys />}
    </>
  );
}
