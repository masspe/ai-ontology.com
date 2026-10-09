// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.

import { useRef, useState } from "react";
import { t } from "../lib/i18n";

interface Props {
  onFile: (file: File) => void;
  accept?: string;
  hint?: string;
  disabled?: boolean;
}

export default function Dropzone({ onFile, accept, hint, disabled }: Props) {
  const [active, setActive] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  return (
    <div
      className={`dropzone${active ? " active" : ""}`}
      onClick={() => !disabled && inputRef.current?.click()}
      onDragOver={(e) => {
        e.preventDefault();
        if (!disabled) setActive(true);
      }}
      onDragLeave={() => setActive(false)}
      onDrop={(e) => {
        e.preventDefault();
        setActive(false);
        if (disabled) return;
        const f = e.dataTransfer.files?.[0];
        if (f) onFile(f);
      }}
    >
      <div style={{ fontSize: 28 }}>⇪</div>
      <h4>{t("Déposez des fichiers ici ou cliquez pour téléverser")}</h4>
      <p>{hint ?? t("JSONL, CSV, XLSX, triplets, texte ou JSON d'ontologie")}</p>
      <input
        ref={inputRef}
        type="file"
        accept={accept}
        style={{ display: "none" }}
        onChange={(e) => {
          const f = e.target.files?.[0];
          if (f) onFile(f);
          e.target.value = "";
        }}
      />
    </div>
  );
}
