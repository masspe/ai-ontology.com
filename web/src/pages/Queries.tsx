// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { useEffect, useState } from "react";
import { useSearchParams } from "react-router-dom";
import Card from "../components/Card";
import StreamingAnswer from "../components/StreamingAnswer";
import {
  createQuery,
  deleteQuery,
  getQueries,
  runQuery,
  type RagAnswer,
  type SavedQuery,
} from "../api";

export default function Queries() {
  const [params] = useSearchParams();
  const initialQ = params.get("q") ?? "";

  const [queries, setQueries] = useState<SavedQuery[]>([]);
  const [name, setName] = useState("");
  const [query, setQuery] = useState("");
  const [topK, setTopK] = useState(8);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lastResult, setLastResult] = useState<{ name: string; answer: RagAnswer } | null>(null);

  const refresh = async () => {
    try {
      const res = await getQueries();
      setQueries(res.queries);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  const save = async () => {
    if (!name.trim() || !query.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await createQuery({ name, query, top_k: topK });
      setName("");
      setQuery("");
      await refresh();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const run = async (q: SavedQuery) => {
    setBusy(true);
    setError(null);
    setLastResult(null);
    try {
      const ans = await runQuery(q.id);
      setLastResult({ name: q.name, answer: ans });
      await refresh();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const remove = async (id: number) => {
    if (!window.confirm("Delete this saved query?")) return;
    try {
      await deleteQuery(id);
      await refresh();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <>
      <div className="page-header">
        <div>
          <h1 className="page-title">Queries</h1>
          <p className="page-subtitle">Ask the graph or save reusable retrievals.</p>
        </div>
      </div>

      {error && <div className="error-banner">{error}</div>}

      <Card title="Ask now" subtitle="Streams the LLM response with grounding citations." style={{ marginBottom: 16 }}>
        <StreamingAnswer defaultQuery={initialQ} />
      </Card>

      <div className="grid grid-2" style={{ alignItems: "start" }}>
        <Card title="Save a query">
          <div className="field">
            <label>Name</label>
            <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Renewal obligations" />
          </div>
          <div className="field">
            <label>Query</label>
            <textarea value={query} onChange={(e) => setQuery(e.target.value)} rows={3} placeholder="What are the upcoming renewals…" />
          </div>
          <div className="field" style={{ maxWidth: 160 }}>
            <label>Top-K</label>
            <input type="number" min={1} max={50} value={topK} onChange={(e) => setTopK(Number(e.target.value))} />
          </div>
          <button className="btn-primary" onClick={save} disabled={busy || !name.trim() || !query.trim()}>
            Save
          </button>
        </Card>

        <Card title="Saved queries" actions={<button onClick={refresh}>Reload</button>}>
          {queries.length === 0 ? (
            <div className="empty">No saved queries yet.</div>
          ) : (
            <table className="table">
              <thead>
                <tr>
                  <th>Name</th>
                  <th>Top-K</th>
                  <th>Last run</th>
                  <th className="actions">Actions</th>
                </tr>
              </thead>
              <tbody>
                {queries.map((q) => (
                  <tr key={q.id}>
                    <td>
                      <strong>{q.name}</strong>
                      <div className="muted" style={{ fontSize: 11 }}>{q.query.slice(0, 60)}{q.query.length > 60 ? "…" : ""}</div>
                    </td>
                    <td>{q.top_k}</td>
                    <td className="muted">
                      {q.last_run_at ? new Date(q.last_run_at * 1000).toLocaleString() : "—"}
                    </td>
                    <td className="actions">
                      <button onClick={() => run(q)} disabled={busy}>Run</button>{" "}
                      <button className="btn-danger" onClick={() => remove(q.id)}>Delete</button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Card>
      </div>

      {lastResult && (
        <Card title={`Result · ${lastResult.name}`} style={{ marginTop: 16 }}>
          <div className="answer-box">{lastResult.answer.answer}</div>
          {lastResult.answer.subgraph && lastResult.answer.subgraph.concepts.length > 0 && (
            <div className="citations">
              {lastResult.answer.subgraph.concepts.slice(0, 12).map((c) => (
                <span key={c.id} className="badge badge-accent">{c.concept_type} · {c.name}</span>
              ))}
            </div>
          )}
        </Card>
      )}
    </>
  );
}
