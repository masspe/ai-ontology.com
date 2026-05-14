import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
import { useState } from "react";
import { askStream } from "../api";
export default function StreamingAnswer({ defaultQuery = "" }) {
    const [query, setQuery] = useState(defaultQuery);
    const [answer, setAnswer] = useState("");
    const [error, setError] = useState(null);
    const [streaming, setStreaming] = useState(false);
    const [retrieved, setRetrieved] = useState(null);
    const run = async () => {
        if (!query.trim())
            return;
        setAnswer("");
        setError(null);
        setRetrieved(null);
        setStreaming(true);
        try {
            await askStream({ query }, {
                onRetrieved: setRetrieved,
                onToken: (t) => setAnswer((prev) => prev + t),
                onEnd: () => setStreaming(false),
                onError: (e) => {
                    setError(e);
                    setStreaming(false);
                },
            });
            setStreaming(false);
        }
        catch (e) {
            const msg = e instanceof Error ? e.message : String(e);
            setError(msg);
            setStreaming(false);
        }
    };
    return (_jsxs("div", { children: [_jsx("div", { className: "field-row", style: { marginBottom: 12 }, children: _jsx("div", { className: "field", style: { flex: 1 }, children: _jsx("textarea", { placeholder: "Ask the ontology\u2026", value: query, onChange: (e) => setQuery(e.target.value), rows: 3 }) }) }), _jsxs("div", { className: "row", style: { marginBottom: 12 }, children: [_jsx("button", { className: "btn-primary", onClick: run, disabled: streaming || !query.trim(), children: streaming ? "Streaming…" : "Ask" }), _jsx("span", { className: "muted", style: { fontSize: 12 }, children: retrieved ? `Grounded on ${retrieved.concepts.length} concepts, ${retrieved.relations.length} relations` : "" })] }), error && _jsx("div", { className: "error-banner", children: error }), (answer || streaming) && (_jsxs("div", { className: "answer-box", children: [answer, streaming && _jsx("span", { style: { opacity: 0.5 }, children: "\u258D" })] })), retrieved && retrieved.concepts.length > 0 && (_jsx("div", { className: "citations", children: retrieved.concepts.slice(0, 12).map((c) => (_jsxs("span", { className: "badge badge-accent", children: [c.concept_type, " \u00B7 ", c.name] }, c.id))) }))] }));
}
