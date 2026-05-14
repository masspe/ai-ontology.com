// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { useRef, useState } from "react";

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
      <h4>Drop files here or click to upload</h4>
      <p>{hint ?? "JSONL, CSV, XLSX, triples, text or ontology JSON"}</p>
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
