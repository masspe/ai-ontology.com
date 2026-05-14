// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { useEffect, useMemo, useState, type ReactNode } from "react";
import Card from "../components/Card";
import Sparkline from "../components/Sparkline";
import {
  createRule,
  deleteRule,
  getOntology,
  getStats,
  listConcepts,
  listRules,
  updateRule,
  type Concept,
  type Ontology,
  type Rule,
  type Stats,
} from "../api";

// ---------------------------------------------------------------------------
// Rule row derived from the live API.
// ---------------------------------------------------------------------------

type RuleStatus = "Active" | "Reviewed" | "Draft" | "Disabled";

interface RuleRow {
  id: number;
  name: string;
  type: string;
  scope: string;
  description: string;
  status: RuleStatus;
  updated: string;
}

function ruleStatus(r: Rule): RuleStatus {
  const raw = (r.properties?.status as string | undefined)?.toLowerCase();
  if (raw === "reviewed") return "Reviewed";
  if (raw === "draft") return "Draft";
  if (raw === "disabled") return "Disabled";
  return r.strict ? "Active" : "Draft";
}

function ruleUpdatedAt(r: Rule): string {
  const v = r.properties?.updated_at ?? r.properties?.created_at;
  if (typeof v === "number") {
    return new Date(v * 1000).toLocaleDateString("en-US", { year: "numeric", month: "short", day: "numeric" });
  }
  if (typeof v === "string") {
    const t = Date.parse(v);
    if (!isNaN(t)) return new Date(t).toLocaleDateString("en-US", { year: "numeric", month: "short", day: "numeric" });
  }
  return "—";
}

function ruleToRow(r: Rule): RuleRow {
  return {
    id: r.id,
    name: r.name,
    type: r.rule_type,
    scope: (r.applies_to?.length ? `${r.applies_to.length} concept(s)` : "—"),
    description: r.description ?? r.when ?? "",
    status: ruleStatus(r),
    updated: ruleUpdatedAt(r),
  };
}

const PALETTE = ["#2563eb", "#7c3aed", "#d97706", "#16a34a", "#dc2626", "#0ea5e9", "#db2777"];

function typeColor(name: string): string {
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) >>> 0;
  return PALETTE[h % PALETTE.length]!;
}

// ---------------------------------------------------------------------------
// Inline icons (kept local — no extra deps)
// ---------------------------------------------------------------------------

const Icon = {
  total: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
      <path d="M14 2v6h6" />
      <path d="M9 13h6" /><path d="M9 17h4" />
    </svg>
  ),
  shield: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
      <path d="m9 12 2 2 4-4" />
    </svg>
  ),
  check: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="12" cy="12" r="10" />
      <path d="m9 12 2 2 4-4" />
    </svg>
  ),
  bolt: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M13 2 3 14h7l-1 8 10-12h-7z" />
    </svg>
  ),
  search: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="11" cy="11" r="7" />
      <path d="m21 21-4.3-4.3" />
    </svg>
  ),
  plus: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 5v14" />
      <path d="M5 12h14" />
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
  flask: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M9 2h6" />
      <path d="M10 2v6L4 20a2 2 0 0 0 2 3h12a2 2 0 0 0 2-3l-6-12V2" />
    </svg>
  ),
  download: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
      <path d="m7 10 5 5 5-5" />
      <path d="M12 15V3" />
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
      <path d="m17 8-5-5-5 5" />
      <path d="M12 3v12" />
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
  play: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="12" cy="12" r="10" />
      <path d="m10 8 6 4-6 4z" />
    </svg>
  ),
};

// ---------------------------------------------------------------------------
// Type → visual mapping
// ---------------------------------------------------------------------------

function typeBadge(type: string): { cls: string; dot: string } {
  // Built-in categories get distinct CSS classes; fall back to a deterministic
  // color from the palette so user-defined rule types stay visually stable.
  const known: Record<string, { cls: string; dot: string }> = {
    Validation:     { cls: "rule-type-validation",     dot: "#2563eb" },
    Inference:      { cls: "rule-type-inference",      dot: "#7c3aed" },
    Constraint:     { cls: "rule-type-constraint",     dot: "#d97706" },
    Transformation: { cls: "rule-type-transformation", dot: "#16a34a" },
  };
  return known[type] ?? { cls: "rule-type-validation", dot: typeColor(type) };
}

