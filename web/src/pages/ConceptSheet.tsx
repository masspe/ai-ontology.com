// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The sheet of one concept: on one screen,
// what it is, its links to other sheets, the documents it came from, the
// rules and actions that concern it, and the corrections, made in place.

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
      setError(`Fiche inconnue : ${raw}`);
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
          <Link to="/concepts">Retour aux fiches</Link>
        </p>
      </>
    );
  }
  if (!concept) return <p className="muted">Chargement…</p>;

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
      <Link to={`/concepts/${other}`}>fiche {other}</Link>
    );
  };

  return (
    <>
      <div className="page-header">
        <div>
          <p className="muted" style={{ margin: 0 }}>
            <Link to="/concepts">Fiches</Link> › {concept.concept_type}
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
                aria-label="Nom"
                value={draft.name}
                onChange={(e) => setDraft((d) => ({ ...d, name: e.target.value }))}
                required
              />
              <textarea
                aria-label="Description"
                rows={3}
                value={draft.description}
                onChange={(e) => setDraft((d) => ({ ...d, description: e.target.value }))}
              />
              <div className="onboarding-actions">
                <button className="btn-primary" type="submit">
                  Enregistrer
                </button>
                <button className="btn-outline" type="button" onClick={() => setEditing(false)}>
                  Annuler
                </button>
              </div>
            </form>
          ) : (
            <>
              <h1 className="page-title">{concept.name}</h1>
              <p className="page-subtitle">{concept.description || "Pas de description."}</p>
            </>
          )}
        </div>
        {!editing && (
          <div className="onboarding-actions">
            <button className="btn-outline" onClick={() => setEditing(true)}>
              Corriger
            </button>
            <Link className="btn-outline" to={`/graph?focus=${concept.id}`}>
              Voir dans le graphe
            </Link>
          </div>
        )}
      </div>
      {error && <div className="error-banner">{error}</div>}

      <div className="dash-row sheet-row">
        <Card title={`Liens (${relations.length})`}>
          {relations.length === 0 ? (
            <p className="muted">
              Aucun lien pour l'instant. Les liens viennent des documents importés ou du{" "}
              <Link to="/graph">graphe</Link>.
            </p>
          ) : (
            <ul className="sheet-list">
              {outgoing.map((r) => (
                <li key={r.id}>
                  <span className="badge">{r.relation_type}</span> {link(r.target)}
                </li>
              ))}
              {incoming.map((r) => (
                <li key={r.id}>
                  {link(r.source)} <span className="badge">{r.relation_type}</span> <span className="muted">→ cette fiche</span>
                </li>
              ))}
            </ul>
          )}
        </Card>

        <Card title={`Informations (${props.length})`}>
          {props.length === 0 ? (
            <p className="muted">Aucune information structurée.</p>
          ) : (
            <table className="compact-table">
              <tbody>
                {props.map(([k, v]) => (
                  <tr key={k}>
                    <td className="muted">{k}</td>
                    <td>{text(v)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Card>
      </div>

      <div className="dash-row sheet-row">
        <Card title="Documents d'origine">
          {origin.length === 0 ? (
            <p className="muted">
              {sourceFile
                ? `Importée depuis ${sourceFile}.`
                : "Origine inconnue : fiche créée à la main ou importée avant le suivi des documents."}
            </p>
          ) : (
            <ul className="sheet-list">
              {origin.map((f) => (
                <li key={f.id}>
                  <Link to="/files">{f.name}</Link>{" "}
                  <span className="muted">
                    {f.concepts} fiche(s), {f.relations} lien(s)
                  </span>
                </li>
              ))}
            </ul>
          )}
        </Card>

        <Card title={`Règles et actions (${rules.length + actions.length})`}>
          {rules.length + actions.length === 0 ? (
            <p className="muted">
              Aucune règle ni action ne concerne cette fiche. <Link to="/rules">Définir une règle</Link>
            </p>
          ) : (
            <ul className="sheet-list">
              {rules.map((r) => (
                <li key={`r${r.id}`}>
                  <span className="badge">{r.applies_to.length === 0 ? "règle générale" : "règle"}</span>{" "}
                  <Link to="/rules">{r.name}</Link>
                  {r.strict && <span className="badge badge-warning"> stricte</span>}
                </li>
              ))}
              {actions.map((a) => (
                <li key={`a${a.id}`}>
                  <span className="badge">action</span> <Link to="/actions">{a.name}</Link>
                </li>
              ))}
            </ul>
          )}
        </Card>
      </div>
    </>
  );
}
