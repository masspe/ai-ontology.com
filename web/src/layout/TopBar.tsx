// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.

import { useNavigate } from "react-router-dom";
import { useEffect, useRef, useState } from "react";
import FeedbackModal from "../components/FeedbackModal";
import { openTour } from "../components/Tour";
import TopNav from "./TopNav";
// @ts-expect-error JS module without types (shared with the auth pages)
import { msBE } from "../lib/msBE";
import { getLang, setLang, t } from "../lib/i18n";

function initials(name: string | undefined, email: string | undefined): string {
  const src = (name || email || "").trim();
  if (!src) return "U";
  const parts = src.split(/[\s.@_-]+/).filter(Boolean);
  const two = parts.length >= 2 ? parts[0]![0]! + parts[1]![0]! : src.slice(0, 2);
  return two.toUpperCase();
}

export default function TopBar() {
  const nav = useNavigate();
  const [q, setQ] = useState("");
  const [feedbackOpen, setFeedbackOpen] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  const user = msBE.auth.currentUser() as { name?: string; email?: string } | null;

  const onSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!q.trim()) return;
    nav(`/queries?q=${encodeURIComponent(q)}`);
  };

  // The account menu closes on a click elsewhere or on Escape.
  useEffect(() => {
    if (!menuOpen) return;
    const away = (e: MouseEvent) => {
      if (!menuRef.current?.contains(e.target as Node)) setMenuOpen(false);
    };
    const esc = (e: KeyboardEvent) => {
      if (e.key === "Escape") setMenuOpen(false);
    };
    document.addEventListener("mousedown", away);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("mousedown", away);
      document.removeEventListener("keydown", esc);
    };
  }, [menuOpen]);

  const logout = async () => {
    setMenuOpen(false);
    await msBE.auth.logout();
    nav("/login", { replace: true });
  };

  return (
    <header className="topbar">
      <TopNav />
      <form className="topbar-search" onSubmit={onSubmit} data-tour="search">
        <span className="topbar-search-icon">⌕</span>
        <input
          placeholder={t("Rechercher une fiche, poser une question…")}
          value={q}
          onChange={(e) => setQ(e.target.value)}
        />
      </form>
      <div className="topbar-actions">
        <button
          className="btn btn-outline"
          onClick={() => setFeedbackOpen(true)}
          title={t("Envoyer un feedback")}
          data-tour="feedback"
          style={{ display: "inline-flex", alignItems: "center", gap: 6 }}
        >
          💬 Feedback
        </button>
        <button
          className="btn-ghost lang-toggle"
          title={t("Changer de langue")}
          aria-label={t("Changer de langue")}
          onClick={() => {
            if (window.confirm(t("Changer de langue recharge la page : ce qui n'est pas enregistré sera perdu. Continuer ?"))) {
              setLang(getLang() === "fr" ? "en" : "fr");
            }
          }}
        >
          {getLang() === "fr" ? "EN" : "FR"}
        </button>
        <button className="icon-btn" title={t("Guide")} aria-label={t("Guide")} data-tour="help" onClick={openTour}>
          ?
        </button>
        <div className="account" ref={menuRef} data-tour="account">
          <button
            className="avatar"
            title={t("Compte")}
            aria-label={t("Compte")}
            aria-expanded={menuOpen}
            onClick={() => setMenuOpen((v) => !v)}
          >
            {initials(user?.name, user?.email)}
          </button>
          {menuOpen && (
            <div className="account-menu" role="menu">
              <div className="account-who">
                <strong>{user?.name || t("Utilisateur")}</strong>
                {user?.email && <span className="muted">{user.email}</span>}
              </div>
              <button className="btn-ghost" role="menuitem" onClick={() => { setMenuOpen(false); nav("/settings"); }}>
                {t("Paramètres")}
              </button>
              <button className="btn-ghost" role="menuitem" onClick={() => void logout()}>
                {t("Se déconnecter")}
              </button>
            </div>
          )}
        </div>
      </div>
      <FeedbackModal open={feedbackOpen} onClose={() => setFeedbackOpen(false)} />
    </header>
  );
}
