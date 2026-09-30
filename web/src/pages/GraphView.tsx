// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.

import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate, Link, useSearchParams } from "react-router-dom";
import Card from "../components/Card";
import Sparkline from "../components/Sparkline";
import GraphCanvas, { type GraphCanvasHandle, type LayoutDir } from "../components/GraphCanvas";
import {
  getFiles,
  getOntology,
  getQueries,
  getStats,
  getStatsHistory,
  getSubgraph,
  type ActionTypeDef,
  type Concept,
  type ConceptTypeDef,
  type FileRecord,
  type Ontology,
  type Relation,
  type RuleTypeDef,
  type SavedQuery,
  type StatsHistory,
  type Subgraph,
} from "../api";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function fmtNum(n: number): string {
  return n.toLocaleString("en-US");
}

function fmtAgo(ts: number): string {
  const diff = Math.max(0, Math.floor(Date.now() / 1000 - ts));
  if (diff < 60) return `il y a ${diff} s`;
  if (diff < 3600) return `il y a ${Math.floor(diff / 60)} min`;
  if (diff < 86400) return `il y a ${Math.floor(diff / 3600)} h`;
  return `il y a ${Math.floor(diff / 86400)} j`;
}

function xsdFor(value: unknown): string {
  if (value == null) return "xsd:string";
  if (typeof value === "boolean") return "xsd:boolean";
  if (typeof value === "number") return Number.isInteger(value) ? "xsd:integer" : "xsd:decimal";
  if (typeof value === "string") {
    if (/^\d{4}-\d{2}-\d{2}/.test(value)) return "xsd:date";
    return "xsd:string";
  }
  return "xsd:any";
}

const TYPE_PALETTE = ["#2563eb", "#7c3aed", "#16a34a", "#dc2626", "#d97706", "#0891b2", "#db2777", "#0d9488"];

// ---------------------------------------------------------------------------
// Icons
// ---------------------------------------------------------------------------

const Icon = {
  layers: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 2 2 7l10 5 10-5-10-5z" />
      <path d="m2 17 10 5 10-5" />
      <path d="m2 12 10 5 10-5" />
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
  funnel: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M22 3H2l8 9.46V19l4 2v-8.54z" />
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
  layout: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3" y="3" width="7" height="7" rx="1" />
      <rect x="14" y="3" width="7" height="7" rx="1" />
      <rect x="3" y="14" width="7" height="7" rx="1" />
      <rect x="14" y="14" width="7" height="7" rx="1" />
    </svg>
  ),
  plus: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 5v14M5 12h14" />
    </svg>
  ),
  minus: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M5 12h14" />
    </svg>
  ),
  fit: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M3 8V5a2 2 0 0 1 2-2h3" />
      <path d="M16 3h3a2 2 0 0 1 2 2v3" />
      <path d="M21 16v3a2 2 0 0 1-2 2h-3" />
      <path d="M8 21H5a2 2 0 0 1-2-2v-3" />
    </svg>
  ),
  expand: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M15 3h6v6" />
      <path d="M9 21H3v-6" />
      <path d="m21 3-7 7" />
      <path d="m3 21 7-7" />
    </svg>
  ),
  refresh: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M3 12a9 9 0 0 1 15-6.7L21 8" />
      <path d="M21 3v5h-5" />
      <path d="M21 12a9 9 0 0 1-15 6.7L3 16" />
      <path d="M3 21v-5h5" />
    </svg>
  ),
  chevR: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="m9 18 6-6-6-6" />
    </svg>
  ),
  chevL: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="m15 18-6-6 6-6" />
    </svg>
  ),
  person: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2" />
      <circle cx="12" cy="7" r="4" />
    </svg>
  ),
  focus: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="12" cy="12" r="3" />
      <circle cx="12" cy="12" r="9" />
    </svg>
  ),
  play: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <polygon points="5 3 19 12 5 21 5 3" />
    </svg>
  ),
  pathIcon: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="5" cy="6" r="2" />
      <circle cx="19" cy="6" r="2" />
      <circle cx="12" cy="18" r="2" />
      <path d="M7 6h10M6.5 7.5l4 8.5M17.5 7.5l-4 8.5" />
    </svg>
  ),
  activity: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M22 12h-4l-3 9L9 3l-3 9H2" />
    </svg>
  ),
};

// ---------------------------------------------------------------------------
// KPI tile (matches Files/Dashboard style)
// ---------------------------------------------------------------------------

