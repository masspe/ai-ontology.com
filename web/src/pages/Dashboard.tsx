// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.

import { useEffect, useState, type ReactNode } from "react";
import { Link, useNavigate } from "react-router-dom";
import Card from "../components/Card";
import Onboarding from "../components/Onboarding";
import { pinnedIds } from "../lib/pins";
import Sparkline from "../components/Sparkline";
import {
  getFiles,
  getOntology,
  getQueries,
  getStats,
  getStatsHistory,
  listRules,
  type Rule,
  type FileRecord,
  type Ontology,
  type SavedQuery,
  type Stats,
  type StatsHistory,
} from "../api";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function fmtBytes(b: number): string {
  if (b < 1024) return `${b} B`;
  if (b < 1024 * 1024) return `${(b / 1024).toFixed(1)} KB`;
  return `${(b / (1024 * 1024)).toFixed(1)} MB`;
}

function fmtNum(n: number): string {
  return n.toLocaleString("en-US");
}

function fmtAgo(ts: number): string {
  const diff = Math.max(0, Math.floor(Date.now() / 1000 - ts));
  if (diff < 60) return `${diff}s ago`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}

function fileKindClass(kind: string): string {
  const k = kind.toLowerCase();
  if (k.includes("pdf")) return "file-icon pdf";
  if (k.includes("csv")) return "file-icon csv";
  if (k.includes("doc")) return "file-icon doc";
  if (k.includes("xls") || k.includes("sheet")) return "file-icon xls";
  if (k.includes("json")) return "file-icon json";
  return "file-icon generic";
}

function ingestStatus(f: FileRecord): { label: string; cls: string } {
  const s = (f.status || "").toLowerCase();
  if (s === "processed" || s === "ingested" || s === "done") return { label: "Traité", cls: "badge-success" };
  if (s === "pending" || s === "queued") return { label: "En attente", cls: "badge-warn" };
  if (s === "failed" || s === "error") return { label: "Échec", cls: "badge-danger" };
  if (s === "analyzed") return { label: "Analysé", cls: "badge-accent" };
  return { label: f.status || "—", cls: "badge" };
}

// ---------------------------------------------------------------------------
// Inline SVG icons
// ---------------------------------------------------------------------------

const Icon = {
  layers: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 2 2 7l10 5 10-5-10-5z" />
      <path d="m2 17 10 5 10-5" />
      <path d="m2 12 10 5 10-5" />
    </svg>
  ),
  users: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2" />
      <circle cx="9" cy="7" r="4" />
      <path d="M22 21v-2a4 4 0 0 0-3-3.87" />
      <path d="M16 3.13a4 4 0 0 1 0 7.75" />
    </svg>
  ),
  share: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="18" cy="5" r="3" />
      <circle cx="6" cy="12" r="3" />
      <circle cx="18" cy="19" r="3" />
      <path d="m8.59 13.51 6.83 3.98" />
      <path d="m15.41 6.51-6.82 3.98" />
    </svg>
  ),
  shield: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
      <path d="m9 12 2 2 4-4" />
    </svg>
  ),
  upload: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
      <path d="m17 8-5-5-5 5" />
      <path d="M12 3v12" />
    </svg>
  ),
  search: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="11" cy="11" r="7" />
      <path d="m21 21-4.3-4.3" />
    </svg>
  ),
  graph: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="5" cy="12" r="2.5" />
      <circle cx="19" cy="5" r="2.5" />
      <circle cx="19" cy="19" r="2.5" />
      <path d="m7 11 10-5" />
      <path d="m7 13 10 5" />
    </svg>
  ),
  plus: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 5v14" />
      <path d="M5 12h14" />
    </svg>
  ),
  check: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="m20 6-11 11-5-5" />
    </svg>
  ),
  spark: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="m12 3 1.9 5.8H20l-4.9 3.6L17 18.2 12 14.6 7 18.2l1.9-5.8L4 8.8h6.1z" />
    </svg>
  ),
};

// ---------------------------------------------------------------------------
// Rich stat tile (icon + value + delta + sparkline)
// ---------------------------------------------------------------------------

interface RichStatProps {
  label: string;
  value: number;
  deltaPct?: number;
  icon: ReactNode;
  tone: "blue" | "violet" | "amber" | "green";
  spark: number[];
  sparkColor: string;
}

