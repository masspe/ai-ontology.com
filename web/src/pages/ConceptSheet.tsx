// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The sheet of one concept: on one screen,
// what it is, its links to other sheets, the documents it came from, the
// rules and actions that concern it, and the corrections, made in place.

import { t } from "../lib/i18n";
import { useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import Card from "../components/Card";
import {
  getConcept,
  getFiles,
  getSubgraph,
  listActions,
  listRules,
  updateConcept,
  type Action,
  type Concept,
  type FileRecord,
  type Relation,
  type Rule,
} from "../api";

function text(v: unknown): string {
  if (Array.isArray(v)) return v.map(text).join(", ");
  if (v === null || v === undefined) return "";
  return String(v);
}

/** Relations grouped by type, in first-seen order, each with the sheet at its other end. */
function groups<R extends { id: number; relation_type: string }>(
  rs: R[],
  other: (r: R) => number,
): Array<[string, Array<[R, number]>]> {
  const out = new Map<string, Array<[R, number]>>();
  for (const r of rs) {
    if (!out.has(r.relation_type)) out.set(r.relation_type, []);
    out.get(r.relation_type)!.push([r, other(r)]);
  }
  return Array.from(out.entries());
}

export default function ConceptSheet() {
  const raw = useParams().id ?? "";
  const id = Number(raw);
  const [concept, setConcept] = useState<Concept | null>(null);
  const [neighbours, setNeighbours] = useState<Map<number, Concept>>(new Map());
  const [relations, setRelations] = useState<Relation[]>([]);
  const [rules, setRules] = useState<Rule[]>([]);
  const [actions, setActions] = useState<Action[]>([]);
  const [files, setFiles] = useState<FileRecord[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState({ name: "", description: "" });

  useEffect(() => {
    let cancelled = false;
    setConcept(null);
    setEditing(false);
    setError(null);
    if (!Number.isInteger(id) || id < 0) {
      setError(t("Fiche inconnue : {raw}", { raw }));
      return;
    }
    (async () => {
      try {
        const [c, sg, r, a, f] = await Promise.all([
          getConcept(id),
          getSubgraph({ seed_concept_ids: [id], expansion_depth: 1, limit: 200 }),
          listRules().catch(() => [] as Rule[]),
          listActions().catch(() => [] as Action[]),
          getFiles().catch(() => ({ files: [] as FileRecord[] })),
        ]);
        if (cancelled) return;
        setConcept(c);
        setDraft({ name: c.name, description: c.description ?? "" });
        setNeighbours(new Map(sg.subgraph.concepts.map((n) => [n.id, n])));
        setRelations(sg.subgraph.relations.filter((rel) => rel.source === id || rel.target === id));
        setRules(r.filter((rule) => rule.applies_to.length === 0 || rule.applies_to.includes(id)));
        setActions(a.filter((act) => act.subject === id || act.object === id));
        setFiles(f.files);
        setError(null);
      } catch (e: unknown) {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [id]);

  const save = async () => {
    if (!concept) return;
    try {
      const updated = await updateConcept(concept.id, {
        name: draft.name.trim(),
        description: draft.description.trim(),
      });
      setConcept(updated);
      setEditing(false);
      setError(null);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  if (error && !concept) {
    return (
      <>
        <div className="error-banner">{error}</div>
        <p>
          <Link to="/concepts">{t("Retour aux fiches")}</Link>
        </p>
      </>
    );
  }
  if (!concept) return <p className="muted">{t("Chargement…")}</p>;

  const props = Object.entries(concept.properties ?? {}).filter(([k]) => k !== "source_file" && k !== "lang");
  const sourceFile = text(concept.properties?.source_file);
  const origin = files.filter((f) => f.name === sourceFile || (f.kind === "text" && f.name === concept.name));
  const outgoing = relations.filter((r) => r.source === id);
  const incoming = relations.filter((r) => r.target === id && r.source !== id);

  const link = (other: number) => {
    const n = neighbours.get(other);
    return n ? (
      <Link to={`/concepts/${other}`}>
        {n.name} <span className="muted">({n.concept_type})</span>
      </Link>
    ) : (
      <Link to={`/concepts/${other}`}>{t("fiche {n}", { n: other })}</Link>
    );
  };

  return (
    <>
      <div className="page-header">
        <div>
          <p className="muted sheet-crumb" style={{ margin: 0 }}>
            <Link to="/concepts">{t("Fiches")}</Link> › <span className="badge badge-accent">{concept.concept_type}</span>
          </p>
          {editing ? (
            <form
              className="sheet-edit"
              onSubmit={(e) => {
                e.preventDefault();
                void save();
              }}
            >
              <input
                aria-label={t("Nom")}
                value={draft.name}
                onChange={(e) => setDraft((d) => ({ ...d, name: e.target.value }))}
                required
              />
              <textarea
                aria-label={t("Description")}
                rows={3}
                value={draft.description}
                onChange={(e) => setDraft((d) => ({ ...d, description: e.target.value }))}
              />
              <div className="onboarding-actions">
                <button className="btn-primary" type="submit">
                  {t("Enregistrer")}
                </button>
                <button className="btn-outline" type="button" onClick={() => setEditing(false)}>
                  {t("Annuler")}
                </button>
              </div>
            </form>
          ) : (
            <>
              <h1 className="page-title sheet-name">{concept.name}</h1>
              <p className="page-subtitle sheet-desc">{concept.description || t("Pas de description.")}</p>
            </>
          )}
        </div>
        {!editing && (
          <div className="onboarding-actions">
            <button className="btn-outline" onClick={() => setEditing(true)}>
              {t("Corriger")}
            </button>
            <Link className="btn-outline" to={`/graph?focus=${concept.id}`}>
              {t("Voir dans le graphe")}
            </Link>
          </div>
        )}
      </div>
      {error && <div className="error-banner">{error}</div>}

      <div className="dash-row sheet-row">
        <Card title={t("Liens ({n})", { n: relations.length })}>
          {relations.length === 0 ? (
            <p className="muted">
              {t("Aucun lien pour l'instant. Les liens viennent des documents importés ou du")}{" "}
              <Link to="/graph">{t("graphe")}</Link>.
            </p>
          ) : (
            <div className="link-groups">
              {groups(outgoing, (r) => r.target).map(([type, rs]) => (
                <div key={`out-${type}`} className="link-group">
                  <h3 className="link-group-title">
                    <span className="badge">{type}</span> <span className="muted">({rs.length})</span>
                  </h3>
                  <ul className="sheet-list">
                    {rs.map(([r, other]) => (
                      <li key={r.id}>{link(other)}</li>
                    ))}
                  </ul>
                </div>
              ))}
              {groups(incoming, (r) => r.source).map(([type, rs]) => (
                <div key={`in-${type}`} className="link-group">
                  <h3 className="link-group-title">
                    <span className="badge">{type}</span> <span className="muted">{t("→ cette fiche")} ({rs.length})</span>
                  </h3>
                  <ul className="sheet-list">
                    {rs.map(([r, other]) => (
                      <li key={r.id}>{link(other)}</li>
                    ))}
                  </ul>
                </div>
              ))}
            </div>
          )}
        </Card>

        <Card title={t("Informations ({n})", { n: props.length })}>
          {props.length === 0 ? (
            <p className="muted">{t("Aucune information structurée.")}</p>
          ) : (
            <dl className="props">
              {props.map(([k, v]) => (
                <div key={k} className="prop">
                  <dt>{k}</dt>
                  <dd>{text(v)}</dd>
                </div>
              ))}
            </dl>
          )}
        </Card>
      </div>

      <div className="dash-row sheet-row">
        <Card title={t("Documents d'origine")}>
          {origin.length === 0 ? (
            <p className="muted">
              {sourceFile
                ? t("Importée depuis {file}.", { file: sourceFile })
                : t("Origine inconnue : fiche créée à la main ou importée avant le suivi des documents.")}
            </p>
          ) : (
            <ul className="sheet-list">
              {origin.map((f) => (
                <li key={f.id}>
                  <Link to="/files">{f.name}</Link>{" "}
                  <span className="muted">
                    {t("{n} fiche(s), {m} lien(s)", { n: f.concepts, m: f.relations })}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </Card>

        <Card title={t("Règles et actions ({n})", { n: rules.length + actions.length })}>
          {rules.length + actions.length === 0 ? (
            <p className="muted">
              {t("Aucune règle ni action ne concerne cette fiche.")} <Link to="/rules">{t("Définir une règle")}</Link>
            </p>
          ) : (
            <ul className="sheet-list">
              {rules.map((r) => (
                <li key={`r${r.id}`}>
                  <span className="badge">{r.applies_to.length === 0 ? t("règle générale") : t("règle")}</span>{" "}
                  <Link to="/rules">{r.name}</Link>
                  {r.strict && <span className="badge badge-warning"> {t("stricte")}</span>}
                </li>
              ))}
              {actions.map((a) => (
                <li key={`a${a.id}`}>
                  <span className="badge">{t("action")}</span> <Link to="/actions">{a.name}</Link>
                </li>
              ))}
            </ul>
          )}
        </Card>
      </div>
    </>
  );
}
