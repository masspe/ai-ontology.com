// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// "Types du modèle": the concept types and relation types of the data model,
// listed one by one so the owner can view, edit, add and delete them. Every
// change rebuilds the whole ontology (immutable copy) and saves it with
// `replaceOntology`; the server stays the final judge and its refusal is shown.

import { useEffect, useState } from "react";
import Card from "./Card";
// @ts-expect-error JSX module
import { useConfirm } from "./ConfirmDialog.jsx";
import { listConcepts, replaceOntology, type ConceptTypeDef, type Ontology, type RelationTypeDef } from "../api";
import { t } from "../lib/i18n";

interface Props {
  ontology: Ontology | null;
  onChanged: () => unknown;
}

/** The server's `Cardinality` enum, spelled as in the ready-made models. */
const CARDINALITIES = ["ManyToMany", "ManyToOne", "OneToMany", "OneToOne"];
// ponytail: sheet counts are fetched for the first 100 types only; page them if models grow past that.
const COUNT_CAP = 100;

type Kind = "concept" | "relation";
interface Form {
  kind: Kind;
  /** `null` while adding a new type. */
  editing: string | null;
  name: string;
  description: string;
  parent: string;
  properties: string;
  domain: string;
  range: string;
  cardinality: string;
}

const propList = (p: ConceptTypeDef["properties"]): string[] =>
  Array.isArray(p) ? p : p ? Object.keys(p) : [];

const matches = (q: string, name: string, description?: string | null) =>
  !q || `${name} ${description ?? ""}`.toLowerCase().includes(q.toLowerCase());

