// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { useEffect, useMemo, useState, type ReactNode } from "react";
import { Link } from "react-router-dom";
import Card from "../components/Card";
import Sparkline from "../components/Sparkline";
import {
  createConcept,
  createRelation,
  deleteConcept,
  deleteRelation,
  getOntology,
  getStats,
  getStatsHistory,
  getSubgraph,
  listConcepts,
  listRelations,
  updateConcept,
  type Concept,
  type ConceptTypeDef,
  type Ontology,
  type Relation,
  type Stats,
  type StatsHistory,
} from "../api";

// ---------------------------------------------------------------------------
// Constants & helpers
// ---------------------------------------------------------------------------

const PAGE_SIZE = 7;

function fmtNum(n: number): string {
  return n.toLocaleString("en-US");
}

function fmtDate(ts?: number | null): string {
  if (!ts) return "—";
  return new Date(ts * 1000).toLocaleDateString("en-US", {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

function conceptStatus(c: Concept): { label: string; cls: string } {
  const raw = (c.properties?.status as string | undefined)?.toLowerCase();
  if (raw === "reviewed") return { label: "Reviewed", cls: "badge-accent" };
  if (raw === "draft") return { label: "Draft", cls: "badge-warn" };
  if (raw === "archived") return { label: "Archived", cls: "badge-danger" };
  return { label: "Active", cls: "badge-success" };
}

function conceptUpdatedAt(c: Concept): number | null {
  const v = c.properties?.updated_at ?? c.properties?.created_at;
  if (typeof v === "number") return v;
  if (typeof v === "string") {
    const t = Date.parse(v);
    return isNaN(t) ? null : Math.floor(t / 1000);
  }
  return null;
}

function conceptDomain(c: Concept, types: Record<string, ConceptTypeDef>): string {
  // Walk up the parent chain to find the root concept type, fall back to the
  // direct type if there's no parent.
  const direct = c.concept_type;
  let cur = direct;
  const seen = new Set<string>();
  while (cur && !seen.has(cur)) {
    seen.add(cur);
    const def = types[cur];
    if (!def?.parent) return cur;
    cur = def.parent;
  }
  return direct;
}

function conceptIconColor(name: string): string {
  // Deterministic pastel based on the name's char codes.
  const palette = ["#2563eb", "#7c3aed", "#16a34a", "#d97706", "#dc2626", "#0ea5e9", "#db2777"];
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) >>> 0;
  return palette[h % palette.length];
}

function initials(name: string): string {
  return name
    .split(/\s+/)
    .map((w) => w[0])
    .filter(Boolean)
    .slice(0, 2)
    .join("")
    .toUpperCase();
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
  group: (
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
  expand: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M15 3h6v6" />
      <path d="M9 21H3v-6" />
      <path d="M21 3l-7 7" />
      <path d="M3 21l7-7" />
    </svg>
  ),
  edit: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 20h9" />
      <path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4z" />
    </svg>
  ),
  upload: (
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
  pencil: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5z" />
    </svg>
  ),
  check: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="m20 6-11 11-5-5" />
    </svg>
  ),
  chevDown: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="m6 9 6 6 6-6" />
    </svg>
  ),
  chevRight: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="m9 6 6 6-6 6" />
    </svg>
  ),
  dots: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="12" cy="5" r="1.5" />
      <circle cx="12" cy="12" r="1.5" />
      <circle cx="12" cy="19" r="1.5" />
    </svg>
  ),
  download: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
      <path d="m7 10 5 5 5-5" />
      <path d="M12 15V3" />
    </svg>
  ),
};

// ---------------------------------------------------------------------------
// Stat tile (reused style from Dashboard)
// ---------------------------------------------------------------------------

interface RichStatProps {
  label: string;
  value: number;
  display?: string;
  deltaPct?: number;
  icon: ReactNode;
  tone: "blue" | "violet" | "amber" | "green";
  spark: number[];
  sparkColor: string;
}

