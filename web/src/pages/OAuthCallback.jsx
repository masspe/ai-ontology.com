// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Consumes ?token=...&next=... after the OAuth provider redirects through the backend.
import { useEffect } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { msBE } from "../lib/msBE";
import { useToast } from "../components/Toast.jsx";

function safeNext(raw) {
  if (!raw || !raw.startsWith("/") || raw.startsWith("//")) return "/";
  return raw;
}

export default function OAuthCallback() {
  const nav = useNavigate();
  const loc = useLocation();
  const toast = useToast();

  useEffect(() => {
    const p = new URLSearchParams(loc.search);
    const token = p.get("token");
    const err = p.get("error");
    const next = safeNext(p.get("next"));
    if (err) {
      toast.error(`OAuth: ${err}`);
      nav("/login", { replace: true });
      return;
    }
    if (msBE.auth.consumeOAuthToken(token)) {
      msBE.auth.me().finally(() => nav(next, { replace: true }));
    } else {
      toast.error("Jeton OAuth manquant");
      nav("/login", { replace: true });
    }
  }, [loc.search, nav, toast]);

  return <div style={{ padding: 24 }}>Finalisation de la connexion…</div>;
}
