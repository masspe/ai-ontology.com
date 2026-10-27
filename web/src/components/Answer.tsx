// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// An answer as a reader sees it. The model cites sheets by id (`#616`) and
// ends with a `Cited: [#…]` line; here every id that names a retrieved
// sheet becomes that sheet — type and name, linked — and the `Cited:` line
// becomes the row of sheets the answer rests on, cited ones first.

import { Fragment, type ReactNode } from "react";
import { Link } from "react-router-dom";
import type { Concept } from "../api";
import { t } from "../lib/i18n";

const MAX_SHEETS = 12;
const CITED_LINE = /(^|\n)\s*Cited\s*:/i;

/** The prose without the `Cited:` line, and the ids that line names. */
export function splitCited(text: string): { body: string; cited: number[] } {
  const at = text.search(CITED_LINE);
  if (at < 0) return { body: text, cited: [] };
  const cited = Array.from(text.slice(at).matchAll(/#(\d+)/g), (m) => Number(m[1]));
  return { body: text.slice(0, at).trimEnd(), cited };
}

function Sheet({ c, cited }: { c: Concept; cited?: boolean }) {
  return (
    <Link
      to={`/concepts/${c.id}`}
      className={`sheet-chip${cited ? " cited" : ""}`}
      title={cited ? t("Citée dans la réponse") : c.description || undefined}
    >
      <span className="sheet-type">{c.concept_type}</span>
      {" · "}
      <span className="sheet-name">{c.name}</span>
    </Link>
  );
}

interface Props {
  text: string;
  /** The sheets the answer was grounded on (the retrieved subgraph). */
  concepts?: Concept[];
  /** Ids the server already parsed from the `Cited:` line; else parsed here. */
  cited?: number[];
  /** Shown at the end of the prose while tokens are still arriving. */
  cursor?: boolean;
}

export default function Answer({ text, concepts = [], cited, cursor }: Props) {
  const byId = new Map(concepts.map((c) => [c.id, c]));
  const split = splitCited(text);
  const citedIds = Array.from(new Set((cited ?? split.cited).filter((id) => byId.has(id))));
  const citedSet = new Set(citedIds);

  // Inline `#id` → the sheet; ids the subgraph does not know stay as text.
  const parts: ReactNode[] = [];
  let last = 0;
  for (const m of split.body.matchAll(/#(\d+)/g)) {
    const c = byId.get(Number(m[1]));
    if (!c) continue;
    parts.push(split.body.slice(last, m.index), <Sheet key={`${m.index}`} c={c} cited={citedSet.has(c.id)} />);
    last = m.index! + m[0].length;
  }
  parts.push(split.body.slice(last));

  const rest = concepts.filter((c) => !citedSet.has(c.id)).slice(0, Math.max(0, MAX_SHEETS - citedIds.length));
  const sheets = [...citedIds.map((id) => byId.get(id)!), ...rest];

  return (
    <>
      <div className="answer-box">
        {parts.map((p, i) => (
          <Fragment key={i}>{p}</Fragment>
        ))}
        {cursor && <span style={{ opacity: 0.5 }}>▍</span>}
      </div>
      {sheets.length > 0 && (
        <div className="citations" aria-label={t("Fiches consultées")}>
          {sheets.map((c) => (
            <Sheet key={c.id} c={c} cited={citedSet.has(c.id)} />
          ))}
        </div>
      )}
    </>
  );
}
