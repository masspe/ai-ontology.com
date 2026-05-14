// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

interface Props {
  label: string;
  value: number | string;
  deltaPct?: number;
  hint?: string;
}

function fmt(n: number | string): string {
  if (typeof n === "string") return n;
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + "M";
  if (n >= 10_000) return (n / 1_000).toFixed(1) + "k";
  return String(n);
}

export default function StatCard({ label, value, deltaPct, hint }: Props) {
  const cls = deltaPct == null ? "flat" : deltaPct > 0.5 ? "up" : deltaPct < -0.5 ? "down" : "flat";
  const arrow = cls === "up" ? "▲" : cls === "down" ? "▼" : "•";
  return (
    <div className="card stat-card">
      <span className="label">{label}</span>
      <span className="value">{fmt(value)}</span>
      {deltaPct != null && (
        <span className={`delta ${cls}`}>
          {arrow} {Math.abs(deltaPct).toFixed(1)}% {hint ?? "vs first sample"}
        </span>
      )}
      {deltaPct == null && hint && <span className="delta flat">{hint}</span>}
    </div>
  );
}
