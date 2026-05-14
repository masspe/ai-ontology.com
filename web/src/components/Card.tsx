// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { ReactNode } from "react";

interface Props {
  title?: ReactNode;
  subtitle?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
  style?: React.CSSProperties;
}

export default function Card({ title, subtitle, actions, children, className, style }: Props) {
  return (
    <section className={`card${className ? " " + className : ""}`} style={style}>
      {(title || actions) && (
        <header className="card-title">
          <span>{title}</span>
          {actions && <span>{actions}</span>}
        </header>
      )}
      {subtitle && <div className="card-subtitle">{subtitle}</div>}
      {children}
    </section>
  );
}
