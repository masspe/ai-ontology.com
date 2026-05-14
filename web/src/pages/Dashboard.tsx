// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { useEffect, useMemo, useState, type ReactNode } from "react";
import { Link } from "react-router-dom";
import Card from "../components/Card";
import Sparkline from "../components/Sparkline";
import {
  getFiles,
  getOntology,
  getQueries,
  getStats,
  getStatsHistory,
  listConcepts,
  type Concept,
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
  if (s === "processed" || s === "ingested" || s === "done") return { label: "Processed", cls: "badge-success" };
  if (s === "pending" || s === "queued") return { label: "Pending", cls: "badge-warn" };
  if (s === "failed" || s === "error") return { label: "Failed", cls: "badge-danger" };
  if (s === "analyzed") return { label: "Analyzed", cls: "badge-accent" };
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

interface ChartProps {
  samples: { ts: number; value: number }[];
}

function GrowthChart({ samples }: ChartProps) {
  if (samples.length === 0) {
    return <div className="empty">No samples yet. The dashboard auto-refreshes every 15s.</div>;
  }
  const W = 600;
  const H = 220;
  const padL = 44;
  const padR = 12;
  const padT = 12;
  const padB = 28;
  const innerW = W - padL - padR;
  const innerH = H - padT - padB;
  const xs = samples.map((s) => s.ts);
  const ys = samples.map((s) => s.value);
  const xMin = Math.min(...xs);
  const xMax = Math.max(...xs);
  const yMin = 0;
  const yMax = Math.max(1, Math.max(...ys));
  const sx = (t: number) => padL + ((t - xMin) / Math.max(1, xMax - xMin)) * innerW;
  const sy = (v: number) => padT + innerH - ((v - yMin) / (yMax - yMin)) * innerH;
  const pts = samples.map((s) => `${sx(s.ts).toFixed(1)},${sy(s.value).toFixed(1)}`).join(" ");
  const areaPts = `${padL},${padT + innerH} ${pts} ${padL + innerW},${padT + innerH}`;
  const ticks = [0, 0.25, 0.5, 0.75, 1].map((p) => {
    const v = yMin + (yMax - yMin) * (1 - p);
    return { y: padT + innerH * p, v };
  });
  const xLabelIdx = samples.length <= 7
    ? samples.map((_, i) => i)
    : [0, Math.floor(samples.length / 4), Math.floor(samples.length / 2), Math.floor((3 * samples.length) / 4), samples.length - 1];
  return (
    <svg viewBox={`0 0 ${W} ${H}`} className="growth-chart" preserveAspectRatio="none">
      {ticks.map((t, i) => (
        <g key={i}>
          <line x1={padL} x2={padL + innerW} y1={t.y} y2={t.y} stroke="var(--border)" strokeDasharray="3 3" />
          <text x={padL - 8} y={t.y + 4} fontSize="10" fill="var(--muted)" textAnchor="end">
            {t.v >= 1000 ? `${(t.v / 1000).toFixed(1)}k` : Math.round(t.v)}
          </text>
        </g>
      ))}
      <polygon points={areaPts} fill="var(--accent-soft)" opacity={0.55} />
      <polyline points={pts} fill="none" stroke="var(--accent)" strokeWidth={2} />
      {samples.map((s, i) => (
        <circle key={i} cx={sx(s.ts)} cy={sy(s.value)} r={2} fill="var(--accent)" />
      ))}
      {xLabelIdx.map((i) => {
        const s = samples[i]!;
        const d = new Date(s.ts * 1000);
        const label = d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
        return (
          <text key={i} x={sx(s.ts)} y={H - 8} fontSize="10" fill="var(--muted)" textAnchor="middle">
            {label}
          </text>
        );
      })}
    </svg>
  );
}

// ---------------------------------------------------------------------------
// Decorative mini network preview
// ---------------------------------------------------------------------------

function NetworkPreview({ ontology }: { ontology: Ontology | null }) {
  const names = ontology ? Object.keys(ontology.concept_types).slice(0, 6) : [];
  if (names.length === 0) {
    return <div className="empty">No ontology defined yet.</div>;
  }
  const center = { x: 240, y: 130, label: names[0]! };
  const radius = 100;
  const around = names.slice(1).map((label, i, arr) => {
    const angle = (i / Math.max(1, arr.length)) * Math.PI * 2 - Math.PI / 2;
    return {
      label,
      x: center.x + Math.cos(angle) * radius * (1 + (i % 2) * 0.1),
      y: center.y + Math.sin(angle) * (radius - 30),
    };
  });
  const palette = ["#dbeafe", "#dcfce7", "#fef3c7", "#fee2e2", "#ede9fe", "#cffafe"];
  const stroke = ["#2563eb", "#16a34a", "#d97706", "#dc2626", "#7c3aed", "#0891b2"];
  return (
    <div className="network-preview">
      <svg viewBox="0 0 480 260" preserveAspectRatio="xMidYMid meet">
        {around.map((n, i) => (
          <line key={`l-${i}`} x1={center.x} y1={center.y} x2={n.x} y2={n.y} stroke="#cbd5e1" strokeWidth={1.2} />
        ))}
        <g>
          <rect x={center.x - 50} y={center.y - 16} width={100} height={32} rx={16} fill="#dbeafe" stroke="#2563eb" />
          <text x={center.x} y={center.y + 4} fontSize="12" fontWeight="600" fill="#1d4ed8" textAnchor="middle">
            {center.label}
          </text>
        </g>
        {around.map((n, i) => (
          <g key={`n-${i}`}>
            <rect
              x={n.x - 44}
              y={n.y - 14}
              width={88}
              height={28}
              rx={14}
              fill={palette[i % palette.length]}
              stroke={stroke[i % stroke.length]}
            />
            <text x={n.x} y={n.y + 4} fontSize="11" fontWeight="600" fill={stroke[i % stroke.length]} textAnchor="middle">
              {n.label}
            </text>
          </g>
        ))}
      </svg>
      <div className="network-legend">
        <span><i style={{ background: "#2563eb" }} /> Class</span>
        <span><i style={{ background: "#16a34a" }} /> Entity</span>
        <span><i style={{ background: "#d97706" }} /> Relation</span>
        <span><i style={{ background: "#dc2626" }} /> Constraint</span>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Dashboard page
// ---------------------------------------------------------------------------

export default function Dashboard() {
  const [stats, setStats] = useState<Stats | null>(null);
  const [history, setHistory] = useState<StatsHistory | null>(null);
  const [files, setFiles] = useState<FileRecord[]>([]);
  const [queries, setQueries] = useState<SavedQuery[]>([]);
  const [ontology, setOntology] = useState<Ontology | null>(null);
  const [concepts, setConcepts] = useState<Concept[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const tick = async () => {
      try {
        const [s, h, f, q, o, c] = await Promise.all([
          getStats(),
          getStatsHistory(),
          getFiles(),
          getQueries().catch(() => ({ queries: [] as SavedQuery[] })),
          getOntology().catch(() => null),
          listConcepts({ limit: 500 }).catch(() => ({ total: 0, concepts: [] as Concept[] })),
        ]);
        if (cancelled) return;
        setStats(s);
        setHistory(h);
        setFiles(f.files);
        setQueries(q.queries);
        setOntology(o);
        setConcepts(c.concepts);
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
  }, []);

  const samples = history?.samples ?? [];
  const sparkConcepts = samples.map((s) => s.concepts);
  const sparkRelations = samples.map((s) => s.relations);
  const sparkConceptTypes = samples.map((s) => s.concept_types);
  const sparkRelationTypes = samples.map((s) => s.relation_types);

  const topTypes = useMemo(() => {
    const counts = new Map<string, number>();
    for (const c of concepts) counts.set(c.concept_type, (counts.get(c.concept_type) ?? 0) + 1);
    return Array.from(counts.entries())
      .sort((a, b) => b[1] - a[1])
      .slice(0, 5);
  }, [concepts]);
  const topMax = topTypes.length > 0 ? topTypes[0]![1] : 1;

  const conceptTypes = ontology ? Object.entries(ontology.concept_types).slice(0, 5) : [];

  return (
    <>
      <div className="page-header">
        <div>
          <h1 className="page-title">Dashboard</h1>
          <p className="page-subtitle">Monitor your ontology projects, files, and graph activity.</p>
        </div>
      </div>

      {error && <div className="error-banner">{error}</div>}

      {/* Row 1 — stat tiles */}
      <div className="dash-row dash-row-stats">
        <RichStat
          label="Concept Types"
          value={stats?.concept_types ?? 0}
          deltaPct={stats?.deltas.concept_types_pct}
          icon={Icon.layers}
          tone="blue"
          spark={sparkConceptTypes}
          sparkColor="#2563eb"
        />
        <RichStat
          label="Entities"
          value={stats?.concepts ?? 0}
          deltaPct={stats?.deltas.concepts_pct}
          icon={Icon.users}
          tone="violet"
          spark={sparkConcepts}
          sparkColor="#7c3aed"
        />
        <RichStat
          label="Relations"
          value={stats?.relations ?? 0}
          deltaPct={stats?.deltas.relations_pct}
          icon={Icon.share}
          tone="amber"
          spark={sparkRelations}
          sparkColor="#d97706"
        />
        <RichStat
          label="Relation Types"
          value={stats?.relation_types ?? 0}
          deltaPct={stats?.deltas.relation_types_pct}
          icon={Icon.shield}
          tone="green"
          spark={sparkRelationTypes}
          sparkColor="#16a34a"
        />
      </div>

      {/* Row 2 — growth chart + network preview */}
      <div className="dash-row dash-row-chart">
        <Card
          title="Ontology Growth (Entities)"
          actions={<Link to="/graph" className="btn-ghost-link">View Analytics</Link>}
        >
          <GrowthChart samples={samples.map((s) => ({ ts: s.ts, value: s.concepts }))} />
          <div className="chart-legend">
            <span><i style={{ background: "var(--accent)" }} /> Entities Added</span>
            <span><i style={{ background: "#94a3b8" }} /> Trend</span>
          </div>
        </Card>
        <Card
          title="Ontology Network Preview"
          actions={<Link to="/graph" className="btn-ghost-link">Open Graph ↗</Link>}
        >
          <NetworkPreview ontology={ontology} />
        </Card>
      </div>

      {/* Row 3 — recent concept types + ingestion + quick actions */}
      <div className="dash-row dash-row-three">
        <Card
          title="Recent Concept Types"
          actions={<Link to="/builder" className="btn-ghost-link">View All</Link>}
        >
          {conceptTypes.length === 0 ? (
            <div className="empty">No ontology defined yet.</div>
          ) : (
            <table className="table compact-table">
              <thead>
                <tr>
                  <th>Name</th>
                  <th>Parent</th>
                  <th>Properties</th>
                  <th>Status</th>
                </tr>
              </thead>
              <tbody>
                {conceptTypes.map(([name, def]) => (
                  <tr key={name}>
                    <td><strong>{name}</strong></td>
                    <td className="muted">{def.parent ?? "—"}</td>
                    <td className="muted">{def.properties ? Object.keys(def.properties).length : 0}</td>
                    <td><span className="badge badge-success">Active</span></td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Card>

        <Card
          title="Files / Ingestion Status"
          actions={<Link to="/files" className="btn-ghost-link">View All</Link>}
        >
          {files.length === 0 ? (
            <div className="empty">No files uploaded yet.</div>
          ) : (
            <ul className="file-list">
              {files.slice(0, 5).map((f) => {
                const st = ingestStatus(f);
                return (
                  <li key={f.id} className="file-row">
                    <div className={fileKindClass(f.kind)}>{f.kind.toUpperCase().slice(0, 4)}</div>
                    <div className="file-meta">
                      <div className="file-name">{f.name}</div>
                      <div className="muted file-sub">{fmtBytes(f.size)} · {f.kind}</div>
                    </div>
                    <span className={`badge ${st.cls}`}>{st.label}</span>
                    <span className="muted file-time">{fmtAgo(f.uploaded_at)}</span>
                  </li>
                );
              })}
            </ul>
          )}
          <Link to="/files" className="dropzone-mini">
            <span className="muted">Drag &amp; drop files anywhere to upload</span>
            <span className="upload-link">{Icon.upload} Upload Files</span>
          </Link>
        </Card>

        <div className="dash-stack">
          <Card title="Quick Actions">
            <div className="quick-actions">
              <Link to="/builder" className="quick-action qa-blue">
                <div className="qa-icon">{Icon.plus}</div>
                <div>
                  <div className="qa-title">Create Ontology</div>
                  <div className="qa-sub muted">Start a new ontology</div>
                </div>
              </Link>
              <Link to="/files" className="quick-action qa-green">
                <div className="qa-icon">{Icon.upload}</div>
                <div>
                  <div className="qa-title">Upload Files</div>
                  <div className="qa-sub muted">Import and process data</div>
                </div>
              </Link>
              <Link to="/queries" className="quick-action qa-violet">
                <div className="qa-icon">{Icon.search}</div>
                <div>
                  <div className="qa-title">Run Query</div>
                  <div className="qa-sub muted">Search your graph</div>
                </div>
              </Link>
              <Link to="/graph" className="quick-action qa-amber">
                <div className="qa-icon">{Icon.graph}</div>
                <div>
                  <div className="qa-title">Open Graph</div>
                  <div className="qa-sub muted">Explore relationships</div>
                </div>
              </Link>
            </div>
          </Card>

          <Card
            title="Recent Queries"
            actions={<Link to="/queries" className="btn-ghost-link">View All</Link>}
          >
            {queries.length === 0 ? (
              <div className="empty">No saved queries yet.</div>
            ) : (
              <ul className="query-list">
                {queries.slice(0, 5).map((q) => (
                  <li key={q.id}>
                    <span className="query-dot" />
                    <span className="query-text" title={q.query}>{q.name}</span>
                    <span className="muted query-time">
                      {q.last_run_at ? fmtAgo(q.last_run_at) : "—"}
                    </span>
                    <span className="query-check">{Icon.check}</span>
                  </li>
                ))}
              </ul>
            )}
          </Card>
        </div>
      </div>

      {/* Row 4 — team activity + top entity types + insights */}
      <div className="dash-row dash-row-three">
        <Card title="Team Activity" actions={<Link to="/files" className="btn-ghost-link">View All</Link>}>
          {files.length === 0 ? (
            <div className="empty">No activity yet.</div>
          ) : (
            <ul className="activity-list">
              {files.slice(0, 4).map((f) => (
                <li key={f.id} className="activity-item">
                  <div className="activity-avatar">{(f.name[0] ?? "?").toUpperCase()}</div>
                  <div className="activity-body">
                    <div className="activity-text">
                      <strong>System</strong> ingested <em>{f.name}</em>
                    </div>
                    <div className="muted activity-time">{fmtAgo(f.uploaded_at)}</div>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </Card>

        <Card title="Top Entity Types" subtitle={`Across ${fmtNum(concepts.length)} entities`}>
          {topTypes.length === 0 ? (
            <div className="empty">No entities yet.</div>
          ) : (
            <ul className="bar-list">
              {topTypes.map(([name, count]) => (
                <li key={name} className="bar-row">
                  <span className="bar-label">{name}</span>
                  <div className="bar-track">
                    <div
                      className="bar-fill"
                      style={{ width: `${Math.max(4, (count / topMax) * 100)}%` }}
                    />
                  </div>
                  <span className="bar-value">{fmtNum(count)}</span>
                </li>
              ))}
            </ul>
          )}
        </Card>

        <Card title="Insights">
          <div className="insights-card">
            <div className="insights-icon">{Icon.spark}</div>
            <div>
              <div className="insights-title">
                {stats && stats.deltas.concepts_pct > 0
                  ? `Entity growth is up ${stats.deltas.concepts_pct.toFixed(0)}%`
                  : "Graph activity overview"}
              </div>
              <div className="muted insights-body">
                {stats
                  ? `You have ${fmtNum(stats.concepts)} entities across ${fmtNum(stats.concept_types)} concept types and ${fmtNum(stats.relations)} relations.`
                  : "Awaiting stats from the server."}
              </div>
            </div>
          </div>
        </Card>
      </div>
    </>
  );
}
