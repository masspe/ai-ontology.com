// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.

import { t } from "../lib/i18n";
import { useEffect, useState } from "react";
import JSZip from "jszip";
import Card from "../components/Card";
import Dropzone from "../components/Dropzone";
import OntologyGraph from "../components/OntologyGraph";
import ModelTypes from "../components/ModelTypes";
// @ts-expect-error JSX module
import { useConfirm } from "../components/ConfirmDialog.jsx";
import { modelImpact, wordImpact } from "../lib/impact";
import {
  ApplyReportView,
  ReviewPanel,
  defaultDecisionFor,
} from "../components/IngestReview";
import { analyzeIngest, applyIngest } from "../lib/ingestApi";
import { mergeProposals, type ProposalPart } from "../lib/mergeProposals";
import {
  needsPreprocessing,
  prepareForIngest,
  terminateOcrWorker,
} from "../lib/extractText";
import type {
  ApplyDecision,
  ApplyReport,
  DecisionAction,
  OntologyProposal,
} from "../lib/proposalTypes";
import { iterRefs } from "../lib/proposalTypes";
import {
  deleteFile,
  exportGraphUrl,
  generateOntology,
  getFiles,
  getOntology,
  getStats,
  getSubgraph,
  replaceOntology,
  type FileRecord,
  type Ontology,
  type Stats,
  type Subgraph,
} from "../api";

const MAX_DESC = 4000;

const examples = (): string[] => [
  t("Un suivi des contrats : parties, accords, clauses, obligations et dates de renouvellement."),
  t("Une base de connaissances de recherche : publications, auteurs, institutions, sujets et citations."),
  t("Un modèle clinique couvrant patients, diagnostics, médicaments, interventions et résultats."),
];

const KIND_BY_EXT: Record<string, string> = {
  json: "ontology",
  jsonl: "jsonl",
  ndjson: "jsonl",
  triples: "triples",
  csv: "csv",
  xlsx: "xlsx",
  txt: "text",
  md: "text",
  pdf: "text",
  docx: "text",
  doc: "text",
  rtf: "text",
  html: "text",
  htm: "text",
  xml: "text",
  yaml: "text",
  yml: "text",
  // Images (OCR'd client-side before /ingest/analyze).
  png: "image",
  jpg: "image",
  jpeg: "image",
  webp: "image",
  bmp: "image",
  gif: "image",
  tif: "image",
  tiff: "image",
};

const SKIP_IN_ZIP = (name: string): boolean => {
  const base = name.split("/").pop() ?? name;
  if (!base || base.startsWith(".")) return true;
  if (name.includes("__MACOSX/")) return true;
  if (base === "Thumbs.db" || base === ".DS_Store") return true;
  return false;
};

const ICON_BY_KIND: Record<string, { label: string; bg: string; fg: string }> = {
  ontology: { label: "{ }", bg: "#fef3c7", fg: "#d97706" },
  jsonl: { label: "{ }", bg: "#fef3c7", fg: "#d97706" },
  triples: { label: "△", bg: "#e0e7ff", fg: "#4f46e5" },
  csv: { label: "CSV", bg: "#dcfce7", fg: "#16a34a" },
  xlsx: { label: "XLS", bg: "#dcfce7", fg: "#16a34a" },
  text: { label: "TXT", bg: "#ede9fe", fg: "#7c3aed" },
  image: { label: "IMG", bg: "#fce7f3", fg: "#be185d" },
};

function iconForFile(f: FileRecord): { label: string; bg: string; fg: string } {
  const ext = (f.name.split(".").pop() ?? "").toLowerCase();
  if (ext === "pdf") return { label: "PDF", bg: "#fee2e2", fg: "#dc2626" };
  if (ext === "docx" || ext === "doc") return { label: "W", bg: "#dbeafe", fg: "#2563eb" };
  return ICON_BY_KIND[f.kind] ?? { label: f.kind.slice(0, 3).toUpperCase(), bg: "#e5e7eb", fg: "#374151" };
}

function fmtBytes(b: number): string {
  if (b < 1024) return `${b} B`;
  if (b < 1024 * 1024) return `${(b / 1024).toFixed(1)} KB`;
  return `${(b / (1024 * 1024)).toFixed(1)} MB`;
}

function fmtAgo(ts: number): string {
  const diff = Math.max(0, Math.floor(Date.now() / 1000 - ts));
  if (diff < 60) return t("il y a {n} s", { n: diff });
  if (diff < 3600) return t("il y a {n} min", { n: Math.floor(diff / 60) });
  if (diff < 86400) return t("il y a {n} h", { n: Math.floor(diff / 3600) });
  return t("il y a {n} j", { n: Math.floor(diff / 86400) });
}

