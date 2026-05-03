// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
// 
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

import { useRef, useState } from "react";
import { askStream, type ScoredConcept } from "./api";

interface Concept {
  id: number;
  concept_type: string;
  name: string;
  description?: string;
}

interface Subgraph {
  concepts: Concept[];
}

interface UsageInfo {
  input_tokens?: number;
  output_tokens?: number;
  cache_creation_input_tokens?: number;
  cache_read_input_tokens?: number;
}

const SAMPLE_QUESTIONS = [
  "Quels contrats Acme Labs a-t-elle signés en 2025 ?",
  "Quel est le montant total facturé à Initech ?",
  "Qui a signé le contrat C-2025-002 ?",
  "Quelle est la plus grosse ligne facturée et à quel contrat est-elle liée ?",
];

export function AskPanel() {
  const [query, setQuery] = useState("");
  const [streaming, setStreaming] = useState(false);
  const [answer, setAnswer] = useState("");
  const [retrieved, setRetrieved] = useState<ScoredConcept[]>([]);
  const [subgraph, setSubgraph] = useState<Subgraph | null>(null);
  const [usage, setUsage] = useState<UsageInfo | null>(null);
  const [model, setModel] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const abortRef = useRef<AbortController | null>(null);

  const ask = async (q: string) => {
    if (!q.trim() || streaming) return;
    setStreaming(true);
    setAnswer("");
    setRetrieved([]);
    setSubgraph(null);
    setUsage(null);
    setError(null);
    setModel(null);
    abortRef.current = new AbortController();
    try {
      await askStream(
        q,
        {
          onRetrieved: (p) => {
            setRetrieved(p.scored);
            setSubgraph(p.subgraph as Subgraph);
          },
          onToken: (t) => setAnswer((a) => a + t),
          onEnd: (p) => {
            setUsage(p.usage);
            if (p.model) setModel(p.model);
          },
          onError: (m) => setError(m),
        },
        { signal: abortRef.current.signal, top_k: 8, depth: 2 },
      );
    } catch (e) {
      if ((e as Error).name !== "AbortError") setError(String(e));
    } finally {
      setStreaming(false);
    }
  };

  const cancel = () => abortRef.current?.abort();

  const renderCitation = (s: ScoredConcept) => {
    const c = subgraph?.concepts.find((x) => x.id === s.id);
    const label = c ? `(${c.concept_type}) ${c.name}` : `#${s.id}`;
    return (
      <span key={s.id} className="cite" title={`score=${s.score.toFixed(3)}`}>
        {label}
      </span>
    );
  };

  return (
    <>
      <div className="card">
        <h2>Ask the knowledge graph</h2>
        <small>
          Your question is routed through the hybrid retrieval (TF-IDF +
          vector + subgraph expansion) to ground the LLM answer. Watch the
          tokens stream in as the model writes.
        </small>

        <div className="row" style={{ marginTop: 16 }}>
          <input
            type="text"
            className="grow"
            placeholder="Type a question and press Enter…"
            value={query}
            disabled={streaming}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") ask(query);
            }}
          />
          {streaming ? (
            <button className="secondary" onClick={cancel}>
              Stop
            </button>
          ) : (
            <button onClick={() => ask(query)} disabled={!query.trim()}>
              Ask
            </button>
          )}
        </div>

        <div style={{ marginTop: 12, display: "flex", flexWrap: "wrap", gap: 8 }}>
          {SAMPLE_QUESTIONS.map((q) => (
            <button
              key={q}
              className="secondary"
              onClick={() => {
                setQuery(q);
                ask(q);
              }}
              disabled={streaming}
              style={{ fontSize: 13 }}
            >
              {q}
            </button>
          ))}
        </div>
      </div>

      <div className="card">
        <h2>Answer</h2>
        <div className="answer">
          {answer ||
            (streaming
              ? "…"
              : error
                ? <span style={{ color: "#c0392b" }}>{error}</span>
                : <small>nothing yet — ask something above.</small>)}
        </div>

        {retrieved.length > 0 && (
          <>
            <h2>Citations</h2>
            <div className="citations">{retrieved.map(renderCitation)}</div>
          </>
        )}

        {(usage || model) && (
          <p className="usage">
            {model ? `model=${model} ` : ""}
            {usage
              ? `· input=${usage.input_tokens ?? 0} output=${usage.output_tokens ?? 0} cache_write=${usage.cache_creation_input_tokens ?? 0} cache_read=${usage.cache_read_input_tokens ?? 0}`
              : ""}
          </p>
        )}
      </div>
    </>
  );
}
