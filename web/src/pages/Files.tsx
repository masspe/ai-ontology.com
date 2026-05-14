// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

import { useEffect, useState } from "react";
import Card from "../components/Card";
import Dropzone from "../components/Dropzone";
import { deleteFile, exportGraphUrl, getFiles, getOntology, upload, type FileRecord, type Ontology } from "../api";

const KINDS = [
  { value: "jsonl", label: "JSONL records" },
  { value: "ontology", label: "Ontology JSON" },
  { value: "triples", label: "Triples (.triples)" },
  { value: "csv", label: "CSV (one concept per row)" },
  { value: "xlsx", label: "XLSX spreadsheet" },
  { value: "text", label: "Text document" },
];

function fmtBytes(b: number): string {
  if (b < 1024) return `${b} B`;
  if (b < 1024 * 1024) return `${(b / 1024).toFixed(1)} KB`;
  return `${(b / (1024 * 1024)).toFixed(1)} MB`;
}

export default function Files() {
  const [files, setFiles] = useState<FileRecord[]>([]);
  const [ontology, setOntology] = useState<Ontology | null>(null);
  const [kind, setKind] = useState("jsonl");
  const [conceptType, setConceptType] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);

  const refresh = async () => {
    try {
      const [f, o] = await Promise.all([getFiles(), getOntology()]);
      setFiles(f.files);
      setOntology(o);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  const onUpload = async (file: File) => {
    setBusy(true);
    setError(null);
    setInfo(null);
    try {
      const needsCt = ["csv", "xlsx", "text"].includes(kind);
      if (needsCt && !conceptType.trim()) {
        throw new Error(`Kind "${kind}" requires a concept type.`);
      }
      const res = await upload(file, { kind, conceptType: needsCt ? conceptType : undefined });
      setInfo(`Ingested ${res.ingested.concepts} concepts, ${res.ingested.relations} relations.`);
      await refresh();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const onDelete = async (id: number) => {
    if (!window.confirm("Remove this file record? (Already ingested data stays in the graph.)")) return;
    try {
      await deleteFile(id);
      await refresh();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const conceptTypeOptions = ontology ? Object.keys(ontology.concept_types) : [];

  return (
    <>
      <div className="page-header">
        <div>
          <h1 className="page-title">Files</h1>
          <p className="page-subtitle">Upload data sources and browse ingest history.</p>
        </div>
        <div className="row">
          <a href={exportGraphUrl("jsonl")} className="btn-primary" style={{ textDecoration: "none", padding: "8px 14px", borderRadius: 8 }}>
            Export JSONL
          </a>
        </div>
      </div>

      {error && <div className="error-banner">{error}</div>}
      {info && <div className="success-banner">{info}</div>}

      <Card title="Upload">
        <div className="field-row">
          <div className="field">
            <label>Kind</label>
            <select value={kind} onChange={(e) => setKind(e.target.value)}>
              {KINDS.map((k) => (
                <option key={k.value} value={k.value}>{k.label}</option>
              ))}
            </select>
          </div>
          {(kind === "csv" || kind === "xlsx" || kind === "text") && (
            <div className="field">
              <label>Concept type</label>
              {conceptTypeOptions.length > 0 ? (
                <select value={conceptType} onChange={(e) => setConceptType(e.target.value)}>
                  <option value="">— select —</option>
                  {conceptTypeOptions.map((t) => <option key={t}>{t}</option>)}
                </select>
              ) : (
                <input value={conceptType} onChange={(e) => setConceptType(e.target.value)} placeholder="e.g. Contract" />
              )}
            </div>
          )}
        </div>
        <Dropzone onFile={onUpload} disabled={busy} />
      </Card>

      <Card title="Uploaded files" style={{ marginTop: 16 }} actions={<button onClick={refresh}>Reload</button>}>
        {files.length === 0 ? (
          <div className="empty">No uploads yet.</div>
        ) : (
          <table className="table">
            <thead>
              <tr>
                <th>Name</th>
                <th>Kind</th>
                <th>Size</th>
                <th>Concepts</th>
                <th>Relations</th>
                <th>Status</th>
                <th className="actions">Actions</th>
              </tr>
            </thead>
            <tbody>
              {files.map((f) => (
                <tr key={f.id}>
                  <td>{f.name}</td>
                  <td><span className="badge">{f.kind}</span></td>
                  <td>{fmtBytes(f.size)}</td>
                  <td>{f.concepts}</td>
                  <td>{f.relations}</td>
                  <td><span className="badge badge-success">{f.status}</span></td>
                  <td className="actions">
                    <button className="btn-danger" onClick={() => onDelete(f.id)}>Delete</button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>
    </>
  );
}
