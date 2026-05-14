import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
import { useEffect, useState } from "react";
import { useSearchParams } from "react-router-dom";
import Card from "../components/Card";
import StreamingAnswer from "../components/StreamingAnswer";
import { createQuery, deleteQuery, getQueries, runQuery, } from "../api";
export default function Queries() {
    const [params] = useSearchParams();
    const initialQ = params.get("q") ?? "";
    const [queries, setQueries] = useState([]);
    const [name, setName] = useState("");
    const [query, setQuery] = useState("");
    const [topK, setTopK] = useState(8);
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState(null);
    const [lastResult, setLastResult] = useState(null);
    const refresh = async () => {
        try {
            const res = await getQueries();
            setQueries(res.queries);
        }
        catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
    };
    useEffect(() => {
        refresh();
    }, []);
    const save = async () => {
        if (!name.trim() || !query.trim())
            return;
        setBusy(true);
        setError(null);
        try {
            await createQuery({ name, query, top_k: topK });
            setName("");
            setQuery("");
            await refresh();
        }
        catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
        finally {
            setBusy(false);
        }
    };
    const run = async (q) => {
        setBusy(true);
        setError(null);
        setLastResult(null);
        try {
            const ans = await runQuery(q.id);
            setLastResult({ name: q.name, answer: ans });
            await refresh();
        }
        catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
        finally {
            setBusy(false);
        }
    };
    const remove = async (id) => {
        if (!window.confirm("Delete this saved query?"))
            return;
        try {
            await deleteQuery(id);
            await refresh();
        }
        catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
    };
    return (_jsxs(_Fragment, { children: [_jsx("div", { className: "page-header", children: _jsxs("div", { children: [_jsx("h1", { className: "page-title", children: "Queries" }), _jsx("p", { className: "page-subtitle", children: "Ask the graph or save reusable retrievals." })] }) }), error && _jsx("div", { className: "error-banner", children: error }), _jsx(Card, { title: "Ask now", subtitle: "Streams the LLM response with grounding citations.", style: { marginBottom: 16 }, children: _jsx(StreamingAnswer, { defaultQuery: initialQ }) }), _jsxs("div", { className: "grid grid-2", style: { alignItems: "start" }, children: [_jsxs(Card, { title: "Save a query", children: [_jsxs("div", { className: "field", children: [_jsx("label", { children: "Name" }), _jsx("input", { value: name, onChange: (e) => setName(e.target.value), placeholder: "Renewal obligations" })] }), _jsxs("div", { className: "field", children: [_jsx("label", { children: "Query" }), _jsx("textarea", { value: query, onChange: (e) => setQuery(e.target.value), rows: 3, placeholder: "What are the upcoming renewals\u2026" })] }), _jsxs("div", { className: "field", style: { maxWidth: 160 }, children: [_jsx("label", { children: "Top-K" }), _jsx("input", { type: "number", min: 1, max: 50, value: topK, onChange: (e) => setTopK(Number(e.target.value)) })] }), _jsx("button", { className: "btn-primary", onClick: save, disabled: busy || !name.trim() || !query.trim(), children: "Save" })] }), _jsx(Card, { title: "Saved queries", actions: _jsx("button", { onClick: refresh, children: "Reload" }), children: queries.length === 0 ? (_jsx("div", { className: "empty", children: "No saved queries yet." })) : (_jsxs("table", { className: "table", children: [_jsx("thead", { children: _jsxs("tr", { children: [_jsx("th", { children: "Name" }), _jsx("th", { children: "Top-K" }), _jsx("th", { children: "Last run" }), _jsx("th", { className: "actions", children: "Actions" })] }) }), _jsx("tbody", { children: queries.map((q) => (_jsxs("tr", { children: [_jsxs("td", { children: [_jsx("strong", { children: q.name }), _jsxs("div", { className: "muted", style: { fontSize: 11 }, children: [q.query.slice(0, 60), q.query.length > 60 ? "…" : ""] })] }), _jsx("td", { children: q.top_k }), _jsx("td", { className: "muted", children: q.last_run_at ? new Date(q.last_run_at * 1000).toLocaleString() : "—" }), _jsxs("td", { className: "actions", children: [_jsx("button", { onClick: () => run(q), disabled: busy, children: "Run" }), " ", _jsx("button", { className: "btn-danger", onClick: () => remove(q.id), children: "Delete" })] })] }, q.id))) })] })) })] }), lastResult && (_jsxs(Card, { title: `Result · ${lastResult.name}`, style: { marginTop: 16 }, children: [_jsx("div", { className: "answer-box", children: lastResult.answer.answer }), lastResult.answer.subgraph && lastResult.answer.subgraph.concepts.length > 0 && (_jsx("div", { className: "citations", children: lastResult.answer.subgraph.concepts.slice(0, 12).map((c) => (_jsxs("span", { className: "badge badge-accent", children: [c.concept_type, " \u00B7 ", c.name] }, c.id))) }))] }))] }));
}
