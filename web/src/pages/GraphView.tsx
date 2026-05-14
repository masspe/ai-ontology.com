// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import Card from "../components/Card";
import Sparkline from "../components/Sparkline";
import GraphCanvas, { type GraphCanvasHandle, type LayoutDir } from "../components/GraphCanvas";
import {
  getFiles,
  getOntology,
  getQueries,
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
  if (diff < 60) return `${diff}s ago`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
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
            <span>{arrow} {Math.abs(deltaPct).toFixed(0)}%</span>
            <span className="muted">{deltaLabel ?? "vs last month"}</span>
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
  { value: "LR", label: "Left → Right" },
  { value: "TB", label: "Top → Bottom" },
  { value: "RL", label: "Right → Left" },
  { value: "BT", label: "Bottom → Top" },
];

type InspectorTab = "inspector" | "rules" | "actions";

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
  const [nodeType, setNodeType] = useState("All Types");
  const [relType, setRelType] = useState("All Relations");
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

  const loadSubgraph = async (d = depth) => {
    setBusy(true);
    setError(null);
    try {
      const res = await getSubgraph({
        seed_query: search.trim() || undefined,
        seed_concept_types: nodeType !== "All Types" ? [nodeType] : [],
        expansion_depth: d,
        limit: 250,
      });
      setSubgraph(res.subgraph);
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
    loadSubgraph(depth);
    const t = window.setInterval(() => loadSubgraph(depth), 30_000);
    return () => window.clearInterval(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Refetch when depth changes (debounced via slider release would be nicer; effect is fine)
  useEffect(() => {
    loadSubgraph(depth);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [depth]);

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
      if (nodeType !== "All Types" && c.concept_type !== nodeType) return false;
      if (q && !c.name.toLowerCase().includes(q) && !c.concept_type.toLowerCase().includes(q)) return false;
      return true;
    };
    const concepts = subgraph.concepts.filter(keepConcept);
    const keepIds = new Set(concepts.map((c) => c.id));
    const relations: Relation[] = subgraph.relations.filter((r) => {
      if (relType !== "All Relations" && r.relation_type !== relType) return false;
      return keepIds.has(r.source) && keepIds.has(r.target);
    });
    return { concepts, relations };
  }, [subgraph, search, nodeType, relType]);

  // Active filters count for KPI
  const activeFiltersCount = useMemo(() => {
    let n = 0;
    if (search.trim()) n++;
    if (nodeType !== "All Types") n++;
    if (relType !== "All Relations") n++;
    if (depth !== 3) n++;
    return n;
  }, [search, nodeType, relType, depth]);

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
      out.push(`Find all ${r.domain} ${r.name.replace(/([A-Z])/g, " $1").trim().toLowerCase()} a specific ${r.range}`);
    }
    if (cts.length >= 2) out.push(`Show all ${cts[0]} created by an ${cts[1]}`);
    if (cts.length >= 2) out.push(`List all ${cts[0]} related to ${cts[1]}`);
    if (cts.length >= 1) out.push(`Find recent ${cts[cts.length - 1]}s`);
    return out.slice(0, 5);
  }, [ontology]);

  // Graph activity (from recent files)
  const activity = useMemo(() => {
    return [...files].sort((a, b) => b.uploaded_at - a.uploaded_at).slice(0, 4);
  }, [files]);

  // Recent paths from saved queries
  const recentPaths = useMemo(() => queries.slice(0, 4), [queries]);

  // ---------- Handlers ----------

  const onNodeClick = (id: string) => {
    setSelectedId(id);
    setTab("inspector");
    setCollapseInspector(false);
  };

  const onFocusNode = () => {
    if (selectedId) canvasRef.current?.focusNode(selectedId);
  };

  const onExpandNeighbors = async () => {
    if (!selectedConcept) return;
    setNodeType(selectedConcept.concept_type);
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
          <h1 className="page-title">Graph View</h1>
          <p className="page-subtitle">Explore ontology entities, classes, and relationships visually.</p>
        </div>
      </div>

      {error && <div className="error-banner">{error}</div>}

      {/* Row 1 — KPI tiles */}
      <div className="dash-row dash-row-stats">
        <Kpi
          label="Nodes"
          value={fmtNum(nodesCount)}
          icon={Icon.layers}
          tone="blue"
          spark={headerSampleConcepts}
          sparkColor="#2563eb"
        />
        <Kpi
          label="Relations"
          value={fmtNum(relsCount)}
          icon={Icon.share}
          tone="violet"
          spark={headerSampleRelations}
          sparkColor="#7c3aed"
        />
        <Kpi
          label="Active Filters"
          value={fmtNum(activeFiltersCount)}
          icon={Icon.funnel}
          tone="amber"
          spark={[]}
          sparkColor="#d97706"
        />
        <Kpi
          label="Graph Health"
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
        <Card title="Filters & Controls">
          <div className="gv-filter-group">
            <div className="files-search">
              <span className="files-search-icon">{Icon.search}</span>
              <input placeholder="Search nodes…" value={search} onChange={(e) => setSearch(e.target.value)} />
            </div>
          </div>

          <div className="gv-filter-group">
            <label className="gv-filter-label">Node Type</label>
            <select value={nodeType} onChange={(e) => setNodeType(e.target.value)}>
              <option>All Types</option>
              {conceptTypes.map((t) => <option key={t}>{t}</option>)}
            </select>
          </div>

          <div className="gv-filter-group">
            <label className="gv-filter-label">Relation Type</label>
            <select value={relType} onChange={(e) => setRelType(e.target.value)}>
              <option>All Relations</option>
              {relationTypes.map((t) => <option key={t}>{t}</option>)}
            </select>
          </div>

          <div className="gv-filter-group">
            <div className="gv-slider-head">
              <label className="gv-filter-label">Traversal Depth</label>
              <span className="muted gv-depth-value">{depth} levels</span>
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
            <Toggle checked={showLabels} onChange={setShowLabels} label="Show labels" icon={Icon.funnel} />
            <Toggle checked={clusterView} onChange={setClusterView} label="Cluster view" icon={Icon.layers} />
            <Toggle checked={highlightPaths} onChange={setHighlightPaths} label="Highlight paths" icon={Icon.share} />
            <Toggle checked={showConstraints} onChange={setShowConstraints} label="Show constraints" icon={Icon.shield} />
          </div>
        </Card>

        {/* Ontology Graph canvas */}
        <Card
          title={
            <span className="gv-canvas-title">
              <span>Ontology Graph</span>
              <span className="badge-live"><span className="dot" /> Live</span>
            </span>
          }
          actions={
            <div className="gv-toolbar">
              <div className="gv-toolbar-group gv-layout-picker">
                <button className="gv-tool-btn" onClick={() => setLayoutOpen((v) => !v)}>
                  <span className="gv-tool-icon">{Icon.layout}</span>
                  <span>Layout</span>
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
              <button className="gv-tool-btn icon" onClick={() => canvasRef.current?.zoomIn()} aria-label="Zoom in">{Icon.plus}</button>
              <button className="gv-tool-btn icon" onClick={() => canvasRef.current?.zoomOut()} aria-label="Zoom out">{Icon.minus}</button>
              <button className="gv-tool-btn icon" onClick={() => canvasRef.current?.fit()} aria-label="Fit view">{Icon.fit}</button>
              <button className="gv-tool-btn icon" onClick={onFullscreen} aria-label="Fullscreen">{Icon.expand}</button>
              <button className="gv-tool-btn icon" onClick={() => setHighlightPaths((v) => !v)} aria-label="Toggle filters">{Icon.funnel}</button>
              <button className="gv-tool-btn icon" onClick={() => loadSubgraph()} aria-label="Refresh" disabled={busy}>{Icon.refresh}</button>
            </div>
          }
        >
          <div id="gv-canvas-wrap" className="gv-canvas">
            <GraphCanvas
              ref={canvasRef}
              subgraph={filteredSubgraph}
              layoutDir={layoutDir}
              showLabels={showLabels}
              highlightPaths={highlightPaths}
              selectedNodeId={selectedId}
              conceptTypeColors={conceptTypeColors}
              onNodeClick={onNodeClick}
            />
          </div>
          <div className="gv-legend">
            <span className="gv-legend-item"><span className="dot" style={{ background: "#2563eb" }} /> Class</span>
            <span className="gv-legend-item"><span className="dot" style={{ background: "#16a34a" }} /> Entity</span>
            <span className="gv-legend-item"><span className="dot" style={{ background: "#7c3aed" }} /> Relation</span>
            <span className="gv-legend-item"><span className="dot" style={{ background: "#d97706" }} /> Data Type</span>
            <span className="gv-legend-item"><span className="dot" style={{ background: "#dc2626" }} /> Constraint</span>
          </div>
        </Card>

        {/* Node Inspector */}
        <Card
          className="gv-inspector-card"
          title={
            <span className="gv-inspector-head">
              <span>Node Inspector</span>
            </span>
          }
          actions={
            <span className="gv-inspector-nav">
              <button className="icon-btn" aria-label="Previous" onClick={() => setCollapseInspector(true)}>{Icon.chevL}</button>
              <button className="icon-btn" aria-label="Next" onClick={() => setCollapseInspector(false)}>{Icon.chevR}</button>
            </span>
          }
        >
          <div className="gv-tabs">
            <button className={`gv-tab${tab === "inspector" ? " active" : ""}`} onClick={() => setTab("inspector")}>Inspector</button>
            <button className={`gv-tab${tab === "rules" ? " active" : ""}`} onClick={() => setTab("rules")}>
              Rules <span className="gv-tab-count">{rules.length}</span>
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
                  <span className="badge badge-accent">Class</span>
                </div>
                <div className="gv-inspector-uri">
                  <span className="muted">URI:</span> <span className="mono">ex:{selectedConcept.name}</span>
                </div>
                {(selectedConcept.description || selectedTypeDef?.description) && (
                  <p className="gv-inspector-desc">{selectedConcept.description || selectedTypeDef?.description}</p>
                )}

                <h4 className="gv-section-h">Overview</h4>
                <ul className="gv-overview">
                  <li><span className="muted">Node Type</span><span>{selectedConcept.concept_type}<span className="gv-chev">{Icon.chevR}</span></span></li>
                  <li><span className="muted">Total Connections</span><span>{selectedStats.total}<span className="gv-chev">{Icon.chevR}</span></span></li>
                  <li><span className="muted">Incoming Relations</span><span>{selectedStats.incoming}<span className="gv-chev">{Icon.chevR}</span></span></li>
                  <li><span className="muted">Outgoing Relations</span><span>{selectedStats.outgoing}<span className="gv-chev">{Icon.chevR}</span></span></li>
                </ul>

                <h4 className="gv-section-h">Properties ({selectedProps.length})</h4>
                {selectedProps.length === 0 ? (
                  <div className="muted gv-empty-mini">No declared properties.</div>
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
                    <span>{Icon.focus}</span> Focus Node
                  </button>
                  <button className="btn-ghost-outline" onClick={onExpandNeighbors}>
                    <span>{Icon.share}</span> Expand Neighbors
                  </button>
                  <button className="btn-primary gv-run" onClick={onRunQuery}>
                    <span>{Icon.play}</span> Run Query
                  </button>
                </div>
              </div>
            ) : (
              <div className="empty gv-empty">Click a node in the graph to inspect its concept type, properties, and connections.</div>
            )
          )}

          {tab === "rules" && (
            rules.length === 0 ? (
              <div className="empty gv-empty">No rules declared in this ontology.</div>
            ) : (
              <ul className="gv-rule-list">
                {rules.map((r) => (
                  <li key={r.name} className="gv-rule-card">
                    <div className="gv-rule-head">
                      <span className="gv-rule-name">{r.name}</span>
                      {r.strict ? <span className="badge badge-danger">strict</span> : <span className="badge badge-accent">advisory</span>}
                    </div>
                    {r.description && <p className="muted gv-rule-desc">{r.description}</p>}
                    {r.when && <div className="gv-rule-row"><span className="gv-rule-k">WHEN</span><span className="gv-rule-v">{r.when}</span></div>}
                    {r.then && <div className="gv-rule-row"><span className="gv-rule-k">THEN</span><span className="gv-rule-v">{r.then}</span></div>}
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
              <div className="empty gv-empty">No actions declared in this ontology.</div>
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
                      <span className="gv-rule-k">SUBJECT</span>
                      <span className="gv-rule-v">
                        {a.subject}{a.object ? <> → <strong>{a.object}</strong></> : null}
                      </span>
                    </div>
                    {a.parameters && a.parameters.length > 0 && (
                      <div className="gv-rule-row">
                        <span className="gv-rule-k">PARAMS</span>
                        <span className="gv-rule-tags">
                          {a.parameters.map((p) => <span key={p} className="badge">{p}</span>)}
                        </span>
                      </div>
                    )}
                    {a.effect && <div className="gv-rule-row"><span className="gv-rule-k">EFFECT</span><span className="gv-rule-v">{a.effect}</span></div>}
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
          title="Recent Paths / Saved Views"
          actions={<button className="btn-ghost-link" onClick={() => navigate("/queries")}>View All</button>}
        >
          {recentPaths.length === 0 ? (
            <div className="empty gv-empty">No saved views yet.</div>
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
          title="Graph Activity"
          actions={<button className="btn-ghost-link" onClick={() => navigate("/files")}>View All</button>}
        >
          {activity.length === 0 ? (
            <div className="empty gv-empty">No recent activity.</div>
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
                  tone === "ok" ? "Bulk import completed for"
                  : tone === "err" ? "Validation failed on"
                  : tone === "info" ? "Node updated from"
                  : "New ingestion in progress for";
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
          title="Query Suggestions"
          actions={<button className="btn-ghost-link" onClick={() => navigate("/queries")}>View All</button>}
        >
          {querySuggestions.length === 0 ? (
            <div className="empty gv-empty">No suggestions available.</div>
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
