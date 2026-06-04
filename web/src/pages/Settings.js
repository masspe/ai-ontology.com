import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
import { useEffect, useState } from "react";
import Card from "../components/Card";
import { apiBase, getSettings, getStats, patchSettings, listFeedbacks, deleteFeedback, getOcrStatus } from "../api";
import { loadProviderConfig, saveProviderConfig } from "../lib/providerConfig";
export default function Settings() {
    const [tab, setTab] = useState("general");
    const [s, setS] = useState(null);
    const [error, setError] = useState(null);
    const [info, setInfo] = useState(null);
    // Provider & Keys state
    const [cfg, setCfg] = useState(() => loadProviderConfig());
    const [openSection, setOpenSection] = useState("provider");
    const [showOpenaiKey, setShowOpenaiKey] = useState(false);
    const [showAnthropicKey, setShowAnthropicKey] = useState(false);
    const [showInfomaniakKey, setShowInfomaniakKey] = useState(false);
    const [showBearer, setShowBearer] = useState(false);
    const [testing, setTesting] = useState(false);
    const [testResult, setTestResult] = useState(null);
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
    const updateCfg = (key, value) => {
        setCfg((prev) => ({ ...prev, [key]: value }));
    };
    const handleSave = () => {
        saveProviderConfig(cfg);
        setInfo("Provider & connection settings saved. Refresh other tabs to pick up the new values.");
        setError(null);
    };
    const handleTest = async () => {
        setTesting(true);
        setTestResult(null);
        try {
            const base = cfg.ontologyApiUrl.trim().replace(/\/$/, "");
            if (!base)
                throw new Error("Ontology API base URL is empty");
            const headers = {};
            if (cfg.ontologyBearerToken.trim()) {
                headers["authorization"] = `Bearer ${cfg.ontologyBearerToken.trim()}`;
            }
            const res = await fetch(`${base}/healthz`, { headers });
            if (!res.ok)
                throw new Error(`HTTP ${res.status} ${res.statusText}`);
            let version = "unknown";
            try {
                const data = await res.json();
                if (data && typeof data.version === "string")
                    version = data.version;
            }
            catch {
                /* /healthz may return text — ignore */
            }
            setTestResult({ ok: true, message: `✅ Server reachable — version ${version}` });
        }
        catch (e) {
            const msg = e instanceof Error ? e.message : String(e);
            setTestResult({ ok: false, message: `❌ ${msg}` });
        }
        finally {
            setTesting(false);
        }
    };
    return (_jsxs(_Fragment, { children: [_jsx("div", { className: "page-header", children: _jsxs("div", { children: [_jsx("h1", { className: "page-title", children: "Settings" }), _jsx("p", { className: "page-subtitle", children: "Retrieval defaults, UI preferences, providers and server connection." })] }) }), _jsxs("div", { className: "tabs", style: { display: "flex", gap: 4, borderBottom: "1px solid var(--border)", marginBottom: 16 }, children: [_jsx("button", { className: `btn ${tab === "general" ? "btn-primary" : "btn-ghost"}`, style: { borderRadius: "var(--radius-sm) var(--radius-sm) 0 0" }, onClick: () => setTab("general"), children: "\u2699 G\u00E9n\u00E9ral" }), _jsx("button", { className: `btn ${tab === "diagnostics" ? "btn-primary" : "btn-ghost"}`, style: { borderRadius: "var(--radius-sm) var(--radius-sm) 0 0" }, onClick: () => setTab("diagnostics"), children: "\u2726 Diagnostique" }), _jsx("button", { className: `btn ${tab === "feedback" ? "btn-primary" : "btn-ghost"}`, style: { borderRadius: "var(--radius-sm) var(--radius-sm) 0 0" }, onClick: () => setTab("feedback"), children: "\uD83D\uDCAC Feedback" })] }), tab === "diagnostics" ? (_jsx(DiagnosticsPanel, {})) : tab === "feedback" ? (_jsx(FeedbackListPanel, {})) : (_jsx(GeneralSettings, { s: s, error: error, info: info, cfg: cfg, openSection: openSection, setOpenSection: setOpenSection, showOpenaiKey: showOpenaiKey, setShowOpenaiKey: setShowOpenaiKey, showAnthropicKey: showAnthropicKey, setShowAnthropicKey: setShowAnthropicKey, showInfomaniakKey: showInfomaniakKey, setShowInfomaniakKey: setShowInfomaniakKey, showBearer: showBearer, setShowBearer: setShowBearer, testing: testing, testResult: testResult, apply: apply, updateCfg: updateCfg, handleSave: handleSave, handleTest: handleTest }))] }));
}
function GeneralSettings({ s, error, info, cfg, openSection, setOpenSection, showOpenaiKey, setShowOpenaiKey, showAnthropicKey, setShowAnthropicKey, showInfomaniakKey, setShowInfomaniakKey, showBearer, setShowBearer, testing, testResult, apply, updateCfg, handleSave, handleTest, }) {
    return (_jsxs(_Fragment, { children: [error && _jsx("div", { className: "error-banner", children: error }), info && _jsx("div", { className: "success-banner", children: info }), _jsxs("div", { className: "grid grid-2", style: { alignItems: "start", marginBottom: 16 }, children: [_jsx(Card, { title: "Retrieval defaults", children: !s ? (_jsx("div", { className: "empty", children: "Loading\u2026" })) : (_jsxs(_Fragment, { children: [_jsxs("div", { className: "setting-row", children: [_jsxs("div", { className: "meta", children: [_jsx("strong", { children: "Top-K" }), "Number of seed concepts retrieved per query."] }), _jsx("input", { type: "number", min: 1, max: 50, value: s.retrieval.top_k, onChange: (e) => apply({ retrieval: { top_k: Number(e.target.value) } }) })] }), _jsxs("div", { className: "setting-row", children: [_jsxs("div", { className: "meta", children: [_jsx("strong", { children: "Lexical weight" }), "0 = vector-only, 1 = BM25-only."] }), _jsx("input", { type: "number", step: 0.1, min: 0, max: 1, value: s.retrieval.lexical_weight, onChange: (e) => apply({ retrieval: { lexical_weight: Number(e.target.value) } }) })] }), _jsxs("div", { className: "setting-row", children: [_jsxs("div", { className: "meta", children: [_jsx("strong", { children: "Expansion depth" }), "Hops added to each seed when traversing."] }), _jsx("input", { type: "number", min: 0, max: 5, value: s.retrieval.expansion_depth, onChange: (e) => apply({ retrieval: { expansion_depth: Number(e.target.value) } }) })] })] })) }), _jsx(Card, { title: "UI preferences", children: !s ? (_jsx("div", { className: "empty", children: "Loading\u2026" })) : (_jsxs(_Fragment, { children: [_jsxs("div", { className: "setting-row", children: [_jsxs("div", { className: "meta", children: [_jsx("strong", { children: "Theme" }), "Color scheme (light only at the moment)."] }), _jsxs("select", { value: s.ui.theme, onChange: (e) => apply({ ui: { theme: e.target.value } }), children: [_jsx("option", { value: "light", children: "Light" }), _jsx("option", { value: "dark", children: "Dark" })] })] }), _jsxs("div", { className: "setting-row", children: [_jsxs("div", { className: "meta", children: [_jsx("strong", { children: "Graph layout" }), "Layout engine for Graph View."] }), _jsxs("select", { value: s.ui.graph_layout, onChange: (e) => apply({ ui: { graph_layout: e.target.value } }), children: [_jsx("option", { value: "dagre", children: "Dagre (hierarchical)" }), _jsx("option", { value: "force", children: "Force-directed" })] })] })] })) })] }), s && (_jsx(LlmServerSection, { settings: s, onPatch: (patch) => apply({ llm: patch }) })), s && (_jsx(OcrSection, { settings: s, onPatch: (patch) => apply({ ocr: patch }) })), _jsxs(Card, { title: "Provider & Keys", subtitle: "API keys and server endpoints. Stored in your browser only.", children: [_jsxs("div", { className: "provider-section", children: [_jsxs("div", { className: "provider-section-header", onClick: () => setOpenSection(openSection === "provider" ? null : "provider"), children: [_jsx("span", { children: "LLM provider" }), _jsx("span", { children: openSection === "provider" ? "▾" : "▸" })] }), openSection === "provider" && (_jsxs("div", { className: "provider-section-body", children: [_jsxs("label", { className: "field", children: [_jsx("span", { children: "Active provider" }), _jsxs("select", { value: cfg.activeLLMProvider, onChange: (e) => updateCfg("activeLLMProvider", e.target.value), children: [_jsx("option", { value: "default", children: "Default (server-configured)" }), _jsx("option", { value: "openai", children: "OpenAI" }), _jsx("option", { value: "anthropic", children: "Anthropic" }), _jsx("option", { value: "infomaniak", children: "Infomaniak AI (Swiss cloud)" })] }), _jsx("p", { className: "field-hint", children: "This provider is used by the ingest wizard and ontology builder when the per-page selector is left on \"default\"." })] }), cfg.activeLLMProvider === "infomaniak" && (_jsxs(_Fragment, { children: [_jsxs("label", { className: "field", children: [_jsx("span", { children: "Infomaniak base URL" }), _jsx("input", { type: "text", value: cfg.infomaniakBaseUrl, onChange: (e) => updateCfg("infomaniakBaseUrl", e.target.value), placeholder: "https://api.infomaniak.com/1/ai" })] }), _jsxs("label", { className: "field", children: [_jsx("span", { children: "Infomaniak model (optional)" }), _jsx("input", { type: "text", value: cfg.infomaniakModel, onChange: (e) => updateCfg("infomaniakModel", e.target.value), placeholder: "mixtral8x22b" })] })] }))] }))] }), _jsxs("div", { className: "provider-section", children: [_jsxs("div", { className: "provider-section-header", onClick: () => setOpenSection(openSection === "keys" ? null : "keys"), children: [_jsx("span", { children: "API keys" }), _jsx("span", { children: openSection === "keys" ? "▾" : "▸" })] }), openSection === "keys" && (_jsxs("div", { className: "provider-section-body", children: [_jsx(KeyField, { label: "OpenAI API key", value: cfg.openaiKey, onChange: (v) => updateCfg("openaiKey", v), show: showOpenaiKey, onToggle: () => setShowOpenaiKey((v) => !v), placeholder: "sk-\u2026" }), _jsx(KeyField, { label: "Anthropic API key", value: cfg.anthropicKey, onChange: (v) => updateCfg("anthropicKey", v), show: showAnthropicKey, onToggle: () => setShowAnthropicKey((v) => !v), placeholder: "sk-ant-\u2026" }), _jsx(KeyField, { label: "Infomaniak API key", value: cfg.infomaniakKey, onChange: (v) => updateCfg("infomaniakKey", v), show: showInfomaniakKey, onToggle: () => setShowInfomaniakKey((v) => !v), placeholder: "(Infomaniak personal API token)" }), _jsx("p", { className: "field-hint", children: "Keys are stored in plaintext in this browser's localStorage. Only enter them on trusted devices." })] }))] }), _jsxs("div", { style: { marginTop: 12, display: "flex", flexDirection: "column", gap: 12 }, children: [_jsxs("label", { className: "field", children: [_jsx("span", { children: "Ontology API base URL" }), _jsx("input", { type: "text", value: cfg.ontologyApiUrl, onChange: (e) => updateCfg("ontologyApiUrl", e.target.value), placeholder: "http://localhost:5000" })] }), _jsxs("div", { className: "field", children: [_jsx("label", { children: "Bearer token (optional)" }), _jsx(KeyField, { label: "", value: cfg.ontologyBearerToken, onChange: (v) => updateCfg("ontologyBearerToken", v), show: showBearer, onToggle: () => setShowBearer((v) => !v), placeholder: "(leave blank if server is open)" })] }), _jsxs("label", { className: "field", children: [_jsx("span", { children: "Auth server URL" }), _jsx("input", { type: "text", value: cfg.authApiUrl, onChange: (e) => updateCfg("authApiUrl", e.target.value), placeholder: "http://localhost:4000" })] })] }), _jsxs("div", { style: { display: "flex", gap: 8, marginTop: 16 }, children: [_jsx("button", { className: "btn btn-primary", onClick: handleSave, children: "\uD83D\uDCBE Save all" }), _jsx("button", { className: "btn btn-outline", onClick: handleTest, disabled: testing, children: testing ? "Testing…" : "🔌 Test connection" })] }), testResult && (_jsx("div", { className: testResult.ok ? "success-banner" : "error-banner", style: { marginTop: 12 }, children: testResult.message })), _jsxs("p", { className: "field-hint", style: { marginTop: 12 }, children: ["Currently resolving API base to ", _jsx("code", { children: apiBase() || "(same-origin)" }), "."] })] })] }));
}
function DiagnosticsPanel() {
    const [running, setRunning] = useState(false);
    const [checks, setChecks] = useState([]);
    const [totalMs, setTotalMs] = useState(0);
    const [lastRun, setLastRun] = useState(null);
    const [expanded, setExpanded] = useState({});
    const run = async () => {
        setRunning(true);
        setChecks([]);
        const results = [];
        const overallStart = performance.now();
        const time = async (fn) => {
            const start = performance.now();
            try {
                const value = await fn();
                return { value, err: null, ms: performance.now() - start };
            }
            catch (err) {
                return { value: null, err, ms: performance.now() - start };
            }
        };
        // 1. Server reachability
        {
            const base = apiBase() || window.location.origin;
            const r = await time(async () => {
                const res = await fetch(`${base}/healthz`);
                if (!res.ok)
                    throw new Error(`HTTP ${res.status}`);
                return await res.text();
            });
            results.push({
                name: "Serveur ontologie",
                status: r.err ? "error" : "ok",
                summary: r.err ? "Injoignable" : `Connexion active (${(r.value ?? "").toString().trim()})`,
                latencyMs: r.ms,
                details: r.err ? errMsg(r.err) : `GET ${base}/healthz → 200`,
            });
        }
        // 2. Settings load
        const settingsRes = await time(() => getSettings());
        results.push({
            name: "Paramètres serveur",
            status: settingsRes.err ? "error" : "ok",
            summary: settingsRes.err ? "Échec du chargement" : "Settings chargés",
            latencyMs: settingsRes.ms,
            details: settingsRes.err ? errMsg(settingsRes.err) : `provider actif: ${settingsRes.value?.llm.active_provider ?? "—"}`,
        });
        // 3. Stats / Données
        const statsRes = await time(() => getStats());
        if (statsRes.err) {
            results.push({
                name: "Données",
                status: "error",
                summary: "Impossible de charger les statistiques",
                latencyMs: statsRes.ms,
                details: errMsg(statsRes.err),
            });
        }
        else {
            const s = statsRes.value;
            const empty = s.concepts === 0 && s.relations === 0;
            results.push({
                name: "Données",
                status: empty ? "warn" : "ok",
                summary: `${s.concepts} concepts, ${s.relations} relations, ${s.rules} règles, ${s.actions} actions`,
                latencyMs: statsRes.ms,
                details: `Types — concepts:${s.concept_types}, relations:${s.relation_types}, rules:${s.rule_types}, actions:${s.action_types}`,
            });
        }
        // 4. LLM connection
        {
            const llm = settingsRes.value?.llm;
            const provider = llm?.active_provider && llm.active_provider !== "default" ? llm.active_provider : "openai";
            const r = await time(async () => {
                const res = await fetch(`${apiBase()}/settings/llm/test`, {
                    method: "POST",
                    headers: {
                        "content-type": "application/json",
                        ...(localStorage.getItem("msBE.token")
                            ? { authorization: `Bearer ${localStorage.getItem("msBE.token")}` }
                            : {}),
                    },
                    body: JSON.stringify({ provider }),
                });
                const data = await res.json().catch(() => ({}));
                if (!res.ok || !data.ok)
                    throw new Error(data.error || `HTTP ${res.status}`);
                return data;
            });
            results.push({
                name: `LLM (${provider})`,
                status: r.err ? "warn" : "ok",
                summary: r.err ? "Non configuré ou injoignable" : `Connexion OK${r.value?.model ? ` (${r.value.model})` : ""}`,
                latencyMs: r.ms,
                details: r.err ? errMsg(r.err) : JSON.stringify(r.value),
            });
        }
        // 5. Auth / JWT
        {
            const start = performance.now();
            const jwt = typeof window !== "undefined" ? window.localStorage.getItem("msBE.token") : null;
            results.push({
                name: "Authentification",
                status: jwt ? "ok" : "warn",
                summary: jwt ? "JWT présent dans le navigateur" : "Aucun JWT — accès anonyme",
                latencyMs: performance.now() - start,
                details: jwt ? `Longueur du token: ${jwt.length} caractères` : "Connectez-vous pour activer les routes protégées.",
            });
        }
        // 6. Provider config (localStorage)
        {
            const start = performance.now();
            const hasUrl = cfgHas("ontologyApiUrl");
            results.push({
                name: "Configuration locale",
                status: hasUrl ? "ok" : "warn",
                summary: hasUrl ? "providerConfig chargé" : "providerConfig vide",
                latencyMs: performance.now() - start,
                details: `apiBase = ${apiBase() || "(same-origin)"}`,
            });
        }
        // 7. Système de fichiers (cookies / localStorage availability)
        {
            const start = performance.now();
            let ok = true;
            try {
                const k = "__diag_probe__";
                window.localStorage.setItem(k, "1");
                window.localStorage.removeItem(k);
            }
            catch {
                ok = false;
            }
            results.push({
                name: "Stockage navigateur",
                status: ok ? "ok" : "error",
                summary: ok ? "localStorage accessible" : "localStorage indisponible",
                latencyMs: performance.now() - start,
            });
        }
        setChecks(results);
        setTotalMs(performance.now() - overallStart);
        setLastRun(new Date());
        setRunning(false);
    };
    useEffect(() => { void run(); /* eslint-disable-next-line react-hooks/exhaustive-deps */ }, []);
    const okCount = checks.filter((c) => c.status === "ok").length;
    const warnCount = checks.filter((c) => c.status === "warn").length;
    const errCount = checks.filter((c) => c.status === "error").length;
    const overallStatus = errCount > 0 ? "error" : warnCount > 0 ? "warn" : "ok";
    const ua = typeof navigator !== "undefined" ? navigator.userAgent : "—";
    const platform = typeof navigator !== "undefined" ? (navigator.platform || "—") : "—";
    const env = import.meta.env.MODE || "development";
    return (_jsxs(Card, { title: _jsxs("span", { style: { display: "inline-flex", alignItems: "center", gap: 8 }, children: [_jsx("span", { style: { color: "var(--accent)" }, children: "\u2726" }), "Auto-diagnostic de l'application"] }), actions: _jsx("button", { className: "btn btn-primary", onClick: run, disabled: running, children: running ? "Analyse…" : "↻ Relancer" }), children: [checks.length > 0 && (_jsxs("div", { className: overallStatus === "error" ? "error-banner"
                    : overallStatus === "warn" ? "warn-banner"
                        : "success-banner", style: { marginBottom: 16 }, children: [_jsx("strong", { style: { display: "block", marginBottom: 4 }, children: overallStatus === "error" ? "Erreurs détectées"
                            : overallStatus === "warn" ? "Avertissements détectés"
                                : "Tous les contrôles sont passés" }), okCount, " OK \u00B7 ", warnCount, " avertissement", warnCount > 1 ? "s" : "", " \u00B7 ", errCount, " erreur", errCount > 1 ? "s" : "", " \u00B7 ", Math.round(totalMs), " ms"] })), _jsxs("div", { className: "grid grid-4", style: { gap: 12, marginBottom: 16 }, children: [_jsx(InfoTile, { label: "Environnement", value: env }), _jsx(InfoTile, { label: "Plateforme", value: platform }), _jsx(InfoTile, { label: "API base", value: apiBase() || "(same-origin)" }), _jsx(InfoTile, { label: "Derni\u00E8re v\u00E9rification", value: lastRun ? lastRun.toLocaleTimeString() : "—" })] }), checks.length === 0 && running && _jsx("div", { className: "empty", children: "Analyse en cours\u2026" }), _jsx("div", { style: { display: "flex", flexDirection: "column", gap: 8 }, children: checks.map((c, i) => (_jsx(DiagRow, { check: c, open: !!expanded[i], onToggle: () => setExpanded((e) => ({ ...e, [i]: !e[i] })) }, i))) }), _jsxs("p", { className: "field-hint", style: { marginTop: 16 }, children: ["User-Agent: ", _jsx("code", { style: { wordBreak: "break-all" }, children: ua })] })] }));
}
function InfoTile({ label, value }) {
    return (_jsxs("div", { style: {
            padding: "10px 12px",
            background: "var(--panel-2)",
            border: "1px solid var(--border)",
            borderRadius: "var(--radius-sm)",
        }, children: [_jsx("div", { style: { fontSize: 12, color: "var(--muted)" }, children: label }), _jsx("div", { style: { fontWeight: 600, marginTop: 2, wordBreak: "break-all" }, children: value })] }));
}
function DiagRow({ check, open, onToggle }) {
    const palette = check.status === "ok"
        ? { bg: "#ecfdf5", border: "#a7f3d0", icon: "✓", color: "#047857" }
        : check.status === "warn"
            ? { bg: "#fefce8", border: "#fde68a", icon: "⚠", color: "#b45309" }
            : { bg: "#fef2f2", border: "#fecaca", icon: "✕", color: "#b91c1c" };
    return (_jsxs("div", { style: {
            background: palette.bg,
            border: `1px solid ${palette.border}`,
            borderRadius: "var(--radius-sm)",
            padding: "10px 14px",
        }, children: [_jsxs("div", { style: { display: "flex", alignItems: "center", gap: 12, cursor: "pointer" }, onClick: onToggle, children: [_jsx("span", { style: { color: palette.color, fontWeight: 700, fontSize: 16 }, children: palette.icon }), _jsxs("div", { style: { flex: 1 }, children: [_jsx("div", { style: { fontWeight: 600 }, children: check.name }), _jsx("div", { style: { fontSize: 13, color: "var(--muted)" }, children: check.summary })] }), _jsxs("span", { style: { fontSize: 12, color: "var(--muted)" }, children: [Math.round(check.latencyMs), " ms"] }), _jsx("span", { style: { color: "var(--muted)" }, children: open ? "▾" : "▸" })] }), open && check.details && (_jsx("pre", { style: {
                    marginTop: 8,
                    background: "var(--panel)",
                    border: "1px solid var(--border)",
                    borderRadius: "var(--radius-sm)",
                    padding: 10,
                    fontSize: 12,
                    whiteSpace: "pre-wrap",
                    wordBreak: "break-all",
                }, children: check.details }))] }));
}
function errMsg(e) {
    return e instanceof Error ? e.message : String(e);
}
function cfgHas(key) {
    try {
        const raw = window.localStorage.getItem("ontology.providerConfig");
        if (!raw)
            return false;
        const obj = JSON.parse(raw);
        return Boolean(obj && obj[key]);
    }
    catch {
        return false;
    }
}
const PROVIDER_LABELS = {
    openai: "OpenAI",
    anthropic: "Anthropic",
    infomaniak: "Infomaniak AI",
};
const BASE_URL_PRESETS = {
    openai: [
        { value: "", label: "Par défaut (api.openai.com)" },
        { value: "https://eu.api.openai.com/v1", label: "Europe (eu.api.openai.com)" },
        { value: "https://YOUR-RESOURCE.openai.azure.com", label: "Azure OpenAI (personnalisé)" },
    ],
    anthropic: [
        { value: "", label: "Par défaut (api.anthropic.com)" },
    ],
    infomaniak: [
        { value: "https://api.infomaniak.com/1/ai", label: "Infomaniak (Suisse)" },
    ],
};
const MODEL_PRESETS = {
    openai: [
        { value: "gpt-4o", label: "GPT-4o (Recommandé)" },
        { value: "gpt-4o-mini", label: "GPT-4o mini" },
        { value: "gpt-4.1", label: "GPT-4.1" },
        { value: "gpt-4-turbo", label: "GPT-4 Turbo" },
        { value: "gpt-3.5-turbo", label: "GPT-3.5 Turbo" },
    ],
    anthropic: [
        { value: "claude-opus-4-7", label: "Claude Opus 4.7 (Recommandé)" },
        { value: "claude-sonnet-4-6", label: "Claude Sonnet 4.6" },
        { value: "claude-haiku-4-5", label: "Claude Haiku 4.5" },
    ],
    infomaniak: [
        { value: "mixtral8x22b", label: "Mixtral 8x22B" },
        { value: "llama3.1-70b", label: "LLaMA 3.1 70B" },
    ],
};
function LlmServerSection({ settings, onPatch }) {
    const llm = settings.llm;
    const initialProvider = (llm.active_provider && llm.active_provider !== "default")
        ? llm.active_provider : "openai";
    const [provider, setProvider] = useState(initialProvider);
    const [apiKey, setApiKey] = useState("");
    const [showKey, setShowKey] = useState(false);
    const [baseUrl, setBaseUrl] = useState("");
    const [model, setModel] = useState("");
    const [models, setModels] = useState(MODEL_PRESETS.openai);
    const [temperature, setTemperature] = useState(llm.temperature ?? 0.3);
    const [maxTokens, setMaxTokens] = useState(llm.max_tokens ?? 1000);
    const [saving, setSaving] = useState(false);
    const [testing, setTesting] = useState(false);
    const [loadingModels, setLoadingModels] = useState(false);
    const [testMsg, setTestMsg] = useState(null);
    // Re-sync local form when server settings or selected provider change.
    useEffect(() => {
        if (provider === "openai") {
            setBaseUrl(llm.openai_base_url);
            setModel(llm.openai_model || "gpt-4o");
        }
        else if (provider === "anthropic") {
            setBaseUrl(llm.anthropic_base_url);
            setModel(llm.anthropic_model || "claude-opus-4-7");
        }
        else if (provider === "infomaniak") {
            setBaseUrl(llm.infomaniak_base_url);
            setModel(llm.infomaniak_model || "mixtral8x22b");
        }
        setModels(MODEL_PRESETS[provider] ?? []);
        setTemperature(llm.temperature);
        setMaxTokens(llm.max_tokens);
    }, [provider, llm]);
    const hint = provider === "openai" ? llm.openai_api_key_hint
        : provider === "anthropic" ? llm.anthropic_api_key_hint
            : provider === "infomaniak" ? llm.infomaniak_api_key_hint
                : "";
    const configured = Boolean(hint);
    const providerLabel = PROVIDER_LABELS[provider] ?? provider;
    const buildPatch = (includeKey) => {
        const patch = {
            active_provider: provider,
            temperature,
            max_tokens: maxTokens,
        };
        if (provider === "openai") {
            if (includeKey && apiKey.trim())
                patch.openai_api_key = apiKey.trim();
            patch.openai_base_url = baseUrl;
            patch.openai_model = model;
        }
        else if (provider === "anthropic") {
            if (includeKey && apiKey.trim())
                patch.anthropic_api_key = apiKey.trim();
            patch.anthropic_base_url = baseUrl;
            patch.anthropic_model = model;
        }
        else if (provider === "infomaniak") {
            if (includeKey && apiKey.trim())
                patch.infomaniak_api_key = apiKey.trim();
            patch.infomaniak_base_url = baseUrl;
            patch.infomaniak_model = model;
        }
        return patch;
    };
    const save = async () => {
        setSaving(true);
        try {
            await onPatch(buildPatch(true));
            setApiKey("");
        }
        finally {
            setSaving(false);
        }
    };
    const testConnection = async () => {
        setTesting(true);
        setTestMsg(null);
        try {
            await onPatch(buildPatch(true));
            const res = await fetch(`${apiBase()}/settings/llm/test`, {
                method: "POST",
                headers: {
                    "content-type": "application/json",
                    ...(localStorage.getItem("msBE.token")
                        ? { authorization: `Bearer ${localStorage.getItem("msBE.token")}` }
                        : {}),
                },
                body: JSON.stringify({ provider }),
            });
            const data = await res.json().catch(() => ({}));
            if (res.ok && data.ok) {
                setTestMsg(`✅ Connexion ${providerLabel} OK${data.model ? ` (modèle: ${data.model})` : ""}`);
            }
            else {
                setTestMsg(`❌ ${data.error || `HTTP ${res.status}`}`);
            }
            setApiKey("");
        }
        catch (e) {
            setTestMsg(`❌ ${e instanceof Error ? e.message : String(e)}`);
        }
        finally {
            setTesting(false);
        }
    };
    const loadModelsFromApi = async () => {
        setLoadingModels(true);
        try {
            await onPatch(buildPatch(true));
            const res = await fetch(`${apiBase()}/settings/llm/models?provider=${provider}`, {
                headers: localStorage.getItem("msBE.token")
                    ? { authorization: `Bearer ${localStorage.getItem("msBE.token")}` }
                    : {},
            });
            const data = await res.json().catch(() => ({}));
            if (res.ok && Array.isArray(data.models)) {
                const fetched = data.models.map((m) => ({
                    value: m, label: m,
                }));
                if (fetched.length > 0) {
                    setModels(fetched);
                    setTestMsg(`✅ ${fetched.length} modèles chargés depuis l'API`);
                }
                else {
                    setTestMsg("⚠️ Aucun modèle retourné");
                }
            }
            else {
                setTestMsg(`❌ ${data.error || `HTTP ${res.status}`}`);
            }
            setApiKey("");
        }
        catch (e) {
            setTestMsg(`❌ ${e instanceof Error ? e.message : String(e)}`);
        }
        finally {
            setLoadingModels(false);
        }
    };
    const baseUrlPresets = BASE_URL_PRESETS[provider] ?? [];
    const isCustomUrl = baseUrl !== "" && !baseUrlPresets.some((p) => p.value === baseUrl);
    const isCustomModel = model !== "" && !models.some((m) => m.value === model);
    return (_jsxs(Card, { title: _jsxs("span", { style: { display: "inline-flex", alignItems: "center", gap: 8 }, children: [_jsx("span", { style: { color: "var(--accent)" }, children: "\u2726" }), "Configuration ", providerLabel] }), subtitle: "Cl\u00E9 API pour l'extraction intelligente avec IA", actions: configured ? (_jsx("span", { className: "badge badge-success", children: "\u2298 Configur\u00E9" })) : (_jsx("span", { className: "badge badge-warn", children: "Non configur\u00E9" })), children: [_jsxs("label", { className: "field", style: { marginBottom: 4 }, children: [_jsx("span", { children: "Fournisseur" }), _jsx("select", { value: provider, onChange: (e) => setProvider(e.target.value), children: Object.entries(PROVIDER_LABELS).map(([k, v]) => (_jsx("option", { value: k, children: v }, k))) })] }), _jsxs("label", { className: "field", style: { marginTop: 16 }, children: [_jsxs("span", { children: ["Cl\u00E9 API ", providerLabel] }), _jsxs("div", { style: { display: "flex", gap: 8 }, children: [_jsxs("div", { className: "key-field-wrapper", style: { flex: 1 }, children: [_jsx("input", { type: showKey ? "text" : "password", value: apiKey, onChange: (e) => setApiKey(e.target.value), placeholder: configured ? "••••••••••" : "sk-proj-…", autoComplete: "off", spellCheck: false }), _jsx("button", { type: "button", className: "key-toggle-btn", onClick: () => setShowKey((v) => !v), title: showKey ? "Masquer" : "Afficher", children: "\uD83D\uDC41" })] }), _jsx("button", { className: "btn btn-primary", onClick: save, disabled: saving, style: { whiteSpace: "nowrap" }, children: saving ? "Sauvegarde…" : "💾 Sauvegarder" })] }), configured && (_jsxs("div", { style: {
                            marginTop: 8,
                            padding: "8px 12px",
                            background: "var(--panel-2)",
                            border: "1px solid var(--border)",
                            borderRadius: "var(--radius-sm)",
                            display: "flex",
                            justifyContent: "space-between",
                            fontSize: 13,
                        }, children: [_jsx("span", { style: { color: "var(--muted)" }, children: "Cl\u00E9 actuelle :" }), _jsx("code", { children: hint })] })), provider === "openai" && (_jsxs("p", { className: "field-hint", style: { marginTop: 6 }, children: ["Format: sk-proj-\u2026 Obtenez votre cl\u00E9 sur", " ", _jsx("a", { href: "https://platform.openai.com/api-keys", target: "_blank", rel: "noopener noreferrer", children: "platform.openai.com" })] })), provider === "anthropic" && (_jsxs("p", { className: "field-hint", style: { marginTop: 6 }, children: ["Format: sk-ant-\u2026 Obtenez votre cl\u00E9 sur", " ", _jsx("a", { href: "https://console.anthropic.com/settings/keys", target: "_blank", rel: "noopener noreferrer", children: "console.anthropic.com" })] }))] }), _jsxs("label", { className: "field", style: { marginTop: 16 }, children: [_jsx("span", { children: "URL de base API (optionnel)" }), _jsxs("select", { value: isCustomUrl ? "__custom__" : baseUrl, onChange: (e) => {
                            if (e.target.value === "__custom__")
                                setBaseUrl("https://");
                            else
                                setBaseUrl(e.target.value);
                        }, children: [baseUrlPresets.map((p) => (_jsx("option", { value: p.value, children: p.label }, p.value || "default"))), _jsx("option", { value: "__custom__", children: "Personnalis\u00E9\u2026" })] }), (isCustomUrl || baseUrl.startsWith("https://Y")) && (_jsx("input", { type: "text", value: baseUrl, onChange: (e) => setBaseUrl(e.target.value), placeholder: "https://votre-endpoint/v1", style: { marginTop: 8 } })), _jsx("p", { className: "field-hint", children: "Utilisez une URL personnalis\u00E9e pour Azure OpenAI, proxies r\u00E9gionaux (Suisse, Europe) ou endpoints priv\u00E9s." })] }), _jsxs("div", { className: "grid grid-3", style: { gap: 16, marginTop: 16 }, children: [_jsxs("label", { className: "field", children: [_jsxs("span", { style: { display: "flex", justifyContent: "space-between", alignItems: "center" }, children: [_jsx("span", { children: "Mod\u00E8le" }), _jsx("button", { type: "button", className: "link-btn", onClick: loadModelsFromApi, disabled: loadingModels || !configured, style: {
                                            background: "none",
                                            border: "none",
                                            color: "var(--accent)",
                                            cursor: configured ? "pointer" : "not-allowed",
                                            fontSize: 12,
                                            padding: 0,
                                        }, title: configured ? "Charger les modèles depuis l'API" : "Sauvegardez la clé d'abord", children: loadingModels ? "…" : "↻ Charger depuis API" })] }), _jsxs("select", { value: isCustomModel ? "__custom__" : model, onChange: (e) => {
                                    if (e.target.value === "__custom__")
                                        setModel("");
                                    else
                                        setModel(e.target.value);
                                }, children: [models.map((m) => (_jsx("option", { value: m.value, children: m.label }, m.value))), _jsx("option", { value: "__custom__", children: "Personnalis\u00E9\u2026" })] }), isCustomModel && (_jsx("input", { type: "text", value: model, onChange: (e) => setModel(e.target.value), placeholder: "nom-du-mod\u00E8le", style: { marginTop: 8 } }))] }), _jsxs("label", { className: "field", children: [_jsxs("span", { children: ["Temp\u00E9rature (", temperature.toFixed(2), ")"] }), _jsx("input", { type: "range", min: 0, max: 1, step: 0.05, value: temperature, onChange: (e) => setTemperature(Number(e.target.value)) })] }), _jsxs("label", { className: "field", children: [_jsx("span", { children: "Max Tokens" }), _jsx("input", { type: "number", min: 1, max: 32000, value: maxTokens, onChange: (e) => setMaxTokens(Number(e.target.value)) })] })] }), _jsx("button", { className: "btn btn-outline", onClick: testConnection, disabled: testing, style: {
                    marginTop: 20,
                    width: "100%",
                    padding: "12px",
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "center",
                    gap: 8,
                }, children: testing ? "Test en cours…" : _jsxs(_Fragment, { children: ["\u2726 Tester la connexion ", providerLabel] }) }), testMsg && (_jsx("div", { className: testMsg.startsWith("✅") ? "success-banner" : testMsg.startsWith("⚠️") ? "warn-banner" : "error-banner", style: { marginTop: 12 }, children: testMsg }))] }));
}
function OcrSection({ settings, onPatch }) {
    const ocr = settings.ocr;
    const [status, setStatus] = useState(null);
    const [apiKey, setApiKey] = useState("");
    const [showKey, setShowKey] = useState(false);
    const [saving, setSaving] = useState(false);
    useEffect(() => {
        getOcrStatus().then(setStatus).catch(() => setStatus(null));
    }, [ocr.google_api_key_hint]);
    const saveKey = async () => {
        if (!apiKey.trim())
            return;
        setSaving(true);
        try {
            await onPatch({ google_api_key: apiKey.trim() });
            setApiKey("");
        }
        finally {
            setSaving(false);
        }
    };
    const tessAvail = status?.tesseract.available ?? false;
    const gsAvail = status?.tesseract.ghostscript_available ?? false;
    const googleConfigured = status?.google_vision.configured ?? Boolean(ocr.google_api_key_hint);
    return (_jsxs(Card, { title: _jsxs("span", { style: { display: "inline-flex", alignItems: "center", gap: 8 }, children: [_jsx("span", { style: { color: "var(--accent)" }, children: "\uD83D\uDC41" }), "Moteur OCR"] }), subtitle: "S\u00E9lectionnez le moteur de reconnaissance optique de caract\u00E8res", children: [_jsxs("label", { className: "field", children: [_jsx("span", { children: "Fournisseur OCR principal" }), _jsxs("select", { value: ocr.provider, onChange: (e) => onPatch({ provider: e.target.value }), children: [_jsx("option", { value: "tesseract", children: "Tesseract (Local \u2014 gratuit)" }), _jsx("option", { value: "google_vision", children: "Google Cloud Vision" })] })] }), _jsxs("div", { className: "grid grid-2", style: { gap: 16, marginTop: 16 }, children: [_jsxs("div", { style: {
                            border: "1px solid var(--border)",
                            borderRadius: "var(--radius-sm)",
                            padding: 14,
                            background: "var(--panel-2)",
                        }, children: [_jsxs("div", { style: { display: "flex", justifyContent: "space-between", alignItems: "center" }, children: [_jsx("strong", { children: "Tesseract (Local)" }), _jsx("span", { className: tessAvail ? "badge badge-success" : "badge badge-warn", children: tessAvail ? "⊘ Disponible" : "⚠ Indisponible" })] }), _jsxs("div", { style: { marginTop: 8, fontSize: 13, color: "var(--muted)", display: "flex", flexDirection: "column", gap: 2 }, children: [_jsxs("span", { children: ["ocrmypdf: ", status?.tesseract.ocrmypdf_version ?? "—"] }), _jsxs("span", { children: ["Tesseract: ", tessAvail ? "✓" : "✕"] }), _jsxs("span", { children: ["Ghostscript: ", gsAvail ? "✓" : "✕"] })] })] }), _jsxs("div", { style: {
                            border: "1px solid var(--border)",
                            borderRadius: "var(--radius-sm)",
                            padding: 14,
                            background: "var(--panel-2)",
                        }, children: [_jsxs("div", { style: { display: "flex", justifyContent: "space-between", alignItems: "center" }, children: [_jsx("strong", { children: "Google Cloud Vision" }), _jsx("span", { className: googleConfigured ? "badge badge-success" : "badge badge-warn", children: googleConfigured ? "⊘ Configuré" : "Non configuré" })] }), _jsxs("div", { style: { marginTop: 8, fontSize: 13, color: "var(--muted)" }, children: ["Auth: Cl\u00E9 API ", ocr.google_api_key_hint && (_jsx("code", { style: { marginLeft: 6 }, children: ocr.google_api_key_hint }))] })] })] }), _jsxs("div", { style: {
                    marginTop: 16,
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "space-between",
                    gap: 16,
                    padding: "12px 14px",
                    background: "var(--panel-2)",
                    border: "1px solid var(--border)",
                    borderRadius: "var(--radius-sm)",
                }, children: [_jsxs("div", { children: [_jsx("div", { style: { fontWeight: 600 }, children: "Fallback automatique" }), _jsx("div", { style: { fontSize: 13, color: "var(--muted)" }, children: "Si le moteur principal \u00E9choue ou produit un r\u00E9sultat insuffisant, basculer automatiquement sur l'autre moteur." })] }), _jsxs("label", { style: { position: "relative", display: "inline-block", width: 44, height: 24 }, children: [_jsx("input", { type: "checkbox", checked: ocr.auto_fallback, onChange: (e) => onPatch({ auto_fallback: e.target.checked }), style: { opacity: 0, width: 0, height: 0 } }), _jsx("span", { style: {
                                    position: "absolute",
                                    inset: 0,
                                    background: ocr.auto_fallback ? "var(--success, #10b981)" : "#cbd5e1",
                                    borderRadius: 999,
                                    transition: "background .15s",
                                    cursor: "pointer",
                                } }), _jsx("span", { style: {
                                    position: "absolute",
                                    top: 2,
                                    left: ocr.auto_fallback ? 22 : 2,
                                    width: 20,
                                    height: 20,
                                    background: "#fff",
                                    borderRadius: "50%",
                                    transition: "left .15s",
                                    boxShadow: "0 1px 2px rgba(0,0,0,.2)",
                                    pointerEvents: "none",
                                } })] })] }), _jsxs("label", { className: "field", style: { marginTop: 16 }, children: [_jsx("span", { children: "Seuil minimum de texte (caract\u00E8res)" }), _jsx("input", { type: "number", min: 0, max: 10000, value: ocr.min_text_threshold, onChange: (e) => onPatch({ min_text_threshold: Number(e.target.value) }) }), _jsxs("p", { className: "field-hint", children: ["Si l'OCR retourne moins de ", ocr.min_text_threshold, " caract\u00E8res, le fallback est d\u00E9clench\u00E9."] })] }), _jsxs("label", { className: "field", style: { marginTop: 16 }, children: [_jsx("span", { children: "Langues OCR (Tesseract)" }), _jsx("input", { type: "text", value: ocr.tesseract_languages, onChange: (e) => onPatch({ tesseract_languages: e.target.value }), placeholder: "fra+deu+eng" }), _jsxs("p", { className: "field-hint", children: ["Codes ISO 639-2 s\u00E9par\u00E9s par ", _jsx("code", { children: "+" }), " (ex. ", _jsx("code", { children: "fra+deu+eng" }), "). Les packs correspondants doivent \u00EAtre install\u00E9s."] })] }), _jsxs("label", { className: "field", style: { marginTop: 16 }, children: [_jsx("span", { children: "Cl\u00E9 API Google Cloud Vision" }), _jsxs("div", { style: { display: "flex", gap: 8 }, children: [_jsxs("div", { className: "key-field-wrapper", style: { flex: 1 }, children: [_jsx("input", { type: showKey ? "text" : "password", value: apiKey, onChange: (e) => setApiKey(e.target.value), placeholder: googleConfigured ? "••••••••••" : "AIza…", autoComplete: "off", spellCheck: false }), _jsx("button", { type: "button", className: "key-toggle-btn", onClick: () => setShowKey((v) => !v), title: showKey ? "Masquer" : "Afficher", children: "\uD83D\uDC41" })] }), _jsx("button", { className: "btn btn-primary", onClick: saveKey, disabled: saving || !apiKey.trim(), style: { whiteSpace: "nowrap" }, children: saving ? "Sauvegarde…" : "💾 Sauvegarder" })] })] })] }));
}
function KeyField({ label, value, onChange, show, onToggle, placeholder }) {
    return (_jsxs("label", { className: "field", children: [label && _jsx("span", { children: label }), _jsxs("div", { className: "key-field-wrapper", children: [_jsx("input", { type: show ? "text" : "password", value: value, onChange: (e) => onChange(e.target.value), placeholder: placeholder, autoComplete: "off", spellCheck: false }), _jsx("button", { type: "button", className: "key-toggle-btn", onClick: onToggle, "aria-label": show ? "Hide key" : "Show key", title: show ? "Hide" : "Show", children: "\uD83D\uDC41" })] })] }));
}
function FeedbackListPanel() {
    const [items, setItems] = useState(null);
    const [error, setError] = useState(null);
    const [selected, setSelected] = useState(null);
    const reload = () => {
        listFeedbacks()
            .then((v) => setItems(v))
            .catch((e) => setError(e instanceof Error ? e.message : String(e)));
    };
    useEffect(() => { reload(); }, []);
    const remove = async (id) => {
        try {
            await deleteFeedback(id);
            if (selected?.id === id)
                setSelected(null);
            reload();
        }
        catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
    };
    const kindBadge = (k) => {
        const map = {
            bug: { bg: "#fde7e9", fg: "#b3261e", label: "🐞 Bug" },
            error: { bg: "#fff4e0", fg: "#8a5a00", label: "⚠ Erreur" },
            evolution: { bg: "#e1ecff", fg: "#1f4ba8", label: "✦ Évolution" },
            improvement: { bg: "#e3f6e8", fg: "#1f7a3a", label: "💡 Amélioration" },
        };
        const m = map[k] ?? { bg: "#eee", fg: "#444", label: k };
        return (_jsx("span", { style: {
                background: m.bg, color: m.fg, padding: "2px 8px",
                borderRadius: 999, fontSize: 12, whiteSpace: "nowrap",
            }, children: m.label }));
    };
    return (_jsxs(Card, { title: "Feedback re\u00E7us", subtitle: "Bugs, suggestions et am\u00E9liorations envoy\u00E9s depuis l'application.", children: [error && _jsx("div", { className: "error-banner", children: error }), !items ? (_jsx("div", { className: "empty", children: "Chargement\u2026" })) : items.length === 0 ? (_jsx("div", { className: "empty", children: "Aucun feedback pour l'instant." })) : (_jsxs("div", { style: { display: "grid", gridTemplateColumns: selected ? "1fr 1fr" : "1fr", gap: 16 }, children: [_jsx("div", { children: _jsxs("table", { style: { width: "100%", borderCollapse: "collapse" }, children: [_jsx("thead", { children: _jsxs("tr", { style: { textAlign: "left", borderBottom: "1px solid var(--border)" }, children: [_jsx("th", { style: { padding: 8 }, children: "Type" }), _jsx("th", { style: { padding: 8 }, children: "Titre" }), _jsx("th", { style: { padding: 8 }, children: "Date" }), _jsx("th", { style: { padding: 8 } })] }) }), _jsx("tbody", { children: items.map((f) => (_jsxs("tr", { style: {
                                            borderBottom: "1px solid var(--border)",
                                            cursor: "pointer",
                                            background: selected?.id === f.id ? "var(--panel-2)" : "transparent",
                                        }, onClick: () => setSelected(f), children: [_jsx("td", { style: { padding: 8 }, children: kindBadge(f.kind) }), _jsx("td", { style: { padding: 8 }, children: f.title }), _jsx("td", { style: { padding: 8, color: "var(--muted)", fontSize: 12 }, children: new Date(f.created_at * 1000).toLocaleString() }), _jsx("td", { style: { padding: 8 }, children: _jsx("button", { className: "btn btn-ghost", onClick: (e) => { e.stopPropagation(); remove(f.id); }, title: "Supprimer", children: "\uD83D\uDDD1" }) })] }, f.id))) })] }) }), selected && (_jsxs("div", { style: { borderLeft: "1px solid var(--border)", paddingLeft: 16 }, children: [_jsxs("div", { style: { display: "flex", justifyContent: "space-between", alignItems: "center" }, children: [_jsx("h3", { style: { margin: 0 }, children: selected.title }), _jsx("button", { className: "icon-btn", onClick: () => setSelected(null), children: "\u00D7" })] }), _jsxs("div", { style: { marginTop: 8, display: "flex", gap: 8, alignItems: "center" }, children: [kindBadge(selected.kind), _jsx("span", { style: { color: "var(--muted)", fontSize: 12 }, children: new Date(selected.created_at * 1000).toLocaleString() })] }), selected.description && (_jsx("p", { style: { whiteSpace: "pre-wrap", marginTop: 12 }, children: selected.description })), selected.url && (_jsxs("p", { style: { fontSize: 12, color: "var(--muted)" }, children: ["URL : ", _jsx("code", { children: selected.url })] })), selected.user_agent && (_jsxs("p", { style: { fontSize: 12, color: "var(--muted)" }, children: ["User-Agent : ", _jsx("code", { children: selected.user_agent })] })), selected.screenshot && (_jsxs("div", { style: { marginTop: 12 }, children: [_jsx("div", { style: { fontWeight: 600, marginBottom: 4 }, children: "Capture d'\u00E9cran" }), _jsx("img", { src: selected.screenshot, alt: "screenshot", style: { maxWidth: "100%", border: "1px solid var(--border)", borderRadius: 6 } })] })), selected.frontend_logs && (_jsxs("details", { style: { marginTop: 12 }, children: [_jsxs("summary", { children: ["Logs frontend (", selected.frontend_logs.split("\n").length, " lignes)"] }), _jsx("pre", { style: { maxHeight: 240, overflow: "auto", background: "var(--panel-2)", padding: 8, fontSize: 11 }, children: selected.frontend_logs })] })), selected.backend_logs && (_jsxs("details", { style: { marginTop: 8 }, children: [_jsxs("summary", { children: ["Logs backend (", selected.backend_logs.split("\n").length, " lignes)"] }), _jsx("pre", { style: { maxHeight: 240, overflow: "auto", background: "var(--panel-2)", padding: 8, fontSize: 11 }, children: selected.backend_logs })] }))] }))] }))] }));
}
