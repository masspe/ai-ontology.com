// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// A dropdown multi-select in the Power BI manner: a button that shows the
// state, a panel with a search, t("Tout sélectionner") and one checkbox per
// option. None selected means no filter (all). The open state and the search
// live here, so opening or typing never re-renders the page around it.

import { useEffect, useId, useMemo, useRef, useState } from "react";
import { t } from "../lib/i18n";

export interface MultiSelectOption {
  value: string;
  label: string;
  hint?: string;
}

interface Props {
  label: string;
  options: MultiSelectOption[];
  selected: string[];
  onChange: (next: string[]) => void;
  /** Shown on the button when nothing (or everything) is selected. */
  allLabel: string;
  /** A line above the options (e.g. why the list is not narrowed). */
  note?: string;
}

/** Lower case, accents removed: "Équipe" matches "equi". */
const fold = (s: string) => s.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase();

export default function MultiSelect({ label, options, selected, onChange, allLabel, note }: Props) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const wrap = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const allBox = useRef<HTMLInputElement>(null);
  const panelId = useId();

  const chosen = useMemo(() => new Set(selected), [selected]);
  const visible = useMemo(() => {
    const q = fold(query.trim());
    return q ? options.filter((o) => fold(o.label).includes(q)) : options;
  }, [options, query]);
  const nVisible = visible.filter((o) => chosen.has(o.value)).length;
  const allVisible = visible.length > 0 && nVisible === visible.length;
  const someVisible = nVisible > 0 && !allVisible;

  useEffect(() => {
    if (allBox.current) allBox.current.indeterminate = someVisible;
  });

  // Outside click closes (a click on a label inside moves focus to the body,
  // so the blur handler below cannot tell; this listener can).
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!wrap.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  // Next selection in the options' order; values no longer offered drop out.
  const emit = (next: Set<string>) => onChange(options.map((o) => o.value).filter((v) => next.has(v)));
  const toggle = (v: string) => {
    const next = new Set(chosen);
    if (next.has(v)) next.delete(v);
    else next.add(v);
    emit(next);
  };
  const toggleVisible = () => {
    const next = new Set(chosen);
    for (const o of visible) {
      if (allVisible) next.delete(o.value);
      else next.add(o.value);
    }
    emit(next);
  };

  const n = selected.length;
  const summary =
    n === 0 || n === options.length
      ? allLabel
      : n === 1
        ? (options.find((o) => o.value === selected[0])?.label ?? selected[0])
        : t("{n} sur {total}", { n, total: options.length });

  return (
    <div
      className="ms"
      ref={wrap}
      onKeyDown={(e) => {
        if (e.key === "Escape" && open) {
          e.stopPropagation();
          setOpen(false);
          button.current?.focus();
        }
      }}
      onBlur={(e) => {
        const to = e.relatedTarget as Node | null;
        if (to && !wrap.current?.contains(to)) setOpen(false);
      }}
    >
      <button
        ref={button}
        type="button"
        className={`ms-button${n > 0 && n < options.length ? " active" : ""}`}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={open ? panelId : undefined}
        onClick={() => setOpen((v) => !v)}
      >
        <span className="ms-label">{label}</span>
        <span className="ms-value">{summary}</span>
        <span className="gv-caret" aria-hidden="true">▾</span>
      </button>
      {open && (
        <div className="ms-panel" id={panelId} role="dialog" aria-label={label}>
          <input
            className="ms-search"
            type="search"
            placeholder={t(t("Rechercher…"))}
            aria-label={t("Rechercher dans {label}", { label })}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            autoFocus
          />
          {note && <div className="ms-note">{note}</div>}
          <label className="ms-option ms-all">
            <input
              ref={allBox}
              type="checkbox"
              checked={allVisible}
              onChange={toggleVisible}
              disabled={visible.length === 0}
            />
            <span>{t(t("Tout sélectionner"))}</span>
          </label>
          <div className="ms-options">
            {visible.length === 0 && <div className="ms-empty muted">{t(t("Aucun résultat."))}</div>}
            {visible.map((o) => (
              <label key={o.value} className="ms-option">
                <input type="checkbox" checked={chosen.has(o.value)} onChange={() => toggle(o.value)} />
                <span>{o.label}</span>
                {o.hint && <span className="ms-hint muted">{o.hint}</span>}
              </label>
            ))}
          </div>
          <div className="ms-foot">
            <button type="button" className="btn-ghost-link" onClick={() => onChange([])} disabled={n === 0}>
              {t(t("Effacer"))}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
