// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { useEffect, useMemo, useState, type ReactNode } from "react";
import Card from "../components/Card";
import Sparkline from "../components/Sparkline";
import {
  createAction,
  deleteAction,
  getOntology,
  getStats,
  listActions,
  listConcepts,
  updateAction,
  type Action as ActionInstance,
  type Concept,
  type Ontology,
  type Stats,
} from "../api";

// ---------------------------------------------------------------------------
// Live action rows derived from the backend.
// ---------------------------------------------------------------------------

type ActionStatus = "Active" | "Reviewed" | "Draft" | "Paused";

interface ActionRow {
  id: number;
  name: string;
  type: string;
  trigger: string;
  description: string;
  status: ActionStatus;
  lastRun: string;
}

function actionStatus(a: ActionInstance): ActionStatus {
  const raw = (a.parameters?.status as string | undefined)?.toLowerCase();
  if (raw === "reviewed") return "Reviewed";
  if (raw === "draft") return "Draft";
  if (raw === "paused") return "Paused";
  return "Active";
}

function actionUpdatedAt(a: ActionInstance): string {
  const v = a.parameters?.updated_at ?? a.parameters?.created_at;
  if (typeof v === "number") {
    return new Date(v * 1000).toLocaleDateString("en-US", { year: "numeric", month: "short", day: "numeric" });
  }
  if (typeof v === "string") {
    const t = Date.parse(v);
    if (!isNaN(t)) return new Date(t).toLocaleDateString("en-US", { year: "numeric", month: "short", day: "numeric" });
  }
  return "—";
}

function actionToRow(a: ActionInstance): ActionRow {
  return {
    id: a.id,
    name: a.name,
    type: a.action_type,
    trigger: (a.parameters?.trigger as string | undefined) ?? "Manual",
    description: a.description ?? a.effect ?? "",
    status: actionStatus(a),
    lastRun: actionUpdatedAt(a),
  };
}

const PALETTE = ["#2563eb", "#7c3aed", "#16a34a", "#d97706", "#dc2626", "#0891b2", "#db2777"];
function typeColor(name: string): string {
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) >>> 0;
  return PALETTE[h % PALETTE.length]!;
}

// ---------------------------------------------------------------------------
// Inline icons (local — no extra deps)
// ---------------------------------------------------------------------------

const Icon = {
  bolt: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M13 2 3 14h7l-1 8 10-12h-7z" />
    </svg>
  ),
  bot: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3" y="8" width="18" height="12" rx="2" />
      <path d="M12 4v4" />
      <circle cx="8.5" cy="14" r="1.2" fill="currentColor" />
      <circle cx="15.5" cy="14" r="1.2" fill="currentColor" />
      <path d="M9 18h6" />
    </svg>
  ),
  checkCircle: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="12" cy="12" r="10" />
      <path d="m9 12 2 2 4-4" />
    </svg>
  ),
  alert: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="12" cy="12" r="10" />
      <path d="M12 8v4" /><path d="M12 16h.01" />
    </svg>
  ),
  search: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="11" cy="11" r="7" /><path d="m21 21-4.3-4.3" />
    </svg>
  ),
  plus: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 5v14" /><path d="M5 12h14" />
    </svg>
  ),
  copy: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <rect x="9" y="9" width="13" height="13" rx="2" />
      <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
    </svg>
  ),
  edit: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 20h9" />
      <path d="M16.5 3.5a2.121 2.121 0 1 1 3 3L7 19l-4 1 1-4z" />
    </svg>
  ),
  play: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="12" cy="12" r="10" />
      <path d="m10 8 6 4-6 4z" />
    </svg>
  ),
  download: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
      <path d="m7 10 5 5 5-5" /><path d="M12 15V3" />
    </svg>
  ),
  expand: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M3 9V3h6" /><path d="M21 15v6h-6" />
      <path d="M3 3l7 7" /><path d="m14 14 7 7" />
    </svg>
  ),
  more: (
    <svg viewBox="0 0 24 24" fill="currentColor">
      <circle cx="5" cy="12" r="1.6" /><circle cx="12" cy="12" r="1.6" /><circle cx="19" cy="12" r="1.6" />
    </svg>
  ),
  import: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
      <path d="m17 8-5-5-5 5" /><path d="M12 3v12" />
    </svg>
  ),
  spark: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="m12 3 1.9 5.8H20l-4.9 3.6L17 18.2 12 14.6 7 18.2l1.9-5.8L4 8.8h6.1z" />
    </svg>
  ),
  bulk: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3" y="4" width="18" height="4" rx="1" />
      <rect x="3" y="10" width="18" height="4" rx="1" />
      <rect x="3" y="16" width="18" height="4" rx="1" />
    </svg>
  ),
  calendar: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3" y="5" width="18" height="16" rx="2" />
      <path d="M16 3v4" /><path d="M8 3v4" /><path d="M3 11h18" />
    </svg>
  ),
  xCircle: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="12" cy="12" r="10" />
      <path d="m15 9-6 6" /><path d="m9 9 6 6" />
    </svg>
  ),
};

