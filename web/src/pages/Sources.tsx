// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// Data sources: a database or a file the graph is kept in step with.
// A source (where, how to connect) carries mappings (a table → sheets
// of a type, with links from its foreign keys), previewed before they
// run, run by hand or on a schedule as background jobs, and named
// queries (a trigger and a SELECT with blanks) that answer a question
// live. Administrators.
import { useEffect, useRef, useState } from "react";
import Card from "../components/Card";
// @ts-expect-error JSX module
import { useConfirm } from "../components/ConfirmDialog.jsx";
import {
  createMapping,
  createNamedQuery,
  createSource,
  deleteMapping,
  deleteNamedQuery,
  deleteSource,
  listSourceColumns,
  listSources,
  listSourceTables,
  odbcCatalog,
  previewMapping,
  runMapping,
  testSource,
  tryNamedQuery,
  updateMapping,
  updateNamedQuery,
  updateSource,
  uploadSourceFile,
  type DataSource,
  type MappingPatch,
  type MappingSpec,
  type NamedQuery,
  type NamedQueryPatch,
  type SourcePatch,
} from "../api";
import { t } from "../lib/i18n";

const KINDS = ["csv", "sqlserver", "postgres", "mysql", "odbc"] as const;
// The engines reached by host and port, with the port each listens on.
const PORTS: Record<string, number> = { sqlserver: 1433, postgres: 5432, mysql: 3306 };

const list = (s: string) => s.split(",").map((x) => x.trim()).filter(Boolean);

type Catalog = { available: boolean; drivers: string[]; dsns: { name: string; driver: string }[] };

/** A secret being typed: hidden, shown on the eye. The stored one is never
 * served, so only what is typed can be seen. */
function Secret({ value, onChange, placeholder, label, autoComplete }: { value: string; onChange: (v: string) => void; placeholder?: string; label?: string; autoComplete: string }) {
  const [shown, setShown] = useState(false);
  return (
    <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
      <input type={shown ? "text" : "password"} autoComplete={autoComplete} aria-label={label} value={value} onChange={(e) => onChange(e.target.value)} placeholder={placeholder} />
      <button type="button" className="btn-ghost" aria-label={shown ? t("Masquer") : t("Afficher")} aria-pressed={shown} onClick={() => setShown((s) => !s)} style={{ padding: "6px 8px", flexShrink: 0 }}>
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
          <path d="M1 12s4-7 11-7 11 7 11 7-4 7-11 7S1 12 1 12z" />
          <circle cx="12" cy="12" r="3" />
          {shown && <line x1="3" y1="3" x2="21" y2="21" />}
        </svg>
      </button>
    </div>
  );
}

/** CSV: a path on the server, or a file of this computer sent there. */
function CsvLocation({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const input = useRef<HTMLInputElement>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const pick = (file: File | undefined) => {
    if (!file) return;
    setBusy(true);
    setError(null);
    uploadSourceFile(file)
      .then((r) => onChange(r.path))
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)))
      .finally(() => setBusy(false));
  };
  return (
    <div className="field" style={{ gridColumn: "1 / -1" }}>
      <label htmlFor="csv-location">{"File path or HTTPS URL"}</label>
      <div style={{ display: "flex", gap: 8 }}>
        <input id="csv-location" value={value} onChange={(e) => onChange(e.target.value)} placeholder="/data/exports/clients.csv" />
        <button type="button" className="btn-outline" disabled={busy} onClick={() => input.current?.click()}>
          {busy ? t("Envoi…") : t("Parcourir…")}
        </button>
        <input
          ref={input}
          type="file"
          accept=".csv,.tsv,.txt,text/csv"
          hidden
          data-testid="csv-file"
          onChange={(e) => pick(e.target.files?.[0])}
        />
      </div>
      {error && <p className="field-error">{error}</p>}
    </div>
  );
}

