// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// Ready-made data models for the first day:
// schemas only, bundled from `examples/models` and the finance example.

import chantiers from "../../../examples/models/chantiers.json?raw";
import personnes from "../../../examples/models/personnes-organisations.json?raw";
import finance from "../../../examples/finance/ontology.json?raw";
import { replaceOntology, type Ontology } from "../api";

export const MODELS: { key: string; label: string; json: string }[] = [
  { key: "chantiers", label: "Chantiers et sous-traitants", json: chantiers },
  { key: "finance", label: "Contrats et factures", json: finance },
  { key: "personnes", label: "Personnes et organisations", json: personnes },
];

/** Install the schema of a model; the graph stays empty. */
export async function loadModel(key: string): Promise<void> {
  const m = MODELS.find((x) => x.key === key);
  if (!m) throw new Error(`modèle inconnu : ${key}`);
  await replaceOntology(JSON.parse(m.json) as Ontology);
}