// ---------------------------------------------------------------------------
// Type → visual mapping
// ---------------------------------------------------------------------------

function typeBadge(type: string): { cls: string; dot: string } {
  const known: Record<string, { cls: string; dot: string }> = {
    Automation:     { cls: "action-type-automation",     dot: "#2563eb" },
    Inference:      { cls: "action-type-inference",      dot: "#7c3aed" },
    Validation:     { cls: "action-type-validation",     dot: "#16a34a" },
    Transformation: { cls: "action-type-transformation", dot: "#d97706" },
    Alert:          { cls: "action-type-alert",          dot: "#dc2626" },
    "AI Action":    { cls: "action-type-ai",             dot: "#0891b2" },
  };
  return known[type] ?? { cls: "action-type-automation", dot: typeColor(type) };
}

function statusBadge(s: ActionStatus): string {
  switch (s) {
    case "Active":   return "badge-success";
    case "Reviewed": return "badge-accent";
    case "Draft":    return "badge-warn";
    case "Paused":   return "badge-danger";
  }
}

// ---------------------------------------------------------------------------
// Stat tile
// ---------------------------------------------------------------------------

interface RichStatProps {
  label: string;
  value: string;
  deltaPct?: number;
  deltaDir?: "up" | "down";
  icon: ReactNode;
  tone: "blue" | "violet" | "green" | "amber" | "red";
  spark: number[];
  sparkColor: string;
}