export default function ModelTypes({ ontology, onChanged }: Props) {
  const confirm = useConfirm();
  const [tab, setTab] = useState<Kind>("concept");
  const [query, setQuery] = useState("");
  const [counts, setCounts] = useState<Record<string, number | null>>({});
  const [form, setForm] = useState<Form | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const ct = ontology?.concept_types ?? {};
  const rt = ontology?.relation_types ?? {};
  const typeNames = Object.keys(ct).sort((a, b) => a.localeCompare(b));
  const relNames = Object.keys(rt).sort((a, b) => a.localeCompare(b));
  const countKey = typeNames.join("\n");

  useEffect(() => {
    let live = true;
    const names = countKey ? countKey.split("\n") : [];
    // Beyond the cap the count is unknown (`null`), not loading.
    setCounts(Object.fromEntries(names.slice(COUNT_CAP).map((ty) => [ty, null])));
    names.slice(0, COUNT_CAP).forEach((type) =>
      listConcepts({ type, limit: 1, include_subtypes: false })
        .then((r) => r.total, () => null)
        .then((n) => live && setCounts((c) => ({ ...c, [type]: n }))),
    );
    return () => {
      live = false;
    };
  }, [countKey]);

  if (!ontology) return null;

  const save = async (next: Ontology) => {
    setError(null);
    setBusy(true);
    try {
      await replaceOntology(next);
      setForm(null);
      await onChanged();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  /** True when `ty` is `anc` or one of its descendants (a parent there would make a cycle). */
  const isUnder = (ty: string, anc: string) => {
    for (let p: string | null | undefined = ty, i = 0; p && i < 64; p = ct[p]?.parent, i++) if (p === anc) return true;
    return false;
  };

  const blockers = (ty: string): string[] => {
    const out: string[] = [];
    const n = counts[ty];
    if (n) out.push(t("{n} fiche(s) de ce type : supprimez-les ou changez leur type d'abord", { n }));
    const rels = relNames.filter((r) => rt[r].domain === ty || rt[r].range === ty);
    if (rels.length) out.push(t("utilisé par le(s) type(s) de lien : {list}", { list: rels.join(", ") }));
    const kids = typeNames.filter((c) => ct[c].parent === ty);
    if (kids.length) out.push(t("parent de : {list}", { list: kids.join(", ") }));
    const rules = Object.values(ontology?.rule_types ?? {}).filter((r) => r.applies_to?.includes(ty)).map((r) => r.name);
    if (rules.length) out.push(t("visé par le(s) type(s) de règle : {list}", { list: rules.join(", ") }));
    const acts = Object.values(ontology?.action_types ?? {}).filter((a) => a.subject === ty || a.object === ty).map((a) => a.name);
    if (acts.length) out.push(t("visé par le(s) type(s) d'action : {list}", { list: acts.join(", ") }));
    return out;
  };

  const openForm = (kind: Kind, name: string | null) => {
    setError(null);
    const c = name && kind === "concept" ? ct[name] : null;
    const r = name && kind === "relation" ? rt[name] : null;
    setForm({
      kind,
      editing: name,
      name: name ?? "",
      description: (c?.description ?? r?.description) || "",
      parent: c?.parent ?? "",
      properties: propList(c?.properties).join(", "),
      domain: r?.domain ?? typeNames[0] ?? "",
      range: r?.range ?? typeNames[0] ?? "",
      cardinality: r?.cardinality ?? "ManyToMany",
    });
  };

  const submit = () => {
    if (!form) return;
    const name = form.editing ?? form.name.trim();
    const pool = form.kind === "concept" ? ct : rt;
    if (!name) return setError(t("Le nom est obligatoire."));
    if (!form.editing && name in pool) return setError(t("Le nom « {name} » est déjà pris.", { name }));
    const description = form.description.trim();
    if (form.kind === "concept") {
      const properties = form.properties.split(",").map((s) => s.trim()).filter(Boolean);
      const def: ConceptTypeDef = {
        ...(ct[name] ?? {}),
        name,
        description,
        parent: form.parent || null,
        // An emptied list keeps a closed type closed ([]); only a type that
        // was open (null) stays open.
        properties: properties.length ? properties : ct[name]?.properties === null || ct[name]?.properties === undefined ? null : [],
      };
      save({ ...ontology, concept_types: { ...ct, [name]: def } });
    } else {
      const def: RelationTypeDef = {
        ...(rt[name] ?? {}),
        symmetric: rt[name]?.symmetric ?? false,
        name,
        domain: form.domain,
        range: form.range,
        cardinality: form.cardinality,
        description,
      };
      save({ ...ontology, relation_types: { ...rt, [name]: def } });
    }
  };

  const remove = async (kind: Kind, name: string) => {
    const ok = await confirm({
      title: kind === "concept" ? t("Supprimer ce type de fiche ?") : t("Supprimer ce type de lien ?"),
      message: t("« {name} » sera retiré du modèle de données.", { name }),
      confirmLabel: t("Supprimer"),
      danger: true,
    });
    if (!ok) return;
    if (kind === "relation") {
      const { [name]: _gone, ...rest } = rt;
      return save({ ...ontology, relation_types: rest });
    }
    const rest: Record<string, ConceptTypeDef> = {};
    for (const [k, v] of Object.entries(ct)) {
      if (k === name) continue;
      // A dropped type cannot stay listed as disjoint from another.
      rest[k] = v.disjoint_with?.includes(name) ? { ...v, disjoint_with: v.disjoint_with.filter((d) => d !== name) } : v;
    }
    save({ ...ontology, concept_types: rest });
  };

  const set = (patch: Partial<Form>) => setForm((f) => f && { ...f, ...patch });
  const typeOptions = (exclude?: (ty: string) => boolean) =>
    typeNames.filter((ty) => !exclude?.(ty)).map((ty) => <option key={ty} value={ty}>{ty}</option>);
  const field = { display: "flex", flexDirection: "column", gap: 4, fontSize: 12 } as const;

  const formView = form && (
    <div style={{ border: "1px solid var(--border, #e2e8f0)", borderRadius: 8, padding: 12, margin: "8px 0", display: "grid", gap: 8 }}>
      <strong style={{ fontSize: 13 }}>
        {form.editing
          ? form.kind === "concept"
            ? t("Modifier le type de fiche « {name} »", { name: form.editing })
            : t("Modifier le type de lien « {name} »", { name: form.editing })
          : form.kind === "concept" ? t("Nouveau type de fiche") : t("Nouveau type de lien")}
      </strong>
      <label style={field}>
        {t("Nom")}
        <input value={form.name} readOnly={!!form.editing} onChange={(e) => set({ name: e.target.value })} />
        {form.editing && (
          <span className="muted">{t("Le nom ne peut pas être changé : les fiches et les liens existants y sont rattachés.")}</span>
        )}
      </label>
      <label style={field}>
        {t("Description")}
        <input value={form.description} onChange={(e) => set({ description: e.target.value })} />
      </label>
      {form.kind === "concept" ? (
        <>
          <label style={field}>
            {t("Parent")}
            <select value={form.parent} onChange={(e) => set({ parent: e.target.value })}>
              <option value="">{t("(aucun)")}</option>
              {typeOptions((ty) => !!form.editing && isUnder(ty, form.editing))}
            </select>
          </label>
          <label style={field}>
            {t("Propriétés (séparées par des virgules)")}
            <input value={form.properties} onChange={(e) => set({ properties: e.target.value })} />
          </label>
        </>
      ) : (
        <>
          <label style={field}>
            {t("De (type de fiche)")}
            <select value={form.domain} onChange={(e) => set({ domain: e.target.value })}>{typeOptions()}</select>
          </label>
          <label style={field}>
            {t("Vers (type de fiche)")}
            <select value={form.range} onChange={(e) => set({ range: e.target.value })}>{typeOptions()}</select>
          </label>
          <label style={field}>
            {t("Cardinalité")}
            <select value={form.cardinality} onChange={(e) => set({ cardinality: e.target.value })}>
              {CARDINALITIES.map((c) => <option key={c} value={c}>{c}</option>)}
            </select>
          </label>
        </>
      )}
      <div style={{ display: "flex", gap: 8 }}>
        <button className="btn-primary" onClick={submit} disabled={busy}>{t("Enregistrer")}</button>
        <button className="btn-ghost" onClick={() => setForm(null)} disabled={busy}>{t("Annuler")}</button>
      </div>
    </div>
  );

  const actions = (kind: Kind, name: string, blocked: string[] = []) => (
    <td style={{ whiteSpace: "nowrap" }}>
      <button className="btn-ghost" onClick={() => openForm(kind, name)} disabled={busy}>{t("Modifier")}</button>{" "}
      <button
        className="btn-ghost"
        onClick={() => remove(kind, name)}
        disabled={busy || blocked.length > 0}
        title={blocked.length ? t("Suppression impossible : {list}", { list: blocked.join(" · ") }) : undefined}
      >
        {t("Supprimer")}
      </button>
    </td>
  );

  const shownTypes = typeNames.filter((ty) => matches(query, ty, ct[ty].description));
  const shownRels = relNames.filter((r) => matches(query, r, rt[r].description));
  const shown = tab === "concept" ? shownTypes : shownRels;

  return (
    <Card title={t("Types du modèle")} style={{ marginBottom: 16 }}>
      {error && <div className="error-banner" role="alert">{error}</div>}
      {typeNames.length + relNames.length === 0 && (
        <p className="muted">
          {t("Aucun type pour l'instant : installez un modèle prêt à l'emploi depuis l'accueil, ou générez-en un ci-dessous.")}
        </p>
      )}
      <div role="tablist" style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap", marginBottom: 8 }}>
        {(["concept", "relation"] as const).map((k) => (
          <button
            key={k}
            role="tab"
            aria-selected={tab === k}
            className={tab === k ? "btn-primary" : "btn-outline"}
            onClick={() => { setTab(k); setForm(null); }}
          >
            {k === "concept" ? t("Types de fiche ({n})", { n: typeNames.length }) : t("Types de lien ({n})", { n: relNames.length })}
          </button>
        ))}
        <input
          type="search"
          placeholder={t("Rechercher par nom ou description")}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          style={{ flex: 1, minWidth: 160 }}
        />
        <button
          className="btn-outline"
          onClick={() => openForm(tab, null)}
          disabled={busy || (tab === "relation" && typeNames.length === 0)}
        >
          {tab === "concept" ? t("Ajouter un type de fiche") : t("Ajouter un type de lien")}
        </button>
      </div>
      {formView}
      {shown.length === 0 ? (
        query && <p className="muted">{t("Aucun type ne correspond à « {query} ».", { query })}</p>
      ) : tab === "concept" ? (
        <table className="compact-table" style={{ width: "100%" }}>
          <thead>
            <tr><th>{t("Nom")}</th><th>{t("Description")}</th><th>{t("Parent")}</th><th>{t("Propriétés")}</th><th>{t("Fiches")}</th><th>{t("Actions")}</th></tr>
          </thead>
          <tbody>
            {shownTypes.map((ty) => {
              const props = propList(ct[ty].properties);
              const n = counts[ty];
              return (
                <tr key={ty}>
                  <td><strong>{ty}</strong></td>
                  <td className="muted">{ct[ty].description}</td>
                  <td>{ct[ty].parent ?? ""}</td>
                  <td title={props.join(", ")}>{props.length || ""}</td>
                  <td>{n === undefined ? "…" : n === null ? "?" : n}</td>
                  {actions("concept", ty, blockers(ty))}
                </tr>
              );
            })}
          </tbody>
        </table>
      ) : (
        <table className="compact-table" style={{ width: "100%" }}>
          <thead>
            <tr><th>{t("Nom")}</th><th>{t("De → Vers")}</th><th>{t("Cardinalité")}</th><th>{t("Description")}</th><th>{t("Actions")}</th></tr>
          </thead>
          <tbody>
            {shownRels.map((r) => (
              <tr key={r}>
                <td><strong>{r}</strong></td>
                <td>{rt[r].domain} → {rt[r].range}</td>
                <td><span className="badge">{rt[r].cardinality}</span></td>
                <td className="muted">{rt[r].description}</td>
                {actions("relation", r)}
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </Card>
  );
}
