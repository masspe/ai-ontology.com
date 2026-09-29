// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.

import { NavLink } from "react-router-dom";
import { NAV_GROUPS } from "./nav";

interface Props {
  collapsed: boolean;
  onToggle: () => void;
}

/// The menu follows the order of the work (prepare, explore, automate);
/// every entry carries a `data-tour` anchor for the guide.
export default function Sidebar({ collapsed, onToggle }: Props) {
  return (
    <aside className={`sidebar${collapsed ? " collapsed" : ""}`}>
      <div className="sidebar-brand">
        <div className="sidebar-logo">A</div>
        <span className="sidebar-brand-text">AI Ontology Studio</span>
      </div>

      {NAV_GROUPS.map((group) => (
        <nav key={group.title} aria-label={group.title}>
          <div className="sidebar-section-title">{group.title}</div>
          {group.items.map((item) => (
            <NavLink
              key={item.to}
              to={item.to}
              end={item.end}
              data-tour={item.tour}
              className={({ isActive }) => `sidebar-item${isActive ? " active" : ""}`}
              title={collapsed ? item.label : undefined}
            >
              <span className="sidebar-item-icon">{item.icon}</span>
              <span className="sidebar-item-label">{item.label}</span>
            </NavLink>
          ))}
        </nav>
      ))}

      <div className="sidebar-footer">
        <button
          className="btn-ghost"
          style={{ width: "100%", justifyContent: "flex-start" }}
          onClick={onToggle}
          aria-label={collapsed ? "Déployer le menu" : "Réduire le menu"}
        >
          <span className="sidebar-item-icon">{collapsed ? "›" : "‹"}</span>
          {!collapsed && <span style={{ marginLeft: 8 }}>Réduire</span>}
        </button>
      </div>
    </aside>
  );
}
