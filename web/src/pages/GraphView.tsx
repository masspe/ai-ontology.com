// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { useEffect, useState } from "react";
import Card from "../components/Card";
import GraphCanvas from "../components/GraphCanvas";
import { getOntology, getSubgraph, type Ontology, type Subgraph } from "../api";

export default function GraphView() {
  const [ontology, setOntology] = useState<Ontology | null>(null);
  const [subgraph, setSubgraph] = useState<Subgraph | null>(null);
  const [query, setQuery] = useState("");
  const [type, setType] = useState("");
  const [depth, setDepth] = useState(1);
  const [limit, setLimit] = useState(150);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = async () => {
    setBusy(true);
    setError(null);
    try {
      const res = await getSubgraph({
        seed_query: query.trim() || undefined,
        seed_concept_types: type ? [type] : [],
        expansion_depth: depth,
        limit,
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
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const conceptTypes = ontology ? Object.keys(ontology.concept_types) : [];

  return (
    <>
      <div className="page-header">
        <div>
          <h1 className="page-title">Graph View</h1>
          <p className="page-subtitle">Visualize a bounded slice of the knowledge graph.</p>
        </div>
      </div>

      {error && <div className="error-banner">{error}</div>}

      <Card title="Filters" style={{ marginBottom: 16 }}>
        <div className="field-row">
          <div className="field" style={{ flex: 2 }}>
            <label>Seed by query (optional)</label>
            <input
              placeholder="e.g. renewal clauses"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && load()}
            />
          </div>
          <div className="field">
            <label>Concept type</label>
            <select value={type} onChange={(e) => setType(e.target.value)}>
              <option value="">All</option>
              {conceptTypes.map((t) => <option key={t}>{t}</option>)}
            </select>
          </div>
          <div className="field" style={{ maxWidth: 120 }}>
            <label>Depth</label>
            <input type="number" min={0} max={5} value={depth} onChange={(e) => setDepth(Number(e.target.value))} />
          </div>
          <div className="field" style={{ maxWidth: 120 }}>
            <label>Limit</label>
            <input type="number" min={1} max={2000} value={limit} onChange={(e) => setLimit(Number(e.target.value))} />
          </div>
          <div className="field" style={{ display: "flex", alignItems: "flex-end" }}>
            <button className="btn-primary" onClick={load} disabled={busy}>
              {busy ? "Loading…" : "Refresh"}
            </button>
          </div>
        </div>
        <div className="muted" style={{ fontSize: 12 }}>
          {subgraph
            ? `${subgraph.concepts.length} concepts · ${subgraph.relations.length} relations`
            : ""}
        </div>
      </Card>

      <div className="graph-canvas">
        <GraphCanvas subgraph={subgraph} />
      </div>
    </>
  );
}
