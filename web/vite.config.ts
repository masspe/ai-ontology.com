// SPDX-License-Identifier: LGPL-3.0-or-later
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
// 
// This file is part of ai-ontology.com.
// It is licensed under the GNU Lesser General Public License v3.0
// or (at your option) any later version. See LICENSE and LICENSE.GPL.

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
