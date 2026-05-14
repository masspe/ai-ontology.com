// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { useEffect, useMemo, useState } from "react";
import Card from "../components/Card";
import Dropzone from "../components/Dropzone";
import GraphCanvas from "../components/GraphCanvas";
import {
  deleteFile,
  exportGraphUrl,
  generateOntology,
  getFiles,
  getOntology,
  getStats,
  getSubgraph,
  replaceOntology,
  upload,
  type FileRecord,
  type Ontology,
  type Stats,
  type Subgraph,
} from "../api";

const MAX_DESC = 4000;

const EXAMPLES = [
  "A contract management system tracking parties, agreements, clauses, obligations and renewal dates.",
  "A research knowledge base of papers, authors, institutions, topics and citations.",
  "A clinical ontology covering patients, diagnoses, medications, procedures and outcomes.",
];

const KIND_BY_EXT: Record<string, string> = {
  json: "ontology",
  jsonl: "jsonl",
  ndjson: "jsonl",
  triples: "triples",
  csv: "csv",
  xlsx: "xlsx",
  txt: "text",
  md: "text",
  pdf: "text",
  docx: "text",
};

function fmtBytes(b: number): string {
  if (b < 1024) return `${b} B`;
  if (b < 1024 * 1024) return `${(b / 1024).toFixed(1)} KB`;
  return `${(b / (1024 * 1024)).toFixed(1)} MB`;
}

function fmtAgo(ts: number): string {
  const diff = Math.max(0, Math.floor(Date.now() / 1000 - ts));
  if (diff < 60) return `${diff}s ago`;
  if (diff < 3600) return `${Math.floor(diff / 60)} min ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} h ago`;
  return `${Math.floor(diff / 86400)} d ago`;
}

