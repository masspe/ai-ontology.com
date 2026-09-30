// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.

import { useEffect, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { pinnedIds, togglePin } from "../lib/pins";
import Card from "../components/Card";
import StreamingAnswer from "../components/StreamingAnswer";
// @ts-expect-error JSX module
import { useConfirm } from "../components/ConfirmDialog.jsx";
import {
  createQuery,
  deleteQuery,
  getQueries,
  runQuery,
  type RagAnswer,
  type SavedQuery,
} from "../api";

export default function Queries() {
  const confirm = useConfirm();
  const [params] = useSearchParams();
  const initialQ = params.get("q") ?? "";
  const runId = params.get("run");
  const [pinned, setPinned] = useState<number[]>(() => pinnedIds());

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

  // `/queries?run=<id>` (a pinned question on the home page): replay it.
  const replayed = useRef<string | null>(null);
  useEffect(() => {
    if (!runId || replayed.current === runId) return;
    const q = queries.find((x) => String(x.id) === runId);
    if (!q) return;
    replayed.current = runId;
    void run(q);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runId, queries]);

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
    if (!(await confirm({ title: "Supprimer la question", message: "Supprimer cette question enregistrée ?", confirmLabel: "Supprimer", cancelLabel: "Annuler", danger: true }))) return;
    try {
      await deleteQuery(id);
      if (pinned.includes(id)) setPinned(togglePin(id));
      await refresh();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <>
      <div className="page-header">
        <div>
          <h1 className="page-title">Questions</h1>
          <p className="page-subtitle">Posez une question à vos données, gardez celles qui servent, épinglez-les à l'accueil.</p>
        </div>
      </div>

      {error && <div className="error-banner">{error}</div>}

      <Card title="Poser une question" subtitle="La réponse arrive au fil de l'eau et cite les fiches sur lesquelles elle s'appuie." style={{ marginBottom: 16 }}>
        <StreamingAnswer defaultQuery={initialQ} />
      </Card>

      <div className="grid grid-2" style={{ alignItems: "start" }}>
        <Card title="Enregistrer une question">
          <div className="field">
            <label>Nom</label>
            <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Échéances de renouvellement" />
          </div>
          <div className="field">
            <label>Question</label>
            <textarea value={query} onChange={(e) => setQuery(e.target.value)} rows={3} placeholder="Quels sont les prochains renouvellements…" />
          </div>
          <div className="field" style={{ maxWidth: 160 }}>
            <label>Fiches consultées (Top-K)</label>
            <input type="number" min={1} max={50} value={topK} onChange={(e) => setTopK(Number(e.target.value))} />
          </div>
          <button className="btn-primary" onClick={save} disabled={busy || !name.trim() || !query.trim()}>
            Enregistrer
          </button>
        </Card>

        <Card title="Questions enregistrées" actions={<button onClick={refresh}>Recharger</button>}>
          {queries.length === 0 ? (
            <div className="empty">Aucune question enregistrée. Posez-en une ci-dessus : la réponse cite les fiches d'où elle vient, et vous pourrez la garder.</div>
          ) : (
            <table className="table">
              <thead>
                <tr>
                  <th>Nom</th>
                  <th>Fiches consultées</th>
                  <th>Dernière exécution</th>
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
                      <button onClick={() => run(q)} disabled={busy}>Exécuter</button>{" "}
                      <button
                        onClick={() => setPinned(togglePin(q.id))}
                        aria-pressed={pinned.includes(q.id)}
                        title={pinned.includes(q.id) ? "Retirer de l'accueil" : "Épingler à l'accueil"}
                      >
                        {pinned.includes(q.id) ? "★" : "☆"}
                      </button>{" "}
                      <button className="btn-danger" onClick={() => remove(q.id)}>Supprimer</button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Card>
      </div>

      {lastResult && (
        <Card title={`Réponse · ${lastResult.name}`} style={{ marginTop: 16 }}>
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
