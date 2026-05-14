// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { useEffect, useState } from "react";
import Card from "../components/Card";
import { apiBase, apiToken, getSettings, patchSettings, type Settings as ServerSettings } from "../api";

export default function Settings() {
  const [s, setS] = useState<ServerSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [base, setBase] = useState<string>(apiBase());
  const [token, setToken] = useState<string>(apiToken() ?? "");

  useEffect(() => {
    getSettings().then(setS).catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, []);

  const apply = async (patch: Parameters<typeof patchSettings>[0]) => {
    setError(null);
    setInfo(null);
    try {
      const next = await patchSettings(patch);
      setS(next);
      setInfo("Settings saved.");
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const saveConnection = () => {
    if (typeof window === "undefined") return;
    if (base.trim()) window.localStorage.setItem("ontology.apiBase", base.trim().replace(/\/$/, ""));
    else window.localStorage.removeItem("ontology.apiBase");
    if (token.trim()) window.localStorage.setItem("ontology.apiToken", token.trim());
    else window.localStorage.removeItem("ontology.apiToken");
    setInfo("Connection saved. Refresh other tabs to pick up the new base URL.");
  };

  return (
    <>
      <div className="page-header">
        <div>
          <h1 className="page-title">Settings</h1>
          <p className="page-subtitle">Retrieval defaults, UI preferences and server connection.</p>
        </div>
      </div>

      {error && <div className="error-banner">{error}</div>}
      {info && <div className="success-banner">{info}</div>}

      <div className="grid grid-2" style={{ alignItems: "start", marginBottom: 16 }}>
        <Card title="Retrieval defaults">
          {!s ? (
            <div className="empty">Loading…</div>
          ) : (
            <>
              <div className="setting-row">
                <div className="meta">
                  <strong>Top-K</strong>
                  Number of seed concepts retrieved per query.
                </div>
                <input
                  type="number"
                  min={1}
                  max={50}
                  value={s.retrieval.top_k}
                  onChange={(e) => apply({ retrieval: { top_k: Number(e.target.value) } })}
                />
              </div>
              <div className="setting-row">
                <div className="meta">
                  <strong>Lexical weight</strong>
                  0 = vector-only, 1 = BM25-only.
                </div>
                <input
                  type="number"
                  step={0.1}
                  min={0}
                  max={1}
                  value={s.retrieval.lexical_weight}
                  onChange={(e) => apply({ retrieval: { lexical_weight: Number(e.target.value) } })}
                />
              </div>
              <div className="setting-row">
                <div className="meta">
                  <strong>Expansion depth</strong>
                  Hops added to each seed when traversing.
                </div>
                <input
                  type="number"
                  min={0}
                  max={5}
                  value={s.retrieval.expansion_depth}
                  onChange={(e) => apply({ retrieval: { expansion_depth: Number(e.target.value) } })}
                />
              </div>
            </>
          )}
        </Card>

        <Card title="UI preferences">
          {!s ? (
            <div className="empty">Loading…</div>
          ) : (
            <>
              <div className="setting-row">
                <div className="meta">
                  <strong>Theme</strong>
                  Color scheme (light only at the moment).
                </div>
                <select
                  value={s.ui.theme}
                  onChange={(e) => apply({ ui: { theme: e.target.value } })}
                >
                  <option value="light">Light</option>
                  <option value="dark">Dark</option>
                </select>
              </div>
              <div className="setting-row">
                <div className="meta">
                  <strong>Graph layout</strong>
                  Layout engine for Graph View.
                </div>
                <select
                  value={s.ui.graph_layout}
                  onChange={(e) => apply({ ui: { graph_layout: e.target.value } })}
                >
                  <option value="dagre">Dagre (hierarchical)</option>
                  <option value="force">Force-directed</option>
                </select>
              </div>
            </>
          )}
        </Card>
      </div>

      <Card title="Server connection" subtitle="Stored in your browser only. Empty base URL ⇒ same-origin.">
        <div className="field">
          <label>API base URL</label>
          <input value={base} onChange={(e) => setBase(e.target.value)} placeholder="http://localhost:7373" />
        </div>
        <div className="field">
          <label>Bearer token (optional)</label>
          <input value={token} onChange={(e) => setToken(e.target.value)} placeholder="(leave blank if server is open)" />
        </div>
        <button className="btn-primary" onClick={saveConnection}>Save</button>
      </Card>

      <Card title="LLM provider" subtitle="Bound at server start (CLI flags) — read-only here." style={{ marginTop: 16 }}>
        <div className="muted">
          The provider and model are selected when launching <code>ontology serve</code>
          (e.g. <code>--anthropic --model claude-sonnet-4</code>). Restart the server to change them.
        </div>
      </Card>
    </>
  );
}