interface KpiProps {
  label: string;
  value: string;
  deltaPct?: number;
  deltaLabel?: string;
  icon: React.ReactNode;
  tone: "blue" | "violet" | "amber" | "green";
  spark: number[];
  sparkColor: string;
}

function Kpi({ label, value, deltaPct, deltaLabel, icon, tone, spark, sparkColor }: KpiProps) {
  const cls = deltaPct == null ? "flat" : deltaPct > 0.05 ? "up" : deltaPct < -0.05 ? "down" : "flat";
  const arrow = cls === "up" ? "↑" : cls === "down" ? "↓" : "—";
  return (
    <div className="card stat-rich">
      <div className={`stat-icon tone-${tone}`}>{icon}</div>
      <div className="stat-body">
        <div className="stat-label">{label}</div>
        <div className="stat-value">{value}</div>
        {deltaPct != null && (
          <div className={`stat-delta ${cls}`}>
            <span>{arrow} {Math.abs(deltaPct).toFixed(0)} %</span>
            <span className="muted">{deltaLabel ?? "vs le mois dernier"}</span>
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
// Toggle switch
// ---------------------------------------------------------------------------

function Toggle({ checked, onChange, label, icon }: { checked: boolean; onChange: (v: boolean) => void; label: string; icon?: React.ReactNode }) {
  return (
    <label className="gv-toggle">
      <span className="gv-toggle-label">
        {icon && <span className="gv-toggle-icon">{icon}</span>}
        {label}
      </span>
      <span className={`gv-switch${checked ? " on" : ""}`} onClick={() => onChange(!checked)} role="switch" aria-checked={checked}>
        <span className="gv-switch-knob" />
      </span>
    </label>
  );
}

// ---------------------------------------------------------------------------
// Graph View page
// ---------------------------------------------------------------------------

const LAYOUTS: { value: LayoutDir; label: string }[] = [
  { value: "LR", label: "Gauche → Droite" },
  { value: "TB", label: "Haut → Bas" },
  { value: "RL", label: "Droite → Gauche" },
  { value: "BT", label: "Bas → Haut" },
];

type InspectorTab = "inspector" | "rules" | "actions";

/** Up to this many sheets the whole graph is drawn; beyond, by selection. */
const FULL_GRAPH_MAX = 300;
/** Sheets fetched around a selection on a large graph. */
const SUBGRAPH_LIMIT = 250;

export default function GraphView() {
  const navigate = useNavigate();
  const canvasRef = useRef<GraphCanvasHandle>(null);

  // Data
  const [ontology, setOntology] = useState<Ontology | null>(null);
  const [subgraph, setSubgraph] = useState<Subgraph | null>(null);
  const [history, setHistory] = useState<StatsHistory | null>(null);
  const [queries, setQueries] = useState<SavedQuery[]>([]);
  const [files, setFiles] = useState<FileRecord[]>([]);

  // Filters
  const [search, setSearch] = useState("");
  // Several types at once; none selected = all (the small-graph case).
  const [nodeTypes, setNodeTypes] = useState<string[]>([]);
  const [relTypes, setRelTypes] = useState<string[]>([]);
  // Sheets in the store: past FULL_GRAPH_MAX the graph is shown by
  // selection only, never whole (performance of the page and of the eye).
  const [total, setTotal] = useState<number | null>(null);
  const [depth, setDepth] = useState(3);
  const [showLabels, setShowLabels] = useState(true);
  const [clusterView, setClusterView] = useState(false);
  const [highlightPaths, setHighlightPaths] = useState(true);
  const [showConstraints, setShowConstraints] = useState(false);

  // Canvas controls
  const [layoutDir, setLayoutDir] = useState<LayoutDir>("LR");
  const [layoutOpen, setLayoutOpen] = useState(false);
  const [collapseInspector, setCollapseInspector] = useState(false);

  // Selection / inspector tab
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [tab, setTab] = useState<InspectorTab>("inspector");

  // State
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // ---------- Data loading ----------

  // `/graph?focus=<id>`: the graph opens around that sheet, selected.
  const focus = useSearchParams()[0].get("focus");
  const focusId = focus && /^\d+$/.test(focus) ? Number(focus) : null;
  // Once: later reloads (interval, depth) keep what the user selected since.
  const focused = useRef(false);

  const large = total !== null && total > FULL_GRAPH_MAX;
  // A large graph loads only around a selection: types, a search or a sheet.
  const canLoad = !large || nodeTypes.length > 0 || search.trim() !== "" || focusId !== null;

  const loadSubgraph = async (d = depth) => {
    if (!canLoad) {
      setSubgraph(null);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const res = await getSubgraph({
        seed_query: search.trim() || undefined,
        seed_concept_ids: focusId !== null && !search.trim() ? [focusId] : undefined,
        seed_concept_types: nodeTypes,
        expansion_depth: d,
        limit: large ? SUBGRAPH_LIMIT : FULL_GRAPH_MAX,
      });
      setSubgraph(res.subgraph);
      if (focusId !== null && !focused.current && res.subgraph.concepts.some((c) => c.id === focusId)) {
        focused.current = true;
        setSelectedId(String(focusId));
        setTab("inspector");
        // The canvas mounts with the subgraph: focus on the next frame.
        window.setTimeout(() => canvasRef.current?.focusNode(String(focusId)), 0);
      }
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    getOntology().then(setOntology).catch(() => undefined);
    getStatsHistory().then(setHistory).catch(() => undefined);
    getQueries().then((q) => setQueries(q.queries)).catch(() => undefined);
    getFiles().then((f) => setFiles(f.files)).catch(() => undefined);
    // The size decides how the graph loads: whole, or by selection.
    getStats()
      .then((s) => setTotal(s.concepts))
      .catch(() => setTotal(0));
  }, []);

  // Load once the size is known, and again when the depth or the selected
  // types change. A small graph also refreshes every 30 s; a large one only
  // on demand (Actualiser), so the page never reloads 250 sheets by itself.
  useEffect(() => {
    if (total === null) return;
    loadSubgraph(depth);
    if (large) return;
    const t = window.setInterval(() => loadSubgraph(depth), 30_000);
    return () => window.clearInterval(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [total, depth, nodeTypes]);

  // ---------- Derived ----------

  const conceptTypes = useMemo(() => (ontology ? Object.keys(ontology.concept_types) : []), [ontology]);
  const relationTypes = useMemo(() => (ontology ? Object.keys(ontology.relation_types) : []), [ontology]);
  const rules: RuleTypeDef[] = useMemo(
    () => (ontology?.rule_types ? Object.values(ontology.rule_types) : []),
    [ontology]
  );
  const actions: ActionTypeDef[] = useMemo(
    () => (ontology?.action_types ? Object.values(ontology.action_types) : []),
    [ontology]
  );

  // Stable color map for concept types (shared with canvas + legend)
  const conceptTypeColors = useMemo<Record<string, string>>(() => {
    const out: Record<string, string> = {};
    conceptTypes.forEach((t, i) => {
      out[t] = TYPE_PALETTE[i % TYPE_PALETTE.length]!;
    });
    return out;
  }, [conceptTypes]);

  // Client-side filtered subgraph
  const filteredSubgraph = useMemo<Subgraph | null>(() => {
    if (!subgraph) return null;
    const q = search.trim().toLowerCase();
    const keepConcept = (c: Concept): boolean => {
      if (nodeTypes.length > 0 && !nodeTypes.includes(c.concept_type)) return false;
      if (q && !c.name.toLowerCase().includes(q) && !c.concept_type.toLowerCase().includes(q)) return false;
      return true;
    };
    const concepts = subgraph.concepts.filter(keepConcept);
    const keepIds = new Set(concepts.map((c) => c.id));
    const relations: Relation[] = subgraph.relations.filter((r) => {
      if (relTypes.length > 0 && !relTypes.includes(r.relation_type)) return false;
      return keepIds.has(r.source) && keepIds.has(r.target);
    });
    return { concepts, relations };
  }, [subgraph, search, nodeTypes, relTypes]);

  // Active filters count for KPI
  const activeFiltersCount = useMemo(() => {
    let n = 0;
    if (search.trim()) n++;
    if (nodeTypes.length > 0) n++;
    if (relTypes.length > 0) n++;
    if (depth !== 3) n++;
    return n;
  }, [search, nodeTypes, relTypes, depth]);

  // KPI values
  const nodesCount = filteredSubgraph?.concepts.length ?? 0;
  const relsCount = filteredSubgraph?.relations.length ?? 0;
  const samples = history?.samples ?? [];
  const graphHealth = useMemo(() => {
    const c = nodesCount;
    const r = relsCount;
    if (c === 0) return 0;
    return Math.min(100, Math.round((r / Math.max(1, c)) * 50));
  }, [nodesCount, relsCount]);

  // Selected node / concept
  const selectedConcept = useMemo<Concept | null>(() => {
    if (!selectedId || !filteredSubgraph) return null;
    return filteredSubgraph.concepts.find((c) => String(c.id) === selectedId) ?? null;
  }, [selectedId, filteredSubgraph]);

  const selectedTypeDef: ConceptTypeDef | null = useMemo(() => {
    if (!selectedConcept || !ontology) return null;
    return ontology.concept_types[selectedConcept.concept_type] ?? null;
  }, [selectedConcept, ontology]);

  const selectedStats = useMemo(() => {
    if (!selectedConcept || !filteredSubgraph) return { total: 0, incoming: 0, outgoing: 0 };
    let inc = 0;
    let out = 0;
    for (const r of filteredSubgraph.relations) {
      if (r.target === selectedConcept.id) inc++;
      if (r.source === selectedConcept.id) out++;
    }
    return { total: inc + out, incoming: inc, outgoing: out };
  }, [selectedConcept, filteredSubgraph]);

  // Properties: prefer concept's own values, else type schema
  const selectedProps = useMemo(() => {
    if (selectedConcept?.properties && Object.keys(selectedConcept.properties).length > 0) {
      return Object.entries(selectedConcept.properties).map(([k, v]) => ({ name: k, type: xsdFor(v) }));
    }
    const schema = selectedTypeDef?.properties;
    if (schema && typeof schema === "object") {
      return Object.entries(schema).map(([k, v]) => ({ name: k, type: typeof v === "string" ? `xsd:${v}` : xsdFor(v) }));
    }
    return [];
  }, [selectedConcept, selectedTypeDef]);

  // Query suggestions from ontology
  const querySuggestions = useMemo<string[]>(() => {
    if (!ontology) return [];
    const out: string[] = [];
    const cts = Object.keys(ontology.concept_types);
    const rts = Object.values(ontology.relation_types);
    for (const r of rts.slice(0, 3)) {
      out.push(`Trouver tous les ${r.domain} qui ${r.name.replace(/([A-Z])/g, " $1").trim().toLowerCase()} un ${r.range} donné`);
    }
    if (cts.length >= 2) out.push(`Afficher tous les ${cts[0]} créés par un ${cts[1]}`);
    if (cts.length >= 2) out.push(`Lister tous les ${cts[0]} liés à ${cts[1]}`);
    if (cts.length >= 1) out.push(`Trouver les ${cts[cts.length - 1]} récents`);
    return out.slice(0, 5);
  }, [ontology]);

  // Graph activity (from recent files)
  const activity = useMemo(() => {
    return [...files].sort((a, b) => b.uploaded_at - a.uploaded_at).slice(0, 4);
  }, [files]);

  // Recent paths from saved queries
  const recentPaths = useMemo(() => queries.slice(0, 4), [queries]);

  // ---------- Handlers ----------

  // Click selects; clicking the selected node again (or the empty canvas)
  // clears the selection.
  const onNodeClick = (id: string) => {
    setSelectedId((prev) => (prev === id ? null : id));
    setTab("inspector");
    setCollapseInspector(false);
  };

  const onPaneClick = () => setSelectedId(null);

  const onNodeDoubleClick = (id: string) => {
    setSelectedId(id);
    canvasRef.current?.focusNode(id);
  };

  const onFocusNode = () => {
    if (selectedId) canvasRef.current?.focusNode(selectedId);
  };

  const onExpandNeighbors = async () => {
    if (!selectedConcept) return;
    setNodeTypes([selectedConcept.concept_type]);
    setDepth(Math.min(5, depth + 1));
  };

  const onRunQuery = () => {
    if (selectedConcept) {
      navigate(`/queries?q=${encodeURIComponent(selectedConcept.name)}`);
    } else {
      navigate("/queries");
    }
  };

  const onFullscreen = () => {
    const el = document.getElementById("gv-canvas-wrap");
    if (!el) return;
    if (document.fullscreenElement) document.exitFullscreen();
    else el.requestFullscreen?.();
  };

  // ---------- Render ----------

  const headerSampleConcepts = samples.map((s) => s.concepts);
  const headerSampleRelations = samples.map((s) => s.relations);

  return (
    <>
      <div className="page-header">
        <div>
          <h1 className="page-title">Graphe</h1>
          <p className="page-subtitle">Les fiches et leurs liens, autour d'un point de départ, filtrés par type.</p>
        </div>
      </div>

      {error && <div className="error-banner">{error}</div>}

      {/* Row 1 — KPI tiles */}
      <div className="dash-row dash-row-stats">
        <Kpi
          label="Fiches"
          value={fmtNum(nodesCount)}
          icon={Icon.layers}
          tone="blue"
          spark={headerSampleConcepts}
          sparkColor="#2563eb"
        />
        <Kpi
          label="Liens"
          value={fmtNum(relsCount)}
          icon={Icon.share}
          tone="violet"
          spark={headerSampleRelations}
          sparkColor="#7c3aed"
        />
        <Kpi
          label="Filtres actifs"
          value={fmtNum(activeFiltersCount)}
          icon={Icon.funnel}
          tone="amber"
          spark={[]}
          sparkColor="#d97706"
        />
        <Kpi
          label="Santé du graphe"
          value={`${graphHealth}%`}
          icon={Icon.shield}
          tone="green"
          spark={[]}
          sparkColor="#16a34a"
        />
      </div>

      {/* Row 2 — Filters / Graph / Inspector */}
      <div className={`dash-row gv-row-main${collapseInspector ? " inspector-collapsed" : ""}`}>
        {/* Filters & Controls */}
        <Card title="Filtres et réglages">
          <div className="gv-filter-group">
            <div className="files-search">
              <span className="files-search-icon">{Icon.search}</span>
              <input placeholder="Rechercher une fiche…" value={search} onChange={(e) => setSearch(e.target.value)} />
            </div>
          </div>

          <div className="gv-filter-group">
            <label className="gv-filter-label" htmlFor="gv-node-types">
              Types de fiche{" "}
              <span className="muted">
                ({nodeTypes.length === 0 ? "tous" : `${nodeTypes.length} choisi(s)`}
                {large ? ", au moins un sur un grand graphe" : ""})
              </span>
            </label>
            <select
              id="gv-node-types"
              multiple
              size={Math.min(6, Math.max(2, conceptTypes.length))}
              value={nodeTypes}
              onChange={(e) => setNodeTypes(Array.from(e.target.selectedOptions, (o) => o.value))}
            >
              {conceptTypes.map((t) => <option key={t}>{t}</option>)}
            </select>
            {nodeTypes.length > 0 && (
              <button className="btn-ghost" type="button" onClick={() => setNodeTypes([])}>
                {large ? "Vider la sélection" : "Tous les types"}
              </button>
            )}
          </div>

          <div className="gv-filter-group">
            <label className="gv-filter-label" htmlFor="gv-rel-types">
              Types de lien <span className="muted">({relTypes.length === 0 ? "tous" : `${relTypes.length} choisi(s)`})</span>
            </label>
            <select
              id="gv-rel-types"
              multiple
              size={Math.min(6, Math.max(2, relationTypes.length))}
              value={relTypes}
              onChange={(e) => setRelTypes(Array.from(e.target.selectedOptions, (o) => o.value))}
            >
              {relationTypes.map((t) => <option key={t}>{t}</option>)}
            </select>
            {relTypes.length > 0 && (
              <button className="btn-ghost" type="button" onClick={() => setRelTypes([])}>
                Tous les liens
              </button>
            )}
          </div>

          <div className="gv-filter-group">
            <div className="gv-slider-head">
              <label className="gv-filter-label">Profondeur</label>
              <span className="muted gv-depth-value">{depth} niveau{depth > 1 ? "x" : ""}</span>
            </div>
            <input
              type="range"
              min={1}
              max={5}
              step={1}
              value={depth}
              onChange={(e) => setDepth(Number(e.target.value))}
              className="gv-slider"
            />
            <div className="gv-slider-ticks">
              {[1, 2, 3, 4, 5].map((n) => <span key={n}>{n}</span>)}
            </div>
          </div>

          <div className="gv-toggles">
            <Toggle checked={showLabels} onChange={setShowLabels} label="Afficher les libellés" icon={Icon.funnel} />
            <Toggle checked={clusterView} onChange={setClusterView} label="Vue groupée" icon={Icon.layers} />
            <Toggle checked={highlightPaths} onChange={setHighlightPaths} label="Surligner les chemins" icon={Icon.share} />
            <Toggle checked={showConstraints} onChange={setShowConstraints} label="Afficher les contraintes" icon={Icon.shield} />
          </div>
        </Card>

        {/* Ontology Graph canvas */}
        <Card
          title={
            <span className="gv-canvas-title">
              <span>Graphe de l'ontologie</span>
              <span className="badge-live"><span className="dot" /> En direct</span>
            </span>
          }
          actions={
            <div className="gv-toolbar">
              <div className="gv-toolbar-group gv-layout-picker">
                <button className="gv-tool-btn" onClick={() => setLayoutOpen((v) => !v)}>
                  <span className="gv-tool-icon">{Icon.layout}</span>
                  <span>Disposition</span>
                  <span className="gv-caret">▾</span>
                </button>
                {layoutOpen && (
                  <ul className="gv-layout-menu" onMouseLeave={() => setLayoutOpen(false)}>
                    {LAYOUTS.map((l) => (
                      <li key={l.value}>
                        <button
                          className={`gv-layout-item${layoutDir === l.value ? " active" : ""}`}
                          onClick={() => { setLayoutDir(l.value); setLayoutOpen(false); }}
                        >
                          {l.label}
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
              </div>
              <button className="gv-tool-btn icon" onClick={() => canvasRef.current?.zoomIn()} aria-label="Zoom avant">{Icon.plus}</button>
              <button className="gv-tool-btn icon" onClick={() => canvasRef.current?.zoomOut()} aria-label="Zoom arrière">{Icon.minus}</button>
              <button className="gv-tool-btn icon" onClick={() => canvasRef.current?.fit()} aria-label="Ajuster la vue">{Icon.fit}</button>
              <button className="gv-tool-btn icon" onClick={onFullscreen} aria-label="Plein écran">{Icon.expand}</button>
              <button className="gv-tool-btn icon" onClick={() => setHighlightPaths((v) => !v)} aria-label="Basculer le surlignage des chemins">{Icon.funnel}</button>
              <button className="gv-tool-btn icon" onClick={() => loadSubgraph()} aria-label="Actualiser" disabled={busy}>{Icon.refresh}</button>
            </div>
          }
        >
          <div id="gv-canvas-wrap" className="gv-canvas">
            {large && !canLoad && (
              <div className="empty" data-testid="select-first">
                {total!.toLocaleString("fr-CH")} fiches : trop pour tout afficher d'un coup. Choisissez un ou
                plusieurs types de fiche à gauche, ou cherchez une fiche ; le graphe montre alors jusqu'à{" "}
                {SUBGRAPH_LIMIT} fiches autour de la sélection.
              </div>
            )}
            {subgraph && subgraph.concepts.length === 0 && conceptTypes.length > 0 && !search.trim() && (
              <div className="empty" data-testid="model-only">
                Le modèle est en place ({conceptTypes.length} type(s) de fiche) mais il n'y a pas encore de fiche à
                afficher : le graphe se remplit avec vos fichiers.
              </div>
            )}
            <GraphCanvas
              ref={canvasRef}
              subgraph={filteredSubgraph}
              layoutDir={layoutDir}
              showLabels={showLabels}
              highlightPaths={highlightPaths}
              selectedNodeId={selectedId}
              conceptTypeColors={conceptTypeColors}
              onNodeClick={onNodeClick}
              onPaneClick={onPaneClick}
              onNodeDoubleClick={onNodeDoubleClick}
            />
          </div>
          <div className="gv-legend">
            <span className="gv-legend-item"><span className="dot" style={{ background: "#2563eb" }} /> Classe</span>
            <span className="gv-legend-item"><span className="dot" style={{ background: "#16a34a" }} /> Entité</span>
            <span className="gv-legend-item"><span className="dot" style={{ background: "#7c3aed" }} /> Lien</span>
            <span className="gv-legend-item"><span className="dot" style={{ background: "#d97706" }} /> Type de donnée</span>
            <span className="gv-legend-item"><span className="dot" style={{ background: "#dc2626" }} /> Contrainte</span>
          </div>
        </Card>

        {/* Node Inspector */}
        <Card
          className="gv-inspector-card"
          title={
            <span className="gv-inspector-head">
              <span>Inspecteur de fiche</span>
            </span>
          }
          actions={
            <span className="gv-inspector-nav">
              <button className="icon-btn" aria-label="Précédent" onClick={() => setCollapseInspector(true)}>{Icon.chevL}</button>
              <button className="icon-btn" aria-label="Suivant" onClick={() => setCollapseInspector(false)}>{Icon.chevR}</button>
            </span>
          }
        >
          <div className="gv-tabs">
            <button className={`gv-tab${tab === "inspector" ? " active" : ""}`} onClick={() => setTab("inspector")}>Inspecteur</button>
            <button className={`gv-tab${tab === "rules" ? " active" : ""}`} onClick={() => setTab("rules")}>
              Règles <span className="gv-tab-count">{rules.length}</span>
            </button>
            <button className={`gv-tab${tab === "actions" ? " active" : ""}`} onClick={() => setTab("actions")}>
              Actions <span className="gv-tab-count">{actions.length}</span>
            </button>
          </div>

          {tab === "inspector" && (
            selectedConcept ? (
              <div className="gv-inspector">
                <div className="gv-inspector-title">
                  <span className="gv-inspector-avatar" style={{ background: `${conceptTypeColors[selectedConcept.concept_type] ?? "#2563eb"}22`, color: conceptTypeColors[selectedConcept.concept_type] ?? "#2563eb" }}>
                    {Icon.person}
                  </span>
                  <span className="gv-inspector-name" style={{ color: conceptTypeColors[selectedConcept.concept_type] ?? "var(--accent)" }}>
                    {selectedConcept.name}
                  </span>
                  <span className="badge badge-accent">Classe</span>
                </div>
                <div className="gv-inspector-uri">
                  <Link to={`/concepts/${selectedConcept.id}`}>Ouvrir la fiche</Link>
                </div>
                {(selectedConcept.description || selectedTypeDef?.description) && (
                  <p className="gv-inspector-desc">{selectedConcept.description || selectedTypeDef?.description}</p>
                )}

                <h4 className="gv-section-h">Aperçu</h4>
                <ul className="gv-overview">
                  <li><span className="muted">Type de fiche</span><span>{selectedConcept.concept_type}<span className="gv-chev">{Icon.chevR}</span></span></li>
                  <li><span className="muted">Liens au total</span><span>{selectedStats.total}<span className="gv-chev">{Icon.chevR}</span></span></li>
                  <li><span className="muted">Liens entrants</span><span>{selectedStats.incoming}<span className="gv-chev">{Icon.chevR}</span></span></li>
                  <li><span className="muted">Liens sortants</span><span>{selectedStats.outgoing}<span className="gv-chev">{Icon.chevR}</span></span></li>
                </ul>

                <h4 className="gv-section-h">Propriétés ({selectedProps.length})</h4>
                {selectedProps.length === 0 ? (
                  <div className="muted gv-empty-mini">Aucune propriété déclarée.</div>
                ) : (
                  <ul className="gv-props">
                    {selectedProps.map((p) => (
                      <li key={p.name}>
                        <span className="gv-prop-name">{p.name}</span>
                        <span className="gv-prop-type mono muted">{p.type}</span>
                      </li>
                    ))}
                  </ul>
                )}

                <div className="gv-inspector-actions">
                  <button className="btn-ghost-outline" onClick={onFocusNode}>
                    <span>{Icon.focus}</span> Centrer la fiche
                  </button>
                  <button className="btn-ghost-outline" onClick={onExpandNeighbors}>
                    <span>{Icon.share}</span> Étendre le voisinage
                  </button>
                  <button className="btn-primary gv-run" onClick={onRunQuery}>
                    <span>{Icon.play}</span> Lancer la requête
                  </button>
                </div>
              </div>
            ) : (
              <div className="empty gv-empty">Cliquez sur une fiche du graphe pour voir son type, ses propriétés et ses liens.</div>
            )
          )}

          {tab === "rules" && (
            rules.length === 0 ? (
              <div className="empty gv-empty">Aucune règle déclarée dans cette ontologie.</div>
            ) : (
              <ul className="gv-rule-list">
                {rules.map((r) => (
                  <li key={r.name} className="gv-rule-card">
                    <div className="gv-rule-head">
                      <span className="gv-rule-name">{r.name}</span>
                      {r.strict ? <span className="badge badge-danger">stricte</span> : <span className="badge badge-accent">indicative</span>}
                    </div>
                    {r.description && <p className="muted gv-rule-desc">{r.description}</p>}
                    {r.when && <div className="gv-rule-row"><span className="gv-rule-k">SI</span><span className="gv-rule-v">{r.when}</span></div>}
                    {r.then && <div className="gv-rule-row"><span className="gv-rule-k">ALORS</span><span className="gv-rule-v">{r.then}</span></div>}
                    {r.applies_to && r.applies_to.length > 0 && (
                      <div className="gv-rule-tags">
                        {r.applies_to.map((t) => <span key={t} className="badge">{t}</span>)}
                      </div>
                    )}
                  </li>
                ))}
              </ul>
            )
          )}

          {tab === "actions" && (
            actions.length === 0 ? (
              <div className="empty gv-empty">Aucune action déclarée dans cette ontologie.</div>
            ) : (
              <ul className="gv-rule-list">
                {actions.map((a) => (
                  <li key={a.name} className="gv-rule-card">
                    <div className="gv-rule-head">
                      <span className="gv-rule-name">{a.name}</span>
                      <span className="badge badge-success">action</span>
                    </div>
                    {a.description && <p className="muted gv-rule-desc">{a.description}</p>}
                    <div className="gv-rule-row">
                      <span className="gv-rule-k">SUJET</span>
                      <span className="gv-rule-v">
                        {a.subject}{a.object ? <> → <strong>{a.object}</strong></> : null}
                      </span>
                    </div>
                    {a.parameters && a.parameters.length > 0 && (
                      <div className="gv-rule-row">
                        <span className="gv-rule-k">PARAMÈTRES</span>
                        <span className="gv-rule-tags">
                          {a.parameters.map((p) => <span key={p} className="badge">{p}</span>)}
                        </span>
                      </div>
                    )}
                    {a.effect && <div className="gv-rule-row"><span className="gv-rule-k">EFFET</span><span className="gv-rule-v">{a.effect}</span></div>}
                  </li>
                ))}
              </ul>
            )
          )}
        </Card>
      </div>

      {/* Row 3 — Recent Paths / Activity / Suggestions */}
      <div className="dash-row dash-row-three">
        <Card
          title="Chemins récents / vues enregistrées"
          actions={<button className="btn-ghost-link" onClick={() => navigate("/queries")}>Tout voir</button>}
        >
          {recentPaths.length === 0 ? (
            <div className="empty gv-empty">Aucune vue enregistrée pour l'instant.</div>
          ) : (
            <ul className="gv-path-list">
              {recentPaths.map((p) => (
                <li key={p.id} className="gv-path-item">
                  <span className="gv-path-ic">{Icon.pathIcon}</span>
                  <span className="gv-path-text" title={p.query}>{p.name}</span>
                  <span className="muted gv-path-time">{p.last_run_at ? fmtAgo(p.last_run_at) : "—"}</span>
                </li>
              ))}
            </ul>
          )}
        </Card>

        <Card
          title="Activité du graphe"
          actions={<button className="btn-ghost-link" onClick={() => navigate("/files")}>Tout voir</button>}
        >
          {activity.length === 0 ? (
            <div className="empty gv-empty">Aucune activité récente.</div>
          ) : (
            <ul className="gv-activity">
              {activity.map((f) => {
                const s = (f.status || "").toLowerCase();
                const tone =
                  s === "processed" || s === "ingested" || s === "done" ? "ok"
                  : s === "failed" || s === "error" ? "err"
                  : s === "analyzed" ? "info"
                  : "warn";
                const verb =
                  tone === "ok" ? "Import terminé pour"
                  : tone === "err" ? "Validation échouée sur"
                  : tone === "info" ? "Fiche mise à jour depuis"
                  : "Ingestion en cours pour";
                return (
                  <li key={f.id} className="gv-activity-item">
                    <span className={`gv-act-ic gv-act-${tone}`}>{Icon.activity}</span>
                    <span className="gv-act-text">{verb} <em>{f.name}</em></span>
                    <span className="muted gv-act-time">{fmtAgo(f.uploaded_at)}</span>
                  </li>
                );
              })}
            </ul>
          )}
        </Card>

        <Card
          title="Suggestions de requêtes"
          actions={<button className="btn-ghost-link" onClick={() => navigate("/queries")}>Tout voir</button>}
        >
          {querySuggestions.length === 0 ? (
            <div className="empty gv-empty">Aucune suggestion disponible.</div>
          ) : (
            <ul className="gv-suggestions">
              {querySuggestions.map((q, i) => (
                <li key={i} className="gv-suggestion">
                  <span className="gv-sug-ic">{Icon.search}</span>
                  <button className="gv-sug-text" onClick={() => navigate(`/queries?q=${encodeURIComponent(q)}`)}>
                    {q}
                  </button>
                </li>
              ))}
            </ul>
          )}
        </Card>
      </div>
    </>
  );
}
