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
  return (
    <BrowserRouter>
      <ToastProvider>
        <Routes>
          <Route path="/login" element={<Login />} />
          <Route path="/signup" element={<Signup />} />
          <Route path="/oauth/callback" element={<OAuthCallback />} />
          <Route element={<ProtectedRoute><Layout /></ProtectedRoute>}>
            <Route index element={<Dashboard />} />
            <Route path="builder" element={<OntologyBuilder />} />
            <Route path="files" element={<Files />} />
            <Route path="graph" element={<GraphView />} />
            <Route path="queries" element={<Queries />} />
            <Route path="settings" element={<Settings />} />
            <Route path="*" element={<Navigate to="/" replace />} />
          </Route>
        </Routes>
      </ToastProvider>
    </BrowserRouter>
  );
}
