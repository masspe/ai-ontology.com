// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

interface Props {
  values: number[];
  stroke?: string;
}

/** Plain inline SVG sparkline — no external deps. */
export default function Sparkline({ values, stroke = "var(--accent)" }: Props) {
  if (values.length === 0) {
    return <svg className="sparkline" viewBox="0 0 100 36" preserveAspectRatio="none" />;
  }
  const min = Math.min(...values);
  const max = Math.max(...values);
  const span = max - min || 1;
  const stepX = values.length > 1 ? 100 / (values.length - 1) : 0;
  const points = values
    .map((v, i) => {
      const x = i * stepX;
      const y = 32 - ((v - min) / span) * 28 + 2;
      return `${x.toFixed(2)},${y.toFixed(2)}`;
    })
    .join(" ");
  const area = `0,36 ${points} 100,36`;
  return (
    <svg className="sparkline" viewBox="0 0 100 36" preserveAspectRatio="none">
      <polygon points={area} fill="var(--accent-soft)" opacity={0.6} />
      <polyline points={points} fill="none" stroke={stroke} strokeWidth={1.6} />
    </svg>
  );
}
