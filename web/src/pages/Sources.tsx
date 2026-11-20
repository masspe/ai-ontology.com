// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// Data sources: a database or a file the graph is kept in step with.
// A source (where, how to connect) carries mappings (a table → sheets
// of a type, with links from its foreign keys), previewed before they
// run, run by hand or on a schedule as background jobs. Administrators.
import { useEffect, useState } from "react";
import Card from "../components/Card";
// @ts-expect-error JSX module
import { useConfirm } from "../components/ConfirmDialog.jsx";
import {
  createMapping,
  createSource,
  deleteMapping,
  deleteSource,
  listSourceColumns,
  listSources,
  listSourceTables,
  previewMapping,
  runMapping,
  testSource,
  updateMapping,
  updateSource,
  type DataSource,
  type MappingPatch,
  type MappingSpec,
  type SourcePatch,
} from "../api";
import { t } from "../lib/i18n";

const KINDS = ["csv", "sqlserver", "odbc"] as const;

const list = (s: string) => s.split(",").map((x) => x.trim()).filter(Boolean);

function SourceForm({ initial, onSave, onCancel }: { initial?: DataSource; onSave: (p: SourcePatch) => Promise<void>; onCancel: () => void }) {
  const [p, setP] = useState<SourcePatch>({
    name: initial?.name ?? "",
    kind: initial?.kind ?? "csv",
    location: initial?.kind === "odbc" ? "" : initial?.location ?? "",
    host: initial?.host ?? "",
    port: initial?.port ?? 1433,
    database: initial?.database ?? "",
    user: initial?.user ?? "",
    password: "",
    encrypt: initial?.encrypt ?? true,
    trust_cert: initial?.trust_cert ?? false,
    allow_private_hosts: initial?.allow_private_hosts ?? false,
    timeout_secs: initial?.timeout_secs ?? 60,
  });
  const set = <K extends keyof SourcePatch>(k: K, v: SourcePatch[K]) => setP((x) => ({ ...x, [k]: v }));
  const [busy, setBusy] = useState(false);
  return (
    <form
      className="source-form"
      onSubmit={(e) => {
        e.preventDefault();
        setBusy(true);
        void onSave(p).finally(() => setBusy(false));
      }}
    >
      <div className="grid grid-2" style={{ gap: 12 }}>
        <label className="field">
          <span>{t("Nom de la source")}</span>
          <input required value={p.name} onChange={(e) => set("name", e.target.value)} />
        </label>
        <label className="field">
          <span>{t("Type")}</span>
          <select value={p.kind} onChange={(e) => set("kind", e.target.value)}>
            {KINDS.map((k) => (
              <option key={k} value={k}>
                {k === "csv" ? t("Fichier CSV (chemin ou URL)") : k === "sqlserver" ? "SQL Server" : "ODBC (OpenEdge, …)"}
              </option>
            ))}
          </select>
        </label>
        {p.kind === "csv" && (
          <label className="field" style={{ gridColumn: "1 / -1" }}>
            <span>{t("Chemin ou URL HTTPS du fichier")}</span>
            <input value={p.location ?? ""} onChange={(e) => set("location", e.target.value)} placeholder="/data/exports/clients.csv" />
          </label>
        )}
        {p.kind === "odbc" && (
          <label className="field" style={{ gridColumn: "1 / -1" }}>
            <span>{t("Chaîne de connexion ODBC")}</span>
            <input
              type="password"
              autoComplete="off"
              value={p.location ?? ""}
              onChange={(e) => set("location", e.target.value)}
              placeholder={initial?.location || "Driver={Progress OpenEdge 12.2 driver};Host=…;Port=…;DB=…;UID=…;PWD=…"}
            />
          </label>
        )}
        {p.kind === "sqlserver" && (
          <>
            <label className="field">
              <span>{t("Hôte")}</span>
              <input value={p.host ?? ""} onChange={(e) => set("host", e.target.value)} />
            </label>
            <label className="field">
              <span>{t("Port")}</span>
              <input type="number" value={p.port ?? 1433} onChange={(e) => set("port", Number(e.target.value))} />
            </label>
            <label className="field">
              <span>{t("Base")}</span>
              <input value={p.database ?? ""} onChange={(e) => set("database", e.target.value)} />
            </label>
            <label className="field">
              <span>{t("Utilisateur (lecture seule)")}</span>
              <input value={p.user ?? ""} onChange={(e) => set("user", e.target.value)} />
            </label>
            <label className="field">
              <span>{t("Mot de passe")}</span>
              <input type="password" autoComplete="new-password" value={p.password ?? ""} onChange={(e) => set("password", e.target.value)} placeholder={initial?.password_hint || ""} />
            </label>
            <label style={{ display: "flex", gap: 6, alignItems: "center" }}>
              <input type="checkbox" checked={p.encrypt ?? true} onChange={(e) => set("encrypt", e.target.checked)} /> {t("Chiffrer la connexion (TLS)")}
            </label>
            <label style={{ display: "flex", gap: 6, alignItems: "center" }}>
              <input type="checkbox" checked={p.trust_cert ?? false} onChange={(e) => set("trust_cert", e.target.checked)} /> {t("Accepter un certificat auto-signé")}
            </label>
          </>
        )}
        <label style={{ display: "flex", gap: 6, alignItems: "center" }}>
          <input type="checkbox" checked={p.allow_private_hosts ?? false} onChange={(e) => set("allow_private_hosts", e.target.checked)} /> {t("Le serveur est sur le réseau privé")}
        </label>
        <label className="field">
          <span>{t("Délai (secondes)")}</span>
          <input type="number" value={p.timeout_secs ?? 60} onChange={(e) => set("timeout_secs", Number(e.target.value))} />
        </label>
      </div>
      <div className="onboarding-actions" style={{ marginTop: 12 }}>
        <button className="btn-primary" type="submit" disabled={busy}>
          {t("Enregistrer")}
        </button>
        <button className="btn-outline" type="button" onClick={onCancel}>
          {t("Annuler")}
        </button>
      </div>
    </form>
  );
}