function statusBadge(s: RuleStatus): string {
  switch (s) {
    case "Active":   return "badge-success";
    case "Reviewed": return "badge-accent";
    case "Draft":    return "badge-warn";
    case "Disabled": return "badge-danger";
  }
}

// ---------------------------------------------------------------------------
// Stat tile
// ---------------------------------------------------------------------------

interface RichStatProps {
  label: string;
  value: string;
  deltaPct?: number;
  icon: ReactNode;
  tone: "blue" | "violet" | "green" | "amber";
  spark: number[];
  sparkColor: string;
}

function RichStat({ label, value, deltaPct, icon, tone, spark, sparkColor }: RichStatProps) {
  const showDelta = deltaPct != null;
  const cls = showDelta && deltaPct! > 0 ? "up" : showDelta && deltaPct! < 0 ? "down" : "flat";
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
// Donut chart for Rule Categories
// ---------------------------------------------------------------------------

function Donut({ data }: { data: { name: string; count: number; pct: number; color: string }[] }) {
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

export default function Rules() {
  const [rules, setRules] = useState<Rule[]>([]);
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
  const [editing, setEditing] = useState<Rule | "new" | null>(null);

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
        /* non-fatal: form picks degrade to free text */
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
        const [rs, st] = await Promise.all([listRules(), getStats()]);
        if (cancelled) return;
        setRules(rs);
        setStats(st);
        if (rs.length > 0) setSelectedId(rs[0]!.id);
      } catch (e) {
        if (!cancelled) setError((e as Error).message);
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const rows = useMemo(() => rules.map(ruleToRow), [rules]);

  const typeOptions = useMemo(() => {
    const set = new Set<string>(rows.map((r) => r.type));
    return ["All Types", ...Array.from(set).sort()];
  }, [rows]);

  const filtered = useMemo(() => {
    const q = search.trim().toLowerCase();
    let out = rows.filter((r) => {
      if (q && !(r.name.toLowerCase().includes(q) || r.description.toLowerCase().includes(q))) return false;
      if (typeFilter !== "All Types" && r.type !== typeFilter) return false;
      if (statusFilter !== "All Statuses" && r.status !== statusFilter) return false;
      return true;
    });
    if (sort === "Sort: Name" || sort === "Name") {
      out = [...out].sort((a, b) => a.name.localeCompare(b.name));
    } else if (sort === "Sort: Type" || sort === "Type") {
      out = [...out].sort((a, b) => a.type.localeCompare(b.type));
    }
    return out;
  }, [rows, search, typeFilter, statusFilter, sort]);

  const categories = useMemo(() => {
    const counts = new Map<string, number>();
    for (const r of rows) counts.set(r.type, (counts.get(r.type) ?? 0) + 1);
    const total = rows.length || 1;
    return Array.from(counts.entries()).map(([name, count]) => ({
      name,
      count,
      pct: (count / total) * 100,
      color: typeBadge(name).dot,
    }));
  }, [rows]);

  const domains = useMemo(() => {
    // Group by first concept-id in `applies_to` (or "Unscoped").
    const counts = new Map<string, number>();
    for (const r of rules) {
      const key = r.applies_to?.[0] != null ? `Concept #${r.applies_to[0]}` : "Unscoped";
      counts.set(key, (counts.get(key) ?? 0) + 1);
    }
    const total = rules.length || 1;
    return Array.from(counts.entries())
      .sort((a, b) => b[1] - a[1])
      .slice(0, 5)
      .map(([name, count]) => ({ name, count, pct: (count / total) * 100 }));
  }, [rules]);

  const selected = useMemo(
    () => rows.find((r) => r.id === selectedId) ?? rows[0] ?? null,
    [rows, selectedId],
  );
  const selectedRule = useMemo(
    () => rules.find((r) => r.id === selectedId) ?? rules[0] ?? null,
    [rules, selectedId],
  );

  async function onDelete(id: number) {
    if (!confirm("Delete this rule?")) return;
    try {
      await deleteRule(id);
      setRules((rs) => rs.filter((r) => r.id !== id));
    } catch (e) {
      alert("Delete failed: " + (e as Error).message);
    }
  }

  async function onSaveRule(payload: Omit<Rule, "id">, editingId: number | null) {
    try {
      if (editingId == null) {
        const { id } = await createRule(payload);
        setRules((rs) => [...rs, { id, ...payload }]);
        setSelectedId(id);
      } else {
        const updated = await updateRule(editingId, {
          name: payload.name,
          when: payload.when,
          then: payload.then,
          applies_to: payload.applies_to,
          strict: payload.strict,
          description: payload.description,
          properties: payload.properties,
        });
        setRules((rs) => rs.map((r) => (r.id === editingId ? updated : r)));
      }
      setEditing(null);
    } catch (e) {
      alert("Save failed: " + (e as Error).message);
    }
  }

  const totalRules = stats?.rules ?? rules.length;
  const activeRules = rows.filter((r) => r.status === "Active").length;
  const totalRuleTypes = stats?.rule_types ?? new Set(rows.map((r) => r.type)).size;

  return (
    <>
      <div className="page-header">
        <div>
          <h1 className="page-title">Rules</h1>
          <p className="page-subtitle">Create, validate, and manage ontology rules, constraints, and inference logic.</p>
        </div>
      </div>

      {error && <div className="banner banner-error">Failed to load rules: {error}</div>}

      {/* Stats */}
      <div className="dash-row dash-row-stats">
        <RichStat label="Total Rules"         value={String(totalRules)}        icon={Icon.total}  tone="blue"   spark={[]} sparkColor="#2563eb" />
        <RichStat label="Active Rules"        value={String(activeRules)}       icon={Icon.shield} tone="green"  spark={[]} sparkColor="#16a34a" />
        <RichStat label="Rule Types"          value={String(totalRuleTypes)}    icon={Icon.check}  tone="violet" spark={[]} sparkColor="#7c3aed" />
        <RichStat label="Applied Concepts"    value={String(rules.reduce((s, r) => s + (r.applies_to?.length ?? 0), 0))} icon={Icon.bolt}   tone="amber"  spark={[]} sparkColor="#d97706" />
      </div>

      {/* Library + Details */}
      <div className="rules-row">
        <Card
          className="rule-library"
          title="Rule Library"
          actions={
            <button
              className="btn-primary rule-create-btn"
              onClick={() => setEditing("new")}
            >
              <span className="qa-icon-inline">{Icon.plus}</span>
              Create Rule
            </button>
          }
        >
          <div className="rule-toolbar">
            <div className="rule-search">
              <span className="rule-search-icon" aria-hidden>{Icon.search}</span>
              <input
                type="search"
                placeholder="Search rules..."
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
              <option>Disabled</option>
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
                <th>Rule Name</th>
                <th>Type</th>
                <th>Scope</th>
                <th>Description</th>
                <th>Status</th>
                <th>Last Updated</th>
                <th>Actions</th>
              </tr>
            </thead>
            <tbody>
              {loading && (
                <tr><td colSpan={7} className="muted">Loading rules…</td></tr>
              )}
              {!loading && filtered.length === 0 && (
                <tr><td colSpan={7} className="muted">No rules match the current filters.</td></tr>
              )}
              {filtered.map((r) => {
                const tb = typeBadge(r.type);
                return (
                  <tr
                    key={r.id}
                    className={selectedId === r.id ? "is-selected" : ""}
                    onClick={() => setSelectedId(r.id)}
                  >
                    <td>
                      <span className={`rule-name-chip ${tb.cls}`} aria-hidden>
                        <i style={{ background: tb.dot }} />
                      </span>
                      <strong className="rule-name-text">{r.name}</strong>
                    </td>
                    <td className="muted">{r.type}</td>
                    <td className="muted">{r.scope}</td>
                    <td className="muted rule-desc">{r.description}</td>
                    <td><span className={`badge ${statusBadge(r.status)}`}>{r.status}</span></td>
                    <td className="muted">{r.updated}</td>
                    <td>
                      <button
                        className="btn-ghost icon-btn"
                        aria-label="Delete rule"
                        onClick={(e) => { e.stopPropagation(); onDelete(r.id); }}
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
            <span className="muted">Showing {filtered.length} of {rows.length} rules</span>
          </div>
        </Card>

        <Card
          className="rule-details"
          title="Rule Details"
          actions={<button className="btn-ghost icon-btn" aria-label="Expand">{Icon.expand}</button>}
        >
          {selected && selectedRule ? (
            <>
              <div className="rd-header">
                <div className={`rd-avatar ${typeBadge(selected.type).cls}`}>
                  {Icon.shield}
                </div>
                <div className="rd-heading">
                  <h3>
                    {selected.name}
                    <span className="badge badge-accent rd-type-badge">{selected.type}</span>
                  </h3>
                  <p className="muted">{selected.description || "—"}</p>
                </div>
              </div>

              <dl className="rd-grid">
                <dt>🔗 ID</dt>
                <dd className="rd-uri">
                  <span>rule:{selected.id}</span>
                </dd>
                <dt>📦 Rule Type</dt>
                <dd><span className="tag-chip tag-core">{selected.type}</span></dd>
                <dt>⚠ Strict</dt>
                <dd><span className="tag-chip tag-high">{selectedRule.strict ? "Yes" : "No"}</span></dd>
                <dt>🕓 Last Updated</dt>
                <dd>{selected.updated}</dd>
                <dt>🔖 Applies To</dt>
                <dd>
                  {selectedRule.applies_to?.length
                    ? selectedRule.applies_to.map((cid) => (
                        <span key={cid} className="tag-chip">Concept #{cid}</span>
                      ))
                    : <span className="muted">—</span>}
                </dd>
              </dl>

              <div className="rd-logic">
                <div className="rd-logic-title">Rule Logic</div>
                <pre className="rd-code">
                  <code>
                    <span className="ln">1</span><span className="kw">WHEN</span> {selectedRule.when || "(no condition)"}{"\n"}
                    <span className="ln">2</span><span className="kw">THEN</span> {selectedRule.then || "(no action)"}
                  </code>
                </pre>
              </div>

              <div className="rd-actions">
                <button
                  className="btn-ghost"
                  onClick={() => selectedRule && setEditing(selectedRule)}
                >
                  <span className="qa-icon-inline">{Icon.edit}</span> Edit Rule
                </button>
                <button className="btn-ghost" onClick={() => onDelete(selected.id)}>
                  <span className="qa-icon-inline">{Icon.flask}</span> Delete
                </button>
              </div>

              {categories.length > 0 && (
                <div className="rd-categories">
                  <div className="card-title"><span>Rule Categories</span></div>
                  <div className="rd-cat-row">
                    <Donut data={categories} />
                    <ul className="rd-cat-legend">
                      {categories.map((c) => (
                        <li key={c.name}>
                          <i style={{ background: c.color }} />
                          <span className="rd-cat-name">{c.name}</span>
                          <span className="rd-cat-count">{c.count}</span>
                          <span className="muted rd-cat-pct">{c.pct.toFixed(1)}%</span>
                        </li>
                      ))}
                    </ul>
                  </div>
                </div>
              )}
            </>
          ) : (
            <p className="muted">{loading ? "Loading…" : "No rule selected."}</p>
          )}
        </Card>
      </div>

      {/* Bottom row */}
      <div className="rules-bottom">
        <Card title="Recent Rule Activity">
          <ul className="rule-activity">
            {rules.length === 0 && <li className="muted">No activity yet.</li>}
            {rules.slice(0, 5).map((r) => (
              <li key={r.id}>
                <span className="act-icon act-ok">{Icon.check}</span>
                <span className="ra-text">Rule "{r.name}" present</span>
                <span className="muted ra-time">{ruleUpdatedAt(r)}</span>
              </li>
            ))}
          </ul>
        </Card>

        <Card title="Top Scopes">
          <ul className="bar-list">
            {domains.length === 0 && <li className="muted">No scopes.</li>}
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
                <div className="qa-title">Import Rules</div>
                <div className="qa-sub muted">Import rules from files or sources</div>
              </div>
            </a>
            <a className="quick-action qa-violet" href="#" onClick={(e) => e.preventDefault()}>
              <div className="qa-icon">{Icon.spark}</div>
              <div>
                <div className="qa-title">Generate with AI</div>
                <div className="qa-sub muted">Auto-generate rules from data</div>
              </div>
            </a>
            <a className="quick-action qa-amber" href="#" onClick={(e) => e.preventDefault()}>
              <div className="qa-icon">{Icon.bulk}</div>
              <div>
                <div className="qa-title">Bulk Edit</div>
                <div className="qa-sub muted">Edit multiple rules</div>
              </div>
            </a>
            <a className="quick-action qa-green" href="#" onClick={(e) => e.preventDefault()}>
              <div className="qa-icon">{Icon.play}</div>
              <div>
                <div className="qa-title">Run Validation</div>
                <div className="qa-sub muted">Validate all rules and constraints</div>
              </div>
            </a>
          </div>
        </Card>
      </div>

      {editing != null && (
        <RuleModal
          initial={editing === "new" ? null : editing}
          ontology={ontology}
          concepts={allConcepts}
          onCancel={() => setEditing(null)}
          onSave={(payload) =>
            onSaveRule(payload, editing === "new" ? null : editing.id)
          }
        />
      )}
    </>
  );
}

// ---------------------------------------------------------------------------
// Create / Edit modal
// ---------------------------------------------------------------------------

interface RuleModalProps {
  initial: Rule | null;
  ontology: Ontology | null;
  concepts: Concept[];
  onCancel: () => void;
  onSave: (payload: Omit<Rule, "id">) => void;
}

function RuleModal({ initial, ontology, concepts, onCancel, onSave }: RuleModalProps) {
  const ruleTypes = useMemo(
    () => Object.keys(ontology?.rule_types ?? {}).sort(),
    [ontology],
  );
  const [ruleType, setRuleType] = useState(initial?.rule_type ?? ruleTypes[0] ?? "");
  const [name, setName] = useState(initial?.name ?? "");
  const [when, setWhen] = useState(initial?.when ?? "");
  const [then, setThen] = useState(initial?.then ?? "");
  const [strict, setStrict] = useState(initial?.strict ?? false);
  const [description, setDescription] = useState(initial?.description ?? "");
  const [appliesTo, setAppliesTo] = useState<number[]>(initial?.applies_to ?? []);
  const isEdit = initial != null;

  // Default rule_type once ontology loads.
  useEffect(() => {
    if (!ruleType && ruleTypes.length > 0) setRuleType(ruleTypes[0]!);
  }, [ruleTypes, ruleType]);

  function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!name.trim() || !ruleType) return;
    onSave({
      rule_type: ruleType,
      name: name.trim(),
      when,
      then,
      applies_to: appliesTo,
      strict,
      description,
      properties: initial?.properties ?? {},
    });
  }

  return (
    <div className="modal-backdrop" onClick={onCancel}>
      <form
        className="modal-card"
        onClick={(e) => e.stopPropagation()}
        onSubmit={submit}
      >
        <h3 className="modal-title">{isEdit ? "Edit Rule" : "Create Rule"}</h3>

        <label className="modal-field">
          <span>Rule Type</span>
          {ruleTypes.length > 0 ? (
            <select
              value={ruleType}
              onChange={(e) => setRuleType(e.target.value)}
              disabled={isEdit}
              required
            >
              {ruleTypes.map((t) => (
                <option key={t} value={t}>{t}</option>
              ))}
            </select>
          ) : (
            <input
              value={ruleType}
              onChange={(e) => setRuleType(e.target.value)}
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
          <span>Description</span>
          <input value={description} onChange={(e) => setDescription(e.target.value)} />
        </label>

        <label className="modal-field">
          <span>When</span>
          <textarea value={when} onChange={(e) => setWhen(e.target.value)} rows={2} />
        </label>

        <label className="modal-field">
          <span>Then</span>
          <textarea value={then} onChange={(e) => setThen(e.target.value)} rows={2} />
        </label>

        <label className="modal-field">
          <span>Applies To (concepts)</span>
          <select
            multiple
            value={appliesTo.map(String)}
            onChange={(e) =>
              setAppliesTo(
                Array.from(e.target.selectedOptions).map((o) => Number(o.value)),
              )
            }
            size={Math.min(6, Math.max(3, concepts.length))}
          >
            {concepts.map((c) => (
              <option key={c.id} value={c.id}>
                {c.concept_type}: {c.name}
              </option>
            ))}
          </select>
          <small className="muted">Hold Ctrl/Cmd to multi-select. Empty = global.</small>
        </label>

        <label className="modal-check">
          <input
            type="checkbox"
            checked={strict}
            onChange={(e) => setStrict(e.target.checked)}
          />
          <span>Strict (treated as active)</span>
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
          .modal-field select[multiple] { padding: 4px; }
          .modal-check { display: flex; align-items: center; gap: 8px; font-size: 13px; }
          .modal-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 8px; }
        `}</style>
      </form>
    </div>
  );
}
