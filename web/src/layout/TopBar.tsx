// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { useNavigate } from "react-router-dom";
import { useState } from "react";
import FeedbackModal from "../components/FeedbackModal";

export default function TopBar() {
  const nav = useNavigate();
  const [q, setQ] = useState("");
  const [feedbackOpen, setFeedbackOpen] = useState(false);

  const onSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!q.trim()) return;
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
        <button
          className="btn btn-outline"
          onClick={() => setFeedbackOpen(true)}
          title="Envoyer un feedback"
          style={{ display: "inline-flex", alignItems: "center", gap: 6 }}
        >
          💬 Feedback
        </button>
        <button className="icon-btn" title="Help">?</button>
        <button className="icon-btn" title="Notifications">⚑</button>
        <div className="avatar" title="Account">U</div>
      </div>
      <FeedbackModal open={feedbackOpen} onClose={() => setFeedbackOpen(false)} />
    </header>
  );
}
