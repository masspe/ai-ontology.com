import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
import { useRef, useState } from "react";
import { createFeedback } from "../api";
import { getLogTail } from "../lib/logBuffer";
const KINDS = [
    { key: "bug", label: "Bug", icon: "🐞", bg: "#fde7e9", fg: "#b3261e", border: "#f5b5b9" },
    { key: "error", label: "Erreur", icon: "⚠", bg: "#fff4e0", fg: "#8a5a00", border: "#f3d08a" },
    { key: "evolution", label: "Évolution", icon: "✦", bg: "#e1ecff", fg: "#1f4ba8", border: "#bcd1f6" },
    { key: "improvement", label: "Amélioration", icon: "💡", bg: "#e3f6e8", fg: "#1f7a3a", border: "#b6e3c4" },
];
export default function FeedbackModal({ open, onClose, onSubmitted }) {
    const [kind, setKind] = useState("bug");
    const [title, setTitle] = useState("");
    const [description, setDescription] = useState("");
    const [screenshot, setScreenshot] = useState(null);
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState(null);
    const fileInputRef = useRef(null);
    if (!open)
        return null;
    const reset = () => {
        setKind("bug");
        setTitle("");
        setDescription("");
        setScreenshot(null);
        setError(null);
    };
    const close = () => {
        reset();
        onClose();
    };
    const capture = async () => {
        setError(null);
        try {
            const md = navigator.mediaDevices;
            if (!md?.getDisplayMedia) {
                throw new Error("Capture d'écran non supportée par ce navigateur.");
            }
            const stream = await md.getDisplayMedia({ video: true, audio: false });
            const track = stream.getVideoTracks()[0];
            // Wait one frame so the user can pick the surface and content shows up.
            await new Promise((r) => setTimeout(r, 200));
            const video = document.createElement("video");
            video.srcObject = stream;
            video.muted = true;
            await video.play();
            const canvas = document.createElement("canvas");
            canvas.width = video.videoWidth || 1280;
            canvas.height = video.videoHeight || 720;
            const ctx = canvas.getContext("2d");
            if (!ctx)
                throw new Error("Canvas 2D non disponible.");
            ctx.drawImage(video, 0, 0, canvas.width, canvas.height);
            track.stop();
            stream.getTracks().forEach((t) => t.stop());
            setScreenshot(canvas.toDataURL("image/png"));
        }
        catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
    };
    const onImport = async (e) => {
        const file = e.target.files?.[0];
        if (!file)
            return;
        const reader = new FileReader();
        reader.onload = () => setScreenshot(reader.result);
        reader.onerror = () => setError("Lecture du fichier impossible.");
        reader.readAsDataURL(file);
    };
    const submit = async () => {
        setError(null);
        if (!title.trim()) {
            setError("Le titre est obligatoire.");
            return;
        }
        setBusy(true);
        try {
            await createFeedback({
                kind,
                title: title.trim(),
                description,
                screenshot,
                frontend_logs: getLogTail(300),
                user_agent: typeof navigator !== "undefined" ? navigator.userAgent : null,
                url: typeof window !== "undefined" ? window.location.href : null,
            });
            onSubmitted?.();
            close();
        }
        catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
        finally {
            setBusy(false);
        }
    };
    return (_jsx("div", { className: "feedback-backdrop", onClick: close, children: _jsxs("div", { className: "feedback-modal", onClick: (e) => e.stopPropagation(), children: [_jsxs("div", { className: "feedback-head", children: [_jsxs("div", { children: [_jsxs("h2", { style: { margin: 0, display: "flex", alignItems: "center", gap: 8 }, children: [_jsx("span", { children: "\uD83D\uDCAC" }), " Envoyer un feedback"] }), _jsx("p", { style: { marginTop: 6, color: "var(--muted)", fontSize: 13 }, children: "Aidez-nous \u00E0 am\u00E9liorer l'application. Les logs navigateur et serveur r\u00E9cents seront joints automatiquement." })] }), _jsx("button", { className: "icon-btn", onClick: close, title: "Fermer", children: "\u00D7" })] }), _jsxs("div", { className: "feedback-body", children: [_jsxs("div", { className: "field", children: [_jsx("span", { children: "Type" }), _jsx("div", { className: "feedback-kinds", children: KINDS.map((k) => {
                                        const active = k.key === kind;
                                        return (_jsxs("button", { type: "button", onClick: () => setKind(k.key), className: "feedback-kind", style: {
                                                background: active ? k.fg : k.bg,
                                                color: active ? "#fff" : k.fg,
                                                borderColor: active ? k.fg : k.border,
                                            }, children: [_jsx("div", { className: "feedback-kind-icon", children: k.icon }), _jsx("div", { className: "feedback-kind-label", children: k.label })] }, k.key));
                                    }) })] }), _jsxs("label", { className: "field", children: [_jsxs("span", { children: ["Titre ", _jsx("span", { style: { color: "#b3261e" }, children: "*" })] }), _jsx("input", { type: "text", value: title, onChange: (e) => setTitle(e.target.value), placeholder: "R\u00E9sumez en quelques mots\u2026" })] }), _jsxs("label", { className: "field", children: [_jsx("span", { children: "Description" }), _jsx("textarea", { rows: 4, value: description, onChange: (e) => setDescription(e.target.value), placeholder: "D\u00E9crivez ce qui s'est pass\u00E9, les \u00E9tapes pour reproduire, etc." })] }), _jsxs("div", { className: "field", children: [_jsxs("div", { style: { display: "flex", justifyContent: "space-between", alignItems: "center" }, children: [_jsx("span", { children: "Capture d'\u00E9cran" }), _jsxs("div", { style: { display: "flex", gap: 6 }, children: [_jsx("button", { type: "button", className: "btn btn-outline", onClick: capture, children: "\uD83D\uDCF7 Capturer" }), _jsx("button", { type: "button", className: "btn btn-outline", onClick: () => fileInputRef.current?.click(), children: "Importer" }), _jsx("button", { type: "button", className: "btn btn-outline", onClick: () => setScreenshot(null), disabled: !screenshot, title: "Supprimer", children: "\uD83D\uDDD1" }), _jsx("input", { ref: fileInputRef, type: "file", accept: "image/*", hidden: true, onChange: onImport })] })] }), screenshot && (_jsx("div", { className: "feedback-preview", children: _jsx("img", { src: screenshot, alt: "capture" }) }))] }), error && _jsx("div", { className: "error-banner", children: error })] }), _jsxs("div", { className: "feedback-foot", children: [_jsx("button", { className: "btn btn-ghost", onClick: close, disabled: busy, children: "Annuler" }), _jsx("button", { className: "btn btn-primary", onClick: submit, disabled: busy, children: busy ? "Envoi…" : "Envoyer" })] })] }) }));
}
