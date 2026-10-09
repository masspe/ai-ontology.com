// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.

import { useState } from "react";
import { askStream, type Subgraph } from "../api";
import { t } from "../lib/i18n";

const sheets = (n: number) => (n > 1 ? t("{n} fiches", { n }) : t("{n} fiche", { n }));
const links = (n: number) => (n > 1 ? t("{n} liens", { n }) : t("{n} lien", { n }));

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
          onToken: (tok) => setAnswer((prev) => prev + tok),
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
            placeholder={t("Posez votre question…")}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            rows={3}
          />
        </div>
      </div>
      <div className="row" style={{ marginBottom: 12 }}>
        <button className="btn-primary" onClick={run} disabled={streaming || !query.trim()}>
          {streaming ? t("Réponse en cours…") : t("Poser la question")}
        </button>
        <span className="muted" style={{ fontSize: 12 }}>
          {retrieved ? t("Réponse fondée sur {sheets} et {links}", { sheets: sheets(retrieved.concepts.length), links: links(retrieved.relations.length) }) : ""}
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