function MappingForm({ source, initial, onSave, onCancel }: { source: DataSource; initial?: MappingSpec; onSave: (p: MappingPatch) => Promise<void>; onCancel: () => void }) {
  const [p, setP] = useState<MappingPatch>({
    name: initial?.name ?? "",
    from: initial?.from ?? "",
    concept_type: initial?.concept_type ?? "",
    key_column: initial?.key_column ?? "",
    name_column: initial?.name_column ?? "",
    description_column: initial?.description_column ?? "",
    property_columns: initial?.property_columns ?? [],
    relations: initial?.relations ?? [],
    watermark_column: initial?.watermark_column ?? "",
    schedule_minutes: initial?.schedule_minutes ?? 0,
  });
  const [tables, setTables] = useState<string[]>([]);
  const [columns, setColumns] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    listSourceTables(source.id)
      .then((r) => setTables(r.tables.map((x) => (x.schema ? `${x.schema}.${x.name}` : x.name))))
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  }, [source.id]);
  useEffect(() => {
    const from = (p.from ?? "").trim();
    if (!from || from.toLowerCase().startsWith("select")) return;
    listSourceColumns(source.id, from)
      .then((r) => setColumns(r.columns.map((c) => c.name)))
      .catch(() => setColumns([]));
  }, [source.id, p.from]);
  const set = <K extends keyof MappingPatch>(k: K, v: MappingPatch[K]) => setP((x) => ({ ...x, [k]: v }));
  const colList = columns.length ? <datalist id="src-columns">{columns.map((c) => <option key={c} value={c} />)}</datalist> : null;
  return (
    <form
      className="mapping-form"
      onSubmit={(e) => {
        e.preventDefault();
        void onSave({ ...p, name_column: p.name_column || null, description_column: p.description_column || null, watermark_column: p.watermark_column || null });
      }}
    >
      {error && <div className="error-banner">{error}</div>}
      {colList}
      <div className="grid grid-2" style={{ gap: 12 }}>
        <label className="field">
          <span>{t("Nom du mapping")}</span>
          <input required value={p.name ?? ""} onChange={(e) => set("name", e.target.value)} />
        </label>
        <label className="field">
          <span>{t("Table ou SELECT")}</span>
          <input required list="src-tables" value={p.from ?? ""} onChange={(e) => set("from", e.target.value)} />
          <datalist id="src-tables">{tables.map((x) => <option key={x} value={x} />)}</datalist>
        </label>
        <label className="field">
          <span>{t("Type de fiche")}</span>
          <input required value={p.concept_type ?? ""} onChange={(e) => set("concept_type", e.target.value)} placeholder="Client" />
        </label>
        <label className="field">
          <span>{t("Colonne clé")}</span>
          <input required list="src-columns" value={p.key_column ?? ""} onChange={(e) => set("key_column", e.target.value)} />
        </label>
        <label className="field">
          <span>{t("Colonne du nom (sinon la clé)")}</span>
          <input list="src-columns" value={p.name_column ?? ""} onChange={(e) => set("name_column", e.target.value)} />
        </label>
        <label className="field">
          <span>{t("Colonne de description")}</span>
          <input list="src-columns" value={p.description_column ?? ""} onChange={(e) => set("description_column", e.target.value)} />
        </label>
        <label className="field">
          <span>{t("Colonnes gardées en propriétés (vide : toutes)")}</span>
          <input value={(p.property_columns ?? []).join(", ")} onChange={(e) => set("property_columns", list(e.target.value))} />
        </label>
        <label className="field">
          <span>{t("Colonne de suivi (synchronisation incrémentale)")}</span>
          <input list="src-columns" value={p.watermark_column ?? ""} onChange={(e) => set("watermark_column", e.target.value)} />
        </label>
        <label className="field">
          <span>{t("Toutes les N minutes (0 : à la main)")}</span>
          <input type="number" min={0} value={p.schedule_minutes ?? 0} onChange={(e) => set("schedule_minutes", Number(e.target.value))} />
        </label>
      </div>
      <fieldset style={{ marginTop: 12, border: "1px solid var(--border)", borderRadius: "var(--radius-sm)", padding: 12 }}>
        <legend>{t("Liens depuis les clés étrangères")}</legend>
        {(p.relations ?? []).map((r, i) => (
          <div key={i} className="grid grid-3" style={{ gap: 8, marginBottom: 6 }}>
            <input aria-label={t("Colonne")} list="src-columns" placeholder={t("Colonne")} value={r.column} onChange={(e) => set("relations", (p.relations ?? []).map((x, j) => (j === i ? { ...x, column: e.target.value } : x)))} />
            <input aria-label={t("Type de lien")} placeholder={t("Type de lien")} value={r.relation_type} onChange={(e) => set("relations", (p.relations ?? []).map((x, j) => (j === i ? { ...x, relation_type: e.target.value } : x)))} />
            <span style={{ display: "flex", gap: 6 }}>
              <input aria-label={t("Type cible")} placeholder={t("Type cible")} value={r.target_type} onChange={(e) => set("relations", (p.relations ?? []).map((x, j) => (j === i ? { ...x, target_type: e.target.value } : x)))} />
              <button type="button" className="btn-ghost" onClick={() => set("relations", (p.relations ?? []).filter((_, j) => j !== i))}>
                ×
              </button>
            </span>
          </div>
        ))}
        <button type="button" className="btn-ghost" onClick={() => set("relations", [...(p.relations ?? []), { column: "", relation_type: "", target_type: "", create_target: true }])}>
          {t("Ajouter un lien")}
        </button>
      </fieldset>
      <div className="onboarding-actions" style={{ marginTop: 12 }}>
        <button className="btn-primary" type="submit">
          {t("Enregistrer")}
        </button>
        <button className="btn-outline" type="button" onClick={onCancel}>
          {t("Annuler")}
        </button>
      </div>
    </form>
  );
}

