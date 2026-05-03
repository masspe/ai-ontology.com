// SPDX-License-Identifier: LGPL-3.0-or-later
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
// 
// This file is part of ai-ontology.com.
// It is licensed under the GNU Lesser General Public License v3.0
// or (at your option) any later version. See LICENSE and LICENSE.GPL.

import { useRef, useState } from "react";
import { upload } from "./api";

type Kind = "ontology" | "jsonl" | "triples" | "csv" | "xlsx" | "text";

interface LogEntry {
  ts: number;
  msg: string;
  ok: boolean;
}

export function UploadPanel({ onUploaded }: { onUploaded: () => void }) {
  const [kind, setKind] = useState<Kind>("ontology");
  const [conceptType, setConceptType] = useState("");
  const [busy, setBusy] = useState(false);
  const [over, setOver] = useState(false);
  const [log, setLog] = useState<LogEntry[]>([]);
  const fileRef = useRef<HTMLInputElement>(null);

  const needsConceptType = kind === "csv" || kind === "xlsx" || kind === "text";

  const ingestOne = async (file: File) => {
    if (needsConceptType && !conceptType.trim()) {
      pushLog(`skipped ${file.name} — needs a concept type`, false);
      return;
    }
    try {
      const r = await upload(kind, file, {
        concept_type: needsConceptType ? conceptType.trim() : undefined,
      });
      pushLog(
        `${file.name}: +${r.ingested.concepts} concepts, +${r.ingested.relations} relations${
          r.ingested.ontology_updates ? `, +${r.ingested.ontology_updates} ontology` : ""
        }`,
        true,
      );
      onUploaded();
    } catch (e) {
      pushLog(`${file.name}: ${String(e)}`, false);
    }
  };

  const ingestMany = async (files: FileList) => {
    setBusy(true);
    for (const f of Array.from(files)) {
      await ingestOne(f);
    }
    setBusy(false);
  };

  const pushLog = (msg: string, ok: boolean) =>
    setLog((l) => [{ ts: Date.now(), msg, ok }, ...l].slice(0, 100));

  return (
    <>
      <div className="card">
        <h2>Upload data</h2>
        <small>
          The same form handles every adapter. For text files (contracts,
          memos, …) each file becomes one Concept whose description is the
          file body, so the index can find it by content.
        </small>

        <div className="row" style={{ marginTop: 16 }}>
          <select value={kind} onChange={(e) => setKind(e.target.value as Kind)}>
            <option value="ontology">Ontology JSON</option>
            <option value="jsonl">JSONL records</option>
            <option value="triples">Triples (.txt)</option>
            <option value="csv">CSV</option>
            <option value="xlsx">XLSX / XLS / ODS</option>
            <option value="text">Text document</option>
          </select>
          {needsConceptType && (
            <input
              type="text"
              className="grow"
              placeholder="concept type (e.g. Contract, Invoice)"
              value={conceptType}
              onChange={(e) => setConceptType(e.target.value)}
            />
          )}
        </div>

        <div
          className={`dropzone ${over ? "over" : ""}`}
          style={{ marginTop: 16 }}
          onDragOver={(e) => {
            e.preventDefault();
            setOver(true);
          }}
          onDragLeave={() => setOver(false)}
          onDrop={(e) => {
            e.preventDefault();
            setOver(false);
            if (e.dataTransfer.files.length) ingestMany(e.dataTransfer.files);
          }}
          onClick={() => fileRef.current?.click()}
        >
          {busy
            ? "Uploading…"
            : "Drop a file here, or click to pick (multi-select OK for batch ingest)"}
        </div>
        <input
          ref={fileRef}
          type="file"
          multiple
          style={{ display: "none" }}
          onChange={(e) => {
            if (e.target.files?.length) ingestMany(e.target.files);
            e.target.value = "";
          }}
        />
      </div>

      <div className="card">
        <h2>Activity</h2>
        {log.length === 0 ? (
          <small>nothing yet.</small>
        ) : (
          <pre className="log">
            {log
              .map(
                (e) =>
                  `${e.ok ? "✓" : "✗"} ${new Date(e.ts).toLocaleTimeString()}  ${e.msg}`,
              )
              .join("\n")}
          </pre>
        )}
      </div>
    </>
  );
}
