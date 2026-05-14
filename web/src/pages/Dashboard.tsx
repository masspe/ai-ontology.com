// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { useEffect, useState } from "react";
import Card from "../components/Card";
import StatCard from "../components/StatCard";
import Sparkline from "../components/Sparkline";
import { getFiles, getStats, getStatsHistory, type FileRecord, type Stats, type StatsHistory } from "../api";

function fmtBytes(b: number): string {
  if (b < 1024) return `${b} B`;
  if (b < 1024 * 1024) return `${(b / 1024).toFixed(1)} KB`;
  return `${(b / (1024 * 1024)).toFixed(1)} MB`;
}

function fmtAgo(ts: number): string {
  const diff = Math.max(0, Math.floor(Date.now() / 1000 - ts));
  if (diff < 60) return `${diff}s ago`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}

export default function Dashboard() {
  const [stats, setStats] = useState<Stats | null>(null);
  const [history, setHistory] = useState<StatsHistory | null>(null);
  const [files, setFiles] = useState<FileRecord[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const tick = async () => {
      try {
        const [s, h, f] = await Promise.all([getStats(), getStatsHistory(), getFiles()]);
        if (cancelled) return;
        setStats(s);
        setHistory(h);
        setFiles(f.files);
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

  return (
    <>
      <div className="page-header">
        <div>
          <h1 className="page-title">Dashboard</h1>
          <p className="page-subtitle">Overview of your knowledge graph and recent activity.</p>
        </div>
      </div>

      {error && <div className="error-banner">{error}</div>}

      <div className="grid grid-4" style={{ marginBottom: 16 }}>
        <StatCard
          label="Concepts"
          value={stats?.concepts ?? 0}
          deltaPct={stats?.deltas.concepts_pct}
        />
        <StatCard
          label="Relations"
          value={stats?.relations ?? 0}
          deltaPct={stats?.deltas.relations_pct}
        />
        <StatCard
          label="Concept types"
          value={stats?.concept_types ?? 0}
          deltaPct={stats?.deltas.concept_types_pct}
        />
        <StatCard
          label="Relation types"
          value={stats?.relation_types ?? 0}
          deltaPct={stats?.deltas.relation_types_pct}
        />
      </div>

      <div className="grid grid-2" style={{ marginBottom: 16 }}>
        <Card title="Concepts over time">
          {history && history.samples.length > 0 ? (
            <>
              <Sparkline values={history.samples.map((s) => s.concepts)} />
              <div className="muted" style={{ fontSize: 12, marginTop: 8 }}>
                {history.samples.length} samples · last update {fmtAgo(history.samples[history.samples.length - 1]!.ts)}
              </div>
            </>
          ) : (
            <div className="empty">No samples yet. The dashboard auto-refreshes every 15 s.</div>
          )}
        </Card>
        <Card title="Relations over time">
          {history && history.samples.length > 0 ? (
            <Sparkline values={history.samples.map((s) => s.relations)} stroke="#7c3aed" />
          ) : (
            <div className="empty">No samples yet.</div>
          )}
        </Card>
      </div>

      <Card title="Recent files" subtitle="Latest uploads ingested into the graph">
        {files.length === 0 ? (
          <div className="empty">
            Nothing here yet. Head to <strong>Files</strong> to upload your first dataset.
          </div>
        ) : (
          <table className="table">
            <thead>
              <tr>
                <th>File</th>
                <th>Kind</th>
                <th>Size</th>
                <th>Ingested</th>
                <th>Uploaded</th>
              </tr>
            </thead>
            <tbody>
              {files.slice(0, 6).map((f) => (
                <tr key={f.id}>
                  <td>{f.name}</td>
                  <td><span className="badge">{f.kind}</span></td>
                  <td>{fmtBytes(f.size)}</td>
                  <td>
                    <span className="muted">
                      {f.concepts} c · {f.relations} r
                    </span>
                  </td>
                  <td className="muted">{fmtAgo(f.uploaded_at)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>
    </>
  );
}