function RichStat({ label, value, display, deltaPct, icon, tone, spark, sparkColor }: RichStatProps) {
  const cls = deltaPct == null ? "flat" : deltaPct > 0.05 ? "up" : deltaPct < -0.05 ? "down" : "flat";
  const arrow = cls === "up" ? "↑" : cls === "down" ? "↓" : "•";
  return (
    <div className="card stat-rich">
      <div className={`stat-icon tone-${tone}`}>{icon}</div>
      <div className="stat-body">
        <div className="stat-label">{label}</div>
        <div className="stat-value">{display ?? fmtNum(value)}</div>
        {deltaPct != null && (
          <div className={`stat-delta ${cls}`}>
            <span>{arrow} {Math.abs(deltaPct).toFixed(0)}%</span>
            <span className="muted">vs last month</span>
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
// Concept hierarchy tree
// ---------------------------------------------------------------------------

interface TreeNode {
  name: string;
  children: TreeNode[];
}

function buildHierarchy(types: Record<string, ConceptTypeDef>): TreeNode[] {
  const nodes: Record<string, TreeNode> = {};
  const names = Object.keys(types);
  for (const n of names) nodes[n] = { name: n, children: [] };
  const roots: TreeNode[] = [];
  for (const n of names) {
    const parent = types[n].parent;
    if (parent && nodes[parent]) nodes[parent].children.push(nodes[n]);
    else roots.push(nodes[n]);
  }
  const sortRec = (n: TreeNode) => {
    n.children.sort((a, b) => a.name.localeCompare(b.name));
    n.children.forEach(sortRec);
  };
  roots.sort((a, b) => a.name.localeCompare(b.name));
  roots.forEach(sortRec);
  return roots;
}

function HierarchyNode({ node, depth }: { node: TreeNode; depth: number }) {
  const [open, setOpen] = useState(depth < 2);
  const hasChildren = node.children.length > 0;
  return (
    <div className="tree-node" style={{ paddingLeft: depth * 14 }}>
      <div className="tree-row">
        {hasChildren ? (
          <button
            type="button"
            className="tree-toggle"
            onClick={() => setOpen((v) => !v)}
            aria-label={open ? "Collapse" : "Expand"}
          >
            <span className="tree-toggle-icon">{open ? Icon.chevDown : Icon.chevRight}</span>
          </button>
        ) : (
          <span className="tree-toggle tree-toggle-empty" />
        )}
        <span className="tree-bullet" style={{ background: conceptIconColor(node.name) }} />
        <span className="tree-label">{node.name}</span>
      </div>
      {hasChildren && open && (
        <div>
          {node.children.map((c) => (
            <HierarchyNode key={c.name} node={c} depth={depth + 1} />
          ))}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main page
// ---------------------------------------------------------------------------

export default function Concepts() {
  const [stats, setStats] = useState<Stats | null>(null);
  const [history, setHistory] = useState<StatsHistory | null>(null);
  const [ontology, setOntology] = useState<Ontology | null>(null);
  const [coverage, setCoverage] = useState<number | null>(null);

  const [search, setSearch] = useState("");
  const [debouncedSearch, setDebouncedSearch] = useState("");
  const [typeFilter, setTypeFilter] = useState<string>("");
  const [statusFilter, setStatusFilter] = useState<string>("");
  const [sort, setSort] = useState<"updated" | "name" | "type">("updated");

  const [concepts, setConcepts] = useState<Concept[]>([]);
  const [total, setTotal] = useState(0);
  const [page, setPage] = useState(0);

  const [selected, setSelected] = useState<Concept | null>(null);
  const [recent, setRecent] = useState<Concept[]>([]);
  const [domainCounts, setDomainCounts] = useState<Record<string, number>>({});

  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // ---- Debounce search ----
  useEffect(() => {
    const t = setTimeout(() => setDebouncedSearch(search.trim()), 300);
    return () => clearTimeout(t);
  }, [search]);

  // ---- Reset page when filters change ----
  useEffect(() => {
    setPage(0);
  }, [debouncedSearch, typeFilter, statusFilter, sort]);

  // ---- Load stats/history/ontology + coverage ----
  const refreshSidecar = async () => {
    try {
      const [s, h, o, sg] = await Promise.all([
        getStats(),
        getStatsHistory(),
        getOntology(),
        getSubgraph({ limit: 500, expansion_depth: 1 }),
      ]);
      setStats(s);
      setHistory(h);
      setOntology(o);
      // Coverage = fraction of returned concepts that have at least one relation
      const linked = new Set<number>();
      for (const r of sg.subgraph.relations) {
        linked.add(r.source);
        linked.add(r.target);
      }
      const totalNodes = sg.subgraph.concepts.length;
      if (totalNodes > 0) {
        setCoverage(Math.round((linked.size / totalNodes) * 100));
      } else {
        setCoverage(null);
      }
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  useEffect(() => {
    refreshSidecar();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // ---- Recent activity (most-recent ids) ----
  const refreshRecent = async () => {
    try {
      const r = await listConcepts({ limit: 500 });
      const sorted = [...r.concepts].sort((a, b) => b.id - a.id).slice(0, 5);
      setRecent(sorted);
    } catch {
      /* non-fatal */
    }
  };
  useEffect(() => {
    refreshRecent();
  }, []);

  // ---- Domain counts from ontology types ----
  useEffect(() => {
    if (!ontology) return;
    let cancelled = false;
    (async () => {
      const types = Object.keys(ontology.concept_types);
      const counts: Record<string, number> = {};
      // Use the root domain (top-level ancestor) as the grouping bucket.
      const rootOf = (t: string): string => {
        const seen = new Set<string>();
        let cur = t;
        while (cur && !seen.has(cur)) {
          seen.add(cur);
          const p = ontology.concept_types[cur]?.parent;
          if (!p) return cur;
          cur = p;
        }
        return t;
      };
      await Promise.all(
        types.map(async (t) => {
          try {
            const r = await listConcepts({ type: t, limit: 1 });
            const root = rootOf(t);
            counts[root] = (counts[root] ?? 0) + r.total;
          } catch {
            /* ignore */
          }
        }),
      );
      if (!cancelled) setDomainCounts(counts);
    })();
    return () => {
      cancelled = true;
    };
  }, [ontology]);

  // ---- Load concept list whenever filters or page change ----
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        // Server supports type filter natively. Status filter is client-side
        // (the server has no status field). For server-side pagination we
        // fetch the slice and apply client-side status filter on the slice.
        const limit = PAGE_SIZE;
        const offset = page * PAGE_SIZE;
        const r = await listConcepts({
          type: typeFilter || undefined,
          q: debouncedSearch || undefined,
          limit,
          offset,
        });
        if (cancelled) return;
        let rows = r.concepts;
        if (statusFilter) {
          rows = rows.filter((c) => conceptStatus(c).label.toLowerCase() === statusFilter);
        }
        if (sort === "name") rows = [...rows].sort((a, b) => a.name.localeCompare(b.name));
        else if (sort === "type") rows = [...rows].sort((a, b) => a.concept_type.localeCompare(b.concept_type));
        else
          rows = [...rows].sort((a, b) => (conceptUpdatedAt(b) ?? 0) - (conceptUpdatedAt(a) ?? 0));
        setConcepts(rows);
        setTotal(r.total);
        setSelected((prev) => prev ?? rows[0] ?? null);
      } catch (e: unknown) {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [debouncedSearch, typeFilter, statusFilter, sort, page]);

  // ---- Actions ----
  const handleCreate = async () => {
    const types = ontology ? Object.keys(ontology.concept_types) : [];
    if (types.length === 0) {
      setError("No concept types are defined yet. Create one in Ontology Builder first.");
      return;
    }
    const name = window.prompt("Concept name?")?.trim();
    if (!name) return;
    const conceptType = window.prompt(`Concept type? (${types.slice(0, 6).join(", ")}${types.length > 6 ? ", …" : ""})`, types[0])?.trim();
    if (!conceptType) return;
    const description = window.prompt("Definition (optional)?")?.trim() ?? "";
    setBusy(true);
    setError(null);
    try {
      await createConcept({ concept_type: conceptType, name, description });
      setInfo(`Concept "${name}" created.`);
      // Reload list and sidecar.
      setPage(0);
      await Promise.all([refreshSidecar(), refreshRecent()]);
      // Trigger list reload by toggling debouncedSearch (no-op set).
      setDebouncedSearch((s) => s);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleEdit = async (c: Concept) => {
    const name = window.prompt("Concept name", c.name)?.trim();
    if (name === undefined) return;
    const description = window.prompt("Definition", c.description ?? "")?.trim() ?? "";
    setBusy(true);
    try {
      const updated = await updateConcept(c.id, { name, description });
      setInfo(`Concept "${updated.name}" updated.`);
      setSelected(updated);
      // Reload the visible page.
      setDebouncedSearch((s) => s);
      await refreshRecent();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleDelete = async (c: Concept) => {
    if (!window.confirm(`Delete concept "${c.name}"? This cannot be undone.`)) return;
    setBusy(true);
    try {
      await deleteConcept(c.id);
      setInfo(`Deleted "${c.name}".`);
      if (selected?.id === c.id) setSelected(null);
      setDebouncedSearch((s) => s);
      await Promise.all([refreshSidecar(), refreshRecent()]);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  // ---- Derived data ----
  const sparkConcepts = useMemo(
    () => (history?.samples ?? []).map((s) => s.concepts),
    [history],
  );
  const sparkRelations = useMemo(
    () => (history?.samples ?? []).map((s) => s.relations),
    [history],
  );
  const sparkTypes = useMemo(
    () => (history?.samples ?? []).map((s) => s.concept_types),
    [history],
  );
  const sparkCoverage = useMemo(() => {
    // Coverage trend: fraction of relations / concepts at each sample.
    return (history?.samples ?? []).map((s) =>
      s.concepts > 0 ? Math.min(100, Math.round((s.relations / s.concepts) * 100)) : 0,
    );
  }, [history]);

  const types = ontology?.concept_types ?? {};
  const typeNames = Object.keys(types).sort();
  const hierarchy = useMemo(() => buildHierarchy(types), [types]);

  const domainList = useMemo(() => {
    const totalDom = Object.values(domainCounts).reduce((a, b) => a + b, 0);
    return Object.entries(domainCounts)
      .sort((a, b) => b[1] - a[1])
      .slice(0, 6)
      .map(([name, count]) => ({
        name,
        count,
        pct: totalDom > 0 ? Math.round((count / totalDom) * 1000) / 10 : 0,
      }));
  }, [domainCounts]);

  const totalPages = Math.max(1, Math.ceil(total / PAGE_SIZE));
  const showingFrom = total === 0 ? 0 : page * PAGE_SIZE + 1;
  const showingTo = Math.min(total, (page + 1) * PAGE_SIZE);

  // ---------------------------------------------------------------------------
  // Render
  // ---------------------------------------------------------------------------

  return (
    <>
      <div className="page-header">
        <div>
          <h2 className="page-title">Concepts</h2>
          <p className="page-subtitle">
            Browse, organize, and inspect ontology concepts and their definitions.
          </p>
        </div>
      </div>

      {error && <div className="error-banner">{error}</div>}
      {info && <div className="success-banner">{info}</div>}

      {/* Stat row */}
      <div className="dash-row dash-row-stats">
        <RichStat
          label="Total Concepts"
          value={stats?.concepts ?? 0}
          deltaPct={stats ? stats.deltas.concepts_pct * 100 : undefined}
          icon={Icon.layers}
          tone="blue"
          spark={sparkConcepts}
          sparkColor="#2563eb"
        />
        <RichStat
          label="Concept Groups"
          value={stats?.concept_types ?? 0}
          deltaPct={stats ? stats.deltas.concept_types_pct * 100 : undefined}
          icon={Icon.group}
          tone="violet"
          spark={sparkTypes}
          sparkColor="#7c3aed"
        />
        <RichStat
          label="Mapped Relations"
          value={stats?.relations ?? 0}
          deltaPct={stats ? stats.deltas.relations_pct * 100 : undefined}
          icon={Icon.share}
          tone="amber"
          spark={sparkRelations}
          sparkColor="#d97706"
        />
        <RichStat
          label="Coverage Score"
          value={coverage ?? 0}
          display={coverage == null ? "—" : `${coverage}%`}
          icon={Icon.shield}
          tone="green"
          spark={sparkCoverage}
          sparkColor="#16a34a"
        />
      </div>

      {/* Library + Details */}
      <div className="concepts-grid">
        <Card
          title="Concept Library"
          actions={
            <button className="btn-primary" onClick={handleCreate} disabled={busy}>
              <span style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
                <span style={{ width: 14, height: 14, display: "inline-flex" }}>{Icon.plus}</span>
                Create Concept
              </span>
            </button>
          }
        >
          <div className="concept-toolbar">
            <div className="search-input">
              <span className="search-icon">{Icon.search}</span>
              <input
                placeholder="Search concepts…"
                value={search}
                onChange={(e) => setSearch(e.target.value)}
              />
            </div>
            <select value={typeFilter} onChange={(e) => setTypeFilter(e.target.value)}>
              <option value="">All Domains</option>
              {typeNames.map((t) => (
                <option key={t} value={t}>{t}</option>
              ))}
            </select>
            <select value={statusFilter} onChange={(e) => setStatusFilter(e.target.value)}>
              <option value="">All Statuses</option>
              <option value="active">Active</option>
              <option value="reviewed">Reviewed</option>
              <option value="draft">Draft</option>
              <option value="archived">Archived</option>
            </select>
            <select value={sort} onChange={(e) => setSort(e.target.value as typeof sort)}>
              <option value="updated">Sort: Last Updated</option>
              <option value="name">Sort: Name</option>
              <option value="type">Sort: Type</option>
            </select>
          </div>

          <table className="table">
            <thead>
              <tr>
                <th>Concept Name</th>
                <th>Domain</th>
                <th>Definition</th>
                <th>Status</th>
                <th>Linked</th>
                <th>Last Updated</th>
                <th className="actions">Actions</th>
              </tr>
            </thead>
            <tbody>
              {concepts.length === 0 && (
                <tr><td colSpan={7} className="empty">No concepts match the current filters.</td></tr>
              )}
              {concepts.map((c) => {
                const st = conceptStatus(c);
                const dom = conceptDomain(c, types);
                const isSel = selected?.id === c.id;
                return (
                  <tr
                    key={c.id}
                    className={isSel ? "row-active" : undefined}
                    onClick={() => setSelected(c)}
                    style={{ cursor: "pointer" }}
                  >
                    <td>
                      <div className="concept-name-cell">
                        <span
                          className="concept-avatar"
                          style={{ background: conceptIconColor(c.name) }}
                        >
                          {initials(c.name)}
                        </span>
                        <span>{c.name}</span>
                      </div>
                    </td>
                    <td>{dom}</td>
                    <td className="muted def-cell">{c.description || "—"}</td>
                    <td><span className={`badge ${st.cls}`}>{st.label}</span></td>
                    <td>{fmtNum(Number(c.properties?.linked ?? 0))}</td>
                    <td>{fmtDate(conceptUpdatedAt(c))}</td>
                    <td className="actions">
                      <button
                        className="icon-btn"
                        title="Edit"
                        onClick={(e) => { e.stopPropagation(); handleEdit(c); }}
                      >
                        <span style={{ width: 16, height: 16, display: "inline-flex" }}>{Icon.pencil}</span>
                      </button>
                      <button
                        className="icon-btn"
                        title="Delete"
                        onClick={(e) => { e.stopPropagation(); handleDelete(c); }}
                      >
                        <span style={{ width: 16, height: 16, display: "inline-flex" }}>{Icon.dots}</span>
                      </button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>

          <div className="pagination">
            <span className="muted">
              Showing {showingFrom}–{showingTo} of {fmtNum(total)} concepts
            </span>
            <div className="pager">
              <button disabled={page === 0} onClick={() => setPage((p) => Math.max(0, p - 1))}>‹</button>
              {Array.from({ length: Math.min(5, totalPages) }, (_, i) => i).map((i) => (
                <button
                  key={i}
                  className={i === page ? "pager-active" : ""}
                  onClick={() => setPage(i)}
                >
                  {i + 1}
                </button>
              ))}
              {totalPages > 5 && <span className="muted">…</span>}
              {totalPages > 5 && (
                <button onClick={() => setPage(totalPages - 1)}>{totalPages}</button>
              )}
              <button
                disabled={page + 1 >= totalPages}
                onClick={() => setPage((p) => Math.min(totalPages - 1, p + 1))}
              >
                ›
              </button>
            </div>
          </div>
        </Card>

        <div className="concepts-side">
          <Card
            title="Concept Details"
            actions={
              <button className="icon-btn" title="Expand">
                <span style={{ width: 16, height: 16, display: "inline-flex" }}>{Icon.expand}</span>
              </button>
            }
          >
            {selected ? (
              <>
                <ConceptDetails
                  concept={selected}
                  domain={conceptDomain(selected, types)}
                  onEdit={() => handleEdit(selected)}
                />
                <RelationsPanel
                  concept={selected}
                  ontology={ontology}
                  allConcepts={concepts}
                />
              </>
            ) : (
              <div className="empty">Select a concept from the library.</div>
            )}
          </Card>

          <Card title="Concept Hierarchy">
            {hierarchy.length === 0 ? (
              <div className="empty">No concept types defined yet.</div>
            ) : (
              <div className="concept-tree">
                {hierarchy.map((n) => (
                  <HierarchyNode key={n.name} node={n} depth={0} />
                ))}
              </div>
            )}
          </Card>
        </div>
      </div>

      {/* Activity + Domains + Quick Actions */}
      <div className="dash-row dash-row-three">
        <Card
          title="Recent Concept Activity"
          actions={<Link to="/builder" className="btn-ghost-link">View All</Link>}
        >
          {recent.length === 0 ? (
            <div className="empty">No recent activity.</div>
          ) : (
            <ul className="activity-list">
              {recent.map((c) => (
                <li key={c.id}>
                  <span className="activity-dot" style={{ background: conceptIconColor(c.name) }} />
                  <span className="activity-text">
                    Concept <strong>“{c.name}”</strong> added
                  </span>
                  <span className="activity-time muted">{fmtDate(conceptUpdatedAt(c))}</span>
                </li>
              ))}
            </ul>
          )}
        </Card>

        <Card
          title="Top Domains"
          actions={<Link to="/builder" className="btn-ghost-link">View All</Link>}
        >
          {domainList.length === 0 ? (
            <div className="empty">Loading domains…</div>
          ) : (
            <ul className="domain-list">
              {domainList.map((d) => (
                <li key={d.name}>
                  <span className="domain-name">{d.name}</span>
                  <span className="domain-bar">
                    <span
                      className="domain-bar-fill"
                      style={{ width: `${Math.min(100, d.pct)}%`, background: conceptIconColor(d.name) }}
                    />
                  </span>
                  <span className="domain-pct muted">
                    {d.count} ({d.pct.toFixed(1)}%)
                  </span>
                </li>
              ))}
            </ul>
          )}
        </Card>

        <Card title="Quick Actions">
          <div className="quick-actions">
            <Link to="/files" className="quick-action qa-blue">
              <span className="qa-icon">{Icon.upload}</span>
              <div>
                <div className="qa-title">Import Concepts</div>
                <div className="qa-sub muted">Import from files or sources</div>
              </div>
            </Link>
            <Link to="/builder" className="quick-action qa-violet">
              <span className="qa-icon">{Icon.spark}</span>
              <div>
                <div className="qa-title">Generate with AI</div>
                <div className="qa-sub muted">Auto-generate concepts</div>
              </div>
            </Link>
            <button
              type="button"
              className="quick-action qa-amber"
              onClick={() => setInfo("Bulk edit coming soon.")}
            >
              <span className="qa-icon">{Icon.edit}</span>
              <div style={{ textAlign: "left" }}>
                <div className="qa-title">Bulk Edit</div>
                <div className="qa-sub muted">Edit multiple concepts</div>
              </div>
            </button>
            <button
              type="button"
              className="quick-action qa-green"
              onClick={() => setInfo("Definition validation coming soon.")}
            >
              <span className="qa-icon">{Icon.check}</span>
              <div style={{ textAlign: "left" }}>
                <div className="qa-title">Validate Definitions</div>
                <div className="qa-sub muted">Check quality &amp; consistency</div>
              </div>
            </button>
          </div>
        </Card>
      </div>
    </>
  );
}

// ---------------------------------------------------------------------------
// Details sub-component
// ---------------------------------------------------------------------------

interface DetailsProps {
  concept: Concept;
  domain: string;
  onEdit: () => void;
}

function ConceptDetails({ concept, domain, onEdit }: DetailsProps) {
  const synonyms = (() => {
    const v = concept.properties?.synonyms;
    if (Array.isArray(v)) return v.map(String);
    if (typeof v === "string") return v.split(",").map((s) => s.trim()).filter(Boolean);
    return [];
  })();
  const owner = (concept.properties?.owner as string | undefined) ?? "—";
  const updated = conceptUpdatedAt(concept);
  return (
    <div className="concept-details">
      <div className="cd-head">
        <span
          className="cd-avatar"
          style={{ background: conceptIconColor(concept.name) }}
        >
          {initials(concept.name)}
        </span>
        <div className="cd-head-text">
          <div className="cd-name-row">
            <h3 className="cd-name">{concept.name}</h3>
            <span className="badge badge-accent">{concept.concept_type}</span>
          </div>
          <p className="cd-desc muted">{concept.description || "No definition provided."}</p>
        </div>
      </div>

      <dl className="cd-grid">
        <dt>URI</dt>
        <dd className="mono">urn:concept:{concept.concept_type.toLowerCase()}:{concept.id}</dd>

        <dt>Synonyms / Tags</dt>
        <dd>
          {synonyms.length === 0 ? (
            <span className="muted">—</span>
          ) : (
            <span className="tag-row">
              {synonyms.map((s) => <span key={s} className="tag-chip">{s}</span>)}
            </span>
          )}
        </dd>

        <dt>Domain</dt>
        <dd>{domain}</dd>

        <dt>Owner</dt>
        <dd>{owner}</dd>

        <dt>Last Updated</dt>
        <dd>{fmtDate(updated)}</dd>
      </dl>

      <div className="cd-actions">
        <button onClick={onEdit}>
          <span style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
            <span style={{ width: 14, height: 14, display: "inline-flex" }}>{Icon.pencil}</span>
            Edit Concept
          </span>
        </button>
        <Link to={`/graph?seed=${concept.id}`} className="btn-link-wrap">
          <button>
            <span style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
              <span style={{ width: 14, height: 14, display: "inline-flex" }}>{Icon.share}</span>
              View Relations
            </span>
          </button>
        </Link>
        <button>
          <span style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
            <span style={{ width: 14, height: 14, display: "inline-flex" }}>{Icon.download}</span>
            Export
          </span>
        </button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Relations panel — inline create / delete edges incident to a concept
// ---------------------------------------------------------------------------

interface RelationsPanelProps {
  concept: Concept;
  ontology: Ontology | null;
  allConcepts: Concept[];
}

function RelationsPanel({ concept, ontology, allConcepts }: RelationsPanelProps) {
  const [outgoing, setOutgoing] = useState<Relation[]>([]);
  const [incoming, setIncoming] = useState<Relation[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [relType, setRelType] = useState("");
  const [targetId, setTargetId] = useState<number | "">("");

  const conceptById = useMemo(() => {
    const m = new Map<number, Concept>();
    for (const c of allConcepts) m.set(c.id, c);
    return m;
  }, [allConcepts]);

  // Relation types whose domain matches this concept's type.
  const relTypeOptions = useMemo(() => {
    const types = Object.values(ontology?.relation_types ?? {});
    return types.filter((rt) => rt.domain === concept.concept_type);
  }, [ontology, concept.concept_type]);

  // Targets restricted to the chosen relation type's range.
  const targetOptions = useMemo(() => {
    const rt = relTypeOptions.find((t) => t.name === relType);
    if (!rt) return allConcepts;
    return allConcepts.filter((c) => c.concept_type === rt.range);
  }, [allConcepts, relTypeOptions, relType]);

  async function refresh() {
    setLoading(true);
    setError(null);
    try {
      const [out, inc] = await Promise.all([
        listRelations({ source: concept.id, limit: 200 }),
        listRelations({ target: concept.id, limit: 200 }),
      ]);
      setOutgoing(out.relations);
      setIncoming(inc.relations);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    refresh();
    setAdding(false);
    setRelType("");
    setTargetId("");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [concept.id]);

  async function onAdd(e: React.FormEvent) {
    e.preventDefault();
    if (!relType || targetId === "") return;
    try {
      await createRelation({
        relation_type: relType,
        source: concept.id,
        target: Number(targetId),
      });
      setAdding(false);
      setRelType("");
      setTargetId("");
      await refresh();
    } catch (err) {
      alert("Add failed: " + (err as Error).message);
    }
  }

  async function onDelete(id: number) {
    if (!confirm("Delete this relation?")) return;
    try {
      await deleteRelation(id);
      await refresh();
    } catch (err) {
      alert("Delete failed: " + (err as Error).message);
    }
  }

  const renderRow = (r: Relation, otherId: number, direction: "out" | "in") => {
    const other = conceptById.get(otherId);
    return (
      <li key={r.id} className="rel-row">
        <span className="rel-arrow">{direction === "out" ? "→" : "←"}</span>
        <span className="rel-type">{r.relation_type}</span>
        <span className="rel-other">
          {other
            ? `${other.concept_type}: ${other.name}`
            : `Concept #${otherId}`}
        </span>
        <button
          type="button"
          className="rel-del"
          aria-label="Delete relation"
          onClick={() => onDelete(r.id)}
        >
          ×
        </button>
      </li>
    );
  };

  return (
    <div className="rel-panel">
      <div className="rel-head">
        <strong>Relations</strong>
        <button
          type="button"
          className="btn-ghost"
          onClick={() => setAdding((v) => !v)}
          disabled={relTypeOptions.length === 0}
          title={
            relTypeOptions.length === 0
              ? "No relation types defined with this concept's type as domain."
              : ""
          }
        >
          {adding ? "Cancel" : "+ Add"}
        </button>
      </div>

      {adding && (
        <form className="rel-form" onSubmit={onAdd}>
          <select
            value={relType}
            onChange={(e) => {
              setRelType(e.target.value);
              setTargetId("");
            }}
            required
          >
            <option value="" disabled>Relation type…</option>
            {relTypeOptions.map((rt) => (
              <option key={rt.name} value={rt.name}>
                {rt.name} ({rt.domain} → {rt.range})
              </option>
            ))}
          </select>
          <select
            value={targetId === "" ? "" : String(targetId)}
            onChange={(e) =>
              setTargetId(e.target.value === "" ? "" : Number(e.target.value))
            }
            required
          >
            <option value="" disabled>Target concept…</option>
            {targetOptions
              .filter((c) => c.id !== concept.id)
              .map((c) => (
                <option key={c.id} value={c.id}>
                  {c.concept_type}: {c.name}
                </option>
              ))}
          </select>
          <button type="submit" className="btn-primary">Add</button>
        </form>
      )}

      {error && <div className="banner banner-error">{error}</div>}
      {loading ? (
        <div className="muted">Loading relations…</div>
      ) : outgoing.length === 0 && incoming.length === 0 ? (
        <div className="muted">No relations.</div>
      ) : (
        <>
          {outgoing.length > 0 && (
            <>
              <div className="rel-section-title muted">Outgoing</div>
              <ul className="rel-list">
                {outgoing.map((r) => renderRow(r, r.target, "out"))}
              </ul>
            </>
          )}
          {incoming.length > 0 && (
            <>
              <div className="rel-section-title muted">Incoming</div>
              <ul className="rel-list">
                {incoming.map((r) => renderRow(r, r.source, "in"))}
              </ul>
            </>
          )}
        </>
      )}

      <style>{`
        .rel-panel { margin-top: 16px; padding-top: 12px; border-top: 1px solid #e2e8f0;
          display: flex; flex-direction: column; gap: 8px; }
        .rel-head { display: flex; align-items: center; justify-content: space-between; }
        .rel-form { display: flex; flex-direction: column; gap: 6px; padding: 8px;
          background: #f8fafc; border-radius: 8px; }
        .rel-form select { padding: 6px 8px; border: 1px solid #cbd5e1; border-radius: 6px;
          font: inherit; background: #fff; }
        .rel-section-title { font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em;
          margin-top: 4px; }
        .rel-list { list-style: none; margin: 0; padding: 0; display: flex;
          flex-direction: column; gap: 4px; }
        .rel-row { display: flex; align-items: center; gap: 8px; font-size: 13px;
          padding: 4px 6px; border-radius: 6px; }
        .rel-row:hover { background: #f1f5f9; }
        .rel-arrow { color: #64748b; font-weight: 600; }
        .rel-type { color: #2563eb; font-weight: 500; }
        .rel-other { flex: 1; color: #334155; }
        .rel-del { border: none; background: transparent; color: #94a3b8;
          cursor: pointer; font-size: 18px; line-height: 1; padding: 0 4px; }
        .rel-del:hover { color: #dc2626; }
      `}</style>
    </div>
  );
}