function buildFilesContext(files: FileRecord[]): string {
  if (files.length === 0) return "";
  const lines = files.slice(0, 12).map((f) => {
    const status = f.concepts > 0 || f.relations > 0 ? "ingested" : "uploaded";
    const tag = f.concept_type ? ` as ${f.concept_type}` : "";
    return `- ${f.name} (${f.kind}, ${status}${tag}): ${f.concepts} concepts, ${f.relations} relations`;
  });
  return [
    "",
    "## Source documents already ingested into the graph",
    ...lines,
    "Use the entities, relations and terminology from these documents when proposing the ontology.",
  ].join("\n");
}

export default function OntologyBuilder() {
  const confirm = useConfirm();
  const [description, setDescription] = useState("");
  const [showExamples, setShowExamples] = useState(false);

  const [ontology, setOntology] = useState<Ontology | null>(null);
  const [proposed, setProposed] = useState<Ontology | null>(null);
  const [stats, setStats] = useState<Stats | null>(null);
  const [subgraph, setSubgraph] = useState<Subgraph | null>(null);
  const [files, setFiles] = useState<FileRecord[]>([]);

  const [busy, setBusy] = useState<"idle" | "generate" | "upload" | "save">("idle");
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [lastUpdated, setLastUpdated] = useState<number | null>(null);

  // ---- ingest-based concept preview ----
  const [ingestProposal, setIngestProposal] = useState<OntologyProposal | null>(null);
  const [ingestDecisions, setIngestDecisions] = useState<Record<string, DecisionAction>>({});
  const [applyBusy, setApplyBusy] = useState(false);
  const [applyReport, setApplyReport] = useState<ApplyReport | null>(null);

  const refresh = async () => {
    try {
      const [o, s, f, sg] = await Promise.all([
        getOntology(),
        getStats(),
        getFiles(),
        getSubgraph({ limit: 150, expansion_depth: 1 }),
      ]);
      setOntology(o);
      setStats(s);
      setFiles(f.files);
      setSubgraph(sg.subgraph);
      setLastUpdated(Math.floor(Date.now() / 1000));
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  // Release the tesseract.js WASM worker (and its language data) when the
  // builder page unmounts, so navigating away frees ~10 MB of memory.
  useEffect(() => {
    return () => {
      void terminateOcrWorker();
    };
  }, []);

  const handleGenerate = async () => {
    if (!description.trim() && files.length === 0) return;
    setBusy("generate");
    setError(null);
    setInfo(null);
    setProposed(null);
    try {
      const prompt = description.trim() + buildFilesContext(files);
      const res = await generateOntology(prompt);
      setProposed(res.ontology);
      setInfo(
        files.length > 0
          ? t("Brouillon généré à partir de votre description et de {n} fichier(s). Cliquez sur Enregistrer le modèle pour l'appliquer.", { n: files.length })
          : t("Brouillon du modèle généré. Cliquez sur Enregistrer le modèle pour l'appliquer."),
      );
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy("idle");
    }
  };

  const handleSave = async () => {
    if (!proposed) return;
    setBusy("save");
    setError(null);
    try {
      // What the change touches, said before the server refuses it (lot
      // B.7). Unknown (request failed): the server still decides.
      const impact = await modelImpact(ontology, proposed).catch(() => null);
      if (impact && impact.total > 0) {
        setError(wordImpact(impact));
        return;
      }
      await replaceOntology(proposed);
      setProposed(null);
      setInfo(`${t("Modèle enregistré.")} ${impact ? wordImpact(impact) : ""}`.trim());
      await refresh();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy("idle");
    }
  };

  const extractZip = async (file: File): Promise<File[]> => {
    const zip = await JSZip.loadAsync(await file.arrayBuffer());
    const out: File[] = [];
    const entries = Object.values(zip.files);
    for (const entry of entries) {
      if (entry.dir) continue;
      if (SKIP_IN_ZIP(entry.name)) continue;
      const blob = await entry.async("blob");
      const name = entry.name.split("/").pop() || entry.name;
      out.push(new File([blob], name, { type: blob.type }));
    }
    return out;
  };

  const expandZips = async (input: File[]): Promise<File[]> => {
    const queue = [...input];
    const flat: File[] = [];
    while (queue.length) {
      const f = queue.shift()!;
      const fext = (f.name.split(".").pop() ?? "").toLowerCase();
      if (fext === "zip") {
        queue.push(...(await extractZip(f)));
      } else {
        flat.push(f);
      }
    }
    return flat;
  };

  /** Run `/ingest/analyze` on each file (sequentially), merge the proposals
   * and seed default decisions. The resulting `ingestProposal` powers the
   * inline `<ReviewPanel>` and a follow-up `/ingest/apply` call. */
  const analyzeManyAndPreview = async (
    files: File[],
    sourceLabel: string,
  ): Promise<void> => {
    if (files.length === 0) {
      setInfo(t("{src} : aucun fichier importable trouvé.", { src: sourceLabel }));
      return;
    }
    const parts: ProposalPart[] = [];
    const failures: string[] = [];
    const willOcr = files.some(needsPreprocessing);
    if (willOcr) {
      setInfo(
        t("{src} : préparation de {n} fichier(s) — l'OCR peut prendre un moment au premier lancement, le temps de télécharger les données de langue…", { src: sourceLabel, n: files.length }),
      );
    }
    for (let i = 0; i < files.length; i++) {
      const f = files[i];
      try {
        const prepared = await prepareForIngest(f, {
          onProgress: (p) =>
            setInfo(
              t("Analyse {i}/{n} ({name}) : {status}", { i: i + 1, n: files.length, name: f.name, status: p.status }),
            ),
        });
        setInfo(t("Analyse {i}/{n} : {name}…", { i: i + 1, n: files.length, name: f.name }));
        const proposal = await analyzeIngest({ file: prepared });
        parts.push({ file: f.name, proposal });
      } catch (e) {
        failures.push(`${f.name}: ${e instanceof Error ? e.message : String(e)}`);
      }
    }
    if (parts.length === 0) {
      setError(
        failures.length > 0
          ? failures.slice(0, 3).join(" | ")
          : t("{src} : rien n'a pu être analysé.", { src: sourceLabel }),
      );
      setInfo(null);
      return;
    }
    const merged = mergeProposals(parts);
    const decisions: Record<string, DecisionAction> = {};
    const items: { client_ref: string; conflict?: typeof merged.concepts[number]["conflict"] }[] = [
      ...merged.concept_types,
      ...merged.relation_types,
      ...merged.concepts,
      ...merged.relations,
      ...merged.rules,
      ...merged.actions,
    ];
    for (const it of items) decisions[it.client_ref] = defaultDecisionFor(it.conflict);
    setIngestProposal(merged);
    setIngestDecisions(decisions);
    setApplyReport(null);
    const tail = failures.length ? ` · ${t("{n} en échec", { n: failures.length })}` : "";
    setInfo(
      t("{src} : {n} fiche(s) et {m} lien(s) proposés à partir de {ok}/{total} fichier(s){tail}. Vérifiez ci-dessous, puis cliquez sur Ajouter aux données.", { src: sourceLabel, n: merged.concepts.length, m: merged.relations.length, ok: parts.length, total: files.length, tail }),
    );
    if (failures.length) setError(failures.slice(0, 3).join(" | "));
  };

  const handleUpload = async (file: File) => {
    setBusy("upload");
    setError(null);
    setInfo(null);
    try {
      const ext = (file.name.split(".").pop() ?? "").toLowerCase();
      const inputs = ext === "zip" ? await expandZips([file]) : [file];
      await analyzeManyAndPreview(
        inputs,
        ext === "zip" ? t("Archive {name}", { name: file.name }) : file.name,
      );
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy("idle");
    }
  };

  const handleFolder = async (fileList: FileList | null) => {
    if (!fileList || fileList.length === 0) return;
    setBusy("upload");
    setError(null);
    setInfo(null);
    try {
      const all: File[] = [];
      for (let i = 0; i < fileList.length; i++) {
        const f = fileList[i];
        const rel = (f as File & { webkitRelativePath?: string }).webkitRelativePath ?? f.name;
        if (SKIP_IN_ZIP(rel)) continue;
        const ext = (f.name.split(".").pop() ?? "").toLowerCase();
        if (!(ext in KIND_BY_EXT) && ext !== "zip") continue;
        all.push(f);
      }
      const rootName =
        ((fileList[0] as File & { webkitRelativePath?: string }).webkitRelativePath ?? "")
          .split("/")[0] || t("dossier");
      setInfo(t("Parcours de {name} ({n} fichier(s) candidat(s))…", { name: rootName, n: all.length }));
      const flat = await expandZips(all);
      await analyzeManyAndPreview(flat, t("Dossier {name}", { name: rootName }));
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy("idle");
    }
  };

  const cancelIngestProposal = () => {
    setIngestProposal(null);
    setIngestDecisions({});
    setInfo(null);
  };

  const applyIngestProposal = async () => {
    if (!ingestProposal) return;
    setApplyBusy(true);
    setError(null);
    try {
      const decisions: ApplyDecision[] = Object.entries(ingestDecisions).map(
        ([client_ref, action]) => ({ client_ref, action }),
      );
      const report = await applyIngest({
        proposal: ingestProposal,
        decisions,
        defaultAction: "skip",
      });
      setApplyReport(report);
      setIngestProposal(null);
      setIngestDecisions({});
      setInfo(
        t("Appliqué : {a} créé(s), {b} fusionné(s), {c} ignoré(s), {d} en échec.", { a: report.created, b: report.merged, c: report.skipped, d: report.failed }),
      );
      await refresh();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setApplyBusy(false);
    }
  };

  const handleDeleteFile = async (id: number) => {
    try {
      await deleteFile(id);
      await refresh();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const clearAll = async () => {
    if (files.length === 0) return;
    if (!(await confirm({ title: t("Retirer les fichiers importés"), message: t("Retirer {n} fichier(s) de la liste ? Les données déjà importées restent dans le graphe.", { n: files.length }), confirmLabel: t("Retirer"), cancelLabel: t("Annuler"), danger: true }))) return;
    for (const f of files) {
      try {
        await deleteFile(f.id);
      } catch {
        /* ignore */
      }
    }
    await refresh();
  };

  const conceptTypeCount = ontology ? Object.keys(ontology.concept_types).length : 0;
  const confidence = stats && stats.concepts > 0 ? Math.min(99, 70 + Math.round((stats.relations / Math.max(1, stats.concepts)) * 10)) : 92;
  const confidenceLabel = confidence >= 85 ? t("Élevée") : confidence >= 65 ? t("Moyenne") : t("Faible");

  return (
    <>
      <div className="page-header">
        <div>
          <h1 className="page-title">{t("Modèle de données")}</h1>
          <p className="page-subtitle">{t("Les types de fiches et les liens permis, décrits en langage courant ou déduits de vos fichiers.")}</p>
        </div>
      </div>

      {error && <div className="error-banner">{error}</div>}
      {info && <div className="success-banner">{info}</div>}

      <ModelTypes ontology={ontology} onChanged={refresh} />

      {ingestProposal && (
        <Card
          title={
            <span>
              {t("Aperçu des fiches extraites")}{" "}
              <span className="info-dot" title={t("Fiches extraites de vos documents par le modèle de langage — vérifiez puis appliquez")}>
                ⓘ
              </span>
            </span>
          }
          subtitle={
            <span className="muted" style={{ fontSize: 12 }}>
              {t("{n} fiche(s) · {m} lien(s) · {k} nouveau(x) type(s) de fiche", { n: ingestProposal.concepts.length, m: ingestProposal.relations.length, k: ingestProposal.concept_types.length })}
            </span>
          }
          style={{ marginBottom: 16 }}
        >
          <ReviewPanel
            proposal={ingestProposal}
            decisions={ingestDecisions}
            onDecision={(ref, action) =>
              setIngestDecisions((d) => ({ ...d, [ref]: action }))
            }
            onEditConcept={(ref, patch) =>
              setIngestProposal((p) =>
                p
                  ? {
                      ...p,
                      concepts: p.concepts.map((c) =>
                        c.client_ref === ref ? { ...c, ...patch } : c,
                      ),
                    }
                  : p,
              )
            }
            onEditRelation={(ref, patch) =>
              setIngestProposal((p) =>
                p
                  ? {
                      ...p,
                      relations: p.relations.map((r) =>
                        r.client_ref === ref ? { ...r, ...patch } : r,
                      ),
                    }
                  : p,
              )
            }
            onBulkDecision={(action) => {
              if (!ingestProposal) return;
              const next: Record<string, DecisionAction> = {};
              for (const ref of iterRefs(ingestProposal)) next[ref] = action;
              setIngestDecisions(next);
            }}
            onApply={applyIngestProposal}
            onCancel={cancelIngestProposal}
            applyDisabled={applyBusy}
            applyLabel={applyBusy ? t("Application…") : t("Ajouter aux données")}
            cancelLabel={t("Abandonner")}
          />
        </Card>
      )}

      {applyReport && !ingestProposal && (
        <div style={{ marginBottom: 16 }}>
          <ApplyReportView
            report={applyReport}
            onReset={() => setApplyReport(null)}
            resetLabel={t("Fermer")}
          />
        </div>
      )}

      <div className="builder-grid">
        {/* ---------- LEFT COLUMN ---------- */}
        <div className="builder-left">
          {/* Describe Your Ontology */}
          <Card
            title={<span>{t("Décrivez votre modèle de données")} <span className="info-dot" title={t("Description en langage courant utilisée par le modèle de langage")}>ⓘ</span></span>}
            actions={
              <button className="chip-btn" onClick={() => setShowExamples((v) => !v)}>
                <span aria-hidden>⊞</span> {t("Exemples")}
              </button>
            }
          >
            {showExamples && (
              <div className="examples-list">
                {examples().map((ex, i) => (
                  <button
                    key={i}
                    className="example-chip"
                    onClick={() => {
                      setDescription(ex);
                      setShowExamples(false);
                    }}
                  >
                    {ex}
                  </button>
                ))}
              </div>
            )}
            <textarea
              className="desc-textarea"
              rows={6}
              maxLength={MAX_DESC}
              placeholder={t("Décrivez la structure du modèle : types de fiches, liens, propriétés et contraintes...")}
              value={description}
              onChange={(e) => setDescription(e.target.value)}
            />
            <div className="char-counter">{description.length} / {MAX_DESC}</div>
            <div className="generate-row">
              <button
                className="btn-generate"
                onClick={handleGenerate}
                disabled={busy !== "idle" || (!description.trim() && files.length === 0)}
              >
                <span aria-hidden>✦</span> {busy === "generate" ? t("Génération…") : t("Générer le modèle")}
              </button>
            </div>
          </Card>

          {/* Upload Files */}
          <Card
            title={<span>{t("Importer des fichiers (facultatif)")} <span className="info-dot" title={t("Les fichiers sont importés dans le graphe")}>ⓘ</span></span>}
            style={{ marginTop: 16 }}
          >
            <Dropzone
              onFile={handleUpload}
              disabled={busy !== "idle"}
              hint={t("Formats acceptés : ZIP (extrait automatiquement), PDF, DOC/DOCX, images (PNG/JPG/… OCR automatique), CSV, XLSX, JSON, TXT, MD (50 Mo max.)")}
            />

            <div className="folder-row">
              <label className={`folder-btn${busy !== "idle" ? " disabled" : ""}`}>
                <span aria-hidden>📁</span> {t("Choisir un dossier")}
                <input
                  type="file"
                  /* @ts-expect-error non-standard but widely supported */
                  webkitdirectory=""
                  directory=""
                  multiple
                  style={{ display: "none" }}
                  disabled={busy !== "idle"}
                  onChange={(e) => {
                    handleFolder(e.target.files);
                    e.target.value = "";
                  }}
                />
              </label>
              <span className="folder-hint">
                {t("Choisissez un dossier : chaque fichier pris en charge (ZIP imbriqués compris) est extrait et importé.")}
              </span>
            </div>

            {files.length > 0 && (
              <>
                <div className="upload-section-title">{t("Fichiers importés")}</div>
                <ul className="upload-list">
                  {files.slice(0, 8).map((f) => {
                    const ic = iconForFile(f);
                    const processed = f.concepts > 0 || f.relations > 0;
                    return (
                      <li key={f.id} className="upload-item">
                        <span className="file-icon" style={{ background: ic.bg, color: ic.fg }}>{ic.label}</span>
                        <div className="file-meta">
                          <div className="file-name" title={f.name}>{f.name}</div>
                          <div className="file-sub">{fmtBytes(f.size)} · {f.kind.toUpperCase()}</div>
                        </div>
                        <span className={`status-pill ${processed ? "ok" : "warn"}`}>
                          {processed ? t("Traité") : t("Analysé")}
                        </span>
                        <span className={`status-check ${processed ? "ok" : "warn"}`} aria-hidden>
                          {processed ? "✓" : "ⓘ"}
                        </span>
                        <button className="kebab-btn" title={t("Retirer")} onClick={() => handleDeleteFile(f.id)}>⋮</button>
                      </li>
                    );
                  })}
                </ul>
              </>
            )}

            <div className="upload-actions">
              <button className="action-btn" onClick={refresh} disabled={busy !== "idle"}>
                <span aria-hidden>🔍</span> {t("Analyser les fichiers")}
              </button>
              <button className="action-btn" onClick={refresh} disabled={busy !== "idle"}>
                <span aria-hidden>⇄</span> {t("Fusionner les sources")}
              </button>
              <button className="action-btn danger" onClick={clearAll} disabled={busy !== "idle" || files.length === 0}>
                <span aria-hidden>🗑</span> {t("Tout retirer")}
              </button>
            </div>
          </Card>
        </div>

        {/* ---------- RIGHT COLUMN ---------- */}
        <div className="builder-right">
          <Card
            title={<span>{t("Graphe du modèle")} <span className="info-dot" title={t("Vue en direct du modèle de données actuel")}>ⓘ</span></span>}
            className="graph-card"
          >
            <div className="graph-area">
              <OntologyGraph subgraph={subgraph} proposed={ingestProposal} height={460} />
              {proposed && (
                <div className="graph-preview-overlay">
                  <div className="muted" style={{ marginBottom: 6, fontSize: 12 }}>
                    {t("Schéma proposé · cliquez sur")} <strong>{t("Enregistrer le modèle")}</strong> {t("pour l'appliquer")}
                  </div>
                  <pre className="json-view" style={{ maxHeight: 360 }}>
                    {JSON.stringify(proposed, null, 2)}
                  </pre>
                </div>
              )}
            </div>
          </Card>

          <Card
            title={<span>{t("Chiffres du modèle")} <span className="info-dot" title={t("Comptages tirés du graphe en direct")}>ⓘ</span></span>}
            subtitle={
              <span className="insights-sub">
                {t("Dernière mise à jour :")} {lastUpdated ? fmtAgo(lastUpdated) : "—"}
                <button className="link-btn-sm" onClick={refresh} title={t("Actualiser")}>↻</button>
              </span>
            }
            style={{ marginTop: 16 }}
          >
            <div className="insights-grid">
              <InsightTile icon="👥" label={t("Fiches")} value={stats?.concepts ?? 0} delta={stats?.deltas.concepts_pct} color="#dbeafe" iconColor="#2563eb" />
              <InsightTile icon="⌬" label={t("Liens")} value={stats?.relations ?? 0} delta={stats?.deltas.relations_pct} color="#ede9fe" iconColor="#7c3aed" />
              <InsightTile icon="▦" label={t("Types de fiches")} value={conceptTypeCount} delta={stats?.deltas.concept_types_pct} color="#dcfce7" iconColor="#16a34a" />
              <InsightTile icon="✓" label={t("Confiance")} value={`${confidence}%`} hint={confidenceLabel} color="#fef3c7" iconColor="#d97706" />
            </div>
          </Card>
        </div>
      </div>

      {/* ---------- BOTTOM BAR ---------- */}
      <div className="builder-footer">
        <a className="footer-btn" href={exportGraphUrl("jsonl")}>
          <span aria-hidden>⤓</span> {t("Exporter")} <span aria-hidden>▾</span>
        </a>
        <button className="footer-btn primary" onClick={handleSave} disabled={!proposed || busy !== "idle"}>
          <span aria-hidden>💾</span> {t("Enregistrer le modèle")}
        </button>
      </div>

      <style>{`
        .builder-grid {
          display: grid;
          grid-template-columns: minmax(0, 460px) minmax(0, 1fr);
          gap: 18px;
          align-items: start;
        }
        @media (max-width: 1100px) { .builder-grid { grid-template-columns: 1fr; } }

        .info-dot { color: var(--muted); margin-left: 4px; font-size: 13px; cursor: help; }

        /* ---------- chip / pill buttons ---------- */
        .chip-btn {
          display: inline-flex; align-items: center; gap: 6px;
          padding: 6px 12px; font-size: 12px;
          background: var(--panel); border: 1px solid var(--border);
          border-radius: 8px; cursor: pointer; color: var(--text);
        }
        .chip-btn:hover { background: var(--panel-2); }

        .icon-square {
          width: 32px; height: 32px;
          display: inline-grid; place-items: center;
          background: var(--panel); border: 1px solid var(--border);
          border-radius: 8px; cursor: pointer; color: var(--text);
          font-size: 13px;
        }
        .icon-square:hover { background: var(--panel-2); }

        .link-btn-sm { background: none; border: none; color: var(--muted); cursor: pointer; font-size: 14px; margin-left: 4px; }
        .link-btn-sm:hover { color: var(--accent); }

        /* ---------- Describe section ---------- */
        .desc-textarea {
          width: 100%;
          resize: vertical;
          min-height: 130px;
          padding: 12px;
          font: inherit;
          background: var(--panel);
          border: 1px solid var(--border);
          border-radius: var(--radius-sm);
        }
        .desc-textarea:focus { outline: none; border-color: var(--accent); box-shadow: 0 0 0 3px var(--accent-soft); }
        .char-counter { font-size: 11px; color: var(--muted); text-align: right; margin-top: 6px; }

        .examples-list { display: flex; flex-direction: column; gap: 6px; margin-bottom: 10px; }
        .example-chip {
          text-align: left; padding: 8px 10px; font-size: 12px;
          background: var(--panel-2); border: 1px solid var(--border);
          border-radius: 8px; cursor: pointer; color: var(--text);
        }
        .example-chip:hover { background: var(--accent-soft); border-color: var(--accent); color: var(--accent); }

        .generate-row {
          margin-top: 12px;
          display: grid;
          grid-template-columns: 1fr 40px;
          gap: 1px;
          background: var(--accent);
          border-radius: 10px;
          overflow: hidden;
        }
        .btn-generate, .btn-generate-split {
          background: var(--accent); color: #fff;
          border: none; cursor: pointer; padding: 12px 16px;
          font-weight: 600; font-size: 14px;
          display: inline-flex; align-items: center; justify-content: center; gap: 8px;
        }
        .btn-generate:hover, .btn-generate-split:hover { background: #1d4fd1; }
        .btn-generate:disabled, .btn-generate-split:disabled { background: #93b4f5; cursor: not-allowed; }
        .btn-generate-split { padding: 12px 0; font-size: 12px; }

        /* ---------- Upload Files ---------- */
        .folder-row {
          margin-top: 10px;
          display: flex; align-items: center; gap: 10px; flex-wrap: wrap;
        }
        .folder-btn {
          display: inline-flex; align-items: center; gap: 6px;
          padding: 8px 14px; font-size: 12px; font-weight: 500;
          background: var(--panel); border: 1px dashed var(--border);
          border-radius: 8px; cursor: pointer; color: var(--text);
        }
        .folder-btn:hover { background: var(--panel-2); border-color: var(--accent); color: var(--accent); }
        .folder-btn.disabled { opacity: 0.5; cursor: not-allowed; pointer-events: none; }
        .folder-hint { font-size: 11px; color: var(--muted); }

        .upload-section-title { margin: 16px 0 8px; font-size: 12px; font-weight: 600; color: var(--text); }
        .upload-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
        .upload-item {
          display: grid;
          grid-template-columns: 36px 1fr auto auto auto;
          align-items: center; gap: 10px;
          padding: 10px 12px;
          background: var(--panel);
          border: 1px solid var(--border);
          border-radius: 10px;
        }
        .file-icon {
          width: 36px; height: 36px; border-radius: 8px;
          display: grid; place-items: center;
          font-size: 11px; font-weight: 700;
        }
        .file-meta { min-width: 0; }
        .file-name { font-size: 13px; font-weight: 600; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
        .file-sub { font-size: 11px; color: var(--muted); margin-top: 2px; }
        .status-pill {
          font-size: 11px; padding: 3px 10px; border-radius: 999px; font-weight: 500;
        }
        .status-pill.ok { background: #dcfce7; color: #166534; }
        .status-pill.warn { background: #fef3c7; color: #92400e; }
        .status-check { font-size: 14px; }
        .status-check.ok { color: #16a34a; }
        .status-check.warn { color: #d97706; }
        .kebab-btn {
          background: none; border: none; cursor: pointer; color: var(--muted);
          padding: 2px 6px; font-size: 16px; line-height: 1;
        }
        .kebab-btn:hover { color: var(--text); }

        .upload-actions {
          margin-top: 16px; display: grid;
          grid-template-columns: repeat(3, 1fr); gap: 8px;
        }
        .action-btn {
          display: inline-flex; align-items: center; justify-content: center; gap: 6px;
          padding: 9px 10px; font-size: 12px;
          background: var(--panel); border: 1px solid var(--border);
          border-radius: 8px; cursor: pointer; color: var(--text);
          white-space: nowrap;
        }
        .action-btn:hover { background: var(--panel-2); }
        .action-btn.danger { color: #dc2626; }
        .action-btn.danger:hover { background: #fef2f2; border-color: #fecaca; }
        .action-btn:disabled { opacity: 0.5; cursor: not-allowed; }

        /* ---------- Graph ---------- */
        .graph-toolbar { display: inline-flex; gap: 6px; align-items: center; }
        .graph-card .graph-with-legend {
          position: relative;
          display: grid;
          grid-template-columns: 1fr auto;
          gap: 12px;
          min-height: 440px;
        }
        .graph-area {
          position: relative; min-height: 440px;
          border: 1px solid var(--border);
          border-radius: var(--radius-sm);
          overflow: hidden; background: var(--panel);
        }
        .graph-preview-overlay {
          position: absolute; inset: 0; background: rgba(247,248,251,0.97);
          padding: 16px; overflow: auto;
        }
        .graph-legend {
          list-style: none; padding: 12px 14px; margin: 0;
          display: flex; flex-direction: column; gap: 10px;
          font-size: 12px; color: var(--text);
          background: var(--panel); border: 1px solid var(--border); border-radius: var(--radius-sm);
          min-width: 110px;
        }
        .legend-dot { display: inline-block; width: 10px; height: 10px; border-radius: 50%; margin-right: 8px; vertical-align: middle; }

        /* ---------- Insights ---------- */
        .insights-sub { display: inline-flex; align-items: center; gap: 4px; font-size: 12px; color: var(--muted); }
        .insights-grid { display: grid; grid-template-columns: repeat(4, 1fr); gap: 12px; }
        @media (max-width: 900px) { .insights-grid { grid-template-columns: repeat(2, 1fr); } }
        .insight-tile {
          display: flex; gap: 12px; align-items: center;
          padding: 16px; border: 1px solid var(--border);
          border-radius: var(--radius-sm); background: var(--panel);
        }
        .insight-icon {
          width: 44px; height: 44px; border-radius: 12px;
          display: grid; place-items: center;
          font-size: 20px; flex-shrink: 0;
        }
        .insight-meta .value { font-size: 26px; font-weight: 700; line-height: 1.1; color: var(--text); }
        .insight-meta .label { font-size: 12px; color: var(--muted); margin-top: 2px; }
        .insight-meta .delta { font-size: 11px; margin-top: 4px; font-weight: 500; }

        /* ---------- Footer ---------- */
        .builder-footer {
          display: flex; justify-content: flex-end; gap: 10px;
          margin-top: 20px; padding-top: 16px;
        }
        .footer-btn {
          display: inline-flex; align-items: center; gap: 6px;
          padding: 10px 18px; font-size: 13px; font-weight: 500;
          border-radius: 10px; cursor: pointer;
          background: var(--panel); border: 1px solid var(--border);
          color: var(--text); text-decoration: none;
        }
        .footer-btn:hover { background: var(--panel-2); text-decoration: none; }
        .footer-btn.primary {
          background: var(--accent); color: #fff; border-color: var(--accent);
        }
        .footer-btn.primary:hover { background: #1d4fd1; }
        .footer-btn.primary:disabled { background: #93b4f5; border-color: #93b4f5; cursor: not-allowed; }
      `}</style>
    </>
  );
}

interface InsightProps {
  icon: string;
  label: string;
  value: number | string;
  delta?: number;
  hint?: string;
  color: string;
  iconColor: string;
}

function InsightTile({ icon, label, value, delta, hint, color, iconColor }: InsightProps) {
  const cls = delta == null ? "flat" : delta > 0.5 ? "up" : delta < -0.5 ? "down" : "flat";
  const arrow = cls === "up" ? "↑" : cls === "down" ? "↓" : "•";
  return (
    <div className="insight-tile">
      <div className="insight-icon" style={{ background: color, color: iconColor }}>{icon}</div>
      <div className="insight-meta">
        <div className="value">
          {typeof value === "number" && value >= 1000 ? `${(value / 1000).toFixed(1)}k` : value}
        </div>
        <div className="label">{label}</div>
        {delta != null && (
          <div className={`delta ${cls}`} style={{ color: cls === "up" ? "#16a34a" : cls === "down" ? "#dc2626" : "var(--muted)" }}>
            {arrow} {t("{n} % vs dernière exécution", { n: Math.abs(delta).toFixed(0) })}
          </div>
        )}
        {delta == null && hint && (
          <div className="delta" style={{ color: iconColor }}>
            {hint} <span style={{ display: "inline-block", width: 6, height: 6, borderRadius: 999, background: iconColor, marginLeft: 4, verticalAlign: "middle" }} />
          </div>
        )}
      </div>
    </div>
  );
}
