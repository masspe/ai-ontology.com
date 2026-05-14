import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
import { NavLink } from "react-router-dom";
const NAV = [
    { to: "/", label: "Dashboard", icon: "▦", end: true },
    { to: "/builder", label: "Ontology Builder", icon: "✦" },
    { to: "/files", label: "Files", icon: "▤" },
    { to: "/graph", label: "Graph View", icon: "◈" },
    { to: "/queries", label: "Queries", icon: "✎" },
    { to: "/settings", label: "Settings", icon: "⚙" },
];
export default function Sidebar({ collapsed, onToggle }) {
    return (_jsxs("aside", { className: `sidebar${collapsed ? " collapsed" : ""}`, children: [_jsxs("div", { className: "sidebar-brand", children: [_jsx("div", { className: "sidebar-logo", children: "A" }), _jsx("span", { className: "sidebar-brand-text", children: "AI Ontology Studio" })] }), _jsx("div", { className: "sidebar-section-title", children: "Workspace" }), _jsx("nav", { children: NAV.map((item) => (_jsxs(NavLink, { to: item.to, end: item.end, className: ({ isActive }) => `sidebar-item${isActive ? " active" : ""}`, title: collapsed ? item.label : undefined, children: [_jsx("span", { className: "sidebar-item-icon", "aria-hidden": true, children: item.icon }), _jsx("span", { className: "sidebar-item-label", children: item.label })] }, item.to))) }), _jsxs("div", { className: "sidebar-footer", children: [_jsxs("div", { className: "workspace-selector", children: [_jsx("span", { children: "\uD83D\uDCC1" }), _jsx("span", { children: "Default workspace" })] }), _jsxs("button", { className: "btn-ghost", style: { width: "100%", justifyContent: "flex-start" }, onClick: onToggle, children: [_jsx("span", { className: "sidebar-item-icon", children: collapsed ? "›" : "‹" }), !collapsed && _jsx("span", { style: { marginLeft: 8 }, children: "Collapse" })] })] })] }));
}
