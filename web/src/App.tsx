// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
// 
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

import { useEffect, useState } from "react";
import { getStats, type Stats } from "./api";
import { UploadPanel } from "./UploadPanel";
import { AskPanel } from "./AskPanel";
import { BrowsePanel } from "./BrowsePanel";

type Tab = "ask" | "browse" | "upload";

export default function App() {
  const [tab, setTab] = useState<Tab>("ask");
  const [stats, setStats] = useState<Stats | null>(null);
  const [statsErr, setStatsErr] = useState<string | null>(null);

  const refresh = async () => {
    try {
      const s = await getStats();
      setStats(s);
      setStatsErr(null);
    } catch (e) {
      setStatsErr(String(e));
    }
  };

  useEffect(() => {
    refresh();
    const id = setInterval(refresh, 10_000);
    return () => clearInterval(id);
  }, []);

  return (
    <main>
      <header>
        <h1>Ontology RAG</h1>
        <small>
          {stats
            ? `${stats.concepts} concepts · ${stats.relations} relations · ${stats.concept_types} types`
            : statsErr
              ? `server unreachable — ${statsErr}`
              : "loading…"}
        </small>
        <nav style={{ marginTop: 12 }}>
          <a
            href="#"
            className={tab === "ask" ? "active" : ""}
            onClick={(e) => {
              e.preventDefault();
              setTab("ask");
            }}
          >
            Ask
          </a>
          <a
            href="#"
            className={tab === "browse" ? "active" : ""}
            onClick={(e) => {
              e.preventDefault();
              setTab("browse");
            }}
          >
            Browse
          </a>
          <a
            href="#"
            className={tab === "upload" ? "active" : ""}
            onClick={(e) => {
              e.preventDefault();
              setTab("upload");
            }}
          >
            Upload
          </a>
        </nav>
      </header>

      {tab === "ask" ? (
        <AskPanel />
      ) : tab === "browse" ? (
        <BrowsePanel />
      ) : (
        <UploadPanel onUploaded={refresh} />
      )}
    </main>
  );
}
