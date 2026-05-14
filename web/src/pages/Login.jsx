// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
import { useState } from "react";
import { Link, useLocation, useNavigate } from "react-router-dom";
import { msBE } from "../lib/msBE";
import { useToast } from "../components/Toast.jsx";

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

function safeNext(raw) {
  if (!raw) return "/";
  try {
    // Only allow same-origin relative paths.
    if (!raw.startsWith("/") || raw.startsWith("//")) return "/";
    return raw;
  } catch { return "/"; }
}

export default function Login() {
  const toast = useToast();
  const nav = useNavigate();
  const loc = useLocation();
  const next = safeNext(new URLSearchParams(loc.search).get("next"));

  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [loading, setLoading] = useState(false);
  const [errors, setErrors] = useState({});

  function validate() {
    const e = {};
    if (!EMAIL_RE.test(email)) e.email = "Email invalide";
    if (!password) e.password = "Mot de passe requis";
    setErrors(e);
    return Object.keys(e).length === 0;
  }

  async function onSubmit(ev) {
    ev.preventDefault();
    if (!validate() || loading) return;
    setLoading(true);
    try {
      await msBE.auth.login({ email: email.trim(), password });
      toast.success("Connexion réussie");
      nav(next, { replace: true });
    } catch (err) {
      toast.error(err.message || "Échec de connexion");
    } finally {
      setLoading(false);
    }
  }

  return (
    <div style={wrap}>
      <form onSubmit={onSubmit} style={card} noValidate>
        <h1 style={{ marginTop: 0 }}>Se connecter</h1>

        <label style={label}>Email
          <input type="email" autoComplete="email" value={email}
            onChange={(e) => setEmail(e.target.value)} style={input}
            aria-invalid={!!errors.email} />
          {errors.email && <span style={errStyle}>{errors.email}</span>}
        </label>

        <label style={label}>Mot de passe
          <input type="password" autoComplete="current-password" value={password}
            onChange={(e) => setPassword(e.target.value)} style={input}
            aria-invalid={!!errors.password} />
          {errors.password && <span style={errStyle}>{errors.password}</span>}
        </label>

        <button type="submit" disabled={loading} style={primaryBtn}>
          {loading ? "Connexion…" : "Se connecter"}
        </button>

        <div style={divider}><span>ou</span></div>

        <a href={msBE.auth.googleLoginUrl(next)} style={oauthBtn("#fff", "#111", "#ddd")}>
          Continuer avec Google
        </a>
        <a href={msBE.auth.microsoftLoginUrl(next)} style={oauthBtn("#2f2f2f", "#fff", "#2f2f2f")}>
          Continuer avec Microsoft
        </a>

        <p style={{ marginTop: 16 }}>
          Pas de compte ? <Link to={`/signup?next=${encodeURIComponent(next)}`}>Créer un compte</Link>
        </p>
      </form>
    </div>
  );
}

const wrap = { minHeight: "100vh", display: "grid", placeItems: "center", background: "#f7f7f8", padding: 16 };
const card = { width: "100%", maxWidth: 380, background: "#fff", padding: 24, borderRadius: 12, boxShadow: "0 6px 24px rgba(0,0,0,.08)", display: "flex", flexDirection: "column", gap: 12 };
const label = { display: "flex", flexDirection: "column", gap: 4, fontSize: 14 };
const input = { padding: "10px 12px", border: "1px solid #d4d4d8", borderRadius: 8, fontSize: 14 };
const errStyle = { color: "#b91c1c", fontSize: 12 };
const primaryBtn = { padding: "10px 14px", background: "#111", color: "#fff", border: 0, borderRadius: 8, cursor: "pointer", fontWeight: 600 };
const divider = { textAlign: "center", color: "#6b7280", fontSize: 12, margin: "8px 0" };
const oauthBtn = (bg, fg, bd) => ({ display: "block", textAlign: "center", padding: "10px 14px", background: bg, color: fg, border: `1px solid ${bd}`, borderRadius: 8, textDecoration: "none", fontWeight: 600 });
