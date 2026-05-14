import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
import { useState } from "react";
import { Outlet } from "react-router-dom";
import Sidebar from "./Sidebar";
import TopBar from "./TopBar";
export default function Layout() {
    const [collapsed, setCollapsed] = useState(false);
    return (_jsxs("div", { className: `app-shell${collapsed ? " collapsed" : ""}`, children: [_jsx(Sidebar, { collapsed: collapsed, onToggle: () => setCollapsed((v) => !v) }), _jsxs("div", { className: "main-area", children: [_jsx(TopBar, {}), _jsx("main", { className: "content", children: _jsx(Outlet, {}) })] })] }));
}
