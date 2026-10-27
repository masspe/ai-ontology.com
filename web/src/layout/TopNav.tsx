// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The four entries: a page each for
// Accueil and Réglages, a menu for Importer and Explorer. Native
// <details> menus: no state; Escape and the focus leaving the menu close it.

import { NavLink, useLocation } from "react-router-dom";
import { NAV_GROUPS, type NavGroup } from "./nav";
import { t } from "../lib/i18n";

function closeMenu(e: React.MouseEvent) {
  (e.currentTarget as HTMLElement).closest("details")?.removeAttribute("open");
}

function Group({ group }: { group: NavGroup }) {
  const { pathname } = useLocation();
  if (group.items.length === 1) {
    const item = group.items[0]!;
    return (
      <NavLink
        to={item.to}
        end={item.end}
        data-tour={item.tour}
        className={({ isActive }) => `topnav-link${isActive ? " active" : ""}`}
      >
        <span className="topnav-icon">{item.icon}</span>
        <span className="topnav-label">{group.title}</span>
      </NavLink>
    );
  }
  const active = group.items.some((i) => pathname === i.to || pathname.startsWith(i.to + "/"));
  return (
    <details
      className="topnav-group"
      onKeyDown={(e) => {
        if (e.key === "Escape") e.currentTarget.removeAttribute("open");
      }}
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) e.currentTarget.removeAttribute("open");
      }}
    >
      <summary className={`topnav-link${active ? " active" : ""}`} data-tour={group.key}>
        <span className="topnav-label">{group.title}</span> <span aria-hidden>▾</span>
      </summary>
      <div className="topnav-menu" aria-label={group.title}>
        {group.items.map((item) => (
          <NavLink
            key={item.to}
            to={item.to}
            end={item.end}
            onClick={closeMenu}
            className={({ isActive }) => `topnav-item${isActive ? " active" : ""}`}
          >
            <span className="topnav-icon">{item.icon}</span>
            {item.label}
          </NavLink>
        ))}
      </div>
    </details>
  );
}

export default function TopNav() {
  return (
    <nav className="topnav" aria-label={t("Navigation")}>
      <NavLink to="/" className="topnav-brand" aria-label="AI Ontology Studio">
        <span className="brand-mark" aria-hidden>
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
            <circle cx="6" cy="12" r="2.5" />
            <circle cx="18" cy="6" r="2.5" />
            <circle cx="18" cy="18" r="2.5" />
            <path d="m8.2 10.8 7.6-3.6M8.2 13.2l7.6 3.6" />
          </svg>
        </span>
        <span className="brand-word">AI Ontology</span>
      </NavLink>
      {NAV_GROUPS.map((g) => (
        <Group key={g.key} group={g} />
      ))}
    </nav>
  );
}