function RichStat({ label, value, deltaPct, icon, tone, spark, sparkColor }: RichStatProps) {
  const cls = deltaPct == null ? "flat" : deltaPct > 0.05 ? "up" : deltaPct < -0.05 ? "down" : "flat";
  const arrow = cls === "up" ? "↑" : cls === "down" ? "↓" : "•";
  return (
    <div className="card stat-rich">
      <div className={`stat-icon tone-${tone}`}>{icon}</div>
      <div className="stat-body">
        <div className="stat-label">{label}</div>
        <div className="stat-value">{fmtNum(value)}</div>
        {deltaPct != null && (
          <div className={`stat-delta ${cls}`}>
            <span>{arrow} {Math.abs(deltaPct).toFixed(0)}%</span>
            <span className="muted">vs last period</span>
          </div>
        )}
      </div>
      <div className="stat-spark">
        <Sparkline values={spark.length > 1 ? spark : [0, 0]} stroke={sparkColor} />
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Larger line chart for "Ontology Growth"
// ---------------------------------------------------------------------------

/** The draft of a review left in the ingest assistant (same key as the assistant). */
function pendingReview(): { concepts: number; relations: number; name: string } | null {
  try {
    const raw = window.sessionStorage.getItem("ingest.draft.v1");
    if (!raw) return null;
    const d = JSON.parse(raw) as { proposal?: { concepts?: unknown[]; relations?: unknown[]; source?: { name?: string } } };
    if (!d.proposal) return null;
    return {
      concepts: d.proposal.concepts?.length ?? 0,
      relations: d.proposal.relations?.length ?? 0,
      name: d.proposal.source?.name ?? "document",
    };
  } catch {
    return null;
  }
}

const WEEK = 7 * 86_400;

export default function Dashboard() {
  const nav = useNavigate();
  const [q, setQ] = useState("");
  const [stats, setStats] = useState<Stats | null>(null);
  const [history, setHistory] = useState<StatsHistory | null>(null);
  const [files, setFiles] = useState<FileRecord[]>([]);
  const [queries, setQueries] = useState<SavedQuery[]>([]);
  const [ontology, setOntology] = useState<Ontology | null>(null);
  const [rules, setRules] = useState<Rule[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [refresh, setRefresh] = useState(0);

  useEffect(() => {
    let cancelled = false;
    const tick = async () => {
      try {
        const [s, h, f, qs, o, r] = await Promise.all([
          getStats(),
          getStatsHistory(),
          getFiles(),
          getQueries().catch(() => ({ queries: [] as SavedQuery[] })),
          getOntology().catch(() => null),
          listRules().catch(() => [] as Rule[]),
        ]);
        if (cancelled) return;
        setStats(s);
        setHistory(h);
        setFiles(f.files);
        setQueries(qs.queries);
        setOntology(o);
        setRules(r);
        setError(null);
      } catch (e: unknown) {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      }
    };
    tick();
    const id = window.setInterval(tick, 15_000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [refresh]);

  const samples = history?.samples ?? [];

  // The first day (ROADMAP §3.9 lot A): an empty graph shows the three
  // steps instead of empty tiles; the dashboard returns with the data.
  const hasModel = Boolean(ontology && Object.keys(ontology.concept_types).length > 0);
  const hasData = (stats?.concepts ?? 0) > 0;
  if (stats && ontology && !hasData) {
    return (
      <>
        {error && <div className="error-banner">{error}</div>}
        <Onboarding hasModel={hasModel} hasData={hasData} onLoaded={() => setRefresh((n) => n + 1)} />
      </>
    );
  }

  const now = Date.now() / 1000;
  const thisWeek = files.filter((f) => now - f.uploaded_at < WEEK);
  const weekConcepts = thisWeek.reduce((n, f) => n + f.concepts, 0);
  const weekRelations = thisWeek.reduce((n, f) => n + f.relations, 0);
  const failed = files.filter((f) => f.status === "error" || f.status === "failed");
  const review = pendingReview();
  const strict = rules.filter((r) => r.strict);
  const pins = pinnedIds();
  const pinnedQueries = queries.filter((sq) => pins.includes(sq.id));
  const recentQueries = queries
    .filter((sq) => !pins.includes(sq.id))
    .sort((a, b) => (b.last_run_at ?? b.created_at) - (a.last_run_at ?? a.created_at))
    .slice(0, Math.max(0, 5 - pinnedQueries.length));
  const recentFiles = [...files].sort((a, b) => b.uploaded_at - a.uploaded_at).slice(0, 5);
  const todo = (review ? 1 : 0) + failed.length;

  return (
    <>
      <div className="page-header">
        <div>
          <h1 className="page-title">Accueil</h1>
          <p className="page-subtitle">
            {stats
              ? `${fmtNum(stats.concepts)} fiches et ${fmtNum(stats.relations)} liens, ${fmtNum(stats.concept_types)} types de fiches.`
              : "En attente du serveur."}
          </p>
        </div>
      </div>

      {error && <div className="error-banner">{error}</div>}

      <form
        className="home-search"
        data-tour="home-search"
        onSubmit={(e) => {
          e.preventDefault();
          if (q.trim()) nav(`/queries?q=${encodeURIComponent(q.trim())}`);
        }}
      >
        <input
          aria-label="Question"
          placeholder="Posez une question à vos données : « Quels contrats arrivent à échéance ce mois-ci ? »"
          value={q}
          onChange={(e) => setQ(e.target.value)}
        />
        <button className="btn-primary" type="submit" disabled={!q.trim()}>
          Demander
        </button>
      </form>

      <div className="dash-row dash-row-stats">
        <RichStat label="Fiches" value={stats?.concepts ?? 0} deltaPct={stats?.deltas.concepts_pct} icon={Icon.users} tone="violet" spark={samples.map((s) => s.concepts)} sparkColor="#7c3aed" />
        <RichStat label="Liens" value={stats?.relations ?? 0} deltaPct={stats?.deltas.relations_pct} icon={Icon.share} tone="amber" spark={samples.map((s) => s.relations)} sparkColor="#d97706" />
        <RichStat label="Types de fiches" value={stats?.concept_types ?? 0} deltaPct={stats?.deltas.concept_types_pct} icon={Icon.layers} tone="blue" spark={samples.map((s) => s.concept_types)} sparkColor="#2563eb" />
        <RichStat label="Règles" value={stats?.rules ?? 0} icon={Icon.shield} tone="green" spark={[]} sparkColor="#16a34a" />
      </div>

      <div className="dash-row home-row">
        <Card title={`À faire (${todo})`} className="home-todo">
          {todo === 0 ? (
            <p className="muted">Rien en attente. Déposez un document ou posez une question.</p>
          ) : (
            <ul className="sheet-list">
              {review && (
                <li>
                  <Link to="/ingest">
                    Relecture en cours : {review.name}
                  </Link>{" "}
                  <span className="muted">
                    {review.concepts} fiche(s) et {review.relations} lien(s) proposés, à vérifier puis ajouter.
                  </span>
                </li>
              )}
              {failed.map((f) => (
                <li key={f.id}>
                  <Link to="/files">Import en échec : {f.name}</Link>
                </li>
              ))}
            </ul>
          )}
        </Card>

        <Card title="Cette semaine" actions={<Link to="/files" className="btn-ghost-link">Déposer des fichiers</Link>}>
          {thisWeek.length === 0 ? (
            <p className="muted">Aucun document importé ces 7 derniers jours.</p>
          ) : (
            <p>
              <strong>{thisWeek.length}</strong> document(s) importé(s) : <strong>{fmtNum(weekConcepts)}</strong> fiche(s) et{" "}
              <strong>{fmtNum(weekRelations)}</strong> lien(s) ajoutés.
            </p>
          )}
          <ul className="sheet-list">
            {recentFiles.map((f) => {
              const st = ingestStatus(f);
              return (
                <li key={f.id}>
                  <span className={`file-kind ${fileKindClass(f.kind)}`}>{f.kind}</span> {f.name}{" "}
                  <span className="muted">
                    {fmtBytes(f.size)}, {fmtAgo(f.uploaded_at)}
                  </span>{" "}
                  <span className={`badge ${st.cls}`}>{st.label}</span>
                </li>
              );
            })}
          </ul>
        </Card>
      </div>

      <div className="dash-row home-row">
        <Card title="Dernières questions" actions={<Link to="/queries" className="btn-ghost-link">Toutes</Link>}>
          {pinnedQueries.length + recentQueries.length === 0 ? (
            <p className="muted">Aucune question enregistrée. La première se pose ci-dessus.</p>
          ) : (
            <ul className="sheet-list">
              {pinnedQueries.map((sq) => (
                <li key={`p${sq.id}`}>
                  <span aria-label="Épinglée" title="Épinglée">★</span>{" "}
                  <Link to={`/queries?run=${sq.id}`}>{sq.name}</Link>{" "}
                  <span className="muted">rejouer</span>
                </li>
              ))}
              {recentQueries.map((sq) => (
                <li key={sq.id}>
                  <Link to={`/queries?q=${encodeURIComponent(sq.query)}`}>{sq.name}</Link>{" "}
                  <span className="muted">{fmtAgo(sq.last_run_at ?? sq.created_at)}</span>
                </li>
              ))}
            </ul>
          )}
        </Card>

        <Card title="Règles" actions={<Link to="/rules" className="btn-ghost-link">Gérer</Link>}>
          {rules.length === 0 ? (
            <p className="muted">
              Aucune règle. Une règle dit ce qui doit être vrai (« toute facture est liée à un contrat ») et signale les exceptions.
            </p>
          ) : (
            <p>
              <strong>{rules.length}</strong> règle(s), dont <strong>{strict.length}</strong> stricte(s) ;{" "}
              {rules.filter((r) => r.applies_to.length === 0).length} générale(s).
            </p>
          )}
        </Card>
      </div>
    </>
  );
}