function SourceForm({ initial, onSave, onCancel }: { initial?: DataSource; onSave: (p: SourcePatch) => Promise<void>; onCancel: () => void }) {
  const [p, setP] = useState<SourcePatch>({
    name: initial?.name ?? "",
    kind: initial?.kind ?? "csv",
    location: initial?.kind === "odbc" ? "" : initial?.location ?? "",
    dsn: initial?.dsn ?? "",
    driver: initial?.driver ?? "",
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
  const [catalog, setCatalog] = useState<Catalog | null>(null);
  useEffect(() => {
    if (p.kind !== "odbc" || catalog) return;
    odbcCatalog().then(setCatalog, () => setCatalog({ available: false, drivers: [], dsns: [] }));
  }, [p.kind, catalog]);
  const engine = (p.kind ?? "") in PORTS;
  const odbc = p.kind === "odbc";
  return (
    <form
      className="source-form"
      onSubmit={(e) => {
        e.preventDefault();
        setBusy(true);
        // ODBC: the stored connection string is never served, so a blank
        // field keeps it rather than wiping it.
        void onSave(odbc && !p.location ? { ...p, location: undefined } : p).finally(() => setBusy(false));
      }}
    >
      <div className="grid grid-2" style={{ gap: 12 }}>
        <label className="field">
          <span>{"Source name"}</span>
          <input required value={p.name} onChange={(e) => set("name", e.target.value)} />
        </label>
        <label className="field">
          <span>{t("Type")}</span>
          <select value={p.kind} onChange={(e) => setP((x) => ({ ...x, kind: e.target.value, port: PORTS[e.target.value] ?? 0 }))}>
            {KINDS.map((k) => (
              <option key={k} value={k}>
                {k === "csv" ? t("Fichier CSV (chemin ou URL)") : k === "sqlserver" ? "SQL Server" : k === "postgres" ? "PostgreSQL" : k === "mysql" ? "MySQL / MariaDB" : "ODBC (OpenEdge, …)"}
              </option>
            ))}
          </select>
        </label>
        {p.kind === "csv" && <CsvLocation value={p.location ?? ""} onChange={(v) => set("location", v)} />}
        {/* ODBC: the names and the order of the Windows ODBC administrator's
            driver setup, untranslated, so each field is found at a glance. */}
        {odbc && (
          <>
            <p className="field-hint" style={{ gridColumn: "1 / -1" }}>
              {t("Soit un Data Source Name déjà défini dans l'administrateur ODBC de Windows (sur la machine du serveur), soit un pilote avec l'hôte, le port et la base — comme dans la fenêtre du pilote.")}
            </p>
            <label className="field">
              <span>Data Source Name</span>
              <input list="odbc-dsns" value={p.dsn ?? ""} onChange={(e) => set("dsn", e.target.value)} placeholder={catalog?.dsns[0]?.name ?? ""} />
            </label>
            {!p.dsn && (
              <label className="field">
                <span>Driver</span>
                <input list="odbc-drivers" value={p.driver ?? ""} onChange={(e) => set("driver", e.target.value)} placeholder={catalog?.drivers[0] ?? ""} />
              </label>
            )}
            <datalist id="odbc-dsns">
              {catalog?.dsns.map((d) => (
                <option key={d.name} value={d.name}>
                  {d.driver}
                </option>
              ))}
            </datalist>
            <datalist id="odbc-drivers">
              {catalog?.drivers.map((d) => <option key={d} value={d} />)}
            </datalist>
            {catalog && !catalog.available && (
              <p className="field-hint" style={{ gridColumn: "1 / -1" }}>
                {t("Le serveur ne liste aucun pilote ODBC (version sans ODBC, ou pas de gestionnaire de pilotes) : saisissez le nom.")}
              </p>
            )}
          </>
        )}
        {(engine || (odbc && !p.dsn)) && (
          <>
            <label className="field">
              <span>{odbc ? "Host Name" : "Host"}</span>
              <input value={p.host ?? ""} onChange={(e) => set("host", e.target.value)} />
            </label>
            <label className="field">
              <span>{odbc ? "Port Number" : "Port"}</span>
              <input type="number" value={p.port || ""} onChange={(e) => set("port", Number(e.target.value))} />
            </label>
            <label className="field">
              <span>{odbc ? "Database Name" : "Database"}</span>
              <input value={p.database ?? ""} onChange={(e) => set("database", e.target.value)} />
            </label>
          </>
        )}
        {(engine || odbc) && (
          <>
            <label className="field">
              <span>{odbc ? "User ID" : "User (read-only)"}</span>
              <input value={p.user ?? ""} onChange={(e) => set("user", e.target.value)} />
            </label>
            <div className="field">
              <label htmlFor="src-password">Password</label>
              <Secret autoComplete="new-password" value={p.password ?? ""} onChange={(v) => set("password", v)} placeholder={initial?.password_hint || ""} label="Password" />
            </div>
          </>
        )}
        {odbc && (
          <details className="field" style={{ gridColumn: "1 / -1" }}>
            <summary>Connection string (advanced: replaces the fields above)</summary>
            <Secret autoComplete="off" label="Connection string" value={p.location ?? ""} onChange={(v) => set("location", v)} placeholder={initial?.location || "Driver={…};HOST=…;PORT=…;DB=…;UID=…;PWD=…"} />
          </details>
        )}
        {engine && (
          <>
            <label className="checkbox-row">
              <input type="checkbox" checked={p.encrypt ?? true} onChange={(e) => set("encrypt", e.target.checked)} />
              <span>{"Encrypt (TLS)"}</span>
            </label>
            <label className="checkbox-row">
              <input type="checkbox" checked={p.trust_cert ?? false} onChange={(e) => set("trust_cert", e.target.checked)} />
              <span>{"Trust server certificate"}</span>
            </label>
          </>
        )}
        <label className="checkbox-row">
          <input type="checkbox" checked={p.allow_private_hosts ?? false} onChange={(e) => set("allow_private_hosts", e.target.checked)} />
          <span>Allow private hosts (the server is on the local network)</span>
        </label>
        <label className="field">
          <span>{"Timeout (seconds)"}</span>
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
          <span>{"Mapping name"}</span>
          <input required value={p.name ?? ""} onChange={(e) => set("name", e.target.value)} />
        </label>
        <label className="field">
          <span>{"Table or SELECT"}</span>
          <input required list="src-tables" value={p.from ?? ""} onChange={(e) => set("from", e.target.value)} />
          <datalist id="src-tables">{tables.map((x) => <option key={x} value={x} />)}</datalist>
        </label>
        <label className="field">
          <span>{"Sheet type"}</span>
          <input required value={p.concept_type ?? ""} onChange={(e) => set("concept_type", e.target.value)} placeholder="Client" />
        </label>
        <label className="field">
          <span>{"Key column(s) — comma-separated for a composite key"}</span>
          <input required list="src-columns" value={p.key_column ?? ""} onChange={(e) => set("key_column", e.target.value)} />
        </label>
        <label className="field">
          <span>{"Name column (default: the key)"}</span>
          <input list="src-columns" value={p.name_column ?? ""} onChange={(e) => set("name_column", e.target.value)} />
        </label>
        <label className="field">
          <span>{"Description column"}</span>
          <input list="src-columns" value={p.description_column ?? ""} onChange={(e) => set("description_column", e.target.value)} />
        </label>
        <label className="field">
          <span>{"Property columns (empty: all)"}</span>
          <input value={(p.property_columns ?? []).join(", ")} onChange={(e) => set("property_columns", list(e.target.value))} />
        </label>
        <label className="field">
          <span>{"Watermark column (incremental sync)"}</span>
          <input list="src-columns" value={p.watermark_column ?? ""} onChange={(e) => set("watermark_column", e.target.value)} />
        </label>
        <label className="field">
          <span>{"Schedule (minutes, 0: manual)"}</span>
          <input type="number" min={0} value={p.schedule_minutes ?? 0} onChange={(e) => set("schedule_minutes", Number(e.target.value))} />
        </label>
      </div>
      <fieldset style={{ marginTop: 12, border: "1px solid var(--border)", borderRadius: "var(--radius-sm)", padding: 12 }}>
        <legend>{"Links from foreign keys"}</legend>
        {(p.relations ?? []).map((r, i) => (
          <div key={i} className="grid grid-3" style={{ gap: 8, marginBottom: 6 }}>
            <input aria-label={"Column"} list="src-columns" placeholder={"Column"} value={r.column} onChange={(e) => set("relations", (p.relations ?? []).map((x, j) => (j === i ? { ...x, column: e.target.value } : x)))} />
            <input aria-label={"Link type"} placeholder={"Link type"} value={r.relation_type} onChange={(e) => set("relations", (p.relations ?? []).map((x, j) => (j === i ? { ...x, relation_type: e.target.value } : x)))} />
            <span style={{ display: "flex", gap: 6 }}>
              <input aria-label={"Target type"} placeholder={"Target type"} value={r.target_type} onChange={(e) => set("relations", (p.relations ?? []).map((x, j) => (j === i ? { ...x, target_type: e.target.value } : x)))} />
              <button type="button" className="btn-ghost" onClick={() => set("relations", (p.relations ?? []).filter((_, j) => j !== i))}>
                ×
              </button>
            </span>
          </div>
        ))}
        <button type="button" className="btn-ghost" onClick={() => set("relations", [...(p.relations ?? []), { column: "", relation_type: "", target_type: "", create_target: true }])}>
          {"Add link"}
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

function QueryForm({ initial, onSave, onCancel }: { initial?: NamedQuery; onSave: (p: NamedQueryPatch) => Promise<void>; onCancel: () => void }) {
  const [p, setP] = useState<NamedQueryPatch>({
    name: initial?.name ?? "",
    description: initial?.description ?? "",
    trigger: initial?.trigger ?? "",
    sql: initial?.sql ?? "",
    max_rows: initial?.max_rows ?? 20,
    admin_only: initial?.admin_only ?? false,
    enabled: initial?.enabled ?? true,
  });
  const set = <K extends keyof NamedQueryPatch>(k: K, v: NamedQueryPatch[K]) => setP((x) => ({ ...x, [k]: v }));
  return (
    <form
      className="mapping-form"
      onSubmit={(e) => {
        e.preventDefault();
        void onSave(p);
      }}
    >
      <div className="grid grid-2" style={{ gap: 12 }}>
        <label className="field">
          <span>{"Query name"}</span>
          <input required value={p.name ?? ""} onChange={(e) => set("name", e.target.value)} />
        </label>
        <label className="field">
          <span>{"Trigger (regex with named groups)"}</span>
          <input required value={p.trigger ?? ""} onChange={(e) => set("trigger", e.target.value)} placeholder="(?i)ticket\\s*(?:n°|#)?\\s*(?P<id>\\d{1,9})" />
        </label>
        <label className="field" style={{ gridColumn: "1 / -1" }}>
          <span>{"SELECT with {group} blanks"}</span>
          <textarea required rows={3} value={p.sql ?? ""} onChange={(e) => set("sql", e.target.value)} placeholder="SELECT id, title, status FROM tickets WHERE id = {id}" />
        </label>
        <label className="field">
          <span>{t("Description")}</span>
          <input value={p.description ?? ""} onChange={(e) => set("description", e.target.value)} />
        </label>
        <label className="field">
          <span>{"Max rows (200 max)"}</span>
          <input type="number" min={1} max={200} value={p.max_rows ?? 20} onChange={(e) => set("max_rows", Number(e.target.value))} />
        </label>
        <label className="checkbox-row">
          <input type="checkbox" checked={!!p.admin_only} onChange={(e) => set("admin_only", e.target.checked)} />
          <span>{t("Administrateurs seulement")}</span>
        </label>
        <label className="checkbox-row">
          <input type="checkbox" checked={p.enabled !== false} onChange={(e) => set("enabled", e.target.checked)} />
          <span>{t("Activée")}</span>
        </label>
      </div>
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
  // The connection test of a source, shown beside its button: running,
  // or the last result.
  const [tests, setTests] = useState<Record<number, "busy" | { ok: true; ms: number } | { ok: false; error: string }>>({});
  const test = async (s: DataSource) => {
    setTests((x) => ({ ...x, [s.id]: "busy" }));
    try {
      const r = await testSource(s.id);
      setTests((x) => ({ ...x, [s.id]: r.ok ? { ok: true, ms: r.ms ?? 0 } : { ok: false, error: r.error ?? "?" } }));
    } catch (e: unknown) {
      setTests((x) => ({ ...x, [s.id]: { ok: false, error: e instanceof Error ? e.message : String(e) } }));
    }
  };
  const [mappingFor, setMappingFor] = useState<{ source: DataSource; mapping?: MappingSpec } | null>(null);
  const [preview, setPreview] = useState<{ mapping: string; data: Awaited<ReturnType<typeof previewMapping>> } | null>(null);
  const [queryFor, setQueryFor] = useState<{ source: DataSource; query?: NamedQuery } | null>(null);
  const [trying, setTrying] = useState<{ source: number; query: NamedQuery; question: string; result?: Awaited<ReturnType<typeof tryNamedQuery>> } | null>(null);

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
            subtitle={s.kind in PORTS ? `${s.host}:${s.port}/${s.database}` : s.kind === "odbc" && s.dsn ? `DSN ${s.dsn}` : s.kind === "odbc" && s.driver ? `${s.driver} · ${s.host}:${s.port}/${s.database}` : s.location}
            actions={
              <span style={{ display: "inline-flex", gap: 6, alignItems: "center" }}>
                {((r) =>
                  r === "busy" ? (
                    <span className="badge" role="status">{t("Test en cours… (jusqu'à {n} s)", { n: s.timeout_secs })}</span>
                  ) : r?.ok ? (
                    <span className="badge badge-success" role="status">{t("Connexion réussie ({ms} ms)", { ms: r.ms })}</span>
                  ) : r ? (
                    <span className="badge badge-danger" role="status">{t("Échec : {error}", { error: r.error })}</span>
                  ) : null)(tests[s.id])}
                <button className="btn-ghost" disabled={tests[s.id] === "busy"} onClick={() => void test(s)}>
                  {t("Tester")}
                </button>
                <button className="btn-ghost" onClick={() => setEditing(editing === s.id ? null : s.id)}>
                  {t("Modifier")}
                </button>
                <button className="btn-ghost" onClick={() => setMappingFor({ source: s })}>
                  {t("Ajouter un mapping")}
                </button>
                <button className="btn-ghost" onClick={() => setQueryFor({ source: s })}>
                  {t("Ajouter une requête")}
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
            {queryFor?.source.id === s.id && (
              <QueryForm
                initial={queryFor.query}
                onSave={(p) =>
                  run(() => (queryFor.query ? updateNamedQuery(s.id, queryFor.query.id, p) : createNamedQuery(s.id, p)), t("Requête enregistrée.")).then(() => setQueryFor(null))
                }
                onCancel={() => setQueryFor(null)}
              />
            )}
            {(s.queries ?? []).length > 0 && (
              <table className="table" style={{ marginTop: 12 }}>
                <thead>
                  <tr>
                    <th>{t("Requête nommée")}</th>
                    <th>{t("Déclencheur")}</th>
                    <th>SQL</th>
                    <th />
                  </tr>
                </thead>
                <tbody>
                  {s.queries.map((q) => (
                    <tr key={q.id} className={q.enabled ? "" : "muted"}>
                      <td>
                        <strong>{q.name}</strong>
                        {q.admin_only && <span className="badge"> {t("admin")}</span>}
                        {!q.enabled && <span className="muted"> · {t("désactivée")}</span>}
                      </td>
                      <td>
                        <code>{q.trigger}</code>
                      </td>
                      <td>
                        <code>{q.sql}</code>
                      </td>
                      <td style={{ whiteSpace: "nowrap" }}>
                        <button className="btn-ghost" onClick={() => setTrying({ source: s.id, query: q, question: "" })}>
                          {t("Essayer")}
                        </button>
                        <button className="btn-ghost" onClick={() => setQueryFor({ source: s, query: q })}>
                          {t("Modifier")}
                        </button>
                        <button
                          className="btn-ghost"
                          onClick={async () => {
                            if (await confirm({ title: t("Supprimer la requête ?"), message: q.name, danger: true })) void run(() => deleteNamedQuery(s.id, q.id));
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

      {trying && (
        <Card title={t("Essayer : {name}", { name: trying.query.name })} actions={<button className="btn-ghost" onClick={() => setTrying(null)}>{t("Fermer")}</button>}>
          <form
            style={{ display: "flex", gap: 8 }}
            onSubmit={(e) => {
              e.preventDefault();
              void run(async () => {
                const result = await tryNamedQuery(trying.source, trying.query.id, trying.question);
                setTrying((x) => (x ? { ...x, result } : x));
              });
            }}
          >
            <input aria-label={t("Question")} style={{ flex: 1 }} value={trying.question} onChange={(e) => setTrying({ ...trying, question: e.target.value, result: undefined })} placeholder={t("Une question comme un utilisateur la poserait")} />
            <button className="btn-primary" type="submit">
              {t("Essayer")}
            </button>
          </form>
          {trying.result && !trying.result.matched && <p className="muted">{t("Le déclencheur ne correspond pas à cette question.")}</p>}
          {trying.result?.matched && (
            <>
              <p>
                <code>{trying.result.sql}</code>
              </p>
              <pre className="code-block">{trying.result.context}</pre>
            </>
          )}
        </Card>
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
