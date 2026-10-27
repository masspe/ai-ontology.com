// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// A conversation with the knowledge base. Each answer streams in and
// cites its sheets; the next question is sent with the turns before it,
// so « et quels programmes la modifient ? » means what the last answer
// named. Threads are kept in the browser; a question that cited the same
// sheets as an earlier thread is offered as « déjà demandé ».

import { useState } from "react";
import { askStream, type Subgraph, type Turn } from "../api";
import { t } from "../lib/i18n";
import { loadThreads, newThreadId, relatedTurns, saveThread, type Thread, type ThreadTurn } from "../lib/threads";
import Answer, { splitCited } from "./Answer";

const sheets = (n: number) => (n > 1 ? t("{n} fiches", { n }) : t("{n} fiche", { n }));
const links = (n: number) => (n > 1 ? t("{n} liens", { n }) : t("{n} lien", { n }));

interface Props {
  defaultQuery?: string;
}

export default function StreamingAnswer({ defaultQuery = "" }: Props) {
  const [query, setQuery] = useState(defaultQuery);
  const [threadId, setThreadId] = useState(() => newThreadId());
  const [turns, setTurns] = useState<ThreadTurn[]>([]);
  const [asked, setAsked] = useState("");
  const [answer, setAnswer] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [streaming, setStreaming] = useState(false);
  const [retrieved, setRetrieved] = useState<Subgraph | null>(null);

  const history = (): Turn[] => turns.map((x) => ({ question: x.question, answer: splitCited(x.answer).body, cited: x.cited }));

  /** The streamed answer becomes a turn of the thread, kept in the browser. */
  const settle = (question: string, text: string, sub: Subgraph | null) => {
    const ids = new Set((sub?.concepts ?? []).map((c) => c.id));
    const turn: ThreadTurn = {
      question,
      answer: text,
      cited: splitCited(text).cited.filter((id) => ids.has(id)),
      concepts: (sub?.concepts ?? []).map((c) => ({ id: c.id, concept_type: c.concept_type, name: c.name })),
    };
    const next = [...turns, turn];
    setTurns(next);
    saveThread({ id: threadId, started: Number(threadId.split("-")[0] ? parseInt(threadId.split("-")[0]!, 36) : Date.now()), turns: next });
    setAnswer("");
    setAsked("");
    setRetrieved(null);
  };

  const run = async () => {
    const question = query.trim();
    if (!question) return;
    let text = "";
    let sub: Subgraph | null = null;
    setAsked(question);
    setQuery("");
    setAnswer("");
    setError(null);
    setRetrieved(null);
    setStreaming(true);
    const finish = () => {
      setStreaming(false);
      if (text) settle(question, text, sub);
    };
    try {
      await askStream(
        { query: question, history: history() },
        {
          onRetrieved: (s) => {
            sub = s;
            setRetrieved(s);
          },
          onToken: (tok) => {
            text += tok;
            setAnswer((prev) => prev + tok);
          },
          onEnd: finish,
          onError: (e) => {
            setError(e);
            setStreaming(false);
            setQuery(question);
          },
        },
      );
      finish();
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(msg);
      setStreaming(false);
      setQuery(question);
    }
  };

  const restart = () => {
    setThreadId(newThreadId());
    setTurns([]);
    setAnswer("");
    setAsked("");
    setRetrieved(null);
    setError(null);
  };

  const resume = (thread: Thread) => {
    setThreadId(thread.id);
    setTurns(thread.turns);
    setAnswer("");
    setAsked("");
    setRetrieved(null);
    setError(null);
  };

  const last = turns[turns.length - 1];
  const related = last ? relatedTurns(last.cited, threadId) : [];

  return (
    <div className="conversation">
      {turns.map((x, i) => (
        <div key={i} className="turn">
          <p className="turn-question">{x.question}</p>
          <Answer text={x.answer} concepts={x.concepts} cited={x.cited} />
        </div>
      ))}
      {(asked || streaming) && (
        <div className="turn">
          <p className="turn-question">{asked}</p>
          {(answer || streaming || retrieved) && <Answer text={answer} concepts={retrieved?.concepts} cursor={streaming} />}
        </div>
      )}
      {related.length > 0 && (
        <div className="related" aria-label={t("Déjà demandé")}>
          <span className="muted">{t("Déjà demandé :")}</span>{" "}
          {related.map(({ thread, turn }, i) => (
            <button key={`${thread.id}-${i}`} type="button" className="btn-ghost related-link" onClick={() => resume(thread)}>
              {turn.question}
            </button>
          ))}
        </div>
      )}
      {error && <div className="error-banner">{error}</div>}
      <div className="field-row" style={{ marginTop: turns.length > 0 || asked ? 12 : 0, marginBottom: 12 }}>
        <div className="field" style={{ flex: 1 }}>
          <textarea
            placeholder={turns.length > 0 ? t("Posez la question suivante…") : t("Posez votre question…")}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey && !streaming) {
                e.preventDefault();
                void run();
              }
            }}
            rows={3}
          />
        </div>
      </div>
      <div className="row">
        <button className="btn-primary" onClick={run} disabled={streaming || !query.trim()}>
          {streaming ? t("Réponse en cours…") : t("Poser la question")}
        </button>
        {turns.length > 0 && !streaming && (
          <button type="button" className="btn-outline" onClick={restart}>
            {t("Nouvelle conversation")}
          </button>
        )}
        {turns.length === 0 && !asked && loadThreads().length > 0 && (
          <button type="button" className="btn-ghost" onClick={() => resume(loadThreads()[0]!)}>
            {t("Reprendre la dernière conversation")}
          </button>
        )}
        <span className="muted" style={{ fontSize: 12 }}>
          {retrieved ? t("Réponse fondée sur {sheets} et {links}", { sheets: sheets(retrieved.concepts.length), links: links(retrieved.relations.length) }) : ""}
        </span>
      </div>
    </div>
  );
}
