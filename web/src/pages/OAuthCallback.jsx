// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Consumes the token from the URL fragment (#token=...&next=...). Using the
// fragment instead of the query string keeps the JWT out of Referer headers
// and out of any server-side access logs the SPA's host might keep.
import { useEffect } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { msBE } from "../lib/msBE";
import { useToast } from "../components/Toast.jsx";

function safeNext(raw) {
  if (!raw || !raw.startsWith("/") || raw.startsWith("//")) return "/";
  return raw;
}

function parseFragment(hash) {
  // hash starts with "#" — strip it before parsing.
  const s = hash && hash.startsWith("#") ? hash.slice(1) : hash || "";
  return new URLSearchParams(s);
}

export default function OAuthCallback() {
  const nav = useNavigate();
  const loc = useLocation();
  const toast = useToast();

  useEffect(() => {
    // Accept either fragment (preferred) or query (fallback).
    const frag = parseFragment(loc.hash);
    const qs = new URLSearchParams(loc.search);
    const token = frag.get("token") || qs.get("token");
    const err = frag.get("error") || qs.get("error");
    const next = safeNext(frag.get("next") || qs.get("next"));

    // Wipe the fragment from the address bar so the token isn't kept around
    // in browser history.
    if (typeof window !== "undefined" && window.location.hash) {
      try {
        window.history.replaceState(null, "", window.location.pathname);
      } catch { /* ignore */ }
    }

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
  }, [loc.search, loc.hash, nav, toast]);

  return <div style={{ padding: 24 }}>Finalisation de la connexion…</div>;
}
