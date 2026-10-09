// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// Ready-made data models for the first day:
// schemas only, bundled from `examples/models` and the finance example.

import chantiers from "../../../examples/models/chantiers.json?raw";
import personnes from "../../../examples/models/personnes-organisations.json?raw";
import finance from "../../../examples/finance/ontology.json?raw";
import { replaceOntology, type Ontology } from "../api";
import { t } from "./i18n";

// `label` is a getter: read at render time, in the active language.
export const MODELS: { key: string; label: string; json: string }[] = [
  {
    key: "chantiers",
    get label() {
      return t("Chantiers et sous-traitants");
    },
    json: chantiers,
  },
  {
    key: "finance",
    get label() {
      return t("Contrats et factures");
    },
    json: finance,
  },
  {
    key: "personnes",
    get label() {
      return t("Personnes et organisations");
    },
    json: personnes,
  },
];

/** Install the schema of a model; the graph stays empty. */
export async function loadModel(key: string): Promise<void> {
  const m = MODELS.find((x) => x.key === key);
  if (!m) throw new Error(t("modèle inconnu : {key}", { key }));
  await replaceOntology(JSON.parse(m.json) as Ontology);
}
