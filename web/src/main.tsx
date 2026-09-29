// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
// 
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A.. See LICENSE and LICENSE-COMMERCIAL.md.

import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";
import { installLogCapture } from "./lib/logBuffer";

installLogCapture();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
