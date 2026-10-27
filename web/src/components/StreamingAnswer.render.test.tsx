// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// Rendering tests of the streaming ask box: button gating, handler wiring to
// `askStream`, token accumulation, grounding line, citations and error paths.

import { beforeEach, describe, expect, it, vi } from "vitest";
import StreamingAnswer from "./StreamingAnswer";
import { act, renderPage, screen, waitFor } from "../test/render";
import type { AskStreamHandlers, Subgraph } from "../api";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return { ...actual, askStream: vi.fn() };
});

import * as api from "../api";

const askStream = api.askStream as unknown as ReturnType<typeof vi.fn>;

/** Resolve the mocked stream with the handlers the component passed in. */
function captureHandlers(): { handlers: () => AskStreamHandlers; finish: () => void } {
  let captured: AskStreamHandlers | null = null;
  let resolve: () => void = () => {};
  askStream.mockImplementation((_req: unknown, h: AskStreamHandlers) => {
    captured = h;
    return new Promise<void>((r) => {
      resolve = r;
    });
  });
  return {
    handlers: () => {
      if (!captured) throw new Error("askStream not called yet");
      return captured;
    },
    finish: () => resolve(),
  };
}

const subgraph: Subgraph = {
  concepts: Array.from({ length: 14 }, (_, i) => ({ id: i + 1, concept_type: "Contract", name: `C${i + 1}` })),
  relations: [{ id: 1, relation_type: "r", source: 1, target: 2 }],
};

beforeEach(() => {
  askStream.mockResolvedValue(undefined);
});

