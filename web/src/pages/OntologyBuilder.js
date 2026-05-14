import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
import { useEffect, useState } from "react";
import Card from "../components/Card";
import { generateOntology, getOntology, replaceOntology } from "../api";
export default function OntologyBuilder() {
    const [ontology, setOntology] = useState(null);
    const [description, setDescription] = useState("");
    const [generated, setGenerated] = useState(null);
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState(null);
    const [info, setInfo] = useState(null);
    const refresh = async () => {
        try {
            setOntology(await getOntology());
        }
        catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
    };
    useEffect(() => {
        refresh();
    }, []);
    const generate = async () => {
        if (!description.trim())
            return;
        setBusy(true);
        setError(null);
        setInfo(null);
        setGenerated(null);
        try {
            const res = await generateOntology(description);
            setGenerated(res.ontology);
        }
        catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
        finally {
            setBusy(false);
        }
    };
    const apply = async () => {
        if (!generated)
            return;
        setBusy(true);
        setError(null);
        try {
            await replaceOntology(generated);
            setInfo("Ontology applied.");
            setGenerated(null);
            await refresh();
        }
        catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
        finally {
            setBusy(false);
        }
    };
    const conceptTypes = ontology ? Object.values(ontology.concept_types) : [];
    const relationTypes = ontology ? Object.values(ontology.relation_types) : [];
    return (_jsxs(_Fragment, { children: [_jsx("div", { className: "page-header", children: _jsxs("div", { children: [_jsx("h1", { className: "page-title", children: "Ontology Builder" }), _jsx("p", { className: "page-subtitle", children: "Describe your domain \u2014 the configured LLM drafts a schema you can review and apply." })] }) }), error && _jsx("div", { className: "error-banner", children: error }), info && _jsx("div", { className: "success-banner", children: info }), _jsxs("div", { className: "grid grid-2", style: { alignItems: "start", marginBottom: 16 }, children: [_jsxs(Card, { title: "Describe your domain", subtitle: "One paragraph describing the concepts, relationships and rules.", children: [_jsx("div", { className: "field", children: _jsx("textarea", { rows: 6, placeholder: "e.g. A contract management system tracking parties, agreements, clauses, obligations and renewal dates.", value: description, onChange: (e) => setDescription(e.target.value) }) }), _jsxs("div", { className: "row", children: [_jsx("button", { className: "btn-primary", onClick: generate, disabled: busy || !description.trim(), children: busy ? "Generating…" : "Generate ontology" }), _jsx("span", { className: "muted", style: { fontSize: 12 }, children: "Uses the LLM configured at server start." })] })] }), _jsx(Card, { title: "Proposed schema", subtitle: generated ? "Review the JSON before applying." : "Nothing generated yet.", actions: generated && (_jsx("button", { className: "btn-primary", onClick: apply, disabled: busy, children: "Apply to graph" })), children: generated ? (_jsx("pre", { className: "json-view", children: JSON.stringify(generated, null, 2) })) : (_jsx("div", { className: "empty", children: "Generate a draft to see the JSON here." })) })] }), _jsx(Card, { title: "Current schema", actions: _jsx("button", { onClick: refresh, children: "Reload" }), children: !ontology ? (_jsx("div", { className: "empty", children: "Loading\u2026" })) : (_jsxs("div", { className: "grid grid-2", children: [_jsxs("div", { children: [_jsxs("h4", { style: { margin: "0 0 8px 0", fontSize: 13 }, children: ["Concept types (", conceptTypes.length, ")"] }), conceptTypes.length === 0 ? (_jsx("div", { className: "muted", children: "None defined." })) : (_jsxs("table", { className: "table", children: [_jsx("thead", { children: _jsxs("tr", { children: [_jsx("th", { children: "Name" }), _jsx("th", { children: "Parent" }), _jsx("th", { children: "Description" })] }) }), _jsx("tbody", { children: conceptTypes.map((c) => (_jsxs("tr", { children: [_jsx("td", { children: _jsx("strong", { children: c.name }) }), _jsx("td", { className: "muted", children: c.parent ?? "—" }), _jsx("td", { className: "muted", children: c.description ?? "" })] }, c.name))) })] }))] }), _jsxs("div", { children: [_jsxs("h4", { style: { margin: "0 0 8px 0", fontSize: 13 }, children: ["Relation types (", relationTypes.length, ")"] }), relationTypes.length === 0 ? (_jsx("div", { className: "muted", children: "None defined." })) : (_jsxs("table", { className: "table", children: [_jsx("thead", { children: _jsxs("tr", { children: [_jsx("th", { children: "Name" }), _jsx("th", { children: "Domain \u2192 Range" }), _jsx("th", { children: "Card." })] }) }), _jsx("tbody", { children: relationTypes.map((r) => (_jsxs("tr", { children: [_jsx("td", { children: _jsx("strong", { children: r.name }) }), _jsxs("td", { className: "muted", children: [r.domain, " \u2192 ", r.range] }), _jsx("td", { children: _jsx("span", { className: "badge", children: r.cardinality }) })] }, r.name))) })] }))] })] })) })] }));
}
