// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { useEffect, useState } from "react";
import Card from "../components/Card";
import { apiBase, getSettings, getStats, patchSettings, listFeedbacks, deleteFeedback, getOcrStatus, type Feedback, type LlmSettingsPatch, type OcrStatus, type Settings as ServerSettings } from "../api";
import { loadProviderConfig, saveProviderConfig } from "../lib/providerConfig";
import type { LLMProvider, ProviderConfig } from "../types/providerConfig";

interface TestResult { ok: boolean; message: string; }

type SettingsTab = "general" | "diagnostics" | "feedback";

export default function Settings() {
  const [tab, setTab] = useState<SettingsTab>("general");
  const [s, setS] = useState<ServerSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);

  // Provider & Keys state
  const [cfg, setCfg] = useState<ProviderConfig>(() => loadProviderConfig());
  const [openSection, setOpenSection] = useState<"provider" | "keys" | null>("provider");
  const [showOpenaiKey, setShowOpenaiKey] = useState(false);
  const [showAnthropicKey, setShowAnthropicKey] = useState(false);
  const [showInfomaniakKey, setShowInfomaniakKey] = useState(false);
  const [showBearer, setShowBearer] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<TestResult | null>(null);

  useEffect(() => {
    getSettings().then(setS).catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, []);

  const apply = async (patch: Parameters<typeof patchSettings>[0]) => {
    setError(null);
    setInfo(null);
    try {
      const next = await patchSettings(patch);
      setS(next);
      setInfo("Settings saved.");
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const updateCfg = <K extends keyof ProviderConfig>(key: K, value: ProviderConfig[K]) => {
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
      if (!base) throw new Error("Ontology API base URL is empty");
      const headers: Record<string, string> = {};
      if (cfg.ontologyBearerToken.trim()) {
        headers["authorization"] = `Bearer ${cfg.ontologyBearerToken.trim()}`;
      }
      const res = await fetch(`${base}/healthz`, { headers });
      if (!res.ok) throw new Error(`HTTP ${res.status} ${res.statusText}`);
      let version = "unknown";
      try {
        const data = await res.json();
        if (data && typeof data.version === "string") version = data.version;
      } catch {
        /* /healthz may return text — ignore */
      }
      setTestResult({ ok: true, message: `✅ Server reachable — version ${version}` });
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      setTestResult({ ok: false, message: `❌ ${msg}` });
    } finally {
      setTesting(false);
    }
  };

  return (
    <>
      <div className="page-header">
        <div>
          <h1 className="page-title">Settings</h1>
          <p className="page-subtitle">Retrieval defaults, UI preferences, providers and server connection.</p>
        </div>
      </div>

      <div className="tabs" style={{ display: "flex", gap: 4, borderBottom: "1px solid var(--border)", marginBottom: 16 }}>
        <button
          className={`btn ${tab === "general" ? "btn-primary" : "btn-ghost"}`}
          style={{ borderRadius: "var(--radius-sm) var(--radius-sm) 0 0" }}
          onClick={() => setTab("general")}
        >
          ⚙ Général
        </button>
        <button
          className={`btn ${tab === "diagnostics" ? "btn-primary" : "btn-ghost"}`}
          style={{ borderRadius: "var(--radius-sm) var(--radius-sm) 0 0" }}
          onClick={() => setTab("diagnostics")}
        >
          ✦ Diagnostique
        </button>
        <button
          className={`btn ${tab === "feedback" ? "btn-primary" : "btn-ghost"}`}
          style={{ borderRadius: "var(--radius-sm) var(--radius-sm) 0 0" }}
          onClick={() => setTab("feedback")}
        >
          💬 Feedback
        </button>
      </div>

      {tab === "diagnostics" ? (
        <DiagnosticsPanel />
      ) : tab === "feedback" ? (
        <FeedbackListPanel />
      ) : (
        <GeneralSettings
          s={s}
          error={error}
          info={info}
          cfg={cfg}
          openSection={openSection}
          setOpenSection={setOpenSection}
          showOpenaiKey={showOpenaiKey}
          setShowOpenaiKey={setShowOpenaiKey}
          showAnthropicKey={showAnthropicKey}
          setShowAnthropicKey={setShowAnthropicKey}
          showInfomaniakKey={showInfomaniakKey}
          setShowInfomaniakKey={setShowInfomaniakKey}
          showBearer={showBearer}
          setShowBearer={setShowBearer}
          testing={testing}
          testResult={testResult}
          apply={apply}
          updateCfg={updateCfg}
          handleSave={handleSave}
          handleTest={handleTest}
        />
      )}
    </>
  );
}

interface GeneralSettingsProps {
  s: ServerSettings | null;
  error: string | null;
  info: string | null;
  cfg: ProviderConfig;
  openSection: "provider" | "keys" | null;
  setOpenSection: (v: "provider" | "keys" | null) => void;
  showOpenaiKey: boolean;
  setShowOpenaiKey: React.Dispatch<React.SetStateAction<boolean>>;
  showAnthropicKey: boolean;
  setShowAnthropicKey: React.Dispatch<React.SetStateAction<boolean>>;
  showInfomaniakKey: boolean;
  setShowInfomaniakKey: React.Dispatch<React.SetStateAction<boolean>>;
  showBearer: boolean;
  setShowBearer: React.Dispatch<React.SetStateAction<boolean>>;
  testing: boolean;
  testResult: TestResult | null;
  apply: (patch: Parameters<typeof patchSettings>[0]) => Promise<void>;
  updateCfg: <K extends keyof ProviderConfig>(key: K, value: ProviderConfig[K]) => void;
  handleSave: () => void;
  handleTest: () => Promise<void>;
}

function GeneralSettings({
  s, error, info, cfg, openSection, setOpenSection,
  showOpenaiKey, setShowOpenaiKey, showAnthropicKey, setShowAnthropicKey,
  showInfomaniakKey, setShowInfomaniakKey, showBearer, setShowBearer,
  testing, testResult, apply, updateCfg, handleSave, handleTest,
}: GeneralSettingsProps) {
  return (
    <>
      {error && <div className="error-banner">{error}</div>}
      {info && <div className="success-banner">{info}</div>}

      <div className="grid grid-2" style={{ alignItems: "start", marginBottom: 16 }}>
        <Card title="Retrieval defaults">
          {!s ? (
            <div className="empty">Loading…</div>
          ) : (
            <>
              <div className="setting-row">
                <div className="meta">
                  <strong>Top-K</strong>
                  Number of seed concepts retrieved per query.
                </div>
                <input
                  type="number"
                  min={1}
                  max={50}
                  value={s.retrieval.top_k}
                  onChange={(e) => apply({ retrieval: { top_k: Number(e.target.value) } })}
                />
              </div>
              <div className="setting-row">
                <div className="meta">
                  <strong>Lexical weight</strong>
                  0 = vector-only, 1 = BM25-only.
                </div>
                <input
                  type="number"
                  step={0.1}
                  min={0}
                  max={1}
                  value={s.retrieval.lexical_weight}
                  onChange={(e) => apply({ retrieval: { lexical_weight: Number(e.target.value) } })}
                />
              </div>
              <div className="setting-row">
                <div className="meta">
                  <strong>Expansion depth</strong>
                  Hops added to each seed when traversing.
                </div>
                <input
                  type="number"
                  min={0}
                  max={5}
                  value={s.retrieval.expansion_depth}
                  onChange={(e) => apply({ retrieval: { expansion_depth: Number(e.target.value) } })}
                />
              </div>
            </>
          )}
        </Card>

        <Card title="UI preferences">
          {!s ? (
            <div className="empty">Loading…</div>
          ) : (
            <>
              <div className="setting-row">
                <div className="meta">
                  <strong>Theme</strong>
                  Color scheme (light only at the moment).
                </div>
                <select
                  value={s.ui.theme}
                  onChange={(e) => apply({ ui: { theme: e.target.value } })}
                >
                  <option value="light">Light</option>
                  <option value="dark">Dark</option>
                </select>
              </div>
              <div className="setting-row">
                <div className="meta">
                  <strong>Graph layout</strong>
                  Layout engine for Graph View.
                </div>
                <select
                  value={s.ui.graph_layout}
                  onChange={(e) => apply({ ui: { graph_layout: e.target.value } })}
                >
                  <option value="dagre">Dagre (hierarchical)</option>
                  <option value="force">Force-directed</option>
                </select>
              </div>
            </>
          )}
        </Card>
      </div>

      {s && (
        <LlmServerSection
          settings={s}
          onPatch={(patch) => apply({ llm: patch })}
        />
      )}

      {s && (
        <OcrSection
          settings={s}
          onPatch={(patch) => apply({ ocr: patch })}
        />
      )}

      <Card title="Provider & Keys" subtitle="API keys and server endpoints. Stored in your browser only.">
        {/* LLM Provider section */}
        <div className="provider-section">
          <div
            className="provider-section-header"
            onClick={() => setOpenSection(openSection === "provider" ? null : "provider")}
          >
            <span>LLM provider</span>
            <span>{openSection === "provider" ? "▾" : "▸"}</span>
          </div>
          {openSection === "provider" && (
            <div className="provider-section-body">
              <label className="field">
                <span>Active provider</span>
                <select
                  value={cfg.activeLLMProvider}
                  onChange={(e) => updateCfg("activeLLMProvider", e.target.value as LLMProvider)}
                >
                  <option value="default">Default (server-configured)</option>
                  <option value="openai">OpenAI</option>
                  <option value="anthropic">Anthropic</option>
                  <option value="infomaniak">Infomaniak AI (Swiss cloud)</option>
                </select>
                <p className="field-hint">
                  This provider is used by the ingest wizard and ontology builder
                  when the per-page selector is left on "default".
                </p>
              </label>
              {cfg.activeLLMProvider === "infomaniak" && (
                <>
                  <label className="field">
                    <span>Infomaniak base URL</span>
                    <input
                      type="text"
                      value={cfg.infomaniakBaseUrl}
                      onChange={(e) => updateCfg("infomaniakBaseUrl", e.target.value)}
                      placeholder="https://api.infomaniak.com/1/ai"
                    />
                  </label>
                  <label className="field">
                    <span>Infomaniak model (optional)</span>
                    <input
                      type="text"
                      value={cfg.infomaniakModel}
                      onChange={(e) => updateCfg("infomaniakModel", e.target.value)}
                      placeholder="mixtral8x22b"
                    />
                  </label>
                </>
              )}
            </div>
          )}
        </div>

        {/* API Keys section */}
        <div className="provider-section">
          <div
            className="provider-section-header"
            onClick={() => setOpenSection(openSection === "keys" ? null : "keys")}
          >
            <span>API keys</span>
            <span>{openSection === "keys" ? "▾" : "▸"}</span>
          </div>
          {openSection === "keys" && (
            <div className="provider-section-body">
              <KeyField
                label="OpenAI API key"
                value={cfg.openaiKey}
                onChange={(v) => updateCfg("openaiKey", v)}
                show={showOpenaiKey}
                onToggle={() => setShowOpenaiKey((v) => !v)}
                placeholder="sk-…"
              />
              <KeyField
                label="Anthropic API key"
                value={cfg.anthropicKey}
                onChange={(v) => updateCfg("anthropicKey", v)}
                show={showAnthropicKey}
                onToggle={() => setShowAnthropicKey((v) => !v)}
                placeholder="sk-ant-…"
              />
              <KeyField
                label="Infomaniak API key"
                value={cfg.infomaniakKey}
                onChange={(v) => updateCfg("infomaniakKey", v)}
                show={showInfomaniakKey}
                onToggle={() => setShowInfomaniakKey((v) => !v)}
                placeholder="(Infomaniak personal API token)"
              />
              <p className="field-hint">
                Keys are stored in plaintext in this browser&apos;s localStorage.
                Only enter them on trusted devices.
              </p>
            </div>
          )}
        </div>

        {/* Server URLs (always visible) */}
        <div style={{ marginTop: 12, display: "flex", flexDirection: "column", gap: 12 }}>
          <label className="field">
            <span>Ontology API base URL</span>
            <input
              type="text"
              value={cfg.ontologyApiUrl}
              onChange={(e) => updateCfg("ontologyApiUrl", e.target.value)}
              placeholder="http://localhost:5000"
            />
          </label>
          <div className="field">
            <label>Bearer token (optional)</label>
            <KeyField
              label=""
              value={cfg.ontologyBearerToken}
              onChange={(v) => updateCfg("ontologyBearerToken", v)}
              show={showBearer}
              onToggle={() => setShowBearer((v) => !v)}
              placeholder="(leave blank if server is open)"
            />
          </div>
          <label className="field">
            <span>Auth server URL</span>
            <input
              type="text"
              value={cfg.authApiUrl}
              onChange={(e) => updateCfg("authApiUrl", e.target.value)}
              placeholder="http://localhost:4000"
            />
          </label>
        </div>

        <div style={{ display: "flex", gap: 8, marginTop: 16 }}>
          <button className="btn btn-primary" onClick={handleSave}>💾 Save all</button>
          <button className="btn btn-outline" onClick={handleTest} disabled={testing}>
            {testing ? "Testing…" : "🔌 Test connection"}
          </button>
        </div>
        {testResult && (
          <div className={testResult.ok ? "success-banner" : "error-banner"} style={{ marginTop: 12 }}>
            {testResult.message}
          </div>
        )}

        <p className="field-hint" style={{ marginTop: 12 }}>
          Currently resolving API base to <code>{apiBase() || "(same-origin)"}</code>.
        </p>
      </Card>
    </>
  );
}

// -- Diagnostics tab --------------------------------------------------------

type CheckStatus = "ok" | "warn" | "error";

interface CheckResult {
  name: string;
  status: CheckStatus;
  summary: string;
  latencyMs: number;
  details?: string;
}

function DiagnosticsPanel() {
  const [running, setRunning] = useState(false);
  const [checks, setChecks] = useState<CheckResult[]>([]);
  const [totalMs, setTotalMs] = useState(0);
  const [lastRun, setLastRun] = useState<Date | null>(null);
  const [expanded, setExpanded] = useState<Record<number, boolean>>({});

  const run = async () => {
    setRunning(true);
    setChecks([]);
    const results: CheckResult[] = [];
    const overallStart = performance.now();

    const time = async <T,>(fn: () => Promise<T>): Promise<{ value: T | null; err: unknown; ms: number }> => {
      const start = performance.now();
      try {
        const value = await fn();
        return { value, err: null, ms: performance.now() - start };
      } catch (err) {
        return { value: null, err, ms: performance.now() - start };
      }
    };

    // 1. Server reachability
    {
      const base = apiBase() || window.location.origin;
      const r = await time(async () => {
        const res = await fetch(`${base}/healthz`);
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
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
    } else {
      const s = statsRes.value!;
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
        if (!res.ok || !data.ok) throw new Error(data.error || `HTTP ${res.status}`);
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
      } catch {
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
  const overallStatus: CheckStatus = errCount > 0 ? "error" : warnCount > 0 ? "warn" : "ok";

  const ua = typeof navigator !== "undefined" ? navigator.userAgent : "—";
  const platform = typeof navigator !== "undefined" ? (navigator.platform || "—") : "—";
  const env = (import.meta.env.MODE as string) || "development";

  return (
    <Card
      title={
        <span style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
          <span style={{ color: "var(--accent)" }}>✦</span>
          Auto-diagnostic de l'application
        </span>
      }
      actions={
        <button className="btn btn-primary" onClick={run} disabled={running}>
          {running ? "Analyse…" : "↻ Relancer"}
        </button>
      }
    >
      {checks.length > 0 && (
        <div
          className={
            overallStatus === "error" ? "error-banner"
            : overallStatus === "warn" ? "warn-banner"
            : "success-banner"
          }
          style={{ marginBottom: 16 }}
        >
          <strong style={{ display: "block", marginBottom: 4 }}>
            {overallStatus === "error" ? "Erreurs détectées"
              : overallStatus === "warn" ? "Avertissements détectés"
              : "Tous les contrôles sont passés"}
          </strong>
          {okCount} OK · {warnCount} avertissement{warnCount > 1 ? "s" : ""} · {errCount} erreur{errCount > 1 ? "s" : ""} · {Math.round(totalMs)} ms
        </div>
      )}

      <div className="grid grid-4" style={{ gap: 12, marginBottom: 16 }}>
        <InfoTile label="Environnement" value={env} />
        <InfoTile label="Plateforme" value={platform} />
        <InfoTile label="API base" value={apiBase() || "(same-origin)"} />
        <InfoTile label="Dernière vérification" value={lastRun ? lastRun.toLocaleTimeString() : "—"} />
      </div>

      {checks.length === 0 && running && <div className="empty">Analyse en cours…</div>}

      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        {checks.map((c, i) => (
          <DiagRow
            key={i}
            check={c}
            open={!!expanded[i]}
            onToggle={() => setExpanded((e) => ({ ...e, [i]: !e[i] }))}
          />
        ))}
      </div>

      <p className="field-hint" style={{ marginTop: 16 }}>
        User-Agent: <code style={{ wordBreak: "break-all" }}>{ua}</code>
      </p>
    </Card>
  );
}

function InfoTile({ label, value }: { label: string; value: string }) {
  return (
    <div
      style={{
        padding: "10px 12px",
        background: "var(--panel-2)",
        border: "1px solid var(--border)",
        borderRadius: "var(--radius-sm)",
      }}
    >
      <div style={{ fontSize: 12, color: "var(--muted)" }}>{label}</div>
      <div style={{ fontWeight: 600, marginTop: 2, wordBreak: "break-all" }}>{value}</div>
    </div>
  );
}

function DiagRow({ check, open, onToggle }: { check: CheckResult; open: boolean; onToggle: () => void }) {
  const palette = check.status === "ok"
    ? { bg: "#ecfdf5", border: "#a7f3d0", icon: "✓", color: "#047857" }
    : check.status === "warn"
    ? { bg: "#fefce8", border: "#fde68a", icon: "⚠", color: "#b45309" }
    : { bg: "#fef2f2", border: "#fecaca", icon: "✕", color: "#b91c1c" };
  return (
    <div
      style={{
        background: palette.bg,
        border: `1px solid ${palette.border}`,
        borderRadius: "var(--radius-sm)",
        padding: "10px 14px",
      }}
    >
      <div
        style={{ display: "flex", alignItems: "center", gap: 12, cursor: "pointer" }}
        onClick={onToggle}
      >
        <span style={{ color: palette.color, fontWeight: 700, fontSize: 16 }}>{palette.icon}</span>
        <div style={{ flex: 1 }}>
          <div style={{ fontWeight: 600 }}>{check.name}</div>
          <div style={{ fontSize: 13, color: "var(--muted)" }}>{check.summary}</div>
        </div>
        <span style={{ fontSize: 12, color: "var(--muted)" }}>{Math.round(check.latencyMs)} ms</span>
        <span style={{ color: "var(--muted)" }}>{open ? "▾" : "▸"}</span>
      </div>
      {open && check.details && (
        <pre
          style={{
            marginTop: 8,
            background: "var(--panel)",
            border: "1px solid var(--border)",
            borderRadius: "var(--radius-sm)",
            padding: 10,
            fontSize: 12,
            whiteSpace: "pre-wrap",
            wordBreak: "break-all",
          }}
        >
          {check.details}
        </pre>
      )}
    </div>
  );
}

function errMsg(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

function cfgHas(key: keyof ProviderConfig): boolean {
  try {
    const raw = window.localStorage.getItem("ontology.providerConfig");
    if (!raw) return false;
    const obj = JSON.parse(raw);
    return Boolean(obj && obj[key]);
  } catch {
    return false;
  }
}

// -- Server-stored LLM section (clés/URL envoyées au backend) ---------------

interface LlmServerSectionProps {
  settings: ServerSettings;
  onPatch: (patch: LlmSettingsPatch) => Promise<void> | void;
}

const PROVIDER_LABELS: Record<string, string> = {
  openai: "OpenAI",
  anthropic: "Anthropic",
  infomaniak: "Infomaniak AI",
};

const BASE_URL_PRESETS: Record<string, Array<{ value: string; label: string }>> = {
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

const MODEL_PRESETS: Record<string, Array<{ value: string; label: string }>> = {
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

function LlmServerSection({ settings, onPatch }: LlmServerSectionProps) {
  const llm = settings.llm;
  const initialProvider = (llm.active_provider && llm.active_provider !== "default")
    ? llm.active_provider : "openai";
  const [provider, setProvider] = useState<string>(initialProvider);
  const [apiKey, setApiKey] = useState("");
  const [showKey, setShowKey] = useState(false);
  const [baseUrl, setBaseUrl] = useState("");
  const [model, setModel] = useState("");
  const [models, setModels] = useState<Array<{ value: string; label: string }>>(MODEL_PRESETS.openai);
  const [temperature, setTemperature] = useState<number>(llm.temperature ?? 0.3);
  const [maxTokens, setMaxTokens] = useState<number>(llm.max_tokens ?? 1000);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [loadingModels, setLoadingModels] = useState(false);
  const [testMsg, setTestMsg] = useState<string | null>(null);

  // Re-sync local form when server settings or selected provider change.
  useEffect(() => {
    if (provider === "openai") {
      setBaseUrl(llm.openai_base_url);
      setModel(llm.openai_model || "gpt-4o");
    } else if (provider === "anthropic") {
      setBaseUrl(llm.anthropic_base_url);
      setModel(llm.anthropic_model || "claude-opus-4-7");
    } else if (provider === "infomaniak") {
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

  const buildPatch = (includeKey: boolean): LlmSettingsPatch => {
    const patch: LlmSettingsPatch = {
      active_provider: provider,
      temperature,
      max_tokens: maxTokens,
    };
    if (provider === "openai") {
      if (includeKey && apiKey.trim()) patch.openai_api_key = apiKey.trim();
      patch.openai_base_url = baseUrl;
      patch.openai_model = model;
    } else if (provider === "anthropic") {
      if (includeKey && apiKey.trim()) patch.anthropic_api_key = apiKey.trim();
      patch.anthropic_base_url = baseUrl;
      patch.anthropic_model = model;
    } else if (provider === "infomaniak") {
      if (includeKey && apiKey.trim()) patch.infomaniak_api_key = apiKey.trim();
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
    } finally {
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
      } else {
        setTestMsg(`❌ ${data.error || `HTTP ${res.status}`}`);
      }
      setApiKey("");
    } catch (e) {
      setTestMsg(`❌ ${e instanceof Error ? e.message : String(e)}`);
    } finally {
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
        const fetched: Array<{ value: string; label: string }> = data.models.map((m: string) => ({
          value: m, label: m,
        }));
        if (fetched.length > 0) {
          setModels(fetched);
          setTestMsg(`✅ ${fetched.length} modèles chargés depuis l'API`);
        } else {
          setTestMsg("⚠️ Aucun modèle retourné");
        }
      } else {
        setTestMsg(`❌ ${data.error || `HTTP ${res.status}`}`);
      }
      setApiKey("");
    } catch (e) {
      setTestMsg(`❌ ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setLoadingModels(false);
    }
  };

  const baseUrlPresets = BASE_URL_PRESETS[provider] ?? [];
  const isCustomUrl = baseUrl !== "" && !baseUrlPresets.some((p) => p.value === baseUrl);
  const isCustomModel = model !== "" && !models.some((m) => m.value === model);

  return (
    <Card
      title={
        <span style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
          <span style={{ color: "var(--accent)" }}>✦</span>
          Configuration {providerLabel}
        </span>
      }
      subtitle="Clé API pour l'extraction intelligente avec IA"
      actions={
        configured ? (
          <span className="badge badge-success">⊘ Configuré</span>
        ) : (
          <span className="badge badge-warn">Non configuré</span>
        )
      }
    >
      <label className="field" style={{ marginBottom: 4 }}>
        <span>Fournisseur</span>
        <select value={provider} onChange={(e) => setProvider(e.target.value)}>
          {Object.entries(PROVIDER_LABELS).map(([k, v]) => (
            <option key={k} value={k}>{v}</option>
          ))}
        </select>
      </label>

      <label className="field" style={{ marginTop: 16 }}>
        <span>Clé API {providerLabel}</span>
        <div style={{ display: "flex", gap: 8 }}>
          <div className="key-field-wrapper" style={{ flex: 1 }}>
            <input
              type={showKey ? "text" : "password"}
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder={configured ? "••••••••••" : "sk-proj-…"}
              autoComplete="off"
              spellCheck={false}
            />
            <button
              type="button"
              className="key-toggle-btn"
              onClick={() => setShowKey((v) => !v)}
              title={showKey ? "Masquer" : "Afficher"}
            >👁</button>
          </div>
          <button
            className="btn btn-primary"
            onClick={save}
            disabled={saving}
            style={{ whiteSpace: "nowrap" }}
          >
            {saving ? "Sauvegarde…" : "💾 Sauvegarder"}
          </button>
        </div>
        {configured && (
          <div
            style={{
              marginTop: 8,
              padding: "8px 12px",
              background: "var(--panel-2)",
              border: "1px solid var(--border)",
              borderRadius: "var(--radius-sm)",
              display: "flex",
              justifyContent: "space-between",
              fontSize: 13,
            }}
          >
            <span style={{ color: "var(--muted)" }}>Clé actuelle :</span>
            <code>{hint}</code>
          </div>
        )}
        {provider === "openai" && (
          <p className="field-hint" style={{ marginTop: 6 }}>
            Format: sk-proj-… Obtenez votre clé sur{" "}
            <a href="https://platform.openai.com/api-keys" target="_blank" rel="noopener noreferrer">
              platform.openai.com
            </a>
          </p>
        )}
        {provider === "anthropic" && (
          <p className="field-hint" style={{ marginTop: 6 }}>
            Format: sk-ant-… Obtenez votre clé sur{" "}
            <a href="https://console.anthropic.com/settings/keys" target="_blank" rel="noopener noreferrer">
              console.anthropic.com
            </a>
          </p>
        )}
      </label>

      <label className="field" style={{ marginTop: 16 }}>
        <span>URL de base API (optionnel)</span>
        <select
          value={isCustomUrl ? "__custom__" : baseUrl}
          onChange={(e) => {
            if (e.target.value === "__custom__") setBaseUrl("https://");
            else setBaseUrl(e.target.value);
          }}
        >
          {baseUrlPresets.map((p) => (
            <option key={p.value || "default"} value={p.value}>{p.label}</option>
          ))}
          <option value="__custom__">Personnalisé…</option>
        </select>
        {(isCustomUrl || baseUrl.startsWith("https://Y")) && (
          <input
            type="text"
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
            placeholder="https://votre-endpoint/v1"
            style={{ marginTop: 8 }}
          />
        )}
        <p className="field-hint">
          Utilisez une URL personnalisée pour Azure OpenAI, proxies régionaux (Suisse, Europe) ou endpoints privés.
        </p>
      </label>

      <div className="grid grid-3" style={{ gap: 16, marginTop: 16 }}>
        <label className="field">
          <span style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <span>Modèle</span>
            <button
              type="button"
              className="link-btn"
              onClick={loadModelsFromApi}
              disabled={loadingModels || !configured}
              style={{
                background: "none",
                border: "none",
                color: "var(--accent)",
                cursor: configured ? "pointer" : "not-allowed",
                fontSize: 12,
                padding: 0,
              }}
              title={configured ? "Charger les modèles depuis l'API" : "Sauvegardez la clé d'abord"}
            >
              {loadingModels ? "…" : "↻ Charger depuis API"}
            </button>
          </span>
          <select
            value={isCustomModel ? "__custom__" : model}
            onChange={(e) => {
              if (e.target.value === "__custom__") setModel("");
              else setModel(e.target.value);
            }}
          >
            {models.map((m) => (
              <option key={m.value} value={m.value}>{m.label}</option>
            ))}
            <option value="__custom__">Personnalisé…</option>
          </select>
          {isCustomModel && (
            <input
              type="text"
              value={model}
              onChange={(e) => setModel(e.target.value)}
              placeholder="nom-du-modèle"
              style={{ marginTop: 8 }}
            />
          )}
        </label>
        <label className="field">
          <span>Température ({temperature.toFixed(2)})</span>
          <input
            type="range"
            min={0}
            max={1}
            step={0.05}
            value={temperature}
            onChange={(e) => setTemperature(Number(e.target.value))}
          />
        </label>
        <label className="field">
          <span>Max Tokens</span>
          <input
            type="number"
            min={1}
            max={32000}
            value={maxTokens}
            onChange={(e) => setMaxTokens(Number(e.target.value))}
          />
        </label>
      </div>

      <button
        className="btn btn-outline"
        onClick={testConnection}
        disabled={testing}
        style={{
          marginTop: 20,
          width: "100%",
          padding: "12px",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          gap: 8,
        }}
      >
        {testing ? "Test en cours…" : <>✦ Tester la connexion {providerLabel}</>}
      </button>
      {testMsg && (
        <div
          className={testMsg.startsWith("✅") ? "success-banner" : testMsg.startsWith("⚠️") ? "warn-banner" : "error-banner"}
          style={{ marginTop: 12 }}
        >
          {testMsg}
        </div>
      )}
    </Card>
  );
}

// -- OCR section ------------------------------------------------------------

interface OcrSectionProps {
  settings: ServerSettings;
  onPatch: (patch: NonNullable<Parameters<typeof patchSettings>[0]["ocr"]>) => Promise<void> | void;
}

function OcrSection({ settings, onPatch }: OcrSectionProps) {
  const ocr = settings.ocr;
  const [status, setStatus] = useState<OcrStatus | null>(null);
  const [apiKey, setApiKey] = useState("");
  const [showKey, setShowKey] = useState(false);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    getOcrStatus().then(setStatus).catch(() => setStatus(null));
  }, [ocr.google_api_key_hint]);

  const saveKey = async () => {
    if (!apiKey.trim()) return;
    setSaving(true);
    try {
      await onPatch({ google_api_key: apiKey.trim() });
      setApiKey("");
    } finally {
      setSaving(false);
    }
  };

  const tessAvail = status?.tesseract.available ?? false;
  const gsAvail = status?.tesseract.ghostscript_available ?? false;
  const googleConfigured = status?.google_vision.configured ?? Boolean(ocr.google_api_key_hint);

  return (
    <Card
      title={
        <span style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
          <span style={{ color: "var(--accent)" }}>👁</span>
          Moteur OCR
        </span>
      }
      subtitle="Sélectionnez le moteur de reconnaissance optique de caractères"
    >
      <label className="field">
        <span>Fournisseur OCR principal</span>
        <select
          value={ocr.provider}
          onChange={(e) => onPatch({ provider: e.target.value })}
        >
          <option value="tesseract">Tesseract (Local — gratuit)</option>
          <option value="google_vision">Google Cloud Vision</option>
        </select>
      </label>

      <div className="grid grid-2" style={{ gap: 16, marginTop: 16 }}>
        <div
          style={{
            border: "1px solid var(--border)",
            borderRadius: "var(--radius-sm)",
            padding: 14,
            background: "var(--panel-2)",
          }}
        >
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <strong>Tesseract (Local)</strong>
            <span className={tessAvail ? "badge badge-success" : "badge badge-warn"}>
              {tessAvail ? "⊘ Disponible" : "⚠ Indisponible"}
            </span>
          </div>
          <div style={{ marginTop: 8, fontSize: 13, color: "var(--muted)", display: "flex", flexDirection: "column", gap: 2 }}>
            <span>ocrmypdf: {status?.tesseract.ocrmypdf_version ?? "—"}</span>
            <span>Tesseract: {tessAvail ? "✓" : "✕"}</span>
            <span>Ghostscript: {gsAvail ? "✓" : "✕"}</span>
          </div>
        </div>

        <div
          style={{
            border: "1px solid var(--border)",
            borderRadius: "var(--radius-sm)",
            padding: 14,
            background: "var(--panel-2)",
          }}
        >
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <strong>Google Cloud Vision</strong>
            <span className={googleConfigured ? "badge badge-success" : "badge badge-warn"}>
              {googleConfigured ? "⊘ Configuré" : "Non configuré"}
            </span>
          </div>
          <div style={{ marginTop: 8, fontSize: 13, color: "var(--muted)" }}>
            Auth: Clé API {ocr.google_api_key_hint && (<code style={{ marginLeft: 6 }}>{ocr.google_api_key_hint}</code>)}
          </div>
        </div>
      </div>

      <div
        style={{
          marginTop: 16,
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          gap: 16,
          padding: "12px 14px",
          background: "var(--panel-2)",
          border: "1px solid var(--border)",
          borderRadius: "var(--radius-sm)",
        }}
      >
        <div>
          <div style={{ fontWeight: 600 }}>Fallback automatique</div>
          <div style={{ fontSize: 13, color: "var(--muted)" }}>
            Si le moteur principal échoue ou produit un résultat insuffisant, basculer automatiquement sur l'autre moteur.
          </div>
        </div>
        <label style={{ position: "relative", display: "inline-block", width: 44, height: 24 }}>
          <input
            type="checkbox"
            checked={ocr.auto_fallback}
            onChange={(e) => onPatch({ auto_fallback: e.target.checked })}
            style={{ opacity: 0, width: 0, height: 0 }}
          />
          <span
            style={{
              position: "absolute",
              inset: 0,
              background: ocr.auto_fallback ? "var(--success, #10b981)" : "#cbd5e1",
              borderRadius: 999,
              transition: "background .15s",
              cursor: "pointer",
            }}
          />
          <span
            style={{
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
            }}
          />
        </label>
      </div>

      <label className="field" style={{ marginTop: 16 }}>
        <span>Seuil minimum de texte (caractères)</span>
        <input
          type="number"
          min={0}
          max={10000}
          value={ocr.min_text_threshold}
          onChange={(e) => onPatch({ min_text_threshold: Number(e.target.value) })}
        />
        <p className="field-hint">
          Si l'OCR retourne moins de {ocr.min_text_threshold} caractères, le fallback est déclenché.
        </p>
      </label>

      <label className="field" style={{ marginTop: 16 }}>
        <span>Langues OCR (Tesseract)</span>
        <input
          type="text"
          value={ocr.tesseract_languages}
          onChange={(e) => onPatch({ tesseract_languages: e.target.value })}
          placeholder="fra+deu+eng"
        />
        <p className="field-hint">
          Codes ISO 639-2 séparés par <code>+</code> (ex. <code>fra+deu+eng</code>). Les packs correspondants doivent être installés.
        </p>
      </label>

      <label className="field" style={{ marginTop: 16 }}>
        <span>Clé API Google Cloud Vision</span>
        <div style={{ display: "flex", gap: 8 }}>
          <div className="key-field-wrapper" style={{ flex: 1 }}>
            <input
              type={showKey ? "text" : "password"}
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder={googleConfigured ? "••••••••••" : "AIza…"}
              autoComplete="off"
              spellCheck={false}
            />
            <button
              type="button"
              className="key-toggle-btn"
              onClick={() => setShowKey((v) => !v)}
              title={showKey ? "Masquer" : "Afficher"}
            >👁</button>
          </div>
          <button
            className="btn btn-primary"
            onClick={saveKey}
            disabled={saving || !apiKey.trim()}
            style={{ whiteSpace: "nowrap" }}
          >
            {saving ? "Sauvegarde…" : "💾 Sauvegarder"}
          </button>
        </div>
      </label>
    </Card>
  );
}

interface KeyFieldProps {
  label: string;
  value: string;
  onChange: (v: string) => void;
  show: boolean;
  onToggle: () => void;
  placeholder?: string;
}

function KeyField({ label, value, onChange, show, onToggle, placeholder }: KeyFieldProps) {
  return (
    <label className="field">
      {label && <span>{label}</span>}
      <div className="key-field-wrapper">
        <input
          type={show ? "text" : "password"}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={placeholder}
          autoComplete="off"
          spellCheck={false}
        />
        <button
          type="button"
          className="key-toggle-btn"
          onClick={onToggle}
          aria-label={show ? "Hide key" : "Show key"}
          title={show ? "Hide" : "Show"}
        >
          👁
        </button>
      </div>
    </label>
  );
}

function FeedbackListPanel() {
  const [items, setItems] = useState<Feedback[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Feedback | null>(null);

  const reload = () => {
    listFeedbacks()
      .then((v) => setItems(v))
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  };

  useEffect(() => { reload(); }, []);

  const remove = async (id: number) => {
    try {
      await deleteFeedback(id);
      if (selected?.id === id) setSelected(null);
      reload();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const kindBadge = (k: string) => {
    const map: Record<string, { bg: string; fg: string; label: string }> = {
      bug:         { bg: "#fde7e9", fg: "#b3261e", label: "🐞 Bug" },
      error:       { bg: "#fff4e0", fg: "#8a5a00", label: "⚠ Erreur" },
      evolution:   { bg: "#e1ecff", fg: "#1f4ba8", label: "✦ Évolution" },
      improvement: { bg: "#e3f6e8", fg: "#1f7a3a", label: "💡 Amélioration" },
    };
    const m = map[k] ?? { bg: "#eee", fg: "#444", label: k };
    return (
      <span style={{
        background: m.bg, color: m.fg, padding: "2px 8px",
        borderRadius: 999, fontSize: 12, whiteSpace: "nowrap",
      }}>{m.label}</span>
    );
  };

  return (
    <Card title="Feedback reçus" subtitle="Bugs, suggestions et améliorations envoyés depuis l'application.">
      {error && <div className="error-banner">{error}</div>}
      {!items ? (
        <div className="empty">Chargement…</div>
      ) : items.length === 0 ? (
        <div className="empty">Aucun feedback pour l'instant.</div>
      ) : (
        <div style={{ display: "grid", gridTemplateColumns: selected ? "1fr 1fr" : "1fr", gap: 16 }}>
          <div>
            <table style={{ width: "100%", borderCollapse: "collapse" }}>
              <thead>
                <tr style={{ textAlign: "left", borderBottom: "1px solid var(--border)" }}>
                  <th style={{ padding: 8 }}>Type</th>
                  <th style={{ padding: 8 }}>Titre</th>
                  <th style={{ padding: 8 }}>Date</th>
                  <th style={{ padding: 8 }} />
                </tr>
              </thead>
              <tbody>
                {items.map((f) => (
                  <tr
                    key={f.id}
                    style={{
                      borderBottom: "1px solid var(--border)",
                      cursor: "pointer",
                      background: selected?.id === f.id ? "var(--panel-2)" : "transparent",
                    }}
                    onClick={() => setSelected(f)}
                  >
                    <td style={{ padding: 8 }}>{kindBadge(f.kind)}</td>
                    <td style={{ padding: 8 }}>{f.title}</td>
                    <td style={{ padding: 8, color: "var(--muted)", fontSize: 12 }}>
                      {new Date(f.created_at * 1000).toLocaleString()}
                    </td>
                    <td style={{ padding: 8 }}>
                      <button
                        className="btn btn-ghost"
                        onClick={(e) => { e.stopPropagation(); remove(f.id); }}
                        title="Supprimer"
                      >🗑</button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {selected && (
            <div style={{ borderLeft: "1px solid var(--border)", paddingLeft: 16 }}>
              <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                <h3 style={{ margin: 0 }}>{selected.title}</h3>
                <button className="icon-btn" onClick={() => setSelected(null)}>×</button>
              </div>
              <div style={{ marginTop: 8, display: "flex", gap: 8, alignItems: "center" }}>
                {kindBadge(selected.kind)}
                <span style={{ color: "var(--muted)", fontSize: 12 }}>
                  {new Date(selected.created_at * 1000).toLocaleString()}
                </span>
              </div>
              {selected.description && (
                <p style={{ whiteSpace: "pre-wrap", marginTop: 12 }}>{selected.description}</p>
              )}
              {selected.url && (
                <p style={{ fontSize: 12, color: "var(--muted)" }}>
                  URL : <code>{selected.url}</code>
                </p>
              )}
              {selected.user_agent && (
                <p style={{ fontSize: 12, color: "var(--muted)" }}>
                  User-Agent : <code>{selected.user_agent}</code>
                </p>
              )}
              {selected.screenshot && (
                <div style={{ marginTop: 12 }}>
                  <div style={{ fontWeight: 600, marginBottom: 4 }}>Capture d'écran</div>
                  <img
                    src={selected.screenshot}
                    alt="screenshot"
                    style={{ maxWidth: "100%", border: "1px solid var(--border)", borderRadius: 6 }}
                  />
                </div>
              )}
              {selected.frontend_logs && (
                <details style={{ marginTop: 12 }}>
                  <summary>Logs frontend ({selected.frontend_logs.split("\n").length} lignes)</summary>
                  <pre style={{ maxHeight: 240, overflow: "auto", background: "var(--panel-2)", padding: 8, fontSize: 11 }}>
                    {selected.frontend_logs}
                  </pre>
                </details>
              )}
              {selected.backend_logs && (
                <details style={{ marginTop: 8 }}>
                  <summary>Logs backend ({selected.backend_logs.split("\n").length} lignes)</summary>
                  <pre style={{ maxHeight: 240, overflow: "auto", background: "var(--panel-2)", padding: 8, fontSize: 11 }}>
                    {selected.backend_logs}
                  </pre>
                </details>
              )}
            </div>
          )}
        </div>
      )}
    </Card>
  );
}
