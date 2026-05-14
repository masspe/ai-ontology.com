// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
import { useEffect, useState } from "react";
import { Navigate, useLocation } from "react-router-dom";
import { msBE } from "../lib/msBE";

export default function ProtectedRoute({ children }) {
  const loc = useLocation();
  const [state, setState] = useState(
    msBE.auth.isAuthenticated() ? "checking" : "anon",
  );

  useEffect(() => {
    let alive = true;
    if (state !== "checking") return;
    msBE.auth.me()
      .then((u) => { if (alive) setState(u ? "ok" : "anon"); })
      .catch(() => { if (alive) setState("anon"); });
    return () => { alive = false; };
  }, [state]);

  if (state === "checking") {
    return <div style={{ padding: 24 }}>Loading…</div>;
  }
  if (state === "anon") {
    const next = encodeURIComponent(loc.pathname + loc.search);
    return <Navigate to={`/login?next=${next}`} replace />;
  }
  return children;
}
