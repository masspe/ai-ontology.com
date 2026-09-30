// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.

import { useEffect, useMemo, useState, type ReactNode } from "react";
import Card from "../components/Card";
import Sparkline from "../components/Sparkline";
// @ts-expect-error JSX module
import { useToast } from "../components/Toast.jsx";
// @ts-expect-error JSX module
import { useConfirm } from "../components/ConfirmDialog.jsx";
import {
  createRule,
  deleteRule,
  generateRule,
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

type RuleStatus = "Active" | "Vérifiée" | "Brouillon" | "Désactivée";

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
  if (raw === "reviewed") return "Vérifiée";
  if (raw === "draft") return "Brouillon";
  if (raw === "disabled") return "Désactivée";
  return r.strict ? "Active" : "Brouillon";
}

function ruleUpdatedAt(r: Rule): string {
  const v = r.properties?.updated_at ?? r.properties?.created_at;
  if (typeof v === "number") {
    return new Date(v * 1000).toLocaleDateString("fr-CH", { year: "numeric", month: "short", day: "numeric" });
  }
  if (typeof v === "string") {
    const t = Date.parse(v);
    if (!isNaN(t)) return new Date(t).toLocaleDateString("fr-CH", { year: "numeric", month: "short", day: "numeric" });
  }
  return "—";
}

