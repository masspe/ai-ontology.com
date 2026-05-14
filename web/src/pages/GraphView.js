import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
import { useEffect, useState } from "react";
import Card from "../components/Card";
import GraphCanvas from "../components/GraphCanvas";
import { getOntology, getSubgraph } from "../api";
export default function GraphView() {
    const [ontology, setOntology] = useState(null);
    const [subgraph, setSubgraph] = useState(null);
    const [query, setQuery] = useState("");
    const [type, setType] = useState("");
    const [depth, setDepth] = useState(1);
    const [limit, setLimit] = useState(150);
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState(null);
    const load = async () => {
        setBusy(true);
        setError(null);
        try {
            const res = await getSubgraph({
                seed_query: query.trim() || undefined,
                seed_concept_types: type ? [type] : [],
                expansion_depth: depth,
                limit,
            });
            setSubgraph(res.subgraph);
        }
        catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
        finally {
            setBusy(false);
        }
    };
    useEffect(() => {
        getOntology().then(setOntology).catch(() => undefined);
        load();
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);
    const conceptTypes = ontology ? Object.keys(ontology.concept_types) : [];
    return (_jsxs(_Fragment, { children: [_jsx("div", { className: "page-header", children: _jsxs("div", { children: [_jsx("h1", { className: "page-title", children: "Graph View" }), _jsx("p", { className: "page-subtitle", children: "Visualize a bounded slice of the knowledge graph." })] }) }), error && _jsx("div", { className: "error-banner", children: error }), _jsxs(Card, { title: "Filters", style: { marginBottom: 16 }, children: [_jsxs("div", { className: "field-row", children: [_jsxs("div", { className: "field", style: { flex: 2 }, children: [_jsx("label", { children: "Seed by query (optional)" }), _jsx("input", { placeholder: "e.g. renewal clauses", value: query, onChange: (e) => setQuery(e.target.value), onKeyDown: (e) => e.key === "Enter" && load() })] }), _jsxs("div", { className: "field", children: [_jsx("label", { children: "Concept type" }), _jsxs("select", { value: type, onChange: (e) => setType(e.target.value), children: [_jsx("option", { value: "", children: "All" }), conceptTypes.map((t) => _jsx("option", { children: t }, t))] })] }), _jsxs("div", { className: "field", style: { maxWidth: 120 }, children: [_jsx("label", { children: "Depth" }), _jsx("input", { type: "number", min: 0, max: 5, value: depth, onChange: (e) => setDepth(Number(e.target.value)) })] }), _jsxs("div", { className: "field", style: { maxWidth: 120 }, children: [_jsx("label", { children: "Limit" }), _jsx("input", { type: "number", min: 1, max: 2000, value: limit, onChange: (e) => setLimit(Number(e.target.value)) })] }), _jsx("div", { className: "field", style: { display: "flex", alignItems: "flex-end" }, children: _jsx("button", { className: "btn-primary", onClick: load, disabled: busy, children: busy ? "Loading…" : "Refresh" }) })] }), _jsx("div", { className: "muted", style: { fontSize: 12 }, children: subgraph
                            ? `${subgraph.concepts.length} concepts · ${subgraph.relations.length} relations`
                            : "" })] }), _jsx("div", { className: "graph-canvas", children: _jsx(GraphCanvas, { subgraph: subgraph }) })] }));
}
