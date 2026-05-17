// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
// 
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// API base URL the React app talks to. Override at build time with
// VITE_API_BASE — useful when serving the bundle from a different host
// than the ontology server.
// Vite dev server proxies all API calls to the ontology backend so the
// React app can use same-origin URLs ("/stats", "/ask", …) without CORS
// or VITE_API_BASE config. Override the target with ONTOLOGY_API_URL.
const API_TARGET = process.env.ONTOLOGY_API_URL || "http://127.0.0.1:5000";
const AUTH_TARGET = process.env.AUTH_API_URL || "http://127.0.0.1:4000";

const API_PATHS = [
  "/healthz",
  "/stats",
  "/metrics",
  "/ontology",
  "/concepts",
  "/relations",
  "/retrieve",
  "/subgraph",
  "/ask",
  "/path",
  "/compact",
  "/upload",
  "/export",
  "/files",
  "/queries",
  "/settings",
];

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      ...Object.fromEntries(
        API_PATHS.map((p) => [p, { target: API_TARGET, changeOrigin: true, ws: true }]),
      ),
      "/auth": { target: AUTH_TARGET, changeOrigin: true, ws: true },
    },
  },
});