function ruleToRow(r: Rule): RuleRow {
  return {
    id: r.id,
    name: r.name,
    type: r.rule_type,
    scope: (r.applies_to?.length ? `${r.applies_to.length} fiche(s)` : "—"),
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
    case "Vérifiée":      return "badge-accent";
    case "Brouillon":  return "badge-warn";
    case "Désactivée": return "badge-danger";
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
  const toast = useToast();
  const confirm = useConfirm();
  const [rules, setRules] = useState<Rule[]>([]);
  const [stats, setStats] = useState<Stats | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [search, setSearch] = useState("");
  const [typeFilter, setTypeFilter] = useState<string>("Tous les types");
  const [statusFilter, setStatusFilter] = useState<string>("Tous les statuts");
  const [sort, setSort] = useState<string>("Tri : dernière mise à jour");
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
    return ["Tous les types", ...Array.from(set).sort()];
  }, [rows]);

  const filtered = useMemo(() => {
    const q = search.trim().toLowerCase();
    let out = rows.filter((r) => {
      if (q && !(r.name.toLowerCase().includes(q) || r.description.toLowerCase().includes(q))) return false;
      if (typeFilter !== "Tous les types" && r.type !== typeFilter) return false;
      if (statusFilter !== "Tous les statuts" && r.status !== statusFilter) return false;
      return true;
    });
    if (sort === "Tri : nom" || sort === "Nom") {
      out = [...out].sort((a, b) => a.name.localeCompare(b.name));
    } else if (sort === "Tri : type" || sort === "Type") {
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
      const key = r.applies_to?.[0] != null ? `Fiche #${r.applies_to[0]}` : "Sans portée";
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
    if (!(await confirm({ title: "Supprimer la règle", message: "Supprimer cette règle ?", confirmLabel: "Supprimer", cancelLabel: "Annuler", danger: true }))) return;
    try {
      await deleteRule(id);
      setRules((rs) => rs.filter((r) => r.id !== id));
      toast.success("Règle supprimée.");
    } catch (e) {
      toast.error("Échec de la suppression : " + (e as Error).message);
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
      toast.error("Échec de l'enregistrement : " + (e as Error).message);
    }
  }

  const totalRules = stats?.rules ?? rules.length;
  const activeRules = rows.filter((r) => r.status === "Active").length;
  const totalRuleTypes = stats?.rule_types ?? new Set(rows.map((r) => r.type)).size;

  return (
    <>
      <div className="page-header">
        <div>
          <h1 className="page-title">Règles</h1>
          <p className="page-subtitle">Ce qui doit être vrai dans vos données, et les exceptions à signaler.</p>
        </div>
      </div>

      {error && <div className="banner banner-error">Échec du chargement des règles : {error}</div>}

      {/* Stats */}
      <div className="dash-row dash-row-stats">
        <RichStat label="Règles au total"      value={String(totalRules)}        icon={Icon.total}  tone="blue"   spark={[]} sparkColor="#2563eb" />
        <RichStat label="Règles actives"       value={String(activeRules)}       icon={Icon.shield} tone="green"  spark={[]} sparkColor="#16a34a" />
        <RichStat label="Types de règle"       value={String(totalRuleTypes)}    icon={Icon.check}  tone="violet" spark={[]} sparkColor="#7c3aed" />
        <RichStat label="Fiches concernées"    value={String(rules.reduce((s, r) => s + (r.applies_to?.length ?? 0), 0))} icon={Icon.bolt}   tone="amber"  spark={[]} sparkColor="#d97706" />
      </div>

      {/* Library + Details */}
      <div className="rules-row">
        <Card
          className="rule-library"
          title="Bibliothèque de règles"
          actions={
            <button
              className="btn-primary rule-create-btn"
              onClick={() => setEditing("new")}
            >
              <span className="qa-icon-inline">{Icon.plus}</span>
              Créer une règle
            </button>
          }
        >
          <div className="rule-toolbar">
            <div className="rule-search">
              <span className="rule-search-icon" aria-hidden>{Icon.search}</span>
              <input
                type="search"
                placeholder="Rechercher une règle..."
                value={search}
                onChange={(e) => setSearch(e.target.value)}
              />
            </div>
            <select value={typeFilter} onChange={(e) => setTypeFilter(e.target.value)} className="rule-select">
              {typeOptions.map((t) => <option key={t}>{t}</option>)}
            </select>
            <select value={statusFilter} onChange={(e) => setStatusFilter(e.target.value)} className="rule-select">
              <option>Tous les statuts</option>
              <option>Active</option>
              <option>Vérifiée</option>
              <option>Brouillon</option>
              <option>Désactivée</option>
            </select>
            <select value={sort} onChange={(e) => setSort(e.target.value)} className="rule-select">
              <option>Tri : dernière mise à jour</option>
              <option>Tri : nom</option>
              <option>Tri : type</option>
            </select>
          </div>

          <table className="table rule-table">
            <thead>
              <tr>
                <th>Nom</th>
                <th>Type</th>
                <th>Portée</th>
                <th>Description</th>
                <th>Statut</th>
                <th>Dernière mise à jour</th>
                <th>Actions</th>
              </tr>
            </thead>
            <tbody>
              {loading && (
                <tr><td colSpan={7} className="muted">Chargement des règles…</td></tr>
              )}
              {!loading && filtered.length === 0 && (
                <tr><td colSpan={7} className="muted">Aucune règle ne correspond aux filtres.</td></tr>
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
                        aria-label="Supprimer la règle"
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
            <span className="muted">{filtered.length} règle(s) affichée(s) sur {rows.length}</span>
          </div>
        </Card>

        <Card
          className="rule-details"
          title="Détails de la règle"
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
                <dt>📦 Type de règle</dt>
                <dd><span className="tag-chip tag-core">{selected.type}</span></dd>
                <dt>⚠ Stricte</dt>
                <dd><span className="tag-chip tag-high">{selectedRule.strict ? "Oui" : "Non"}</span></dd>
                <dt>🕓 Dernière mise à jour</dt>
                <dd>{selected.updated}</dd>
                <dt>🔖 S'applique à</dt>
                <dd>
                  {selectedRule.applies_to?.length
                    ? selectedRule.applies_to.map((cid) => (
                        <span key={cid} className="tag-chip">Fiche #{cid}</span>
                      ))
                    : <span className="muted">—</span>}
                </dd>
              </dl>

              <div className="rd-logic">
                <div className="rd-logic-title">Logique de la règle</div>
                <pre className="rd-code">
                  <code>
                    <span className="ln">1</span><span className="kw">SI</span> {selectedRule.when || "(aucune condition)"}{"\n"}
                    <span className="ln">2</span><span className="kw">ALORS</span> {selectedRule.then || "(aucune action)"}
                  </code>
                </pre>
              </div>

              <div className="rd-actions">
                <button
                  className="btn-ghost"
                  onClick={() => selectedRule && setEditing(selectedRule)}
                >
                  <span className="qa-icon-inline">{Icon.edit}</span> Modifier la règle
                </button>
                <button className="btn-ghost" onClick={() => onDelete(selected.id)}>
                  <span className="qa-icon-inline">{Icon.flask}</span> Supprimer
                </button>
              </div>

              {categories.length > 0 && (
                <div className="rd-categories">
                  <div className="card-title"><span>Catégories de règles</span></div>
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
            <p className="muted">{loading ? "Chargement…" : "Aucune règle sélectionnée."}</p>
          )}
        </Card>
      </div>

      {/* Bottom row */}
      <div className="rules-bottom">
        <Card title="Activité récente">
          <ul className="rule-activity">
            {rules.length === 0 && <li className="muted">Aucune activité pour l'instant.</li>}
            {rules.slice(0, 5).map((r) => (
              <li key={r.id}>
                <span className="act-icon act-ok">{Icon.check}</span>
                <span className="ra-text">Règle « {r.name} » présente</span>
                <span className="muted ra-time">{ruleUpdatedAt(r)}</span>
              </li>
            ))}
          </ul>
        </Card>

        <Card title="Portées principales">
          <ul className="bar-list">
            {domains.length === 0 && <li className="muted">Aucune portée.</li>}
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
  const [prompt, setPrompt] = useState("");
  const [generating, setGenerating] = useState(false);
  const [genError, setGenError] = useState<string | null>(null);
  const [showAppliesError, setShowAppliesError] = useState(false);
  const isEdit = initial != null;

  // Default rule_type once ontology loads.
  useEffect(() => {
    if (!ruleType && ruleTypes.length > 0) setRuleType(ruleTypes[0]!);
  }, [ruleTypes, ruleType]);

  const canGenerate =
    appliesTo.length > 0 && ruleType.trim() !== "" && prompt.trim() !== "" && !generating;

  async function onGenerate() {
    if (!canGenerate) return;
    setGenError(null);
    setGenerating(true);
    try {
      const out = await generateRule(prompt.trim(), ruleType, appliesTo);
      setName(out.name);
      setWhen(out.when);
      setThen(out.then);
      setDescription(out.description);
      setStrict(out.strict);
    } catch (e) {
      setGenError((e as Error).message);
    } finally {
      setGenerating(false);
    }
  }

  function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!name.trim() || !ruleType) return;
    if (appliesTo.length === 0) {
      setShowAppliesError(true);
      return;
    }
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
        <h3 className="modal-title">{isEdit ? "Modifier la règle" : "Créer une règle"}</h3>

        <div className="modal-ai">
          <div className="modal-ai-title">Générer avec l'IA</div>
          <textarea
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
            rows={2}
            placeholder="Décrivez la règle à créer…"
          />
          <div className="modal-ai-row">
            <button
              type="button"
              className="btn-primary"
              onClick={onGenerate}
              disabled={!canGenerate}
            >
              {generating ? "Génération…" : "Générer"}
            </button>
            <small className="muted">
              {appliesTo.length === 0
                ? "Sélectionnez au moins une fiche sous « S'applique à » avant de générer."
                : !ruleType
                  ? "Choisissez d'abord un type de règle."
                  : !prompt.trim()
                    ? "Décrivez la règle pour activer la génération."
                    : "Remplit Nom, Si, Alors, Description, Stricte."}
            </small>
          </div>
          {genError && <div className="modal-ai-error">{genError}</div>}
        </div>

        <label className="modal-field">
          <span>Type de règle</span>
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
          {isEdit && <small className="muted">Le type ne peut pas être modifié.</small>}
        </label>

        <label className="modal-field">
          <span>Nom</span>
          <input value={name} onChange={(e) => setName(e.target.value)} required />
        </label>

        <label className="modal-field">
          <span>Description</span>
          <input value={description} onChange={(e) => setDescription(e.target.value)} />
        </label>

        <label className="modal-field">
          <span>Si</span>
          <textarea value={when} onChange={(e) => setWhen(e.target.value)} rows={2} />
        </label>

        <label className="modal-field">
          <span>Alors</span>
          <textarea value={then} onChange={(e) => setThen(e.target.value)} rows={2} />
        </label>

        <label className="modal-field">
          <span>S'applique à (fiches) <span className="modal-req">*</span></span>
          <select
            multiple
            value={appliesTo.map(String)}
            onChange={(e) => {
              const next = Array.from(e.target.selectedOptions).map((o) => Number(o.value));
              setAppliesTo(next);
              if (next.length > 0) setShowAppliesError(false);
            }}
            size={Math.min(6, Math.max(3, concepts.length))}
            required
            aria-invalid={showAppliesError && appliesTo.length === 0}
          >
            {concepts.map((c) => (
              <option key={c.id} value={c.id}>
                {c.concept_type}: {c.name}
              </option>
            ))}
          </select>
          <small className="muted">
            Maintenez Ctrl/Cmd pour une sélection multiple. Au moins une fiche est requise.
          </small>
          {showAppliesError && appliesTo.length === 0 && (
            <small className="modal-field-error">Sélectionnez au moins une fiche.</small>
          )}
        </label>

        <label className="modal-check">
          <input
            type="checkbox"
            checked={strict}
            onChange={(e) => setStrict(e.target.checked)}
          />
          <span>Stricte (considérée comme active)</span>
        </label>

        <div className="modal-actions">
          <button type="button" className="btn-ghost" onClick={onCancel}>
            Annuler
          </button>
          <button type="submit" className="btn-primary">
            {isEdit ? "Enregistrer" : "Créer"}
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
          .modal-ai { display: flex; flex-direction: column; gap: 6px;
            padding: 10px; border: 1px dashed #c4b5fd; background: #faf5ff;
            border-radius: 8px; }
          .modal-ai-title { font-weight: 600; font-size: 13px; color: #6d28d9; }
          .modal-ai textarea { padding: 8px 10px; border: 1px solid #cbd5e1;
            border-radius: 8px; font: inherit; background: #fff; }
          .modal-ai-row { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
          .modal-ai-error { color: #b91c1c; font-size: 12px; background: #fef2f2;
            border: 1px solid #fecaca; padding: 6px 8px; border-radius: 6px; }
          .modal-req { color: #dc2626; }
          .modal-field-error { color: #b91c1c; }
        `}</style>
      </form>
    </div>
  );
}
