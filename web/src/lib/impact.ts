// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// What a change of the data model touches (ROADMAP §3.9 lot B, point 7):
// the concept types the new model drops, with the number of sheets that
// still carry each of them, exactly (no subtypes: the server's refusal
// counts that way). The server refuses to drop a type in use, so the
// figure is shown before the save, with what to do, instead of a bare
// refusal.

import { listConcepts, type Ontology } from "../api";

export interface ModelImpact {
  removed: { type: string; count: number }[];
  /** Sheets whose type disappears from the model. */
  total: number;
}

export async function modelImpact(current: Ontology | null, proposed: Ontology): Promise<ModelImpact> {
  const gone = Object.keys(current?.concept_types ?? {}).filter((t) => !(t in proposed.concept_types));
  const removed = await Promise.all(
    gone.map(async (type) => {
      const page = await listConcepts({ type, limit: 1, include_subtypes: false });
      return { type, count: page.total ?? page.concepts.length };
    }),
  );
  return { removed, total: removed.reduce((n, r) => n + r.count, 0) };
}

/** « 3 412 fiches concernées : Contract (3 000), Invoice (412). » */
export function wordImpact(impact: ModelImpact): string {
  if (impact.total === 0) return "Rien à migrer : aucune fiche ne perd son type.";
  const parts = impact.removed
    .filter((r) => r.count > 0)
    .map((r) => `${r.type} (${r.count.toLocaleString("fr-CH")})`)
    .join(", ");
  return `${impact.total.toLocaleString("fr-CH")} fiche(s) concernée(s) : ${parts}. Un type encore utilisé ne peut pas être retiré : réaffectez ou supprimez ces fiches d'abord.`;
}
