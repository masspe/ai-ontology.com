// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { useState } from "react";
import { askStream, type Subgraph } from "../api";

interface Props {
  defaultQuery?: string;
}

export default function StreamingAnswer({ defaultQuery = "" }: Props) {
  const [query, setQuery] = useState(defaultQuery);
  const [answer, setAnswer] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [streaming, setStreaming] = useState(false);
  const [retrieved, setRetrieved] = useState<Subgraph | null>(null);

  const run = async () => {
    if (!query.trim()) return;
    setAnswer("");
    setError(null);
    setRetrieved(null);
    setStreaming(true);
    try {
      await askStream(
        { query },
        {
          onRetrieved: setRetrieved,
          onToken: (t) => setAnswer((prev) => prev + t),
          onEnd: () => setStreaming(false),
          onError: (e) => {
            setError(e);
            setStreaming(false);
          },
        },
      );
      setStreaming(false);
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(msg);
      setStreaming(false);
    }
  };

  return (
    <div>
      <div className="field-row" style={{ marginBottom: 12 }}>
        <div className="field" style={{ flex: 1 }}>
          <textarea
            placeholder="Ask the ontology…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            rows={3}
          />
        </div>
      </div>
      <div className="row" style={{ marginBottom: 12 }}>
        <button className="btn-primary" onClick={run} disabled={streaming || !query.trim()}>
          {streaming ? "Streaming…" : "Ask"}
        </button>
        <span className="muted" style={{ fontSize: 12 }}>
          {retrieved ? `Grounded on ${retrieved.concepts.length} concepts, ${retrieved.relations.length} relations` : ""}
        </span>
      </div>
      {error && <div className="error-banner">{error}</div>}
      {(answer || streaming) && (
        <div className="answer-box">
          {answer}
          {streaming && <span style={{ opacity: 0.5 }}>▍</span>}
        </div>
      )}
      {retrieved && retrieved.concepts.length > 0 && (
        <div className="citations">
          {retrieved.concepts.slice(0, 12).map((c) => (
            <span key={c.id} className="badge badge-accent">
              {c.concept_type} · {c.name}
            </span>
          ))}
        </div>
      )}
    </div>
  );
}