export default function OntologyBuilder() {
  const [description, setDescription] = useState("");
  const [showExamples, setShowExamples] = useState(false);

  const [ontology, setOntology] = useState<Ontology | null>(null);
  const [proposed, setProposed] = useState<Ontology | null>(null);
  const [stats, setStats] = useState<Stats | null>(null);
  const [subgraph, setSubgraph] = useState<Subgraph | null>(null);
  const [files, setFiles] = useState<FileRecord[]>([]);

  const [busy, setBusy] = useState<"idle" | "generate" | "upload" | "save">("idle");
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [lastUpdated, setLastUpdated] = useState<number | null>(null);

  const refresh = async () => {
    try {
      const [o, s, f, sg] = await Promise.all([
        getOntology(),
        getStats(),
        getFiles(),
        getSubgraph({ limit: 150, expansion_depth: 1 }),
      ]);
      setOntology(o);
      setStats(s);
      setFiles(f.files);
      setSubgraph(sg.subgraph);
      setLastUpdated(Math.floor(Date.now() / 1000));
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  const handleGenerate = async () => {
    if (!description.trim()) return;
    setBusy("generate");
    setError(null);
    setInfo(null);
    setProposed(null);
    try {
      const res = await generateOntology(description);
      setProposed(res.ontology);
      setInfo("Draft ontology generated. Review it on the right, then click Save Ontology to apply.");
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy("idle");
    }
  };

  const handleSave = async () => {
    if (!proposed) return;
    setBusy("save");
    setError(null);
    try {
      await replaceOntology(proposed);
      setProposed(null);
      setInfo("Ontology saved.");
      await refresh();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy("idle");
    }
  };

  const handleUpload = async (file: File) => {
    setBusy("upload");
    setError(null);
    setInfo(null);
    try {
      const ext = (file.name.split(".").pop() ?? "").toLowerCase();
      const kind = KIND_BY_EXT[ext] ?? "text";
      const needsCt = ["csv", "xlsx", "text"].includes(kind);
      const conceptType = needsCt
        ? (window.prompt(`Concept type for ${file.name}?`, "Document") ?? "").trim()
        : undefined;
      if (needsCt && !conceptType) {
        setBusy("idle");
        return;
      }
      const res = await upload(file, { kind, conceptType: conceptType || undefined });
      setInfo(`Ingested ${res.ingested.concepts} concepts, ${res.ingested.relations} relations from ${file.name}.`);
      await refresh();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy("idle");
    }
  };

  const handleDeleteFile = async (id: number) => {
    try {
      await deleteFile(id);
      await refresh();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const clearAll = async () => {
    if (files.length === 0) return;
    if (!window.confirm(`Remove ${files.length} file records from history? (Ingested data stays in the graph.)`)) return;
    for (const f of files) {
      try {
        await deleteFile(f.id);
      } catch {
        /* ignore */
      }
    }
    await refresh();
  };

  const legend = useMemo(
    () => [
      { color: "#2563eb", label: "Class" },
      { color: "#16a34a", label: "Entity" },
      { color: "#7c3aed", label: "Relation" },
      { color: "#d97706", label: "Datatype" },
      { color: "#dc2626", label: "Constraint" },
    ],
    [],
  );

  const conceptTypeCount = ontology ? Object.keys(ontology.concept_types).length : 0;

  return (
    <>
      <div className="page-header">
        <div>
          <h1 className="page-title">Ontology Creation</h1>
          <p className="page-subtitle">Create an ontology from natural language instructions or from your files.</p>
        </div>
      </div>

      {error && <div className="error-banner">{error}</div>}
      {info && <div className="success-banner">{info}</div>}

      <div className="builder-grid">
        {/* ---------- LEFT COLUMN ---------- */}
        <div className="builder-left">
          <Card
            title={<span>Describe Your Ontology <span className="muted" title="Plain-English description used by the LLM">ⓘ</span></span>}
            actions={
              <button className="btn-ghost btn-sm" onClick={() => setShowExamples((v) => !v)}>
                ⊞ Examples
              </button>
            }
          >
            {showExamples && (
              <div style={{ marginBottom: 10, display: "flex", flexDirection: "column", gap: 6 }}>
                {EXAMPLES.map((ex, i) => (
                  <button
                    key={i}
                    className="example-chip"
                    onClick={() => {
                      setDescription(ex);
                      setShowExamples(false);
                    }}
                  >
                    {ex}
                  </button>
                ))}
              </div>
            )}
            <textarea
              rows={6}
              maxLength={MAX_DESC}
              placeholder="Describe the ontology structure, entities, relations, and constraints…"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
            />
            <div className="char-counter muted">{description.length} / {MAX_DESC}</div>
            <button
              className="btn-primary btn-block"
              style={{ marginTop: 10 }}
              onClick={handleGenerate}
              disabled={busy !== "idle" || !description.trim()}
            >
              {busy === "generate" ? "⏳ Generating…" : "✦ Generate Ontology"}
            </button>
          </Card>

          <Card
            title={<span>Upload Files (Optional) <span className="muted" title="Files are ingested into the graph">ⓘ</span></span>}
            style={{ marginTop: 16 }}
          >
            <Dropzone
              onFile={handleUpload}
              disabled={busy !== "idle"}
              hint="Supports PDF, DOCX, CSV, JSON, JSONL, XLSX, triples (Max 50 MB)"
            />

            {files.length > 0 && (
              <>
                <div style={{ marginTop: 14, marginBottom: 6, fontSize: 12, fontWeight: 600 }}>Uploaded Files</div>
                <ul className="upload-list">
                  {files.slice(0, 6).map((f) => (
                    <li key={f.id} className="upload-item">
                      <span className="upload-icon" data-kind={f.kind}>{f.kind.slice(0, 4).toUpperCase()}</span>
                      <div className="upload-meta">
                        <div className="upload-name" title={f.name}>{f.name}</div>
                        <div className="upload-size muted">{fmtBytes(f.size)} · {f.kind.toUpperCase()}</div>
                      </div>
                      <span className={`badge ${f.concepts > 0 || f.relations > 0 ? "badge-success" : "badge-accent"}`}>
                        {f.concepts > 0 || f.relations > 0 ? "Processed" : "Analyzed"}
                      </span>
                      <button className="icon-btn" title="Remove" onClick={() => handleDeleteFile(f.id)}>×</button>
                    </li>
                  ))}
                </ul>
              </>
            )}

            <div className="upload-actions">
              <button onClick={refresh} disabled={busy !== "idle"}>↻ Analyze Files</button>
              <button onClick={refresh} disabled={busy !== "idle"}>⇆ Merge Sources</button>
              <button className="btn-danger" onClick={clearAll} disabled={busy !== "idle" || files.length === 0}>
                🗑 Clear All
              </button>
            </div>
          </Card>
        </div>

        {/* ---------- RIGHT COLUMN ---------- */}
        <div className="builder-right">
          <Card
            title={<span>Ontology Graph <span className="muted" title="Live view of the current ontology">ⓘ</span></span>}
            actions={
              <div className="row" style={{ gap: 6 }}>
                <button className="btn-ghost btn-sm">⌧ Layout ▾</button>
                <button className="icon-btn" title="Zoom in">＋</button>
                <button className="icon-btn" title="Zoom out">－</button>
                <button className="icon-btn" title="Fit">⛶</button>
              </div>
            }
            className="graph-card"
          >
            <div className="graph-with-legend">
              <div className="graph-area">
                <GraphCanvas subgraph={proposed ? null : subgraph} />
                {proposed && (
                  <div className="graph-preview-overlay">
                    <div className="muted" style={{ marginBottom: 6, fontSize: 12 }}>Proposed schema · click <strong>Save Ontology</strong> to apply</div>
                    <pre className="json-view" style={{ maxHeight: 360 }}>{JSON.stringify(proposed, null, 2)}</pre>
                  </div>
                )}
              </div>
              <ul className="graph-legend">
                {legend.map((l) => (
                  <li key={l.label}>
                    <span className="legend-dot" style={{ background: l.color }} /> {l.label}
                  </li>
                ))}
              </ul>
            </div>
          </Card>

          <Card
            title={<span>Ontology Insights <span className="muted" title="Counts derived from the live graph">ⓘ</span></span>}
            subtitle={<span>Last updated: {lastUpdated ? fmtAgo(lastUpdated) : "—"} <button className="btn-ghost btn-sm" onClick={refresh} title="Refresh" style={{ marginLeft: 6 }}>↻</button></span>}
            actions={<button className="btn-ghost btn-sm">View Details ›</button>}
            style={{ marginTop: 16 }}
          >
            <div className="insights-grid">
              <InsightTile icon="👥" label="Entities" value={stats?.concepts ?? 0} delta={stats?.deltas.concepts_pct} color="#dbeafe" iconColor="#2563eb" />
              <InsightTile icon="⌬" label="Relations" value={stats?.relations ?? 0} delta={stats?.deltas.relations_pct} color="#ede9fe" iconColor="#7c3aed" />
              <InsightTile icon="▦" label="Classes" value={conceptTypeCount} delta={stats?.deltas.concept_types_pct} color="#dcfce7" iconColor="#16a34a" />
              <InsightTile icon="✓" label="Confidence" value="92%" hint="High" color="#fef3c7" iconColor="#d97706" />
            </div>
          </Card>
        </div>
      </div>

      {/* ---------- BOTTOM BAR ---------- */}
      <div className="builder-footer">
        <a className="btn-icon" href={exportGraphUrl("jsonl")}>⤓ Export ▾</a>
        <button className="btn-primary" onClick={handleSave} disabled={!proposed || busy !== "idle"}>
          💾 Save Ontology
        </button>
      </div>

      <style>{`
        .builder-grid { display: grid; grid-template-columns: minmax(0, 420px) minmax(0, 1fr); gap: 16px; align-items: start; }
        @media (max-width: 1100px) { .builder-grid { grid-template-columns: 1fr; } }
        .btn-sm { padding: 4px 10px; font-size: 12px; }
        .btn-block { width: 100%; }
        .char-counter { font-size: 11px; text-align: right; margin-top: 4px; }
        .example-chip {
          text-align: left; padding: 8px 10px; font-size: 12px;
          background: var(--panel-2); border: 1px solid var(--border);
          border-radius: 8px; cursor: pointer;
        }
        .example-chip:hover { background: var(--accent-soft); border-color: var(--accent); color: var(--accent); }

        .upload-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
        .upload-item { display: flex; align-items: center; gap: 10px; padding: 8px 10px; border-radius: 8px; background: var(--panel-2); }
        .upload-icon {
          width: 32px; height: 32px; border-radius: 6px;
          display: grid; place-items: center; flex-shrink: 0;
          background: var(--accent-soft); color: var(--accent);
          font-size: 10px; font-weight: 700;
        }
        .upload-icon[data-kind="ontology"] { background: #fef3c7; color: #d97706; }
        .upload-icon[data-kind="csv"] { background: #dcfce7; color: #16a34a; }
        .upload-icon[data-kind="xlsx"] { background: #dcfce7; color: #16a34a; }
        .upload-icon[data-kind="text"] { background: #ede9fe; color: #7c3aed; }
        .upload-meta { flex: 1; min-width: 0; }
        .upload-name { font-size: 13px; font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
        .upload-size { font-size: 11px; }
        .upload-actions { display: flex; gap: 8px; margin-top: 14px; flex-wrap: wrap; }
        .upload-actions > * { flex: 1; min-width: 110px; }

        .graph-card .graph-with-legend { position: relative; display: grid; grid-template-columns: 1fr auto; gap: 12px; min-height: 420px; }
        .graph-area { position: relative; min-height: 420px; border: 1px solid var(--border); border-radius: var(--radius-sm); overflow: hidden; background: var(--panel); }
        .graph-preview-overlay { position: absolute; inset: 0; background: rgba(247,248,251,0.97); padding: 16px; overflow: auto; }
        .graph-legend { list-style: none; padding: 8px 12px; margin: 0; display: flex; flex-direction: column; gap: 8px; font-size: 12px; color: var(--muted); }
        .legend-dot { display: inline-block; width: 10px; height: 10px; border-radius: 50%; margin-right: 8px; vertical-align: middle; }

        .insights-grid { display: grid; grid-template-columns: repeat(4, 1fr); gap: 12px; }
        @media (max-width: 900px) { .insights-grid { grid-template-columns: repeat(2, 1fr); } }
        .insight-tile {
          display: flex; gap: 12px; align-items: center;
          padding: 14px; border: 1px solid var(--border); border-radius: var(--radius-sm);
          background: var(--panel);
        }
        .insight-icon { width: 40px; height: 40px; border-radius: 10px; display: grid; place-items: center; font-size: 18px; flex-shrink: 0; }
        .insight-meta .value { font-size: 22px; font-weight: 600; line-height: 1.1; }
        .insight-meta .label { font-size: 12px; color: var(--muted); }
        .insight-meta .delta { font-size: 11px; }

        .builder-footer {
          display: flex; justify-content: flex-end; gap: 10px;
          margin-top: 18px; padding-top: 14px; border-top: 1px solid var(--border);
        }
        .btn-icon { display: inline-flex; align-items: center; gap: 6px; padding: 8px 14px; border-radius: 8px; border: 1px solid var(--border); background: var(--panel); color: var(--text); }
        .btn-icon:hover { background: var(--panel-2); text-decoration: none; }
      `}</style>
    </>
  );
}

interface InsightProps {
  icon: string;
  label: string;
  value: number | string;
  delta?: number;
  hint?: string;
  color: string;
  iconColor: string;
}

function InsightTile({ icon, label, value, delta, hint, color, iconColor }: InsightProps) {
  const cls = delta == null ? "flat" : delta > 0.5 ? "up" : delta < -0.5 ? "down" : "flat";
  const arrow = cls === "up" ? "↑" : cls === "down" ? "↓" : "•";
  return (
    <div className="insight-tile">
      <div className="insight-icon" style={{ background: color, color: iconColor }}>{icon}</div>
      <div className="insight-meta">
        <div className="value">{typeof value === "number" && value >= 1000 ? `${(value / 1000).toFixed(1)}k` : value}</div>
        <div className="label">{label}</div>
        {delta != null && (
          <div className={`delta ${cls}`} style={{ color: cls === "up" ? "var(--success)" : cls === "down" ? "var(--danger)" : "var(--muted)" }}>
            {arrow} {Math.abs(delta).toFixed(0)}% vs last run
          </div>
        )}
        {delta == null && hint && <div className="delta flat muted">{hint} •</div>}
      </div>
    </div>
  );
}
