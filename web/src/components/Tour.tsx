// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The interactive guide behind the "?": one
// step per part of the application, the screen darkened except the part
// explained, a card that says what it is for, the application navigating
// to the page itself. Opens alone on the first visit, reopens any time
// from the "?", remembers in the browser that it was seen. No dependency:
// a fixed overlay, a spotlight drawn with a box-shadow, a card.

import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { NAV_ITEMS } from "../layout/nav";
import { t } from "../lib/i18n";

export interface TourStep {
  /** Page to show for this step. */
  path: string;
  /** `data-tour` value of the element to highlight (may be absent). */
  target: string;
  title: string;
  text: string;
}

const KEY = "tour.v1";
const OPEN_EVENT = "tour:open";

// Getters: the sentences are read when the guide renders, in the active language.
export const TOUR_STEPS: TourStep[] = [
  {
    path: "/",
    target: "help",
    get title() {
      return t("Bienvenue");
    },
    get text() {
      return t("Ce guide présente chaque partie de l'application, page par page. Vous pouvez le rouvrir à tout moment avec ce bouton « ? », et sauter directement à une partie depuis le sommaire.");
    },
  },
  ...NAV_ITEMS.map((item) => ({
    path: item.to,
    target: item.tour,
    get title() {
      return item.label;
    },
    get text() {
      return item.guide;
    },
  })),
  {
    path: "/",
    target: "search",
    get title() {
      return t("Recherche");
    },
    get text() {
      return t("Depuis n'importe quelle page : tapez une question ou un nom de fiche, la réponse arrive sur la page Questions.");
    },
  },
  {
    path: "/",
    target: "feedback",
    get title() {
      return t("Retour d'expérience");
    },
    get text() {
      return t("Un bouton pour nous signaler ce qui manque ou ce qui gêne, avec une capture d'écran si vous le souhaitez.");
    },
  },
];

/** Open the guide from anywhere (the "?" button). */
export function openTour(): void {
  window.dispatchEvent(new Event(OPEN_EVENT));
}

function seen(): boolean {
  try {
    return window.localStorage.getItem(KEY) !== null;
  } catch {
    return true;
  }
}

function remember(): void {
  try {
    window.localStorage.setItem(KEY, JSON.stringify({ done: true, at: Date.now() }));
  } catch {
    // Private mode or blocked storage: the guide simply opens again next time.
  }
}

interface Rect {
  top: number;
  left: number;
  width: number;
  height: number;
}

export default function Tour() {
  const nav = useNavigate();
  const loc = useLocation();
  const [step, setStep] = useState<number | null>(null);
  const [rect, setRect] = useState<Rect | null>(null);
  const [summary, setSummary] = useState(false);
  const cardRef = useRef<HTMLDivElement>(null);
  const before = useRef<Element | null>(null);

  // First visit: open alone. Any time: the "?" button.
  useEffect(() => {
    if (!seen()) setStep(0);
    const open = () => {
      setSummary(false);
      setStep(0);
    };
    window.addEventListener(OPEN_EVENT, open);
    return () => window.removeEventListener(OPEN_EVENT, open);
  }, []);

  const close = useCallback(() => {
    remember();
    setStep(null);
    setRect(null);
    (before.current as HTMLElement | null)?.focus?.();
  }, []);

  // The dialog takes the focus; whoever had it gets it back on close.
  useEffect(() => {
    if (step === null) return;
    if (!before.current) before.current = document.activeElement;
    cardRef.current?.focus();
  }, [step]);

  // The step's page, then the element to spotlight once it is rendered.
  const current = step === null ? null : TOUR_STEPS[step] ?? null;
  useEffect(() => {
    if (!current) return;
    if (loc.pathname !== current.path) nav(current.path);
  }, [current, loc.pathname, nav]);

  useLayoutEffect(() => {
    if (!current) return;
    const measure = () => {
      const el = document.querySelector<HTMLElement>(`[data-tour="${current.target}"]`);
      if (!el) {
        setRect(null);
        return;
      }
      const r = el.getBoundingClientRect();
      setRect({ top: r.top, left: r.left, width: r.width, height: r.height });
    };
    measure();
    window.addEventListener("resize", measure);
    window.addEventListener("scroll", measure, true);
    return () => {
      window.removeEventListener("resize", measure);
      window.removeEventListener("scroll", measure, true);
    };
  }, [current, loc.pathname]);

  useEffect(() => {
    if (step === null) return;
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
      if (e.key === "ArrowRight" && step < TOUR_STEPS.length - 1) setStep(step + 1);
      if (e.key === "ArrowLeft" && step > 0) setStep(step - 1);
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [step, close]);

  if (step === null || !current) return null;
  const last = step === TOUR_STEPS.length - 1;
  const pad = 6;
  const spot = rect
    ? {
        top: rect.top - pad,
        left: rect.left - pad,
        width: rect.width + pad * 2,
        height: rect.height + pad * 2,
      }
    : null;
  // The card sits under the spotlight when there is room, else centred.
  const cardStyle: React.CSSProperties = spot
    ? {
        top: Math.min(spot.top + spot.height + 12, window.innerHeight - 260),
        left: Math.min(Math.max(spot.left, 16), Math.max(16, window.innerWidth - 380)),
      }
    : { top: "50%", left: "50%", transform: "translate(-50%, -50%)" };

  return (
    <div className="tour" role="dialog" aria-modal="true" aria-labelledby="tour-title" data-testid="tour">
      {spot ? (
        <div className="tour-spot" style={spot} data-testid="tour-spot" />
      ) : (
        <div className="tour-backdrop" />
      )}
      <div className="tour-card" style={cardStyle} ref={cardRef} tabIndex={-1}>
        <div className="tour-head">
          <span className="muted">
            {t("Étape {n} sur {total}", { n: step + 1, total: TOUR_STEPS.length })}
          </span>
          <button className="icon-btn" aria-label={t("Fermer le guide")} onClick={close}>
            ×
          </button>
        </div>
        <h2 id="tour-title" className="tour-title">
          {current.title}
        </h2>
        <p className="tour-text">{current.text}</p>
        {summary && (
          <ol className="tour-summary" aria-label={t("Sommaire")}>
            {TOUR_STEPS.map((s, i) => (
              <li key={s.target + i}>
                <button
                  className={`link-btn${i === step ? " active" : ""}`}
                  onClick={() => {
                    setSummary(false);
                    setStep(i);
                  }}
                >
                  {s.title}
                </button>
              </li>
            ))}
          </ol>
        )}
        <div className="tour-actions">
          <button className="btn-ghost" onClick={() => setSummary((v) => !v)} aria-expanded={summary}>
            {t("Sommaire")}
          </button>
          <span style={{ flex: 1 }} />
          <button className="btn-ghost" onClick={() => setStep(step - 1)} disabled={step === 0}>
            {t("Précédent")}
          </button>
          {last ? (
            <button className="btn-primary" onClick={close}>
              {t("Terminer")}
            </button>
          ) : (
            <button className="btn-primary" onClick={() => setStep(step + 1)}>
              {t("Suivant")}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
