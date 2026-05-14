import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
import { useEffect, useState } from "react";
import Card from "../components/Card";
import Dropzone from "../components/Dropzone";
import { deleteFile, exportGraphUrl, getFiles, getOntology, upload } from "../api";
const KINDS = [
    { value: "jsonl", label: "JSONL records" },
    { value: "ontology", label: "Ontology JSON" },
    { value: "triples", label: "Triples (.triples)" },
    { value: "csv", label: "CSV (one concept per row)" },
    { value: "xlsx", label: "XLSX spreadsheet" },
    { value: "text", label: "Text document" },
];
function fmtBytes(b) {
    if (b < 1024)
        return `${b} B`;
    if (b < 1024 * 1024)
        return `${(b / 1024).toFixed(1)} KB`;
    return `${(b / (1024 * 1024)).toFixed(1)} MB`;
}
export default function Files() {
    const [files, setFiles] = useState([]);
    const [ontology, setOntology] = useState(null);
    const [kind, setKind] = useState("jsonl");
    const [conceptType, setConceptType] = useState("");
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState(null);
    const [info, setInfo] = useState(null);
    const refresh = async () => {
        try {
            const [f, o] = await Promise.all([getFiles(), getOntology()]);
            setFiles(f.files);
            setOntology(o);
        }
        catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
    };
    useEffect(() => {
        refresh();
    }, []);
    const onUpload = async (file) => {
        setBusy(true);
        setError(null);
        setInfo(null);
        try {
            const needsCt = ["csv", "xlsx", "text"].includes(kind);
            if (needsCt && !conceptType.trim()) {
                throw new Error(`Kind "${kind}" requires a concept type.`);
            }
            const res = await upload(file, { kind, conceptType: needsCt ? conceptType : undefined });
            setInfo(`Ingested ${res.ingested.concepts} concepts, ${res.ingested.relations} relations.`);
            await refresh();
        }
        catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
        finally {
            setBusy(false);
        }
    };
    const onDelete = async (id) => {
        if (!window.confirm("Remove this file record? (Already ingested data stays in the graph.)"))
            return;
        try {
            await deleteFile(id);
            await refresh();
        }
        catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
    };
    const conceptTypeOptions = ontology ? Object.keys(ontology.concept_types) : [];
    return (_jsxs(_Fragment, { children: [_jsxs("div", { className: "page-header", children: [_jsxs("div", { children: [_jsx("h1", { className: "page-title", children: "Files" }), _jsx("p", { className: "page-subtitle", children: "Upload data sources and browse ingest history." })] }), _jsx("div", { className: "row", children: _jsx("a", { href: exportGraphUrl("jsonl"), className: "btn-primary", style: { textDecoration: "none", padding: "8px 14px", borderRadius: 8 }, children: "Export JSONL" }) })] }), error && _jsx("div", { className: "error-banner", children: error }), info && _jsx("div", { className: "success-banner", children: info }), _jsxs(Card, { title: "Upload", children: [_jsxs("div", { className: "field-row", children: [_jsxs("div", { className: "field", children: [_jsx("label", { children: "Kind" }), _jsx("select", { value: kind, onChange: (e) => setKind(e.target.value), children: KINDS.map((k) => (_jsx("option", { value: k.value, children: k.label }, k.value))) })] }), (kind === "csv" || kind === "xlsx" || kind === "text") && (_jsxs("div", { className: "field", children: [_jsx("label", { children: "Concept type" }), conceptTypeOptions.length > 0 ? (_jsxs("select", { value: conceptType, onChange: (e) => setConceptType(e.target.value), children: [_jsx("option", { value: "", children: "\u2014 select \u2014" }), conceptTypeOptions.map((t) => _jsx("option", { children: t }, t))] })) : (_jsx("input", { value: conceptType, onChange: (e) => setConceptType(e.target.value), placeholder: "e.g. Contract" }))] }))] }), _jsx(Dropzone, { onFile: onUpload, disabled: busy })] }), _jsx(Card, { title: "Uploaded files", style: { marginTop: 16 }, actions: _jsx("button", { onClick: refresh, children: "Reload" }), children: files.length === 0 ? (_jsx("div", { className: "empty", children: "No uploads yet." })) : (_jsxs("table", { className: "table", children: [_jsx("thead", { children: _jsxs("tr", { children: [_jsx("th", { children: "Name" }), _jsx("th", { children: "Kind" }), _jsx("th", { children: "Size" }), _jsx("th", { children: "Concepts" }), _jsx("th", { children: "Relations" }), _jsx("th", { children: "Status" }), _jsx("th", { className: "actions", children: "Actions" })] }) }), _jsx("tbody", { children: files.map((f) => (_jsxs("tr", { children: [_jsx("td", { children: f.name }), _jsx("td", { children: _jsx("span", { className: "badge", children: f.kind }) }), _jsx("td", { children: fmtBytes(f.size) }), _jsx("td", { children: f.concepts }), _jsx("td", { children: f.relations }), _jsx("td", { children: _jsx("span", { className: "badge badge-success", children: f.status }) }), _jsx("td", { className: "actions", children: _jsx("button", { className: "btn-danger", onClick: () => onDelete(f.id), children: "Delete" }) })] }, f.id))) })] })) })] }));
}
