// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
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
  it("disables Ask until the query has non-blank text, and never calls the API for a blank one", async () => {
    const { user } = renderPage(<StreamingAnswer />);
    const ask = screen.getByRole("button", { name: "Ask" });
    expect(ask).toBeDisabled();
    await user.type(screen.getByPlaceholderText("Ask the ontology…"), "   ");
    expect(ask).toBeDisabled();
    await user.type(screen.getByPlaceholderText("Ask the ontology…"), "who?");
    expect(ask).toBeEnabled();
    expect(askStream).not.toHaveBeenCalled();
  });

  it("prefills the textarea from defaultQuery", () => {
    renderPage(<StreamingAnswer defaultQuery="renewals" />);
    expect(screen.getByPlaceholderText("Ask the ontology…")).toHaveValue("renewals");
    expect(screen.getByRole("button", { name: "Ask" })).toBeEnabled();
  });

  it("streams: shows the cursor while running, appends tokens, reports grounding and citations (max 12), then ends", async () => {
    const stream = captureHandlers();
    const { user } = renderPage(<StreamingAnswer defaultQuery="which contracts?" />);
    await user.click(screen.getByRole("button", { name: "Ask" }));

    expect(askStream).toHaveBeenCalledTimes(1);
    expect(askStream.mock.calls[0][0]).toEqual({ query: "which contracts?" });
    expect(screen.getByRole("button", { name: "Streaming…" })).toBeDisabled();
    expect(screen.getByText("▍")).toBeInTheDocument();

    act(() => stream.handlers().onRetrieved?.(subgraph));
    expect(screen.getByText("Grounded on 14 concepts, 1 relations")).toBeInTheDocument();
    expect(screen.getAllByText(/Contract · C\d+/)).toHaveLength(12);
    expect(screen.getByText("Contract · C12")).toBeInTheDocument();
    expect(screen.queryByText("Contract · C13")).not.toBeInTheDocument();

    act(() => {
      stream.handlers().onToken?.("Two ");
      stream.handlers().onToken?.("contracts.");
    });
    expect(screen.getByText(/Two contracts\./)).toBeInTheDocument();

    act(() => stream.handlers().onEnd?.({ usage: {} }));
    expect(screen.getByRole("button", { name: "Ask" })).toBeEnabled();
    expect(screen.queryByText("▍")).not.toBeInTheDocument();
    // The answer stays visible after the stream ends.
    expect(screen.getByText(/Two contracts\./)).toBeInTheDocument();

    act(() => stream.finish());
    await waitFor(() => expect(screen.getByRole("button", { name: "Ask" })).toBeEnabled());
  });

  it("shows the grounding line but no citations when the subgraph has no concepts", async () => {
    const stream = captureHandlers();
    const { user } = renderPage(<StreamingAnswer defaultQuery="q" />);
    await user.click(screen.getByRole("button", { name: "Ask" }));
    act(() => stream.handlers().onRetrieved?.({ concepts: [], relations: [] }));
    expect(screen.getByText("Grounded on 0 concepts, 0 relations")).toBeInTheDocument();
    expect(document.querySelector(".citations")).toBeNull();
    act(() => stream.finish());
    await waitFor(() => expect(screen.getByRole("button", { name: "Ask" })).toBeEnabled());
  });

  it("surfaces a stream 'error' event and stops streaming", async () => {
    const stream = captureHandlers();
    const { user } = renderPage(<StreamingAnswer defaultQuery="q" />);
    await user.click(screen.getByRole("button", { name: "Ask" }));
    act(() => stream.handlers().onError?.("llm quota exceeded"));
    expect(screen.getByText("llm quota exceeded")).toHaveClass("error-banner");
    expect(screen.getByRole("button", { name: "Ask" })).toBeEnabled();
    act(() => stream.finish());
  });

  it("surfaces a thrown Error's message", async () => {
    askStream.mockRejectedValue(new Error("stream failed: 503 Service Unavailable"));
    const { user } = renderPage(<StreamingAnswer defaultQuery="q" />);
    await user.click(screen.getByRole("button", { name: "Ask" }));
    expect(await screen.findByText("stream failed: 503 Service Unavailable")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Ask" })).toBeEnabled();
    expect(document.querySelector(".answer-box")).toBeNull();
  });

  it("stringifies a non-Error rejection", async () => {
    askStream.mockRejectedValue("boom");
    const { user } = renderPage(<StreamingAnswer defaultQuery="q" />);
    await user.click(screen.getByRole("button", { name: "Ask" }));
    expect(await screen.findByText("boom")).toBeInTheDocument();
  });

  it("clears the previous answer, error and grounding when asked again", async () => {
    askStream.mockRejectedValueOnce(new Error("first failure"));
    const stream2 = { current: null as AskStreamHandlers | null };
    const { user } = renderPage(<StreamingAnswer defaultQuery="q" />);
    await user.click(screen.getByRole("button", { name: "Ask" }));
    expect(await screen.findByText("first failure")).toBeInTheDocument();

    askStream.mockImplementation(async (_req: unknown, h: AskStreamHandlers) => {
      stream2.current = h;
      h.onRetrieved?.(subgraph);
      h.onToken?.("ok");
    });
    await user.click(screen.getByRole("button", { name: "Ask" }));
    await waitFor(() => expect(screen.getByText("ok")).toBeInTheDocument());
    expect(screen.queryByText("first failure")).not.toBeInTheDocument();
    expect(stream2.current).not.toBeNull();
  });
});