describe("StreamingAnswer", () => {
  it("disables Poser la question until the query has non-blank text, and never calls the API for a blank one", async () => {
    const { user } = renderPage(<StreamingAnswer />);
    const ask = screen.getByRole("button", { name: "Poser la question" });
    expect(ask).toBeDisabled();
    await user.type(screen.getByPlaceholderText("Posez votre question…"), "   ");
    expect(ask).toBeDisabled();
    await user.type(screen.getByPlaceholderText("Posez votre question…"), "who?");
    expect(ask).toBeEnabled();
    expect(askStream).not.toHaveBeenCalled();
  });

  it("prefills the textarea from defaultQuery", () => {
    renderPage(<StreamingAnswer defaultQuery="renewals" />);
    expect(screen.getByPlaceholderText("Posez votre question…")).toHaveValue("renewals");
    expect(screen.getByRole("button", { name: "Poser la question" })).toBeEnabled();
  });

  it("streams: shows the cursor while running, appends tokens, reports grounding and citations (max 12), then ends", async () => {
    const stream = captureHandlers();
    const { user } = renderPage(<StreamingAnswer defaultQuery="which contracts?" />);
    await user.click(screen.getByRole("button", { name: "Poser la question" }));

    expect(askStream).toHaveBeenCalledTimes(1);
    expect(askStream.mock.calls[0][0]).toEqual({ query: "which contracts?", history: [] });
    expect(screen.getByRole("button", { name: "Réponse en cours…" })).toBeDisabled();
    expect(screen.getByText("▍")).toBeInTheDocument();

    act(() => stream.handlers().onRetrieved?.(subgraph));
    expect(screen.getByText("Réponse fondée sur 14 fiches et 1 lien")).toBeInTheDocument();
    const chips = Array.from(document.querySelectorAll(".citations .sheet-chip"), (el) => el.textContent);
    expect(chips).toHaveLength(12);
    expect(chips).toContain("Contract · C12");
    expect(chips).not.toContain("Contract · C13");

    act(() => {
      stream.handlers().onToken?.("Two ");
      stream.handlers().onToken?.("contracts.");
    });
    expect(screen.getByText(/Two contracts\./)).toBeInTheDocument();

    act(() => stream.handlers().onEnd?.({ usage: {} }));
    expect(screen.queryByText("▍")).not.toBeInTheDocument();
    // The answer stays, as the first turn of the conversation; the box
    // waits for the next question, so the ask button is idle.
    expect(screen.getByText(/Two contracts\./)).toBeInTheDocument();
    expect(screen.getByText("which contracts?")).toHaveClass("turn-question");
    expect(screen.getByRole("button", { name: "Poser la question" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Nouvelle conversation" })).toBeInTheDocument();

    act(() => stream.finish());
    await waitFor(() => expect(screen.getByPlaceholderText("Posez la question suivante…")).toBeInTheDocument());
  });

  it("shows the grounding line but no citations when the subgraph has no concepts", async () => {
    const stream = captureHandlers();
    const { user } = renderPage(<StreamingAnswer defaultQuery="q" />);
    await user.click(screen.getByRole("button", { name: "Poser la question" }));
    act(() => stream.handlers().onRetrieved?.({ concepts: [], relations: [] }));
    expect(screen.getByText("Réponse fondée sur 0 fiche et 0 lien")).toBeInTheDocument();
    expect(document.querySelector(".citations")).toBeNull();
    act(() => stream.finish());
    // Nothing was answered: no turn, the first-question placeholder stays.
    await waitFor(() => expect(screen.queryByText("▍")).toBeNull());
    expect(screen.getByPlaceholderText("Posez votre question…")).toBeInTheDocument();
  });

  it("surfaces a stream 'error' event and stops streaming", async () => {
    const stream = captureHandlers();
    const { user } = renderPage(<StreamingAnswer defaultQuery="q" />);
    await user.click(screen.getByRole("button", { name: "Poser la question" }));
    act(() => stream.handlers().onError?.("llm quota exceeded"));
    expect(screen.getByText("llm quota exceeded")).toHaveClass("error-banner");
    expect(screen.getByRole("button", { name: "Poser la question" })).toBeEnabled();
    act(() => stream.finish());
  });

  it("surfaces a thrown Error's message", async () => {
    askStream.mockRejectedValue(new Error("stream failed: 503 Service Unavailable"));
    const { user } = renderPage(<StreamingAnswer defaultQuery="q" />);
    await user.click(screen.getByRole("button", { name: "Poser la question" }));
    expect(await screen.findByText("stream failed: 503 Service Unavailable")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Poser la question" })).toBeEnabled();
    expect(document.querySelector(".answer-box")).toBeNull();
  });

  it("stringifies a non-Error rejection", async () => {
    askStream.mockRejectedValue("boom");
    const { user } = renderPage(<StreamingAnswer defaultQuery="q" />);
    await user.click(screen.getByRole("button", { name: "Poser la question" }));
    expect(await screen.findByText("boom")).toBeInTheDocument();
  });

  it("clears the previous answer, error and grounding when asked again", async () => {
    askStream.mockRejectedValueOnce(new Error("first failure"));
    const stream2 = { current: null as AskStreamHandlers | null };
    const { user } = renderPage(<StreamingAnswer defaultQuery="q" />);
    await user.click(screen.getByRole("button", { name: "Poser la question" }));
    expect(await screen.findByText("first failure")).toBeInTheDocument();

    askStream.mockImplementation(async (_req: unknown, h: AskStreamHandlers) => {
      stream2.current = h;
      h.onRetrieved?.(subgraph);
      h.onToken?.("ok");
    });
    await user.click(screen.getByRole("button", { name: "Poser la question" }));
    await waitFor(() => expect(screen.getByText("ok")).toBeInTheDocument());
    expect(screen.queryByText("first failure")).not.toBeInTheDocument();
    expect(stream2.current).not.toBeNull();
  });

  it("keeps each answer as a turn and sends the turns before as history, with what they cited", async () => {
    window.localStorage.clear();
    const stream = captureHandlers();
    const { user } = renderPage(<StreamingAnswer defaultQuery="first?" />);
    await user.click(screen.getByRole("button", { name: "Poser la question" }));
    act(() => stream.handlers().onRetrieved?.(subgraph));
    act(() => stream.handlers().onToken?.("One.\nCited: [#2, #99]"));
    act(() => stream.handlers().onEnd?.({}));
    act(() => stream.finish());
    await waitFor(() => expect(screen.getByRole("button", { name: "Nouvelle conversation" })).toBeInTheDocument());
    // The turn stays, question above its answer; the unknown id is not kept.
    expect(screen.getByText("first?")).toHaveClass("turn-question");
    expect(screen.getByText("One.")).toBeInTheDocument();
    expect(screen.getByPlaceholderText("Posez la question suivante…")).toHaveValue("");

    await user.type(screen.getByPlaceholderText("Posez la question suivante…"), "and then?{Enter}");
    await waitFor(() => expect(askStream).toHaveBeenCalledTimes(2));
    expect(askStream.mock.calls[1][0]).toEqual({
      query: "and then?",
      history: [{ question: "first?", answer: "One.", cited: [2] }],
    });
    // The thread is in the browser.
    const stored = JSON.parse(window.localStorage.getItem("conversations.v1")!);
    expect(stored).toHaveLength(1);
    expect(stored[0].turns[0]).toMatchObject({ question: "first?", cited: [2] });

    act(() => stream.handlers().onEnd?.({}));
    act(() => stream.finish());
    await waitFor(() => expect(screen.getByRole("button", { name: "Nouvelle conversation" })).toBeEnabled());
    await user.click(screen.getByRole("button", { name: "Nouvelle conversation" }));
    expect(screen.queryByText("first?")).not.toBeInTheDocument();
    expect(screen.getByPlaceholderText("Posez votre question…")).toBeInTheDocument();
  });

  it("offers an earlier thread that cited the same sheets, and resumes it", async () => {
    window.localStorage.setItem(
      "conversations.v1",
      JSON.stringify([
        {
          id: "old-1",
          started: 1,
          turns: [{ question: "earlier about C2", answer: "Yes.", cited: [2], concepts: [{ id: 2, concept_type: "Contract", name: "C2" }] }],
        },
      ]),
    );
    const stream = captureHandlers();
    const { user } = renderPage(<StreamingAnswer defaultQuery="now?" />);
    expect(screen.getByRole("button", { name: "Reprendre la dernière conversation" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Poser la question" }));
    act(() => stream.handlers().onRetrieved?.(subgraph));
    act(() => stream.handlers().onToken?.("Also.\nCited: [#2]"));
    act(() => stream.handlers().onEnd?.({}));
    act(() => stream.finish());
    const link = await screen.findByRole("button", { name: "earlier about C2" });
    expect(link.closest(".related")).toHaveAttribute("aria-label", "Déjà demandé");
    await user.click(link);
    const questions = Array.from(document.querySelectorAll(".turn-question"), (el) => el.textContent);
    expect(questions).toEqual(["earlier about C2"]);
    // The thread just left is now the related one.
    expect(screen.getByRole("button", { name: "now?" }).closest(".related")).not.toBeNull();
    window.localStorage.clear();
  });

  it("gives the question back after a failed stream", async () => {
    const stream = captureHandlers();
    const { user } = renderPage(<StreamingAnswer defaultQuery="retry me" />);
    await user.click(screen.getByRole("button", { name: "Poser la question" }));
    expect(screen.getByPlaceholderText("Posez votre question…")).toHaveValue("");
    act(() => stream.handlers().onError?.("down"));
    expect(screen.getByPlaceholderText("Posez votre question…")).toHaveValue("retry me");
    act(() => stream.finish());
  });
});
