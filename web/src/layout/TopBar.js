import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
import { useNavigate } from "react-router-dom";
import { useState } from "react";
export default function TopBar() {
    const nav = useNavigate();
    const [q, setQ] = useState("");
    const onSubmit = (e) => {
        e.preventDefault();
        if (!q.trim())
            return;
        // Treat top-bar search as a quick Ask: hop to the Builder page with a hint.
        // For now, navigate to Queries which has a single-shot ask box.
        nav(`/queries?q=${encodeURIComponent(q)}`);
    };
    return (_jsxs("header", { className: "topbar", children: [_jsxs("form", { className: "topbar-search", onSubmit: onSubmit, children: [_jsx("span", { className: "topbar-search-icon", children: "\u2315" }), _jsx("input", { placeholder: "Search concepts, queries, files\u2026", value: q, onChange: (e) => setQ(e.target.value) })] }), _jsxs("div", { className: "topbar-actions", children: [_jsx("button", { className: "icon-btn", title: "Help", children: "?" }), _jsx("button", { className: "icon-btn", title: "Notifications", children: "\u2691" }), _jsx("div", { className: "avatar", title: "Account", children: "U" })] })] }));
}
