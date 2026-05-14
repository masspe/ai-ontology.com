// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { NavLink } from "react-router-dom";

interface NavItem {
  to: string;
  label: string;
  icon: string;
  end?: boolean;
}

const NAV: NavItem[] = [
  { to: "/", label: "Dashboard", icon: "▦", end: true },
  { to: "/builder", label: "Ontology Builder", icon: "✦" },
  { to: "/files", label: "Files", icon: "▤" },
  { to: "/graph", label: "Graph View", icon: "◈" },
  { to: "/queries", label: "Queries", icon: "✎" },
  { to: "/settings", label: "Settings", icon: "⚙" },
];

interface Props {
  collapsed: boolean;
  onToggle: () => void;
}

export default function Sidebar({ collapsed, onToggle }: Props) {
  return (
    <aside className={`sidebar${collapsed ? " collapsed" : ""}`}>
      <div className="sidebar-brand">
        <div className="sidebar-logo">A</div>
        <span className="sidebar-brand-text">AI Ontology Studio</span>
      </div>

      <div className="sidebar-section-title">Workspace</div>
      <nav>
        {NAV.map((item) => (
          <NavLink
            key={item.to}
            to={item.to}
            end={item.end}
            className={({ isActive }) => `sidebar-item${isActive ? " active" : ""}`}
            title={collapsed ? item.label : undefined}
          >
            <span className="sidebar-item-icon" aria-hidden>{item.icon}</span>
            <span className="sidebar-item-label">{item.label}</span>
          </NavLink>
        ))}
      </nav>

      <div className="sidebar-footer">
        <div className="workspace-selector">
          <span>📁</span>
          <span>Default workspace</span>
        </div>
        <button className="btn-ghost" style={{ width: "100%", justifyContent: "flex-start" }} onClick={onToggle}>
          <span className="sidebar-item-icon">{collapsed ? "›" : "‹"}</span>
          {!collapsed && <span style={{ marginLeft: 8 }}>Collapse</span>}
        </button>
      </div>
    </aside>
  );
}
