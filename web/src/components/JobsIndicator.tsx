// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The top bar's "Jobs" light: the running background jobs (a sync, a
// re-index) with their progress, and the ones that failed recently.
// Polls every ten seconds; shows nothing while nothing runs and nothing
// failed.
import { useEffect, useState } from "react";
import { cancelJob, listJobs, type Job } from "../api";
import { t } from "../lib/i18n";

const POLL_MS = 10_000;
const RECENT_MS = 60 * 60 * 1000;

function statusLabel(s: Job["status"]): string {
  switch (s) {
    case "running":
      return t("en cours");
    case "done":
      return t("terminé");
    case "failed":
      return t("en échec");
    case "cancelled":
      return t("annulé");
    default:
      return t("interrompu");
  }
}

function recentFailure(j: Job): boolean {
  if (j.status !== "failed" && j.status !== "interrupted") return false;
  const at = Date.parse(j.finished_at ?? j.created_at);
  return Number.isFinite(at) && Date.now() - at < RECENT_MS;
}

export default function JobsIndicator() {
  const [jobs, setJobs] = useState<Job[]>([]);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    let on = true;
    const tick = () =>
      listJobs()
        .then((r) => {
          if (on) setJobs(r.jobs ?? []);
        })
        .catch(() => {
          /* no server, no jobs route: the light stays off */
        });
    void tick();
    const timer = window.setInterval(() => void tick(), POLL_MS);
    return () => {
      on = false;
      window.clearInterval(timer);
    };
  }, []);

  const running = jobs.filter((j) => j.status === "running");
  const failed = jobs.filter(recentFailure);
  if (running.length === 0 && failed.length === 0) return null;
  const shown = [...running, ...failed].slice(0, 8);
  const label =
    running.length > 0
      ? t("{n} tâche(s) en cours", { n: running.length })
      : t("{n} tâche(s) en échec", { n: failed.length });

  return (
    <div className="jobs-indicator" style={{ position: "relative" }}>
      <button
        type="button"
        className={`btn-ghost jobs-light ${running.length > 0 ? "jobs-running" : "jobs-failed"}`}
        aria-label={label}
        title={label}
        onClick={() => setOpen((o) => !o)}
      >
        <span aria-hidden>{running.length > 0 ? "◌" : "!"}</span> {running.length > 0 ? running.length : failed.length}
      </button>
      {open && (
        <div className="account-menu jobs-menu" role="menu" style={{ right: 0, minWidth: 280 }}>
          {shown.map((j) => (
            <div key={j.id} className="jobs-row" style={{ padding: "6px 8px", borderBottom: "1px solid var(--border)" }}>
              <div style={{ display: "flex", justifyContent: "space-between", gap: 8 }}>
                <strong>{j.label}</strong>
                <span className="muted">{statusLabel(j.status)}</span>
              </div>
              <div className="muted" style={{ fontSize: 12 }}>
                {j.total ? `${j.progress} / ${j.total}` : j.progress > 0 ? `${j.progress}` : ""} {j.message}
              </div>
              {j.status === "running" && (
                <button
                  type="button"
                  className="btn-ghost"
                  style={{ fontSize: 12 }}
                  onClick={() => void cancelJob(j.id).catch(() => undefined)}
                >
                  {t("Arrêter")}
                </button>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
