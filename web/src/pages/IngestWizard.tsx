// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// LLM-assisted ingest wizard.
//
// Flow:
//   1. Upload — pick a document + provider/model/language hint.
//   2. Analyze — call `/ingest/analyze`; show progress.
//   3. Review — walk every item (types → concepts → relations → rules
//      → actions), edit fields inline, pick a per-item decision.
//   4. Apply — POST proposal + decisions to `/ingest/apply`, render the
//      outcome report.
//
// The proposal is held entirely in client state — the server is
// stateless between analyze and apply. To survive a tab reload while
// reviewing, every change is mirrored to `sessionStorage` under
// `ingest.draft`.

import { useEffect, useRef, useState } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import Card from "../components/Card";
// @ts-expect-error JSX module
import { useToast } from "../components/Toast.jsx";
import { getSettings } from "../api";
import { analyzeIngest, applyIngest } from "../lib/ingestApi";
import { prepareForIngest, terminateOcrWorker } from "../lib/extractText";import type {
  ApplyDecision,
  ApplyReport,
  DecisionAction,
  OntologyProposal,
} from "../lib/proposalTypes";
import { iterRefs } from "../lib/proposalTypes";
import {
  ApplyReportView,
  ReviewPanel,
  defaultDecisionFor,
} from "../components/IngestReview";

type Step = "upload" | "analyzing" | "review" | "applying" | "done" | "error";

const STORAGE_KEY = "ingest.draft.v1";

interface DraftState {
  proposal: OntologyProposal;
  decisions: Record<string, DecisionAction>;
}

function loadDraft(): DraftState | null {
  if (typeof window === "undefined") return null;
  try {
    const raw = window.sessionStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    return JSON.parse(raw) as DraftState;
  } catch {
    return null;
  }
}

