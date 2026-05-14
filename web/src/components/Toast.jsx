// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
import { createContext, useCallback, useContext, useState } from "react";

const ToastCtx = createContext(null);

export function ToastProvider({ children }) {
  const [toasts, setToasts] = useState([]);
  const push = useCallback((msg, kind = "info", ttl = 4000) => {
    const id = Math.random().toString(36).slice(2);
    setToasts((t) => [...t, { id, msg, kind }]);
    setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), ttl);
  }, []);
  const api = {
    info: (m) => push(m, "info"),
    success: (m) => push(m, "success"),
    error: (m) => push(m, "error"),
  };
  return (
    <ToastCtx.Provider value={api}>
      {children}
      <div style={{
        position: "fixed", top: 16, right: 16, zIndex: 9999,
        display: "flex", flexDirection: "column", gap: 8, pointerEvents: "none",
      }}>
        {toasts.map((t) => (
          <div key={t.id} role="status" style={{
            padding: "10px 14px", borderRadius: 8, color: "#fff",
            background: t.kind === "error" ? "#b91c1c" : t.kind === "success" ? "#15803d" : "#1f2937",
            boxShadow: "0 4px 12px rgba(0,0,0,.2)", minWidth: 220, pointerEvents: "auto",
          }}>{t.msg}</div>
        ))}
      </div>
    </ToastCtx.Provider>
  );
}

export function useToast() {
  const ctx = useContext(ToastCtx);
  if (!ctx) throw new Error("useToast must be used inside <ToastProvider>");
  return ctx;
}
