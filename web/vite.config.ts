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
export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
  },
});