function saveDraft(state: DraftState): void {
  try {
    window.sessionStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch {
    /* quota — ignore */
  }
}

function clearDraft(): void {
  try {
    window.sessionStorage.removeItem(STORAGE_KEY);
  } catch {
    /* ignore */
  }
}

// ---------- main page ----------

export default function IngestWizard() {
  const toast = useToast();
  const [step, setStep] = useState<Step>("upload");
  const [file, setFile] = useState<File | null>(null);
  // "default" routes to whatever the server has applied, so it is both the
  // initial value and a valid final one. The effect below only makes the
  // active provider *visible* in the selector.
  const [provider, setProvider] = useState<"default" | "openai" | "anthropic" | "infomaniak">(
    "default",
  );
  const [providerTouched, setProviderTouched] = useState(false);

  // Reflect the provider configured in Settings, which is the single source of
  // truth. Skipped once the user has picked one here, so a slow response never
  // overwrites a deliberate choice.
  useEffect(() => {
    let cancelled = false;
    getSettings()
      .then((s) => {
        if (cancelled || providerTouched) return;
        const p = s.llm.active_provider;
        if (p === "openai" || p === "anthropic" || p === "infomaniak") setProvider(p);
      })
      .catch(() => {
        /* selector stays on "default" — the server decides anyway */
      });
    return () => {
      cancelled = true;
    };
  }, [providerTouched]);
  const [modelName, setModelName] = useState("");
  const [languageHint, setLanguageHint] = useState("");
  const [proposal, setProposal] = useState<OntologyProposal | null>(null);
  const [decisions, setDecisions] = useState<Record<string, DecisionAction>>({});
  const [error, setError] = useState<string | null>(null);
  const [report, setReport] = useState<ApplyReport | null>(null);
  const fileInput = useRef<HTMLInputElement | null>(null);

  // Restore in-flight review on tab reload.
  useEffect(() => {
    const draft = loadDraft();
    if (draft && draft.proposal) {
      setProposal(draft.proposal);
      setDecisions(draft.decisions);
      setStep("review");
    }
  }, []);

  // A document dropped on the Files page arrives here and is analysed at
  // once: the review is proposed, not requested.
  const nav = useNavigate();
  const handed = (useLocation().state as { file?: File } | null)?.file;
  const consumed = useRef(false);
  useEffect(() => {
    if (!handed || consumed.current) return;
    consumed.current = true;
    // Consumed: Back or a reload must not analyse it a second time.
    nav(".", { replace: true, state: null });
    if (loadDraft()) {
      toast.info(`Relecture en cours : terminez-la ou annulez-la avant d'importer ${handed.name}`);
      return;
    }
    setFile(handed);
    void onAnalyze(handed);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [handed]);

  // Release tesseract.js worker on unmount.
  useEffect(() => () => void terminateOcrWorker(), []);

  // Persist edits while reviewing.
  useEffect(() => {
    if (step === "review" && proposal) {
      saveDraft({ proposal, decisions });
    }
  }, [proposal, decisions, step]);

  async function onAnalyze(picked: File | null = file) {
    if (!picked) {
      toast.info("Choisissez d'abord un document");
      return;
    }
    setStep("analyzing");
    setError(null);
    try {
      const prepared = await prepareForIngest(picked, {
        onProgress: (p) => {
          setError(null);
          toast.info(p.status);
        },
      });
      const p = await analyzeIngest({
        file: prepared,
        provider,
        model: modelName.trim() || undefined,
        languageHint: languageHint.trim() || undefined,
      });
      // Seed per-item decisions from conflict heuristics.
      const seed: Record<string, DecisionAction> = {};
      for (const c of p.concept_types) seed[c.client_ref] = defaultDecisionFor(c.conflict);
      for (const c of p.relation_types) seed[c.client_ref] = defaultDecisionFor(c.conflict);
      for (const c of p.concepts) seed[c.client_ref] = defaultDecisionFor(c.conflict);
      for (const c of p.relations) seed[c.client_ref] = defaultDecisionFor(c.conflict);
      for (const c of p.rules) seed[c.client_ref] = defaultDecisionFor(c.conflict);
      for (const c of p.actions) seed[c.client_ref] = defaultDecisionFor(c.conflict);
      setProposal(p);
      setDecisions(seed);
      setStep("review");
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
      setStep("error");
    }
  }

  async function onApply() {
    if (!proposal) return;
    setStep("applying");
    try {
      const list: ApplyDecision[] = iterRefs(proposal).map((ref) => ({
        client_ref: ref,
        action: decisions[ref] ?? "skip",
      }));
      const rep = await applyIngest({ proposal, decisions: list, defaultAction: "skip" });
      setReport(rep);
      clearDraft();
      setStep("done");
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
      setStep("error");
    }
  }

  function reset() {
    clearDraft();
    setFile(null);
    setProposal(null);
    setDecisions({});
    setReport(null);
    setError(null);
    setStep("upload");
    if (fileInput.current) fileInput.current.value = "";
  }

  return (
    <div style={{ display: "grid", gap: 16, maxWidth: 1100 }}>
      <h1 style={{ margin: 0 }}>Importer un document</h1>
      <p style={{ color: "#475569", margin: 0 }}>
        Le document est lu par le modèle de langage configuré, qui propose des fiches, des liens et des règles.
        Vous vérifiez chaque proposition avant qu'elle n'entre dans vos données.
      </p>

      {step === "upload" && (
        <UploadStep
          file={file}
          onFile={setFile}
          fileInput={fileInput}
          provider={provider}
          onProvider={(p) => {
            setProviderTouched(true);
            setProvider(p);
          }}
          model={modelName}
          onModel={setModelName}
          languageHint={languageHint}
          onLanguageHint={setLanguageHint}
          onAnalyze={() => void onAnalyze()}
        />
      )}

      {step === "analyzing" && (
        <Card>
          <p>Lecture du document{file ? ` ${file.name}` : ""}… quelques secondes.</p>
        </Card>
      )}

      {step === "review" && proposal && (
        <p className="ingest-summary" data-testid="ingest-summary">
          {summary(proposal)} Vérifiez, corrigez si besoin, puis ajoutez.
        </p>
      )}
      {step === "review" && proposal && (
        <ReviewPanel
          proposal={proposal}
          decisions={decisions}
          onDecision={(ref, action) =>
            setDecisions((d) => ({ ...d, [ref]: action }))
          }
          onEditConcept={(ref, patch) =>
            setProposal((p) =>
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
            setProposal((p) =>
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
            const next: Record<string, DecisionAction> = {};
            for (const ref of iterRefs(proposal)) next[ref] = action;
            setDecisions(next);
          }}
          onApply={onApply}
          onCancel={reset}
        />
      )}

      {step === "applying" && (
        <Card>
          <p>Writing accepted items to the graph…</p>
        </Card>
      )}

      {step === "done" && report && (
        <ApplyReportView report={report} onReset={reset} />
      )}

      {step === "error" && (
        <Card>
          <h2 style={{ marginTop: 0, color: "#b91c1c" }}>Un problème est survenu</h2>
          <pre style={{ whiteSpace: "pre-wrap", color: "#475569" }}>{error}</pre>
          <button onClick={reset} style={{ marginTop: 8 }}>
            Recommencer
          </button>
        </Card>
      )}
    </div>
  );
}

/** « 12 personnes, 4 organisations et 9 liens trouvés dans contrat.pdf. » */
export function summary(p: OntologyProposal): string {
  const byType = new Map<string, number>();
  for (const c of p.concepts) byType.set(c.concept_type, (byType.get(c.concept_type) ?? 0) + 1);
  const parts = [...byType.entries()].sort((a, b) => b[1] - a[1]).map(([t, n]) => `${n} ${t}`);
  const found = parts.length === 0 ? "aucune fiche" : parts.join(", ");
  const where = p.source?.name ? ` dans ${p.source.name}` : "";
  return `${found} et ${p.relations.length} lien(s) trouvés${where}.`;
}

// ---------- Upload step ----------

function UploadStep(props: {
  file: File | null;
  onFile: (f: File | null) => void;
  fileInput: React.RefObject<HTMLInputElement>;
  provider: "default" | "openai" | "anthropic" | "infomaniak";
  onProvider: (v: "default" | "openai" | "anthropic" | "infomaniak") => void;
  model: string;
  onModel: (v: string) => void;
  languageHint: string;
  onLanguageHint: (v: string) => void;
  onAnalyze: () => void;
}) {
  return (
    <Card>
      <div style={{ display: "grid", gap: 12 }}>
        <label style={{ display: "grid", gap: 4 }}>
          <span style={{ fontSize: 12, color: "#475569" }}>Document</span>
          <input
            ref={props.fileInput}
            type="file"
            accept=".txt,.md,.csv,.json,.jsonl,.ndjson,.xlsx,.docx,.pdf"
            onChange={(e) => props.onFile(e.target.files?.[0] ?? null)}
          />
        </label>

        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 12 }}>
          <label style={{ display: "grid", gap: 4 }}>
            <span style={{ fontSize: 12, color: "#475569" }}>LLM provider</span>
            <select
              value={props.provider}
              onChange={(e) =>
                props.onProvider(
                  e.target.value as "default" | "openai" | "anthropic" | "infomaniak",
                )
              }
            >
              <option value="default">Default (server-configured)</option>
              <option value="openai">OpenAI</option>
              <option value="anthropic">Anthropic</option>
              <option value="infomaniak">Infomaniak AI (Swiss cloud)</option>
            </select>
          </label>
          <label style={{ display: "grid", gap: 4 }}>
            <span style={{ fontSize: 12, color: "#475569" }}>Model (optional)</span>
            <input
              type="text"
              placeholder="gpt-4o-mini / claude-3-7-sonnet-latest"
              value={props.model}
              onChange={(e) => props.onModel(e.target.value)}
            />
          </label>
          <label style={{ display: "grid", gap: 4 }}>
            <span style={{ fontSize: 12, color: "#475569" }}>Language hint (ISO-639-1)</span>
            <input
              type="text"
              placeholder="en, fr, it, … (auto-detect if blank)"
              value={props.languageHint}
              onChange={(e) => props.onLanguageHint(e.target.value)}
              maxLength={5}
            />
          </label>
        </div>

        <div style={{ display: "flex", justifyContent: "flex-end" }}>
          <button
            onClick={props.onAnalyze}
            disabled={!props.file}
            style={{
              padding: "8px 16px",
              background: props.file ? "#2563eb" : "#94a3b8",
              color: "white",
              border: 0,
              borderRadius: 4,
              cursor: props.file ? "pointer" : "not-allowed",
            }}
          >
            Analyser le document
          </button>
        </div>
      </div>
    </Card>
  );
}

