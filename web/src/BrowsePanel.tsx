// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

import { useEffect, useMemo, useState } from "react";
import {
  getOntology,
  listConcepts,
  type Concept,
  type OntologySchema,
} from "./api";

const PAGE_SIZE = 100;

export function BrowsePanel() {
  const [ontology, setOntology] = useState<OntologySchema | null>(null);
  const [concepts, setConcepts] = useState<Concept[]>([]);
  const [total, setTotal] = useState(0);
  const [type, setType] = useState<string>("");
  const [q, setQ] = useState<string>("");
  const [offset, setOffset] = useState(0);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Load the schema once. It rarely changes and feeds the type filter.
  useEffect(() => {
    getOntology()
      .then(setOntology)
      .catch((e) => setError(String(e)));
  }, []);

  // Reload concepts whenever filters or pagination change.
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    listConcepts({
      type: type || undefined,
      q: q || undefined,
      limit: PAGE_SIZE,
      offset,
    })
      .then((r) => {
        if (cancelled) return;
        setConcepts(r.concepts);
        setTotal(r.total);
      })
      .catch((e) => !cancelled && setError(String(e)))
      .finally(() => !cancelled && setLoading(false));
    return () => {
      cancelled = true;
    };
  }, [type, q, offset]);

  const conceptTypes = useMemo(
    () =>
      ontology
        ? Object.values(ontology.concept_types).sort((a, b) =>
            a.name.localeCompare(b.name),
          )
        : [],
    [ontology],
  );

  // Group the current page by type so the listing is easier to scan.
  const grouped = useMemo(() => {
    const m = new Map<string, Concept[]>();
    for (const c of concepts) {
      const arr = m.get(c.concept_type) ?? [];
      arr.push(c);
      m.set(c.concept_type, arr);
    }
    return Array.from(m.entries()).sort(([a], [b]) => a.localeCompare(b));
  }, [concepts]);

  const pageEnd = Math.min(offset + concepts.length, total);
  const canPrev = offset > 0;
  const canNext = pageEnd < total;

  return (
    <>
      <div className="card">
        <h2>Ontology nodes</h2>
        <small>
          Browse every concept stored in the graph. Filter by type or search
          by name; results are paginated server-side.
        </small>

        <div className="row" style={{ marginTop: 16 }}>
          <select
            value={type}
            onChange={(e) => {
              setOffset(0);
              setType(e.target.value);
            }}
          >
            <option value="">All types ({ontology ? Object.keys(ontology.concept_types).length : 0})</option>
            {conceptTypes.map((ct) => (
              <option key={ct.name} value={ct.name}>
                {ct.name}
              </option>
            ))}
          </select>
          <input
            type="text"
            className="grow"
            placeholder="Search by name…"
            value={q}
            onChange={(e) => {
              setOffset(0);
              setQ(e.target.value);
            }}
          />
        </div>

        <p className="usage" style={{ marginTop: 12 }}>
          {loading
            ? "loading…"
            : error
              ? `error: ${error}`
              : total === 0
                ? "no concepts match"
                : `showing ${offset + 1}–${pageEnd} of ${total}`}
        </p>

        <div className="row" style={{ marginTop: 8 }}>
          <button
            className="secondary"
            disabled={!canPrev || loading}
            onClick={() => setOffset(Math.max(0, offset - PAGE_SIZE))}
          >
            ← Prev
          </button>
          <button
            className="secondary"
            disabled={!canNext || loading}
            onClick={() => setOffset(offset + PAGE_SIZE)}
          >
            Next →
          </button>
        </div>
      </div>

      {grouped.map(([t, items]) => (
        <div className="card" key={t}>
          <h2>
            {t} <small>({items.length})</small>
          </h2>
          <ul className="node-list">
            {items.map((c) => (
              <li key={c.id} className="node">
                <div className="node-head">
                  <span className="node-name">{c.name}</span>
                  <span className="node-id">#{c.id}</span>
                </div>
                {c.description && <div className="node-desc">{c.description}</div>}
                {c.properties && Object.keys(c.properties).length > 0 && (
                  <div className="node-props">
                    {Object.entries(c.properties).map(([k, v]) => (
                      <span className="prop" key={k}>
                        <b>{k}</b>: {formatValue(v)}
                      </span>
                    ))}
                  </div>
                )}
              </li>
            ))}
          </ul>
        </div>
      ))}

      {ontology && (
        <div className="card">
          <h2>Schema</h2>
          <small>
            {Object.keys(ontology.concept_types).length} concept types ·{" "}
            {Object.keys(ontology.relation_types).length} relation types
          </small>
          <h3 style={{ fontSize: 15, margin: "16px 0 6px" }}>Concept types</h3>
          <div className="citations">
            {conceptTypes.map((ct) => (
              <span
                key={ct.name}
                className="cite"
                title={ct.description || undefined}
              >
                {ct.name}
                {ct.parent ? ` ◂ ${ct.parent}` : ""}
              </span>
            ))}
          </div>
          <h3 style={{ fontSize: 15, margin: "16px 0 6px" }}>Relation types</h3>
          <div className="citations">
            {Object.values(ontology.relation_types)
              .sort((a, b) => a.name.localeCompare(b.name))
              .map((rt) => (
                <span
                  key={rt.name}
                  className="cite"
                  title={rt.description || undefined}
                >
                  {rt.domain} —{rt.name}{rt.symmetric ? "↔" : "→"} {rt.range}
                </span>
              ))}
          </div>
        </div>
      )}
    </>
  );
}

function formatValue(v: unknown): string {
  if (v == null) return "";
  if (typeof v === "object") return JSON.stringify(v);
  return String(v);
}
