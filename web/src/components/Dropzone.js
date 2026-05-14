import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
import { useRef, useState } from "react";
export default function Dropzone({ onFile, accept, hint, disabled }) {
    const [active, setActive] = useState(false);
    const inputRef = useRef(null);
    return (_jsxs("div", { className: `dropzone${active ? " active" : ""}`, onClick: () => !disabled && inputRef.current?.click(), onDragOver: (e) => {
            e.preventDefault();
            if (!disabled)
                setActive(true);
        }, onDragLeave: () => setActive(false), onDrop: (e) => {
            e.preventDefault();
            setActive(false);
            if (disabled)
                return;
            const f = e.dataTransfer.files?.[0];
            if (f)
                onFile(f);
        }, children: [_jsx("div", { style: { fontSize: 28 }, children: "\u21EA" }), _jsx("h4", { children: "Drop files here or click to upload" }), _jsx("p", { children: hint ?? "JSONL, CSV, XLSX, triples, text or ontology JSON" }), _jsx("input", { ref: inputRef, type: "file", accept: accept, style: { display: "none" }, onChange: (e) => {
                    const f = e.target.files?.[0];
                    if (f)
                        onFile(f);
                    e.target.value = "";
                } })] }));
}
