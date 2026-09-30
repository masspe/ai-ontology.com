// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.

import { Outlet } from "react-router-dom";
import TopBar from "./TopBar";
import Tour from "../components/Tour";

export default function Layout() {
  return (
    <div className="app-shell">
      <TopBar />
      <main className="content">
        <Outlet />
      </main>
      <Tour />
    </div>
  );
}
