// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { useNavigate } from "react-router-dom";
import { useState } from "react";

export default function TopBar() {
  const nav = useNavigate();
  const [q, setQ] = useState("");

  const onSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!q.trim()) return;
    // Treat top-bar search as a quick Ask: hop to the Builder page with a hint.
    // For now, navigate to Queries which has a single-shot ask box.
    nav(`/queries?q=${encodeURIComponent(q)}`);
  };

  return (
    <header className="topbar">
      <form className="topbar-search" onSubmit={onSubmit}>
        <span className="topbar-search-icon">⌕</span>
        <input
          placeholder="Search concepts, queries, files…"
          value={q}
          onChange={(e) => setQ(e.target.value)}
        />
      </form>
      <div className="topbar-actions">
        <button className="icon-btn" title="Help">?</button>
        <button className="icon-btn" title="Notifications">⚑</button>
        <div className="avatar" title="Account">U</div>
      </div>
    </header>
  );
}
