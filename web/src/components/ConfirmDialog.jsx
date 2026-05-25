// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
import { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";

const ConfirmCtx = createContext(null);

export function ConfirmProvider({ children }) {
  const [state, setState] = useState(null); // { opts, resolve } | null
  const resolverRef = useRef(null);

  const confirm = useCallback((opts) => {
    return new Promise((resolve) => {
      resolverRef.current = resolve;
      setState({
        opts: {
          title: opts?.title ?? "Confirm",
          message: opts?.message ?? "Are you sure?",
          confirmLabel: opts?.confirmLabel ?? "Confirm",
          cancelLabel: opts?.cancelLabel ?? "Cancel",
          danger: opts?.danger ?? false,
        },
      });
    });
  }, []);

  const close = useCallback((result) => {
    const r = resolverRef.current;
    resolverRef.current = null;
    setState(null);
    if (r) r(result);
  }, []);

  useEffect(() => {
    if (!state) return;
    const onKey = (e) => {
      if (e.key === "Escape") close(false);
      else if (e.key === "Enter") close(true);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [state, close]);

  return (
    <ConfirmCtx.Provider value={confirm}>
      {children}
      {state && (
        <div
          className="modal-backdrop"
          role="dialog"
          aria-modal="true"
          onClick={() => close(false)}
          style={{
            position: "fixed", inset: 0, background: "rgba(15,23,42,.45)",
            display: "flex", alignItems: "center", justifyContent: "center", zIndex: 10000,
          }}
        >
          <div
            className="modal-card"
            onClick={(e) => e.stopPropagation()}
            style={{
              background: "#fff", borderRadius: 12, padding: 24,
              width: "min(440px, 92vw)", boxShadow: "0 20px 50px rgba(0,0,0,.25)",
            }}
          >
            <h3 className="modal-title" style={{ margin: "0 0 8px", fontSize: 18, fontWeight: 600 }}>
              {state.opts.title}
            </h3>
            <p style={{ margin: "0 0 20px", color: "#475569", lineHeight: 1.5 }}>
              {state.opts.message}
            </p>
            <div
              className="modal-actions"
              style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}
            >
              <button type="button" className="btn-ghost" onClick={() => close(false)}>
                {state.opts.cancelLabel}
              </button>
              <button
                type="button"
                className={state.opts.danger ? "btn-primary" : "btn-primary"}
                onClick={() => close(true)}
                autoFocus
                style={state.opts.danger ? { background: "#dc2626", borderColor: "#dc2626" } : undefined}
              >
                {state.opts.confirmLabel}
              </button>
            </div>
          </div>
        </div>
      )}
    </ConfirmCtx.Provider>
  );
}

export function useConfirm() {
  const ctx = useContext(ConfirmCtx);
  if (!ctx) throw new Error("useConfirm must be used inside <ConfirmProvider>");
  return ctx;
}
