// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// "Domaines sur site": which storage domains of the data model must not
// leave the premises, and the provider that answers for them. A question
// whose context holds a sheet of such a domain goes to that provider only;
// without one it is refused. Saved in the settings as soon as it changes.

import { useEffect, useState } from "react";
import Card from "./Card";
import { getSettings, patchSettings, type ConceptTypeDef, type Ontology } from "../api";
import { t } from "../lib/i18n";

const PROVIDERS = ["openai", "anthropic", "infomaniak"];

/** How many types each domain holds: a type's own `ns`, else the nearest ancestor's, else `default`. */
export function domainsOf(ontology: Ontology): Record<string, number> {
  const ct = ontology.concept_types;
  const out: Record<string, number> = {};
  for (const name of Object.keys(ct)) {
    let ns = "default";
    let p: ConceptTypeDef | undefined = ct[name];
    // 64 levels: a cycle of parents ends here.
    for (let i = 0; p && i < 64; i++) {
      if (p.ns) {
        ns = p.ns;
        break;
      }
      p = p.parent ? ct[p.parent] : undefined;
    }
    out[ns] = (out[ns] ?? 0) + 1;
  }
  return out;
}

export default function OnPremisesDomains({ ontology }: { ontology: Ontology | null }) {
  const [kept, setKept] = useState<string[] | null>(null);
  const [provider, setProvider] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    getSettings().then(
      (s) => {
        setKept(s.llm.on_premises_domains ?? []);
        setProvider(s.llm.on_premises_provider ?? "");
      },
      () => setKept(null),
    );
  }, []);

  if (!ontology || kept === null) return null;
  const counts = domainsOf(ontology);
  // A domain kept on the premises stays listed after its last type is gone, so it can be released.
  const domains = [...new Set([...Object.keys(counts), ...kept])].sort((a, b) => a.localeCompare(b));
  if (domains.length === 0) return null;

  const save = async (llm: { on_premises_domains?: string[]; on_premises_provider?: string }) => {
    try {
      const s = await patchSettings({ llm });
      setKept(s.llm.on_premises_domains ?? []);
      setProvider(s.llm.on_premises_provider ?? "");
      setError(null);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <Card
      title={t("Domaines sur site")}
      subtitle={t("Une question dont le contexte contient une fiche d'un domaine coché n'est envoyée qu'au fournisseur sur site.")}
      style={{ marginBottom: 16 }}
    >
      {error && <div className="error-banner" role="alert">{error}</div>}
      <div style={{ display: "flex", gap: 16, flexWrap: "wrap", marginBottom: 12 }}>
        {domains.map((d) => (
          <label key={d} style={{ display: "flex", gap: 6, alignItems: "center" }}>
            <input
              type="checkbox"
              checked={kept.includes(d)}
              onChange={(e) => void save({ on_premises_domains: e.target.checked ? [...kept, d] : kept.filter((x) => x !== d) })}
            />
            <code>{d}</code>
            <span className="muted">{t("{n} type(s)", { n: counts[d] ?? 0 })}</span>
          </label>
        ))}
      </div>
      <label className="field" style={{ maxWidth: 320 }}>
        <span>{t("Fournisseur sur site")}</span>
        <select value={provider} onChange={(e) => void save({ on_premises_provider: e.target.value })}>
          <option value="">{t("aucun")}</option>
          {PROVIDERS.map((p) => (
            <option key={p} value={p}>
              {p}
            </option>
          ))}
        </select>
      </label>
      {kept.length > 0 && !provider && (
        <div className="warn-banner" role="status">
          {t("Aucun fournisseur sur site : les questions qui touchent ces domaines sont refusées.")}
        </div>
      )}
    </Card>
  );
}
