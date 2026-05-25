// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { useRef, useState } from "react";
import { createFeedback, type FeedbackKind } from "../api";
import { getLogTail } from "../lib/logBuffer";

interface Props {
  open: boolean;
  onClose: () => void;
  onSubmitted?: () => void;
}

interface KindMeta {
  key: FeedbackKind;
  label: string;
  icon: string;
  bg: string;
  fg: string;
  border: string;
}

const KINDS: KindMeta[] = [
  { key: "bug",         label: "Bug",         icon: "🐞", bg: "#fde7e9", fg: "#b3261e", border: "#f5b5b9" },
  { key: "error",       label: "Erreur",      icon: "⚠",  bg: "#fff4e0", fg: "#8a5a00", border: "#f3d08a" },
  { key: "evolution",   label: "Évolution",   icon: "✦",  bg: "#e1ecff", fg: "#1f4ba8", border: "#bcd1f6" },
  { key: "improvement", label: "Amélioration",icon: "💡", bg: "#e3f6e8", fg: "#1f7a3a", border: "#b6e3c4" },
];

export default function FeedbackModal({ open, onClose, onSubmitted }: Props) {
  const [kind, setKind] = useState<FeedbackKind>("bug");
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [screenshot, setScreenshot] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  if (!open) return null;

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
      const md = (navigator.mediaDevices as MediaDevices & {
        getDisplayMedia: (c: DisplayMediaStreamOptions) => Promise<MediaStream>;
      });
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
      if (!ctx) throw new Error("Canvas 2D non disponible.");
      ctx.drawImage(video, 0, 0, canvas.width, canvas.height);
      track.stop();
      stream.getTracks().forEach((t) => t.stop());
      setScreenshot(canvas.toDataURL("image/png"));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const onImport = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = () => setScreenshot(reader.result as string);
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
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="feedback-backdrop" onClick={close}>
      <div className="feedback-modal" onClick={(e) => e.stopPropagation()}>
        <div className="feedback-head">
          <div>
            <h2 style={{ margin: 0, display: "flex", alignItems: "center", gap: 8 }}>
              <span>💬</span> Envoyer un feedback
            </h2>
            <p style={{ marginTop: 6, color: "var(--muted)", fontSize: 13 }}>
              Aidez-nous à améliorer l'application. Les logs navigateur et serveur
              récents seront joints automatiquement.
            </p>
          </div>
          <button className="icon-btn" onClick={close} title="Fermer">×</button>
        </div>

        <div className="feedback-body">
          <div className="field">
            <span>Type</span>
            <div className="feedback-kinds">
              {KINDS.map((k) => {
                const active = k.key === kind;
                return (
                  <button
                    key={k.key}
                    type="button"
                    onClick={() => setKind(k.key)}
                    className="feedback-kind"
                    style={{
                      background: active ? k.fg : k.bg,
                      color: active ? "#fff" : k.fg,
                      borderColor: active ? k.fg : k.border,
                    }}
                  >
                    <div className="feedback-kind-icon">{k.icon}</div>
                    <div className="feedback-kind-label">{k.label}</div>
                  </button>
                );
              })}
            </div>
          </div>

          <label className="field">
            <span>Titre <span style={{ color: "#b3261e" }}>*</span></span>
            <input
              type="text"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="Résumez en quelques mots…"
            />
          </label>

          <label className="field">
            <span>Description</span>
            <textarea
              rows={4}
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="Décrivez ce qui s'est passé, les étapes pour reproduire, etc."
            />
          </label>

          <div className="field">
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
              <span>Capture d'écran</span>
              <div style={{ display: "flex", gap: 6 }}>
                <button type="button" className="btn btn-outline" onClick={capture}>
                  📷 Capturer
                </button>
                <button
                  type="button"
                  className="btn btn-outline"
                  onClick={() => fileInputRef.current?.click()}
                >
                  Importer
                </button>
                <button
                  type="button"
                  className="btn btn-outline"
                  onClick={() => setScreenshot(null)}
                  disabled={!screenshot}
                  title="Supprimer"
                >🗑</button>
                <input
                  ref={fileInputRef}
                  type="file"
                  accept="image/*"
                  hidden
                  onChange={onImport}
                />
              </div>
            </div>
            {screenshot && (
              <div className="feedback-preview">
                <img src={screenshot} alt="capture" />
              </div>
            )}
          </div>

          {error && <div className="error-banner">{error}</div>}
        </div>

        <div className="feedback-foot">
          <button className="btn btn-ghost" onClick={close} disabled={busy}>Annuler</button>
          <button className="btn btn-primary" onClick={submit} disabled={busy}>
            {busy ? "Envoi…" : "Envoyer"}
          </button>
        </div>
      </div>
    </div>
  );
}
