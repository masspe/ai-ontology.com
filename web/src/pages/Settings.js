import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
import { useEffect, useState } from "react";
import Card from "../components/Card";
import { apiBase, apiToken, getSettings, patchSettings } from "../api";
export default function Settings() {
    const [s, setS] = useState(null);
    const [error, setError] = useState(null);
    const [info, setInfo] = useState(null);
    const [base, setBase] = useState(apiBase());
    const [token, setToken] = useState(apiToken() ?? "");
    useEffect(() => {
        getSettings().then(setS).catch((e) => setError(e instanceof Error ? e.message : String(e)));
    }, []);
    const apply = async (patch) => {
        setError(null);
        setInfo(null);
        try {
            const next = await patchSettings(patch);
            setS(next);
            setInfo("Settings saved.");
        }
        catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
    };
    const saveConnection = () => {
        if (typeof window === "undefined")
            return;
        if (base.trim())
            window.localStorage.setItem("ontology.apiBase", base.trim().replace(/\/$/, ""));
        else
            window.localStorage.removeItem("ontology.apiBase");
        if (token.trim())
            window.localStorage.setItem("ontology.apiToken", token.trim());
        else
            window.localStorage.removeItem("ontology.apiToken");
        setInfo("Connection saved. Refresh other tabs to pick up the new base URL.");
    };
    return (_jsxs(_Fragment, { children: [_jsx("div", { className: "page-header", children: _jsxs("div", { children: [_jsx("h1", { className: "page-title", children: "Settings" }), _jsx("p", { className: "page-subtitle", children: "Retrieval defaults, UI preferences and server connection." })] }) }), error && _jsx("div", { className: "error-banner", children: error }), info && _jsx("div", { className: "success-banner", children: info }), _jsxs("div", { className: "grid grid-2", style: { alignItems: "start", marginBottom: 16 }, children: [_jsx(Card, { title: "Retrieval defaults", children: !s ? (_jsx("div", { className: "empty", children: "Loading\u2026" })) : (_jsxs(_Fragment, { children: [_jsxs("div", { className: "setting-row", children: [_jsxs("div", { className: "meta", children: [_jsx("strong", { children: "Top-K" }), "Number of seed concepts retrieved per query."] }), _jsx("input", { type: "number", min: 1, max: 50, value: s.retrieval.top_k, onChange: (e) => apply({ retrieval: { top_k: Number(e.target.value) } }) })] }), _jsxs("div", { className: "setting-row", children: [_jsxs("div", { className: "meta", children: [_jsx("strong", { children: "Lexical weight" }), "0 = vector-only, 1 = BM25-only."] }), _jsx("input", { type: "number", step: 0.1, min: 0, max: 1, value: s.retrieval.lexical_weight, onChange: (e) => apply({ retrieval: { lexical_weight: Number(e.target.value) } }) })] }), _jsxs("div", { className: "setting-row", children: [_jsxs("div", { className: "meta", children: [_jsx("strong", { children: "Expansion depth" }), "Hops added to each seed when traversing."] }), _jsx("input", { type: "number", min: 0, max: 5, value: s.retrieval.expansion_depth, onChange: (e) => apply({ retrieval: { expansion_depth: Number(e.target.value) } }) })] })] })) }), _jsx(Card, { title: "UI preferences", children: !s ? (_jsx("div", { className: "empty", children: "Loading\u2026" })) : (_jsxs(_Fragment, { children: [_jsxs("div", { className: "setting-row", children: [_jsxs("div", { className: "meta", children: [_jsx("strong", { children: "Theme" }), "Color scheme (light only at the moment)."] }), _jsxs("select", { value: s.ui.theme, onChange: (e) => apply({ ui: { theme: e.target.value } }), children: [_jsx("option", { value: "light", children: "Light" }), _jsx("option", { value: "dark", children: "Dark" })] })] }), _jsxs("div", { className: "setting-row", children: [_jsxs("div", { className: "meta", children: [_jsx("strong", { children: "Graph layout" }), "Layout engine for Graph View."] }), _jsxs("select", { value: s.ui.graph_layout, onChange: (e) => apply({ ui: { graph_layout: e.target.value } }), children: [_jsx("option", { value: "dagre", children: "Dagre (hierarchical)" }), _jsx("option", { value: "force", children: "Force-directed" })] })] })] })) })] }), _jsxs(Card, { title: "Server connection", subtitle: "Stored in your browser only. Empty base URL \u21D2 same-origin.", children: [_jsxs("div", { className: "field", children: [_jsx("label", { children: "API base URL" }), _jsx("input", { value: base, onChange: (e) => setBase(e.target.value), placeholder: "http://localhost:7373" })] }), _jsxs("div", { className: "field", children: [_jsx("label", { children: "Bearer token (optional)" }), _jsx("input", { value: token, onChange: (e) => setToken(e.target.value), placeholder: "(leave blank if server is open)" })] }), _jsx("button", { className: "btn-primary", onClick: saveConnection, children: "Save" })] }), _jsx(Card, { title: "LLM provider", subtitle: "Bound at server start (CLI flags) \u2014 read-only here.", style: { marginTop: 16 }, children: _jsxs("div", { className: "muted", children: ["The provider and model are selected when launching ", _jsx("code", { children: "ontology serve" }), "(e.g. ", _jsx("code", { children: "--anthropic --model claude-sonnet-4" }), "). Restart the server to change them."] }) })] }));
}
