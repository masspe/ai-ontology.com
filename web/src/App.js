import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";
import Layout from "./layout/Layout";
import Dashboard from "./pages/Dashboard";
import OntologyBuilder from "./pages/OntologyBuilder";
import Files from "./pages/Files";
import GraphView from "./pages/GraphView";
import Queries from "./pages/Queries";
import Settings from "./pages/Settings";
export default function App() {
    return (_jsx(BrowserRouter, { children: _jsx(Routes, { children: _jsxs(Route, { element: _jsx(Layout, {}), children: [_jsx(Route, { index: true, element: _jsx(Dashboard, {}) }), _jsx(Route, { path: "builder", element: _jsx(OntologyBuilder, {}) }), _jsx(Route, { path: "files", element: _jsx(Files, {}) }), _jsx(Route, { path: "graph", element: _jsx(GraphView, {}) }), _jsx(Route, { path: "queries", element: _jsx(Queries, {}) }), _jsx(Route, { path: "settings", element: _jsx(Settings, {}) }), _jsx(Route, { path: "*", element: _jsx(Navigate, { to: "/", replace: true }) })] }) }) }));
}
