// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
import { useState } from "react";
import { Link, useLocation, useNavigate } from "react-router-dom";
import { msBE } from "../lib/msBE";
import { useToast } from "../components/Toast.jsx";

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

function safeNext(raw) {
  if (!raw || !raw.startsWith("/") || raw.startsWith("//")) return "/";
  return raw;
}

function scorePassword(p) {
  let s = 0;
  if (p.length >= 8) s++;
  if (/[A-Z]/.test(p)) s++;
  if (/[a-z]/.test(p)) s++;
  if (/\d/.test(p)) s++;
  if (/[^A-Za-z0-9]/.test(p)) s++;
  return s; // 0..5
}

export default function Signup() {
  const toast = useToast();
  const nav = useNavigate();
  const loc = useLocation();
  const next = safeNext(new URLSearchParams(loc.search).get("next"));

  const [form, setForm] = useState({ name: "", email: "", password: "", confirm: "" });
  const [loading, setLoading] = useState(false);
  const [errors, setErrors] = useState({});

  const set = (k) => (e) => setForm((f) => ({ ...f, [k]: e.target.value }));

  function validate() {
    const e = {};
    if (!form.name.trim()) e.name = "Nom requis";
    if (!EMAIL_RE.test(form.email)) e.email = "Email invalide";
    if (scorePassword(form.password) < 3) e.password = "Mot de passe trop faible (8+ car., majuscule, chiffre)";
    if (form.password !== form.confirm) e.confirm = "Les mots de passe ne correspondent pas";
    setErrors(e);
    return Object.keys(e).length === 0;
  }

  async function onSubmit(ev) {
    ev.preventDefault();
    if (!validate() || loading) return;
    setLoading(true);
    try {
      await msBE.auth.signup({
        email: form.email.trim().toLowerCase(),
        password: form.password,
        name: form.name.trim(),
      });
      toast.success("Compte créé");
      nav(next, { replace: true });
    } catch (err) {
      toast.error(err.message || "Échec de l'inscription");
    } finally {
      setLoading(false);
    }
  }

  const strength = scorePassword(form.password);
  return (
    <div style={wrap}>
      <form onSubmit={onSubmit} style={card} noValidate>
        <h1 style={{ marginTop: 0 }}>Créer un compte</h1>

        <label style={label}>Nom
          <input value={form.name} onChange={set("name")} autoComplete="name" style={input}
            aria-invalid={!!errors.name} />
          {errors.name && <span style={errStyle}>{errors.name}</span>}
        </label>

        <label style={label}>Email
          <input type="email" value={form.email} onChange={set("email")} autoComplete="email" style={input}
            aria-invalid={!!errors.email} />
          {errors.email && <span style={errStyle}>{errors.email}</span>}
        </label>

        <label style={label}>Mot de passe
          <input type="password" value={form.password} onChange={set("password")}
            autoComplete="new-password" style={input} aria-invalid={!!errors.password} />
          <div style={{ display: "flex", gap: 4, marginTop: 4 }}>
            {[1,2,3,4,5].map(i => (
              <div key={i} style={{
                flex: 1, height: 4, borderRadius: 2,
                background: i <= strength ? (strength < 3 ? "#dc2626" : strength < 4 ? "#f59e0b" : "#16a34a") : "#e5e7eb",
              }} />
            ))}
          </div>
          {errors.password && <span style={errStyle}>{errors.password}</span>}
        </label>

        <label style={label}>Confirmer
          <input type="password" value={form.confirm} onChange={set("confirm")}
            autoComplete="new-password" style={input} aria-invalid={!!errors.confirm} />
          {errors.confirm && <span style={errStyle}>{errors.confirm}</span>}
        </label>

        <button type="submit" disabled={loading} style={primaryBtn}>
          {loading ? "Création…" : "Créer mon compte"}
        </button>

        <div style={divider}><span>ou</span></div>

        <a href={msBE.auth.googleLoginUrl(next)} style={oauthBtn("#fff", "#111", "#ddd")}>
          S'inscrire avec Google
        </a>
        <a href={msBE.auth.microsoftLoginUrl(next)} style={oauthBtn("#2f2f2f", "#fff", "#2f2f2f")}>
          S'inscrire avec Microsoft
        </a>

        <p style={{ marginTop: 16 }}>
          Déjà inscrit ? <Link to={`/login?next=${encodeURIComponent(next)}`}>Se connecter</Link>
        </p>
      </form>
    </div>
  );
}

const wrap = { minHeight: "100vh", display: "grid", placeItems: "center", background: "#f7f7f8", padding: 16 };
const card = { width: "100%", maxWidth: 400, background: "#fff", padding: 24, borderRadius: 12, boxShadow: "0 6px 24px rgba(0,0,0,.08)", display: "flex", flexDirection: "column", gap: 12 };
const label = { display: "flex", flexDirection: "column", gap: 4, fontSize: 14 };
const input = { padding: "10px 12px", border: "1px solid #d4d4d8", borderRadius: 8, fontSize: 14 };
const errStyle = { color: "#b91c1c", fontSize: 12 };
const primaryBtn = { padding: "10px 14px", background: "#111", color: "#fff", border: 0, borderRadius: 8, cursor: "pointer", fontWeight: 600 };
const divider = { textAlign: "center", color: "#6b7280", fontSize: 12, margin: "8px 0" };
const oauthBtn = (bg, fg, bd) => ({ display: "block", textAlign: "center", padding: "10px 14px", background: bg, color: fg, border: `1px solid ${bd}`, borderRadius: 8, textDecoration: "none", fontWeight: 600 });
