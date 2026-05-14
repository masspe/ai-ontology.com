import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
import { useEffect, useMemo, useRef, useState } from "react";
import Card from "../components/Card";
import Sparkline from "../components/Sparkline";
import { deleteFile, exportGraphUrl, getFiles, getOntology, upload, } from "../api";
const KIND_BY_EXT = {
    json: "ontology",
    jsonl: "jsonl",
    ndjson: "jsonl",
    triples: "triples",
    csv: "csv",
    xlsx: "xlsx",
    txt: "text",
    md: "text",
    pdf: "text",
    docx: "text",
};
function iconFor(f) {
    const ext = (f.name.split(".").pop() ?? "").toLowerCase();
    if (ext === "pdf")
        return { label: "PDF", bg: "#fee2e2", fg: "#dc2626" };
    if (ext === "docx" || ext === "doc")
        return { label: "DOC", bg: "#dbeafe", fg: "#2563eb" };
    if (ext === "csv")
        return { label: "CSV", bg: "#dcfce7", fg: "#16a34a" };
    if (ext === "xlsx" || ext === "xls")
        return { label: "XLS", bg: "#dcfce7", fg: "#16a34a" };
    if (ext === "json" || ext === "jsonl" || ext === "ndjson")
        return { label: "{ }", bg: "#fef3c7", fg: "#d97706" };
    if (ext === "triples")
        return { label: "△", bg: "#e0e7ff", fg: "#4f46e5" };
    return { label: "TXT", bg: "#ede9fe", fg: "#7c3aed" };
}
function fmtBytes(b) {
    if (b < 1024)
        return `${b} B`;
    if (b < 1024 * 1024)
        return `${(b / 1024).toFixed(0)} KB`;
    if (b < 1024 * 1024 * 1024)
        return `${(b / (1024 * 1024)).toFixed(1)} MB`;
    return `${(b / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}
function fmtDate(ts) {
    if (!ts)
        return "—";
    const d = new Date(ts * 1000);
    return d.toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
}
function fmtAgo(ts) {
    if (!ts)
        return "—";
    const diff = Date.now() / 1000 - ts;
    if (diff < 60)
        return "just now";
    if (diff < 3600)
        return `${Math.floor(diff / 60)}m ago`;
    if (diff < 86400)
        return `${Math.floor(diff / 3600)}h ago`;
    return `${Math.floor(diff / 86400)}d ago`;
}
function fileType(f) {
    const ext = (f.name.split(".").pop() ?? "").toLowerCase();
    if (["pdf", "csv", "xlsx", "docx", "json", "jsonl"].includes(ext))
        return ext.toUpperCase();
    if (ext === "triples")
        return "TRIPLES";
    if (ext === "txt" || ext === "md")
        return "TEXT";
    return f.kind?.toUpperCase() ?? "FILE";
}
function fileSource(f) {
    return f.concept_type?.trim() || "General";
}
function fileStatus(f) {
    const s = (f.status ?? "").toLowerCase();
    if (s === "failed" || s === "error")
        return { label: "Failed", cls: "fail" };
    if (s === "pending" || s === "queued")
        return { label: "Pending", cls: "warn" };
    if (s === "analyzing" || s === "processing")
        return { label: "Analyzing", cls: "info" };
    if (f.concepts > 0 || f.relations > 0)
        return { label: "Processed", cls: "ok" };
    if (s === "analyzed")
        return { label: "Analyzed", cls: "info" };
    return { label: "Pending", cls: "warn" };
}
function bucketBy(files, buckets = 12) {
    const now = Date.now() / 1000;
    const span = 30 * 24 * 3600; // last 30 days
    const step = span / buckets;
    const totals = Array.from({ length: buckets }, () => ({ count: 0, size: 0 }));
    const processed = Array(buckets).fill(0);
    const pending = Array(buckets).fill(0);
    const size = Array(buckets).fill(0);
    for (const f of files) {
        const age = now - (f.uploaded_at || now);
        if (age < 0 || age > span)
            continue;
        const idx = Math.min(buckets - 1, Math.max(0, Math.floor((span - age) / step)));
        totals[idx].count += 1;
        totals[idx].size += f.size;
        const status = fileStatus(f);
        if (status.cls === "ok")
            processed[idx] += 1;
        if (status.cls === "warn" || status.cls === "fail" || status.cls === "info")
            pending[idx] += 1;
        size[idx] += f.size;
    }
    // cumulative size trend
    let running = 0;
    for (let i = 0; i < buckets; i++) {
        running += size[i];
        size[i] = running;
    }
    return { totals, processed, pending, size };
}
const PAGE_SIZE = 6;
export default function Files() {
    const [files, setFiles] = useState([]);
    const [ontology, setOntology] = useState(null);
    const [kind] = useState("jsonl");
    const [autoKind] = useState(true);
    const [conceptType, setConceptType] = useState("");
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState(null);
    const [info, setInfo] = useState(null);
    const [search, setSearch] = useState("");
    const [filterType, setFilterType] = useState("");
    const [filterStatus, setFilterStatus] = useState("");
    const [sortBy, setSortBy] = useState("updated");
    const [page, setPage] = useState(1);
    const [recentUploads, setRecentUploads] = useState([]);
    const fileInputRef = useRef(null);
    const [dragActive, setDragActive] = useState(false);
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
        const tmpId = Date.now();
        setRecentUploads((u) => [{ id: tmpId, name: file.name, progress: 25, status: "uploading" }, ...u].slice(0, 5));
        try {
            const ext = (file.name.split(".").pop() ?? "").toLowerCase();
            const effectiveKind = autoKind ? (KIND_BY_EXT[ext] ?? kind) : kind;
            const needsCt = ["csv", "xlsx", "text"].includes(effectiveKind);
            if (needsCt && !conceptType.trim()) {
                throw new Error(`Kind "${effectiveKind}" requires a concept type.`);
            }
            setRecentUploads((u) => u.map((r) => r.id === tmpId ? { ...r, progress: 70 } : r));
            const res = await upload(file, {
                kind: effectiveKind,
                conceptType: needsCt ? conceptType : undefined,
            });
            setInfo(`Ingested ${res.ingested.concepts} concepts, ${res.ingested.relations} relations from ${file.name}.`);
            setRecentUploads((u) => u.map((r) => r.id === tmpId ? { ...r, progress: 100, status: "done" } : r));
            await refresh();
        }
        catch (e) {
            setError(e instanceof Error ? e.message : String(e));
            setRecentUploads((u) => u.map((r) => r.id === tmpId ? { ...r, status: "error" } : r));
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
    const allTypes = useMemo(() => {
        const s = new Set();
        for (const f of files)
            s.add(fileType(f));
        return Array.from(s).sort();
    }, [files]);
    const filtered = useMemo(() => {
        const q = search.trim().toLowerCase();
        const arr = files.filter((f) => {
            if (filterType && fileType(f) !== filterType)
                return false;
            if (filterStatus && fileStatus(f).label.toLowerCase() !== filterStatus)
                return false;
            if (q && !f.name.toLowerCase().includes(q))
                return false;
            return true;
        });
        arr.sort((a, b) => {
            switch (sortBy) {
                case "name": return a.name.localeCompare(b.name);
                case "size": return b.size - a.size;
                case "type": return fileType(a).localeCompare(fileType(b));
                case "updated":
                default: return (b.uploaded_at || 0) - (a.uploaded_at || 0);
            }
        });
        return arr;
    }, [files, search, filterType, filterStatus, sortBy]);
    useEffect(() => { setPage(1); }, [search, filterType, filterStatus, sortBy]);
    const pageCount = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE));
    const pageRows = filtered.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE);
    // ---- stat cards: live counts + trends ----
    const stats = useMemo(() => {
        const buckets = bucketBy(files);
        const total = files.length;
        let processed = 0;
        let pending = 0;
        let size = 0;
        for (const f of files) {
            const s = fileStatus(f);
            if (s.cls === "ok")
                processed += 1;
            else if (s.cls === "warn" || s.cls === "fail" || s.cls === "info")
                pending += 1;
            size += f.size;
        }
        return {
            total,
            processed,
            pending,
            size,
            totalSeries: buckets.totals.map((b) => b.count),
            processedSeries: buckets.processed,
            pendingSeries: buckets.pending,
            sizeSeries: buckets.size,
        };
    }, [files]);
    // ---- storage by type donut ----
    const storage = useMemo(() => {
        const map = {};
        for (const f of files) {
            const t = fileType(f);
            map[t] = (map[t] || 0) + f.size;
        }
        const entries = Object.entries(map).sort((a, b) => b[1] - a[1]).slice(0, 6);
        const total = entries.reduce((s, [, v]) => s + v, 0) || 1;
        const palette = ["#dc2626", "#16a34a", "#2563eb", "#f59e0b", "#7c3aed", "#0891b2"];
        return entries.map(([label, bytes], i) => ({
            label,
            bytes,
            pct: (bytes / total) * 100,
            color: palette[i % palette.length],
        }));
    }, [files]);
    const totalSize = storage.reduce((s, x) => s + x.bytes, 0);
    // ---- folders / collections (grouped by source / concept_type) ----
    const folders = useMemo(() => {
        const map = {};
        for (const f of files) {
            const k = fileSource(f);
            map[k] = (map[k] || 0) + 1;
        }
        return Object.entries(map)
            .sort((a, b) => b[1] - a[1])
            .slice(0, 6)
            .map(([name, count]) => ({ name, count }));
    }, [files]);
    // ---- recent activity (derived from files) ----
    const activity = useMemo(() => {
        return [...files]
            .sort((a, b) => (b.uploaded_at || 0) - (a.uploaded_at || 0))
            .slice(0, 5)
            .map((f) => {
            const s = fileStatus(f);
            let verb = "uploaded";
            let tone = "info";
            if (s.cls === "ok") {
                verb = "processed";
                tone = "ok";
            }
            else if (s.cls === "fail") {
                verb = "failed to process";
                tone = "fail";
            }
            else if (s.cls === "info") {
                verb = "is being analyzed";
                tone = "info";
            }
            else {
                verb = "is pending";
                tone = "warn";
            }
            return { id: f.id, name: f.name, verb, tone, ago: fmtAgo(f.uploaded_at) };
        });
    }, [files]);
    const pickFile = () => fileInputRef.current?.click();
    return (_jsxs(_Fragment, { children: [_jsx("div", { className: "page-header", children: _jsxs("div", { children: [_jsx("h1", { className: "page-title", children: "Files" }), _jsx("p", { className: "page-subtitle", children: "Manage uploaded sources, ingestion status, and file organization." })] }) }), error && _jsx("div", { className: "error-banner", children: error }), info && _jsx("div", { className: "success-banner", children: info }), _jsxs("div", { className: "files-stats", children: [_jsx(TrendStat, { icon: "\uD83D\uDCC4", iconBg: "#dbeafe", iconFg: "#2563eb", label: "Total Files", value: stats.total.toLocaleString(), series: stats.totalSeries, color: "#2563eb" }), _jsx(TrendStat, { icon: "\u2713", iconBg: "#dcfce7", iconFg: "#16a34a", label: "Processed", value: stats.processed.toLocaleString(), series: stats.processedSeries, color: "#16a34a" }), _jsx(TrendStat, { icon: "\u23F1", iconBg: "#fef3c7", iconFg: "#d97706", label: "Pending Review", value: stats.pending.toLocaleString(), series: stats.pendingSeries, color: "#d97706" }), _jsx(TrendStat, { icon: "\uD83D\uDDC4", iconBg: "#ede9fe", iconFg: "#7c3aed", label: "Storage Used", value: fmtBytes(stats.size), series: stats.sizeSeries, color: "#7c3aed" })] }), _jsxs("div", { className: "files-main", children: [_jsxs(Card, { title: "File Library", actions: _jsxs("div", { className: "toolbar-row", children: [_jsxs("div", { className: "search-wrap", children: [_jsx("span", { className: "search-ic", children: "\u2315" }), _jsx("input", { className: "lib-search", placeholder: "Search files\u2026", value: search, onChange: (e) => setSearch(e.target.value) })] }), _jsxs("select", { className: "lib-select", value: filterType, onChange: (e) => setFilterType(e.target.value), children: [_jsx("option", { value: "", children: "All Types" }), allTypes.map((t) => _jsx("option", { value: t, children: t }, t))] }), _jsxs("select", { className: "lib-select", value: filterStatus, onChange: (e) => setFilterStatus(e.target.value), children: [_jsx("option", { value: "", children: "All Statuses" }), _jsx("option", { value: "processed", children: "Processed" }), _jsx("option", { value: "analyzing", children: "Analyzing" }), _jsx("option", { value: "analyzed", children: "Analyzed" }), _jsx("option", { value: "pending", children: "Pending" }), _jsx("option", { value: "failed", children: "Failed" })] }), _jsxs("select", { className: "lib-select", value: sortBy, onChange: (e) => setSortBy(e.target.value), children: [_jsx("option", { value: "updated", children: "Sort: Last Updated" }), _jsx("option", { value: "name", children: "Sort: Name" }), _jsx("option", { value: "type", children: "Sort: Type" }), _jsx("option", { value: "size", children: "Sort: Size" })] }), _jsxs("button", { className: "btn-primary upload-btn", onClick: pickFile, disabled: busy, children: [_jsx("span", { "aria-hidden": true, children: "\u2912" }), " Upload Files"] }), _jsx("input", { ref: fileInputRef, type: "file", style: { display: "none" }, onChange: (e) => {
                                        const f = e.target.files?.[0];
                                        if (f)
                                            onUpload(f);
                                        e.target.value = "";
                                    } })] }), children: [(kind === "csv" || kind === "xlsx" || kind === "text") && (_jsxs("div", { className: "ct-row", children: [_jsx("label", { className: "muted", style: { fontSize: 12, marginBottom: 4 }, children: "Concept type for CSV/XLSX/text uploads" }), conceptTypeOptions.length > 0 ? (_jsxs("select", { value: conceptType, onChange: (e) => setConceptType(e.target.value), style: { maxWidth: 260 }, children: [_jsx("option", { value: "", children: "\u2014 select \u2014" }), conceptTypeOptions.map((t) => _jsx("option", { children: t }, t))] })) : (_jsx("input", { value: conceptType, onChange: (e) => setConceptType(e.target.value), placeholder: "e.g. Contract", style: { maxWidth: 260 } }))] })), filtered.length === 0 ? (_jsx("div", { className: "empty", children: "No files match your filters." })) : (_jsxs("table", { className: "table files-table", children: [_jsx("thead", { children: _jsxs("tr", { children: [_jsx("th", { children: "File Name" }), _jsx("th", { children: "Type" }), _jsx("th", { children: "Source" }), _jsxs("th", { children: ["Last Updated ", _jsx("span", { "aria-hidden": true, style: { opacity: 0.5 }, children: "\u2193" })] }), _jsx("th", { children: "Status" }), _jsx("th", { children: "Size" }), _jsx("th", { className: "actions", children: "Actions" })] }) }), _jsx("tbody", { children: pageRows.map((f) => {
                                            const ic = iconFor(f);
                                            const s = fileStatus(f);
                                            return (_jsxs("tr", { children: [_jsx("td", { children: _jsxs("div", { className: "file-cell", children: [_jsx("span", { className: "file-icon", style: { background: ic.bg, color: ic.fg }, children: ic.label }), _jsxs("div", { children: [_jsx("div", { className: "file-name", children: f.name }), f.concept_type && _jsxs("div", { className: "muted", style: { fontSize: 11 }, children: ["as ", f.concept_type] })] })] }) }), _jsx("td", { className: "muted", children: fileType(f) }), _jsx("td", { className: "muted", children: fileSource(f) }), _jsx("td", { className: "muted", children: fmtDate(f.uploaded_at) }), _jsx("td", { children: _jsx("span", { className: `status-pill ${s.cls}`, children: s.label }) }), _jsx("td", { className: "muted", children: fmtBytes(f.size) }), _jsx("td", { className: "actions", children: _jsx("button", { className: "icon-act", "aria-label": "Actions", onClick: () => onDelete(f.id), title: "Delete", children: "\u22EE" }) })] }, f.id));
                                        }) })] })), filtered.length > 0 && (_jsxs("div", { className: "pager", children: [_jsxs("div", { className: "muted", style: { fontSize: 12 }, children: ["Showing ", (page - 1) * PAGE_SIZE + 1, " to ", Math.min(filtered.length, page * PAGE_SIZE), " of ", filtered.length, " files"] }), _jsxs("div", { className: "pages", children: [_jsx("button", { className: "page-btn", disabled: page === 1, onClick: () => setPage((p) => Math.max(1, p - 1)), children: "\u2039" }), Array.from({ length: pageCount }, (_, i) => i + 1).slice(0, 5).map((p) => (_jsx("button", { className: `page-btn${p === page ? " active" : ""}`, onClick: () => setPage(p), children: p }, p))), _jsx("button", { className: "page-btn", disabled: page === pageCount, onClick: () => setPage((p) => Math.min(pageCount, p + 1)), children: "\u203A" })] })] }))] }), _jsxs("div", { className: "files-side", children: [_jsx(Card, { title: "Upload & Ingestion", children: _jsxs("div", { className: `dz${dragActive ? " active" : ""}`, onDragOver: (e) => { e.preventDefault(); setDragActive(true); }, onDragLeave: () => setDragActive(false), onDrop: (e) => {
                                        e.preventDefault();
                                        setDragActive(false);
                                        if (busy)
                                            return;
                                        const f = e.dataTransfer.files?.[0];
                                        if (f)
                                            onUpload(f);
                                    }, onClick: pickFile, children: [_jsx("div", { className: "dz-ic", children: "\u2912" }), _jsx("div", { className: "dz-title", children: "Drag & drop files here, or click to browse" }), _jsx("div", { className: "dz-sub", children: "Supports PDF, DOCX, CSV, JSON (Max 100 MB per file)" }), _jsx("button", { className: "btn-primary", type: "button", onClick: (e) => { e.stopPropagation(); pickFile(); }, children: "Browse Files" })] }) }), _jsx(Card, { title: "Recent Uploads", actions: _jsx("a", { className: "muted small-link", href: "#", onClick: (e) => e.preventDefault(), children: "View All" }), children: recentUploads.length === 0 && files.length === 0 ? (_jsx("div", { className: "muted", style: { fontSize: 12, padding: "8px 0" }, children: "No uploads yet." })) : (_jsx("ul", { className: "recent", children: (recentUploads.length > 0
                                        ? recentUploads
                                        : files.slice(0, 3).map((f) => ({
                                            id: f.id,
                                            name: f.name,
                                            progress: 100,
                                            status: (fileStatus(f).cls === "ok" ? "done" : fileStatus(f).cls === "info" ? "uploading" : "done"),
                                        }))).slice(0, 4).map((r) => {
                                        const ic = iconFor({ name: r.name });
                                        return (_jsxs("li", { className: "recent-row", children: [_jsx("span", { className: "file-icon sm", style: { background: ic.bg, color: ic.fg }, children: ic.label }), _jsxs("div", { className: "recent-meta", children: [_jsx("div", { className: "recent-name", children: r.name }), r.status === "uploading" ? (_jsx("div", { className: "bar", children: _jsx("div", { className: "bar-fill", style: { width: `${r.progress}%` } }) })) : r.status === "error" ? (_jsx("div", { className: "muted", style: { fontSize: 11, color: "var(--danger)" }, children: "Failed" })) : (_jsx("div", { className: "muted", style: { fontSize: 11 }, children: "Processed" }))] }), _jsx("div", { className: "recent-status", children: r.status === "uploading" ? (_jsxs("span", { className: "muted small", children: [r.progress, "%"] })) : r.status === "error" ? (_jsx("span", { className: "status-pill fail", children: "Error" })) : (_jsx("span", { className: "check", children: "\u2713" })) })] }, r.id));
                                    }) })) }), _jsx(Card, { title: "Storage by Type", children: _jsxs("div", { className: "storage", children: [_jsx(Donut, { slices: storage, total: totalSize }), _jsxs("ul", { className: "legend", children: [storage.map((s) => (_jsxs("li", { children: [_jsx("span", { className: "dot", style: { background: s.color } }), _jsx("span", { className: "lbl", children: s.label }), _jsx("span", { className: "muted small", children: fmtBytes(s.bytes) }), _jsxs("span", { className: "pct", children: [s.pct.toFixed(1), "%"] })] }, s.label))), storage.length === 0 && _jsx("li", { className: "muted small", children: "No data" })] })] }) })] })] }), _jsxs("div", { className: "files-bottom", children: [_jsx(Card, { title: "Folders / Collections", actions: _jsx("a", { className: "muted small-link", href: "#", onClick: (e) => e.preventDefault(), children: "View All" }), children: _jsxs("div", { className: "folders", children: [folders.map((f) => (_jsxs("div", { className: "folder", children: [_jsx("div", { className: "folder-ic", children: "\uD83D\uDCC1" }), _jsx("div", { className: "folder-name", children: f.name }), _jsxs("div", { className: "muted small", children: [f.count, " file", f.count === 1 ? "" : "s"] })] }, f.name))), _jsxs("div", { className: "folder folder-new", onClick: pickFile, role: "button", tabIndex: 0, children: [_jsx("div", { className: "folder-ic", children: "\uFF0B" }), _jsx("div", { className: "folder-name", children: "New Folder" })] })] }) }), _jsx(Card, { title: "Recent Activity", actions: _jsx("a", { className: "muted small-link", href: "#", onClick: (e) => e.preventDefault(), children: "View All" }), children: activity.length === 0 ? (_jsx("div", { className: "muted", style: { fontSize: 12 }, children: "No activity yet." })) : (_jsx("ul", { className: "activity", children: activity.map((a) => (_jsxs("li", { children: [_jsx("span", { className: `act-dot ${a.tone}`, children: a.tone === "ok" ? "✓" : a.tone === "fail" ? "!" : a.tone === "warn" ? "i" : "•" }), _jsxs("div", { className: "act-body", children: [_jsxs("div", { children: [_jsx("b", { children: a.name }), " ", _jsx("span", { className: "muted", children: a.verb })] }), _jsx("div", { className: "muted small", children: a.ago })] })] }, a.id))) })) }), _jsx(Card, { title: "Quick Actions", children: _jsxs("div", { className: "quick", children: [_jsxs("button", { className: "qa qa-blue", onClick: pickFile, children: [_jsx("span", { className: "qa-ic", children: "\u2912" }), _jsxs("div", { children: [_jsx("div", { className: "qa-title", children: "Upload Files" }), _jsx("div", { className: "qa-sub", children: "Add new files to your library" })] })] }), _jsxs("button", { className: "qa qa-green", onClick: pickFile, children: [_jsx("span", { className: "qa-ic", children: "\uD83D\uDCC1" }), _jsxs("div", { children: [_jsx("div", { className: "qa-title", children: "Create Folder" }), _jsx("div", { className: "qa-sub", children: "Organize files and collections" })] })] }), _jsxs("button", { className: "qa qa-orange", onClick: refresh, children: [_jsx("span", { className: "qa-ic", children: "\u21BB" }), _jsxs("div", { children: [_jsx("div", { className: "qa-title", children: "Reprocess Failed" }), _jsx("div", { className: "qa-sub", children: "Retry failed or errored files" })] })] }), _jsxs("a", { className: "qa qa-purple", href: exportGraphUrl("jsonl"), children: [_jsx("span", { className: "qa-ic", children: "\u2913" }), _jsxs("div", { children: [_jsx("div", { className: "qa-title", children: "Export Metadata" }), _jsx("div", { className: "qa-sub", children: "Download file metadata" })] })] })] }) })] }), _jsx("style", { children: `
        /* ===== Top stats ===== */
        .files-stats {
          display: grid;
          grid-template-columns: repeat(4, 1fr);
          gap: 16px;
          margin-bottom: 16px;
        }
        @media (max-width: 1100px) { .files-stats { grid-template-columns: repeat(2, 1fr); } }
        @media (max-width: 600px)  { .files-stats { grid-template-columns: 1fr; } }

        .ts-card {
          background: var(--panel);
          border: 1px solid var(--border);
          border-radius: var(--radius);
          padding: 16px 18px;
          display: grid;
          grid-template-columns: 48px 1fr 90px;
          gap: 12px;
          align-items: center;
          box-shadow: var(--shadow-sm);
        }
        .ts-ic {
          width: 48px; height: 48px; border-radius: 12px;
          display: grid; place-items: center;
          font-size: 22px; font-weight: 600;
        }
        .ts-body .ts-label { font-size: 12px; color: var(--muted); }
        .ts-body .ts-value { font-size: 26px; font-weight: 700; line-height: 1.15; color: var(--text); }
        .ts-body .ts-delta { font-size: 11px; margin-top: 2px; color: var(--muted); }
        .ts-body .ts-delta b { color: var(--success); font-weight: 600; margin-right: 4px; }
        .ts-body .ts-delta b.down { color: var(--danger); }
        .ts-spark { width: 90px; }

        /* ===== Main 2-col layout ===== */
        .files-main {
          display: grid;
          grid-template-columns: minmax(0, 1.9fr) minmax(0, 1fr);
          gap: 16px;
          margin-bottom: 16px;
        }
        @media (max-width: 1100px) { .files-main { grid-template-columns: 1fr; } }
        .files-side { display: flex; flex-direction: column; gap: 16px; }

        /* ===== File Library toolbar ===== */
        .toolbar-row {
          display: flex; flex-wrap: wrap; gap: 8px; align-items: center; flex: 1;
        }
        .search-wrap { position: relative; min-width: 200px; flex: 1 1 200px; }
        .search-wrap .search-ic {
          position: absolute; left: 10px; top: 50%; transform: translateY(-50%);
          color: var(--muted); pointer-events: none; font-size: 14px;
        }
        .lib-search {
          width: 100%; padding: 7px 10px 7px 30px; font-size: 13px;
          background: var(--panel); border: 1px solid var(--border); border-radius: 8px;
        }
        .lib-select {
          padding: 7px 10px; font-size: 13px;
          background: var(--panel); border: 1px solid var(--border); border-radius: 8px;
          width: auto;
        }
        .upload-btn {
          padding: 7px 14px; font-size: 13px; border-radius: 8px;
          display: inline-flex; align-items: center; gap: 6px;
        }
        .ct-row { margin-bottom: 12px; }

        /* ===== Files table ===== */
        .files-table { width: 100%; }
        .files-table th {
          font-size: 11px; text-transform: uppercase; letter-spacing: 0.04em;
          color: var(--muted); font-weight: 600; background: transparent;
          border-bottom: 1px solid var(--border);
        }
        .files-table td { padding: 12px 12px; border-bottom: 1px solid var(--border); vertical-align: middle; }
        .files-table tr:hover td { background: var(--panel-2); }
        .file-cell { display: flex; align-items: center; gap: 10px; }
        .file-icon {
          width: 36px; height: 36px; border-radius: 8px;
          display: grid; place-items: center;
          font-size: 11px; font-weight: 700;
          flex-shrink: 0;
        }
        .file-icon.sm { width: 28px; height: 28px; font-size: 10px; border-radius: 6px; }
        .file-name { font-weight: 600; font-size: 13px; color: var(--text); }

        .status-pill {
          font-size: 11px; padding: 3px 10px; border-radius: 999px; font-weight: 500;
          display: inline-block;
        }
        .status-pill.ok   { background: #dcfce7; color: #166534; }
        .status-pill.warn { background: #fef3c7; color: #92400e; }
        .status-pill.fail { background: #fee2e2; color: #991b1b; }
        .status-pill.info { background: #dbeafe; color: #1e40af; }

        .icon-act {
          background: transparent; border: none; color: var(--muted);
          font-size: 18px; padding: 4px 8px; cursor: pointer; border-radius: 6px;
          line-height: 1;
        }
        .icon-act:hover { background: var(--panel-2); color: var(--text); }

        /* ===== Pager ===== */
        .pager {
          display: flex; align-items: center; justify-content: space-between;
          padding-top: 12px; gap: 8px; flex-wrap: wrap;
        }
        .pages { display: flex; gap: 4px; }
        .page-btn {
          min-width: 30px; padding: 5px 10px; font-size: 12px;
          border: 1px solid var(--border); border-radius: 6px;
          background: var(--panel); color: var(--text);
        }
        .page-btn.active { background: var(--accent); border-color: var(--accent); color: #fff; }
        .page-btn:disabled { opacity: 0.4; cursor: not-allowed; }

        /* ===== Side: dropzone ===== */
        .dz {
          border: 2px dashed var(--accent);
          background: var(--accent-soft);
          border-radius: var(--radius);
          padding: 24px 16px;
          text-align: center;
          color: var(--accent);
          cursor: pointer;
          transition: background 0.15s, border-color 0.15s;
        }
        .dz.active, .dz:hover { background: #c7dafc; }
        .dz-ic { font-size: 28px; line-height: 1; margin-bottom: 8px; }
        .dz-title { font-weight: 600; color: var(--text); font-size: 14px; margin-bottom: 4px; }
        .dz-sub { font-size: 12px; color: var(--muted); margin-bottom: 14px; }
        .dz .btn-primary { padding: 7px 16px; font-size: 13px; border-radius: 8px; }

        /* ===== Recent uploads ===== */
        .recent { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 10px; }
        .recent-row { display: flex; gap: 10px; align-items: center; }
        .recent-meta { flex: 1; min-width: 0; }
        .recent-name { font-size: 13px; font-weight: 500; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
        .bar { height: 4px; background: var(--panel-2); border-radius: 999px; overflow: hidden; margin-top: 4px; }
        .bar-fill { height: 100%; background: var(--accent); transition: width 0.3s; }
        .recent-status .check { color: var(--success); font-weight: 700; }
        .small { font-size: 11px; }
        .small-link { font-size: 12px; }

        /* ===== Storage donut ===== */
        .storage { display: grid; grid-template-columns: 110px 1fr; gap: 14px; align-items: center; }
        @media (max-width: 1100px) { .storage { grid-template-columns: 110px 1fr; } }
        .donut-wrap { position: relative; width: 110px; height: 110px; }
        .donut-center {
          position: absolute; inset: 0; display: grid; place-items: center;
          text-align: center;
        }
        .donut-center .v { font-size: 16px; font-weight: 700; line-height: 1; }
        .donut-center .l { font-size: 10px; color: var(--muted); margin-top: 2px; }
        .legend { list-style: none; padding: 0; margin: 0; display: grid; gap: 8px; font-size: 12px; }
        .legend li { display: grid; grid-template-columns: 10px 1fr auto auto; align-items: center; gap: 6px; }
        .legend .dot { width: 8px; height: 8px; border-radius: 2px; display: inline-block; }
        .legend .lbl { color: var(--text); font-weight: 500; }
        .legend .pct { color: var(--text); font-weight: 600; }

        /* ===== Bottom row ===== */
        .files-bottom {
          display: grid;
          grid-template-columns: repeat(3, minmax(0, 1fr));
          gap: 16px;
        }
        @media (max-width: 1100px) { .files-bottom { grid-template-columns: 1fr; } }

        /* Folders */
        .folders {
          display: grid;
          grid-template-columns: repeat(3, 1fr);
          gap: 10px;
        }
        @media (max-width: 700px) { .folders { grid-template-columns: repeat(2, 1fr); } }
        .folder {
          border: 1px solid var(--border);
          border-radius: 10px;
          padding: 12px 12px;
          background: var(--panel);
        }
        .folder-new {
          border-style: dashed;
          display: flex; flex-direction: column; align-items: center; justify-content: center;
          color: var(--muted);
          cursor: pointer;
        }
        .folder-new:hover { background: var(--panel-2); color: var(--accent); }
        .folder-ic { font-size: 22px; line-height: 1; margin-bottom: 6px; }
        .folder-name { font-size: 13px; font-weight: 600; color: var(--text); margin-bottom: 2px; }

        /* Activity */
        .activity { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 14px; }
        .activity li { display: flex; gap: 10px; align-items: flex-start; }
        .act-dot {
          width: 28px; height: 28px; border-radius: 50%;
          display: grid; place-items: center;
          font-size: 12px; font-weight: 700; flex-shrink: 0;
        }
        .act-dot.ok   { background: #dcfce7; color: #166534; }
        .act-dot.warn { background: #fef3c7; color: #92400e; }
        .act-dot.fail { background: #fee2e2; color: #991b1b; }
        .act-dot.info { background: #dbeafe; color: #1e40af; }
        .act-body { flex: 1; font-size: 13px; }

        /* Quick actions */
        .quick { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
        .qa {
          display: flex; align-items: center; gap: 10px;
          padding: 12px;
          border: 1px solid var(--border);
          border-radius: 10px;
          background: var(--panel);
          text-align: left;
          cursor: pointer;
          color: var(--text);
          text-decoration: none;
          transition: background 0.15s, border-color 0.15s;
        }
        .qa:hover { background: var(--panel-2); text-decoration: none; }
        .qa-ic {
          width: 36px; height: 36px; border-radius: 8px;
          display: grid; place-items: center;
          font-size: 16px; flex-shrink: 0;
        }
        .qa-blue   .qa-ic { background: #dbeafe; color: #2563eb; }
        .qa-green  .qa-ic { background: #dcfce7; color: #16a34a; }
        .qa-orange .qa-ic { background: #fed7aa; color: #c2410c; }
        .qa-purple .qa-ic { background: #ede9fe; color: #7c3aed; }
        .qa-title { font-size: 13px; font-weight: 600; }
        .qa-sub { font-size: 11px; color: var(--muted); margin-top: 2px; }
      ` })] }));
}
function TrendStat({ icon, iconBg, iconFg, label, value, delta, deltaTone, hint, series, color }) {
    const safe = series.length > 0 && series.some((v) => v > 0) ? series : [0, 0, 0, 0, 0, 0, 0, 0];
    return (_jsxs("div", { className: "ts-card", children: [_jsx("div", { className: "ts-ic", style: { background: iconBg, color: iconFg }, children: icon }), _jsxs("div", { className: "ts-body", children: [_jsx("div", { className: "ts-label", children: label }), _jsx("div", { className: "ts-value", children: value }), delta && (_jsxs("div", { className: "ts-delta", children: [_jsxs("b", { className: deltaTone === "down" ? "down" : "", children: [deltaTone === "up" ? "↑" : "↓", " ", delta] }), hint] }))] }), _jsx("div", { className: "ts-spark", children: _jsx(Sparkline, { values: safe, stroke: color }) })] }));
}
function Donut({ slices, total }) {
    const r = 42;
    const c = 2 * Math.PI * r;
    let offset = 0;
    return (_jsxs("div", { className: "donut-wrap", children: [_jsxs("svg", { viewBox: "0 0 110 110", width: "110", height: "110", children: [_jsx("circle", { cx: "55", cy: "55", r: r, fill: "none", stroke: "var(--panel-2)", strokeWidth: "14" }), slices.map((s) => {
                        const len = (s.pct / 100) * c;
                        const el = (_jsx("circle", { cx: "55", cy: "55", r: r, fill: "none", stroke: s.color, strokeWidth: "14", strokeDasharray: `${len} ${c - len}`, strokeDashoffset: -offset, transform: "rotate(-90 55 55)" }, s.label));
                        offset += len;
                        return el;
                    })] }), _jsx("div", { className: "donut-center", children: _jsxs("div", { children: [_jsx("div", { className: "v", children: fmtBytes(total) }), _jsx("div", { className: "l", children: "Total" })] }) })] }));
}
