// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The four entries (ROADMAP §3.9 lot B, tranche 2): a page each for
// Accueil and Réglages, a menu for Importer and Explorer. Native
// <details> menus: no state; Escape and the focus leaving the menu close it.

import { NavLink, useLocation } from "react-router-dom";
import { NAV_GROUPS, type NavGroup } from "./nav";

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
    <nav className="topnav" aria-label="Navigation">
      <NavLink to="/" className="topnav-brand" aria-label="AI Ontology Studio">
        <span className="sidebar-logo">A</span>
      </NavLink>
      {NAV_GROUPS.map((g) => (
        <Group key={g.key} group={g} />
      ))}
    </nav>
  );
}