function RichStat({ label, value, deltaPct, deltaDir, icon, tone, spark, sparkColor }: RichStatProps) {
  const showDelta = deltaPct != null;
  const cls = deltaDir ?? (showDelta && deltaPct! > 0 ? "up" : showDelta && deltaPct! < 0 ? "down" : "flat");
  const arrow = cls === "up" ? "↑" : cls === "down" ? "↓" : "•";
  return (
    <div className="card stat-rich">
      <div className={`stat-icon tone-${tone}`}>{icon}</div>
      <div className="stat-body">
        <div className="stat-label">{label}</div>
        <div className="stat-value">{value}</div>
        {showDelta && (
          <div className={`stat-delta ${cls}`}>
            <span>{arrow} {Math.abs(deltaPct!).toFixed(0)}%</span>
          </div>
        )}
      </div>
      <div className="stat-spark">
        <Sparkline values={spark} stroke={sparkColor} />
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Donut chart for Run Status Breakdown
// ---------------------------------------------------------------------------

function Donut({ data }: { data: { name: string; count: number; color: string }[] }) {
  const total = data.reduce((s, d) => s + d.count, 0);
  const R = 46;
  const r = 30;
  const C = 2 * Math.PI * ((R + r) / 2);
  let offset = 0;
  const segments = data.map((d) => {
    const len = (d.count / total) * C;
    const seg = { color: d.color, len, offset };
    offset += len;
    return seg;
  });
  return (
    <svg viewBox="0 0 120 120" className="donut">
      <circle cx="60" cy="60" r={(R + r) / 2} fill="none" stroke="#f1f5f9" strokeWidth={R - r} />
      {segments.map((s, i) => (
        <circle
          key={i}
          cx="60"
          cy="60"
          r={(R + r) / 2}
          fill="none"
          stroke={s.color}
          strokeWidth={R - r}
          strokeDasharray={`${s.len} ${C}`}
          strokeDashoffset={-s.offset}
          transform="rotate(-90 60 60)"
          strokeLinecap="butt"
        />
      ))}
    </svg>
  );
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

export default function Actions() {
  const [actions, setActions] = useState<ActionInstance[]>([]);
  const [stats, setStats] = useState<Stats | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [search, setSearch] = useState("");
  const [typeFilter, setTypeFilter] = useState<string>("All Types");
  const [statusFilter, setStatusFilter] = useState<string>("All Statuses");
  const [sort, setSort] = useState<string>("Last Updated");
  const [ontology, setOntology] = useState<Ontology | null>(null);
  const [allConcepts, setAllConcepts] = useState<Concept[]>([]);
  const [editing, setEditing] = useState<ActionInstance | "new" | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [o, cs] = await Promise.all([
          getOntology(),
          listConcepts({ limit: 500 }),
        ]);
        if (cancelled) return;
        setOntology(o);
        setAllConcepts(cs.concepts);
      } catch {
        /* non-fatal */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [as_, st] = await Promise.all([listActions(), getStats()]);
        if (cancelled) return;
        setActions(as_);
        setStats(st);
        if (as_.length > 0) setSelectedId(as_[0]!.id);
      } catch (e) {
        if (!cancelled) setError((e as Error).message);
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => { cancelled = true; };
  }, []);

  const rows = useMemo(() => actions.map(actionToRow), [actions]);

  const typeOptions = useMemo(() => {
    const set = new Set<string>(rows.map((r) => r.type));
    return ["All Types", ...Array.from(set).sort()];
  }, [rows]);

  const filtered = useMemo(() => {
    const q = search.trim().toLowerCase();
    let out = rows.filter((a) => {
      if (q && !(a.name.toLowerCase().includes(q) || a.description.toLowerCase().includes(q))) return false;
      if (typeFilter !== "All Types" && a.type !== typeFilter) return false;
      if (statusFilter !== "All Statuses" && a.status !== statusFilter) return false;
      return true;
    });
    if (sort === "Sort: Name") out = [...out].sort((a, b) => a.name.localeCompare(b.name));
    else if (sort === "Sort: Type") out = [...out].sort((a, b) => a.type.localeCompare(b.type));
    return out;
  }, [rows, search, typeFilter, statusFilter, sort]);

  const selected = useMemo(
    () => rows.find((r) => r.id === selectedId) ?? rows[0] ?? null,
    [rows, selectedId],
  );
  const selectedAction = useMemo(
    () => actions.find((a) => a.id === selectedId) ?? actions[0] ?? null,
    [actions, selectedId],
  );

  // Status breakdown derived from live rows.
  const runStatus = useMemo(() => {
    const counts: Record<ActionStatus, number> = {
      Active: 0, Reviewed: 0, Draft: 0, Paused: 0,
    };
    for (const r of rows) counts[r.status] += 1;
    return [
      { name: "Active",   count: counts.Active,   color: "#2563eb" },
      { name: "Reviewed", count: counts.Reviewed, color: "#7c3aed" },
      { name: "Draft",    count: counts.Draft,    color: "#d97706" },
      { name: "Paused",   count: counts.Paused,   color: "#dc2626" },
    ].filter((d) => d.count > 0);
  }, [rows]);

  // Top types as a domain-like breakdown.
  const domains = useMemo(() => {
    const counts = new Map<string, number>();
    for (const r of rows) counts.set(r.type, (counts.get(r.type) ?? 0) + 1);
    const total = rows.length || 1;
    return Array.from(counts.entries())
      .sort((a, b) => b[1] - a[1])
      .slice(0, 5)
      .map(([name, count]) => ({ name, count, pct: (count / total) * 100 }));
  }, [rows]);

  async function onDelete(id: number) {
    if (!confirm("Delete this action?")) return;
    try {
      await deleteAction(id);
      setActions((rs) => rs.filter((r) => r.id !== id));
    } catch (e) {
      alert("Delete failed: " + (e as Error).message);
    }
  }

  async function onSaveAction(
    payload: Omit<ActionInstance, "id">,
    editingId: number | null,
  ) {
    try {
      if (editingId == null) {
        const { id } = await createAction(payload);
        setActions((rs) => [...rs, { id, ...payload }]);
        setSelectedId(id);
      } else {
        const updated = await updateAction(editingId, {
          name: payload.name,
          subject: payload.subject,
          object: payload.object ?? null,
          parameters: payload.parameters,
          effect: payload.effect,
          description: payload.description,
        });
        setActions((rs) => rs.map((a) => (a.id === editingId ? updated : a)));
      }
      setEditing(null);
    } catch (e) {
      alert("Save failed: " + (e as Error).message);
    }
  }

  const runTotal = runStatus.reduce((s, d) => s + d.count, 0) || 1;
  const totalActions = stats?.actions ?? actions.length;
  const activeActions = rows.filter((r) => r.status === "Active").length;

  return (
    <>
      <div className="page-header">
        <div>
          <h1 className="page-title">Actions</h1>
          <p className="page-subtitle">Create, schedule, monitor, and manage ontology actions and automations.</p>
        </div>
      </div>

      {error && <div className="banner banner-error">Failed to load actions: {error}</div>}

      {/* Stats */}
      <div className="dash-row dash-row-stats">
        <RichStat label="Total Actions"      value={String(totalActions)}              icon={Icon.bolt}        tone="blue"   spark={[]} sparkColor="#2563eb" />
        <RichStat label="Active Automations" value={String(activeActions)}             icon={Icon.bot}         tone="green"  spark={[]} sparkColor="#16a34a" />
        <RichStat label="Action Types"       value={String(stats?.action_types ?? 0)}  icon={Icon.checkCircle} tone="violet" spark={[]} sparkColor="#7c3aed" />
        <RichStat label="Paused/Draft"       value={String(rows.filter((r) => r.status === "Paused" || r.status === "Draft").length)} icon={Icon.alert} tone="red" spark={[]} sparkColor="#dc2626" />
      </div>

      {/* Library + Details */}
      <div className="rules-row">
        <Card
          className="rule-library"
          title="Action Library"
          actions={
            <button
              className="btn-primary rule-create-btn"
              onClick={() => setEditing("new")}
            >
              <span className="qa-icon-inline">{Icon.plus}</span>
              Create Action
            </button>
          }
        >
          <div className="rule-toolbar">
            <div className="rule-search">
              <span className="rule-search-icon" aria-hidden>{Icon.search}</span>
              <input
                type="search"
                placeholder="Search actions..."
                value={search}
                onChange={(e) => setSearch(e.target.value)}
              />
            </div>
            <select value={typeFilter} onChange={(e) => setTypeFilter(e.target.value)} className="rule-select">
              {typeOptions.map((t) => <option key={t}>{t}</option>)}
            </select>
            <select value={statusFilter} onChange={(e) => setStatusFilter(e.target.value)} className="rule-select">
              <option>All Statuses</option>
              <option>Active</option>
              <option>Reviewed</option>
              <option>Draft</option>
              <option>Paused</option>
            </select>
            <select value={sort} onChange={(e) => setSort(e.target.value)} className="rule-select">
              <option>Sort: Last Updated</option>
              <option>Sort: Name</option>
              <option>Sort: Type</option>
            </select>
          </div>

          <table className="table rule-table">
            <thead>
              <tr>
                <th>Action Name</th>
                <th>Type</th>
                <th>Trigger</th>
                <th>Description</th>
                <th>Status</th>
                <th>Last Run</th>
                <th>Actions</th>
              </tr>
            </thead>
            <tbody>
              {loading && <tr><td colSpan={7} className="muted">Loading actions…</td></tr>}
              {!loading && filtered.length === 0 && <tr><td colSpan={7} className="muted">No actions match the current filters.</td></tr>}
              {filtered.map((a) => {
                const tb = typeBadge(a.type);
                return (
                  <tr
                    key={a.id}
                    className={selectedId === a.id ? "is-selected" : ""}
                    onClick={() => setSelectedId(a.id)}
                  >
                    <td>
                      <span className={`rule-name-chip ${tb.cls}`} aria-hidden>
                        <i style={{ background: tb.dot }} />
                      </span>
                      <strong className="rule-name-text">{a.name}</strong>
                    </td>
                    <td className="muted">{a.type}</td>
                    <td className="muted">{a.trigger}</td>
                    <td className="muted rule-desc">{a.description}</td>
                    <td><span className={`badge ${statusBadge(a.status)}`}>{a.status}</span></td>
                    <td className="muted">{a.lastRun}</td>
                    <td>
                      <button
                        className="btn-ghost icon-btn"
                        aria-label="Delete action"
                        onClick={(e) => { e.stopPropagation(); onDelete(a.id); }}
                      >
                        {Icon.more}
                      </button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>

          <div className="rule-pagination">
            <span className="muted">Showing {filtered.length} of {rows.length} actions</span>
          </div>
        </Card>

        <Card
          className="rule-details"
          title="Action Details"
          actions={<button className="btn-ghost icon-btn" aria-label="Expand">{Icon.expand}</button>}
        >
          {selected && selectedAction ? (
            <>
              <div className="rd-header">
                <div className={`rd-avatar ${typeBadge(selected.type).cls}`}>
                  {Icon.bolt}
                </div>
                <div className="rd-heading">
                  <h3>
                    {selected.name}
                    <span className="badge badge-accent rd-type-badge">{selected.type}</span>
                  </h3>
                </div>
              </div>

              <dl className="rd-grid">
                <dt>🔗 ID</dt>
                <dd className="rd-uri"><span>action:{selected.id}</span></dd>
                <dt>📦 Action Type</dt>
                <dd><span className="tag-chip tag-core">{selected.type}</span></dd>
                <dt>⚡ Trigger</dt>
                <dd><span className="tag-chip">{selected.trigger}</span></dd>
                <dt>🎯 Subject</dt>
                <dd><span className="tag-chip">Concept #{selectedAction.subject}</span></dd>
                <dt>🎯 Object</dt>
                <dd>{selectedAction.object != null ? <span className="tag-chip">Concept #{selectedAction.object}</span> : <span className="muted">—</span>}</dd>
                <dt>🕓 Last Updated</dt>
                <dd>{selected.lastRun}</dd>
              </dl>

              <div className="rd-logic">
                <div className="rd-logic-title">Effect</div>
                <pre className="rd-code">
                  <code>
                    <span className="ln">1</span>{selectedAction.effect || "(no effect declared)"}
                  </code>
                </pre>
              </div>

              <div className="rd-actions">
                <button
                  className="btn-ghost"
                  onClick={() => selectedAction && setEditing(selectedAction)}
                >
                  <span className="qa-icon-inline">{Icon.edit}</span> Edit Action
                </button>
                <button className="btn-ghost"><span className="qa-icon-inline">{Icon.play}</span> Run Now</button>
                <button className="btn-ghost" onClick={() => onDelete(selected.id)}>
                  <span className="qa-icon-inline">{Icon.download}</span> Delete
                </button>
              </div>

              {runStatus.length > 0 && (
                <div className="rd-categories">
                  <div className="card-title"><span>Status Breakdown</span></div>
                  <div className="rd-cat-row">
                    <Donut data={runStatus} />
                    <ul className="rd-cat-legend">
                      {runStatus.map((c) => (
                        <li key={c.name}>
                          <i style={{ background: c.color }} />
                          <span className="rd-cat-name">{c.name}</span>
                          <span className="rd-cat-count">{c.count}</span>
                          <span className="muted rd-cat-pct">{((c.count / runTotal) * 100).toFixed(0)}%</span>
                        </li>
                      ))}
                    </ul>
                  </div>
                </div>
              )}
            </>
          ) : (
            <p className="muted">{loading ? "Loading…" : "No action selected."}</p>
          )}
        </Card>
      </div>

      {/* Bottom row */}
      <div className="actions-bottom">
        <Card title="Recent Action Activity">
          <ul className="rule-activity">
            {actions.length === 0 && <li className="muted">No activity yet.</li>}
            {actions.slice(0, 5).map((a) => (
              <li key={a.id}>
                <span className="act-icon act-ok">{Icon.checkCircle}</span>
                <span className="ra-text">Action "{a.name}" present</span>
                <span className="muted ra-time">{actionUpdatedAt(a)}</span>
              </li>
            ))}
          </ul>
        </Card>

        <Card title="Top Action Types">
          <ul className="bar-list">
            {domains.length === 0 && <li className="muted">No types.</li>}
            {domains.map((d) => (
              <li key={d.name} className="bar-row">
                <span className="bar-label">{d.name}</span>
                <div className="bar-track">
                  <div className="bar-fill" style={{ width: `${(d.count / (domains[0]?.count ?? 1)) * 100}%` }} />
                </div>
                <span className="bar-value">{d.count} ({d.pct.toFixed(1)}%)</span>
              </li>
            ))}
          </ul>
        </Card>

        <Card title="Quick Actions">
          <div className="quick-actions rule-quick">
            <a className="quick-action qa-blue" href="#" onClick={(e) => e.preventDefault()}>
              <div className="qa-icon">{Icon.import}</div>
              <div>
                <div className="qa-title">Import Actions</div>
                <div className="qa-sub muted">Import from files or sources</div>
              </div>
            </a>
            <a className="quick-action qa-violet" href="#" onClick={(e) => e.preventDefault()}>
              <div className="qa-icon">{Icon.spark}</div>
              <div>
                <div className="qa-title">Generate with AI</div>
                <div className="qa-sub muted">Auto-generate actions</div>
              </div>
            </a>
            <a className="quick-action qa-amber" href="#" onClick={(e) => e.preventDefault()}>
              <div className="qa-icon">{Icon.bulk}</div>
              <div>
                <div className="qa-title">Bulk Edit</div>
                <div className="qa-sub muted">Edit multiple actions</div>
              </div>
            </a>
            <a className="quick-action qa-green" href="#" onClick={(e) => e.preventDefault()}>
              <div className="qa-icon">{Icon.play}</div>
              <div>
                <div className="qa-title">Run All Active</div>
                <div className="qa-sub muted">Run all active actions</div>
              </div>
            </a>
          </div>
        </Card>
      </div>

      {editing != null && (
        <ActionModal
          initial={editing === "new" ? null : editing}
          ontology={ontology}
          concepts={allConcepts}
          onCancel={() => setEditing(null)}
          onSave={(payload) =>
            onSaveAction(payload, editing === "new" ? null : editing.id)
          }
        />
      )}
    </>
  );
}

// ---------------------------------------------------------------------------
// Create / Edit modal
// ---------------------------------------------------------------------------

interface ActionModalProps {
  initial: ActionInstance | null;
  ontology: Ontology | null;
  concepts: Concept[];
  onCancel: () => void;
  onSave: (payload: Omit<ActionInstance, "id">) => void;
}

function ActionModal({ initial, ontology, concepts, onCancel, onSave }: ActionModalProps) {
  const actionTypes = useMemo(
    () => Object.keys(ontology?.action_types ?? {}).sort(),
    [ontology],
  );
  const [actionType, setActionType] = useState(initial?.action_type ?? actionTypes[0] ?? "");
  const [name, setName] = useState(initial?.name ?? "");
  const [subject, setSubject] = useState<number | "">(initial?.subject ?? "");
  const [object, setObject] = useState<number | "">(initial?.object ?? "");
  const [effect, setEffect] = useState(initial?.effect ?? "");
  const [description, setDescription] = useState(initial?.description ?? "");
  const isEdit = initial != null;

  useEffect(() => {
    if (!actionType && actionTypes.length > 0) setActionType(actionTypes[0]!);
  }, [actionTypes, actionType]);

  function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!name.trim() || !actionType || subject === "") return;
    onSave({
      action_type: actionType,
      name: name.trim(),
      subject: Number(subject),
      object: object === "" ? null : Number(object),
      parameters: initial?.parameters ?? {},
      effect,
      description,
    });
  }

  return (
    <div className="modal-backdrop" onClick={onCancel}>
      <form
        className="modal-card"
        onClick={(e) => e.stopPropagation()}
        onSubmit={submit}
      >
        <h3 className="modal-title">{isEdit ? "Edit Action" : "Create Action"}</h3>

        <label className="modal-field">
          <span>Action Type</span>
          {actionTypes.length > 0 ? (
            <select
              value={actionType}
              onChange={(e) => setActionType(e.target.value)}
              disabled={isEdit}
              required
            >
              {actionTypes.map((t) => (
                <option key={t} value={t}>{t}</option>
              ))}
            </select>
          ) : (
            <input
              value={actionType}
              onChange={(e) => setActionType(e.target.value)}
              disabled={isEdit}
              required
            />
          )}
          {isEdit && <small className="muted">Type is immutable.</small>}
        </label>

        <label className="modal-field">
          <span>Name</span>
          <input value={name} onChange={(e) => setName(e.target.value)} required />
        </label>

        <label className="modal-field">
          <span>Subject (concept)</span>
          <select
            value={subject === "" ? "" : String(subject)}
            onChange={(e) => setSubject(e.target.value === "" ? "" : Number(e.target.value))}
            required
          >
            <option value="" disabled>Select a subject concept…</option>
            {concepts.map((c) => (
              <option key={c.id} value={c.id}>
                {c.concept_type}: {c.name}
              </option>
            ))}
          </select>
        </label>

        <label className="modal-field">
          <span>Object (concept, optional)</span>
          <select
            value={object === "" ? "" : String(object)}
            onChange={(e) => setObject(e.target.value === "" ? "" : Number(e.target.value))}
          >
            <option value="">(none)</option>
            {concepts.map((c) => (
              <option key={c.id} value={c.id}>
                {c.concept_type}: {c.name}
              </option>
            ))}
          </select>
        </label>

        <label className="modal-field">
          <span>Effect</span>
          <textarea value={effect} onChange={(e) => setEffect(e.target.value)} rows={2} />
        </label>

        <label className="modal-field">
          <span>Description</span>
          <input value={description} onChange={(e) => setDescription(e.target.value)} />
        </label>

        <div className="modal-actions">
          <button type="button" className="btn-ghost" onClick={onCancel}>
            Cancel
          </button>
          <button type="submit" className="btn-primary">
            {isEdit ? "Save" : "Create"}
          </button>
        </div>

        <style>{`
          .modal-backdrop { position: fixed; inset: 0; background: rgba(15,23,42,.45);
            display: flex; align-items: center; justify-content: center; z-index: 1000; }
          .modal-card { background: #fff; border-radius: 12px; padding: 24px;
            width: min(520px, 92vw); max-height: 90vh; overflow: auto;
            display: flex; flex-direction: column; gap: 12px;
            box-shadow: 0 20px 60px rgba(0,0,0,.25); }
          .modal-title { margin: 0 0 4px; font-size: 18px; font-weight: 600; }
          .modal-field { display: flex; flex-direction: column; gap: 4px; font-size: 13px; }
          .modal-field > span { font-weight: 500; color: #334155; }
          .modal-field input, .modal-field textarea, .modal-field select {
            padding: 8px 10px; border: 1px solid #cbd5e1; border-radius: 8px;
            font: inherit; background: #fff; }
          .modal-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 8px; }
        `}</style>
      </form>
    </div>
  );
}
