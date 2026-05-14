import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";
import Layout from "./layout/Layout";
import Dashboard from "./pages/Dashboard";
import OntologyBuilder from "./pages/OntologyBuilder";
import Files from "./pages/Files";
import GraphView from "./pages/GraphView";
import Concepts from "./pages/Concepts";
import Rules from "./pages/Rules";
import Queries from "./pages/Queries";
import Actions from "./pages/Actions";
import Settings from "./pages/Settings";
// @ts-expect-error JSX module
import Login from "./pages/Login.jsx";
// @ts-expect-error JSX module
import Signup from "./pages/Signup.jsx";
// @ts-expect-error JSX module
import OAuthCallback from "./pages/OAuthCallback.jsx";
// @ts-expect-error JSX module
import ProtectedRoute from "./routes/ProtectedRoute.jsx";
// @ts-expect-error JSX module
import { ToastProvider } from "./components/Toast.jsx";
export default function App() {
    return (_jsx(BrowserRouter, { children: _jsx(ToastProvider, { children: _jsxs(Routes, { children: [_jsx(Route, { path: "/login", element: _jsx(Login, {}) }), _jsx(Route, { path: "/signup", element: _jsx(Signup, {}) }), _jsx(Route, { path: "/oauth/callback", element: _jsx(OAuthCallback, {}) }), _jsxs(Route, { element: _jsx(ProtectedRoute, { children: _jsx(Layout, {}) }), children: [_jsx(Route, { index: true, element: _jsx(Dashboard, {}) }), _jsx(Route, { path: "builder", element: _jsx(OntologyBuilder, {}) }), _jsx(Route, { path: "files", element: _jsx(Files, {}) }), _jsx(Route, { path: "graph", element: _jsx(GraphView, {}) }), _jsx(Route, { path: "concepts", element: _jsx(Concepts, {}) }), _jsx(Route, { path: "rules", element: _jsx(Rules, {}) }), _jsx(Route, { path: "queries", element: _jsx(Queries, {}) }), _jsx(Route, { path: "actions", element: _jsx(Actions, {}) }), _jsx(Route, { path: "settings", element: _jsx(Settings, {}) }), _jsx(Route, { path: "*", element: _jsx(Navigate, { to: "/", replace: true }) })] })] }) }) }));
}