export default function Sources() {
  const confirm = useConfirm();
  const [sources, setSources] = useState<DataSource[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [editing, setEditing] = useState<number | null>(null);
  const [mappingFor, setMappingFor] = useState<{ source: DataSource; mapping?: MappingSpec } | null>(null);
  const [preview, setPreview] = useState<{ mapping: string; data: Awaited<ReturnType<typeof previewMapping>> } | null>(null);

  const load = async () => {
    try {
      setSources((await listSources()).sources);
      setError(null);
    } catch (e: unknown) {
      setSources([]);
      setError(e instanceof Error ? e.message : String(e));
    }
  };
  useEffect(() => {
    void load();
  }, []);

  const run = async (f: () => Promise<unknown>, done?: string) => {
    try {
      await f();
      setError(null);
      if (done) setInfo(done);
      await load();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <>
      <div className="page-header">
        <div>
          <h1 className="page-title">{t("Sources de données")}</h1>
          <p className="page-subtitle">
            {t("Une base ou un fichier dont le graphe est tenu à jour : chaque mapping transforme une table en fiches d'un type, avec des liens depuis ses clés étrangères. Lecture seule.")}
          </p>
        </div>
        {!adding && (
          <button className="btn-primary" onClick={() => setAdding(true)}>
            {t("Ajouter une source")}
          </button>
        )}
      </div>
      {error && <div className="error-banner">{error}</div>}
      {info && <div className="info-banner">{info}</div>}

      {adding && (
        <Card title={t("Nouvelle source")}>
          <SourceForm onSave={(p) => run(() => createSource(p), t("Source enregistrée.")).then(() => setAdding(false))} onCancel={() => setAdding(false)} />
        </Card>
      )}

      {sources === null ? (
        <p className="muted">{t("Chargement…")}</p>
      ) : sources.length === 0 && !adding ? (
        <p className="muted">{t("Aucune source pour l'instant.")}</p>
      ) : (
        sources.map((s) => (
          <Card
            key={s.id}
            title={
              <span>
                {s.name} <span className="badge">{s.kind}</span>
              </span>
            }
            subtitle={s.kind === "sqlserver" ? `${s.host}:${s.port}/${s.database}` : s.location}
            actions={
              <span style={{ display: "inline-flex", gap: 6 }}>
                <button className="btn-ghost" onClick={() => run(async () => {
                  const r = await testSource(s.id);
                  if (!r.ok) throw new Error(r.error ?? "?");
                }, t("Connexion réussie."))}>
                  {t("Tester")}
                </button>
                <button className="btn-ghost" onClick={() => setEditing(editing === s.id ? null : s.id)}>
                  {t("Modifier")}
                </button>
                <button className="btn-ghost" onClick={() => setMappingFor({ source: s })}>
                  {t("Ajouter un mapping")}
                </button>
                <button
                  className="btn-ghost"
                  onClick={async () => {
                    if (await confirm({ title: t("Supprimer la source ?"), message: s.name, danger: true })) void run(() => deleteSource(s.id));
                  }}
                >
                  {t("Supprimer")}
                </button>
              </span>
            }
          >
            {editing === s.id && <SourceForm initial={s} onSave={(p) => run(() => updateSource(s.id, p), t("Source enregistrée.")).then(() => setEditing(null))} onCancel={() => setEditing(null)} />}
            {mappingFor?.source.id === s.id && (
              <MappingForm
                source={s}
                initial={mappingFor.mapping}
                onSave={(p) =>
                  run(() => (mappingFor.mapping ? updateMapping(s.id, mappingFor.mapping.id, p) : createMapping(s.id, p)), t("Mapping enregistré.")).then(() => setMappingFor(null))
                }
                onCancel={() => setMappingFor(null)}
              />
            )}
            {s.mappings.length === 0 ? (
              <p className="muted">{t("Aucun mapping : ajoutez-en un pour transformer une table en fiches.")}</p>
            ) : (
              <table className="table">
                <thead>
                  <tr>
                    <th>{t("Mapping")}</th>
                    <th>{t("Table")}</th>
                    <th>{t("Fiches")}</th>
                    <th>{t("Dernière synchronisation")}</th>
                    <th />
                  </tr>
                </thead>
                <tbody>
                  {s.mappings.map((m) => (
                    <tr key={m.id}>
                      <td>
                        <strong>{m.name}</strong>
                        {m.schedule_minutes > 0 && <span className="muted"> · {t("toutes les {n} min", { n: m.schedule_minutes })}</span>}
                      </td>
                      <td>
                        <code>{m.from}</code>
                      </td>
                      <td>{m.concept_type}</td>
                      <td>
                        {m.last_run_at ? (
                          <span className={m.last_status === "ok" ? "" : "error-text"}>
                            {m.last_run_at.slice(0, 16).replace("T", " ")} · {m.last_status}
                            {m.last_report && typeof m.last_report === "object" && "rows" in m.last_report ? ` · ${(m.last_report as { rows: number }).rows} ${t("lignes")}` : ""}
                          </span>
                        ) : (
                          <span className="muted">{t("jamais")}</span>
                        )}
                      </td>
                      <td style={{ whiteSpace: "nowrap" }}>
                        <button className="btn-ghost" onClick={() => run(async () => setPreview({ mapping: m.name, data: await previewMapping(s.id, m.id) }))}>
                          {t("Aperçu")}
                        </button>
                        <button className="btn-ghost" onClick={() => run(() => runMapping(s.id, m.id), t("Synchronisation lancée — suivez-la dans la barre du haut."))}>
                          {t("Synchroniser")}
                        </button>
                        <button className="btn-ghost" onClick={() => setMappingFor({ source: s, mapping: m })}>
                          {t("Modifier")}
                        </button>
                        <button
                          className="btn-ghost"
                          onClick={async () => {
                            if (await confirm({ title: t("Supprimer le mapping ?"), message: m.name, danger: true })) void run(() => deleteMapping(s.id, m.id));
                          }}
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
        ))
      )}

      {preview && (
        <Card title={t("Aperçu : {name}", { name: preview.mapping })} actions={<button className="btn-ghost" onClick={() => setPreview(null)}>{t("Fermer")}</button>}>
          <p className="muted">
            {t("{sheets} fiche(s), {links} lien(s), {skipped} ligne(s) sans clé, sur les 50 premières lignes.", {
              sheets: preview.data.sheets.length,
              links: preview.data.links,
              skipped: preview.data.skipped,
            })}
          </p>
          <table className="table">
            <thead>
              <tr>
                {preview.data.columns.map((c) => (
                  <th key={c.name}>{c.name}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {preview.data.rows.slice(0, 10).map((r, i) => (
                <tr key={i}>
                  {r.map((v, j) => (
                    <td key={j}>{v === null ? "" : String(v)}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </Card>
      )}
    </>
  );
}
