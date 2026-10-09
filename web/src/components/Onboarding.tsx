// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The first day: while the graph is empty,
// the dashboard is replaced by three numbered steps — describe your data,
// drop your files, ask a question — each opening once the previous one is
// done, plus the finance example in one click. It disappears by itself
// once there is data.

import { useState } from "react";
import { Link } from "react-router-dom";
import { loadFinanceExample } from "../lib/example";
import { MODELS, loadModel } from "../lib/models";
import { t } from "../lib/i18n";

interface Props {
  /** The schema has at least one concept type. */
  hasModel: boolean;
  /** The graph has at least one concept. */
  hasData: boolean;
  /** Called after the example was loaded, so the page refreshes. */
  onLoaded?: () => void;
}

export default function Onboarding({ hasModel, hasData, onLoaded }: Props) {
  const [progress, setProgress] = useState<[number, number] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const [model, setModel] = useState("");
  const pickModel = async (key: string) => {
    setModel(key);
    if (!key) return;
    setError(null);
    try {
      await loadModel(key);
      onLoaded?.();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const loadExample = async () => {
    setError(null);
    setProgress([0, 1]);
    try {
      await loadFinanceExample((done, total) => setProgress([done, total]));
      setProgress(null);
      onLoaded?.();
    } catch (e: unknown) {
      setProgress(null);
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const steps = [
    {
      n: 1,
      done: hasModel,
      open: true,
      title: t("Décrivez vos données"),
      text: t("Quels types de fiches (Personne, Contrat, Équipement…) et quels liens entre elles ? Partez d'un modèle prêt à l'emploi, décrivez le vôtre, ou essayez l'exemple."),
      action: (
        <>
          <select aria-label={t("Modèle prêt à l'emploi")} value={model} onChange={(e) => void pickModel(e.target.value)}>
            <option value="">{t("Modèle prêt à l'emploi…")}</option>
            {MODELS.map((m) => (
              <option key={m.key} value={m.key}>
                {m.label}
              </option>
            ))}
          </select>
          <Link className="btn-primary" to="/builder">
            {t("Définir le modèle")}
          </Link>
          <button
            className="btn-outline"
            onClick={() => void loadExample()}
            disabled={progress !== null}
          >
            {progress
              ? t("Chargement… {done} / {total}", { done: progress[0], total: progress[1] })
              : t("Essayer avec l'exemple finance (sociétés, contrats, factures)")}
          </button>
        </>
      ),
    },
    {
      n: 2,
      done: hasData,
      open: hasModel,
      title: t("Déposez vos fichiers"),
      text: t("Word, Excel, CSV, PDF ou texte : l'import propose des fiches et des relations que vous relisez avant de les ajouter."),
      action: (
        <Link className="btn-primary" to="/files">
          {t("Déposer des fichiers")}
        </Link>
      ),
    },
    {
      n: 3,
      done: false,
      open: hasData,
      title: t("Posez une question"),
      text: t("En langage courant. La réponse cite les fiches d'où elle vient."),
      action: (
        <Link className="btn-primary" to="/queries">
          {t("Poser une question")}
        </Link>
      ),
    },
  ];

  return (
    <section className="onboarding" aria-label={t("Premiers pas")}>
      <h1 className="page-title">{t("Bienvenue")}</h1>
      <p className="page-subtitle">{t("Trois étapes pour un premier graphe qui répond à vos questions.")}</p>
      {error && <div className="error-banner">{error}</div>}
      <ol className="onboarding-steps">
        {steps.map((s) => (
          <li
            key={s.n}
            className={`onboarding-step${s.done ? " done" : ""}${s.open ? "" : " locked"}`}
            aria-current={s.open && !s.done ? "step" : undefined}
          >
            <div className="onboarding-num" aria-hidden>
              {s.done ? "✓" : s.n}
            </div>
            <div className="onboarding-body">
              <h2>{s.title}</h2>
              <p className="muted">{s.text}</p>
              {s.open && !s.done && <div className="onboarding-actions">{s.action}</div>}
              {s.done && <span className="badge badge-success">{t("Fait")}</span>}
              {!s.open && <span className="muted">{t("Après l'étape précédente.")}</span>}
            </div>
          </li>
        ))}
      </ol>
    </section>
  );
}
