// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Rendering tests of the feedback modal: kind picker, validation, submit
// payload, screenshot capture (getDisplayMedia) and import (FileReader),
// close paths and the reset on close.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import FeedbackModal from "./FeedbackModal";
import type { CreateFeedbackInput } from "../api";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return { ...actual, createFeedback: vi.fn() };
});
vi.mock("../lib/logBuffer", () => ({ getLogTail: vi.fn(() => "log line 1\nlog line 2") }));

import * as api from "../api";
import { getLogTail } from "../lib/logBuffer";

const createFeedback = api.createFeedback as unknown as ReturnType<typeof vi.fn>;

function mount(props: Partial<Parameters<typeof FeedbackModal>[0]> = {}) {
  const onClose = vi.fn();
  const onSubmitted = vi.fn();
  const user = userEvent.setup();
  const utils = render(<FeedbackModal open onClose={onClose} onSubmitted={onSubmitted} {...props} />);
  return { onClose, onSubmitted, user, ...utils };
}

beforeEach(() => {
  createFeedback.mockResolvedValue({ id: 1 });
});

afterEach(() => {
  // `navigator.mediaDevices` is set per test; leave nothing behind.
  Object.defineProperty(navigator, "mediaDevices", { value: undefined, configurable: true });
});

describe("FeedbackModal", () => {
  it("renders nothing when closed", () => {
    const { container } = render(<FeedbackModal open={false} onClose={() => {}} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("shows the four kinds with 'Bug' selected by default, and switches on click", async () => {
    const { user } = mount();
    expect(screen.getByRole("heading", { name: /Envoyer un feedback/ })).toBeInTheDocument();
    const kinds = ["Bug", "Erreur", "Évolution", "Amélioration"].map((l) => screen.getByText(l).closest("button")!);
    expect(kinds).toHaveLength(4);
    // Active kind uses the foreground colour as background and white text.
    expect(kinds[0]).toHaveStyle({ color: "rgb(255, 255, 255)" });
    expect(kinds[2]).not.toHaveStyle({ color: "rgb(255, 255, 255)" });
    await user.click(kinds[2]);
    expect(kinds[2]).toHaveStyle({ color: "rgb(255, 255, 255)" });
    expect(kinds[0]).not.toHaveStyle({ color: "rgb(255, 255, 255)" });
  });

  it("refuses to submit without a title", async () => {
    const { user } = mount();
    await user.click(screen.getByRole("button", { name: "Envoyer" }));
    expect(screen.getByText("Le titre est obligatoire.")).toHaveClass("error-banner");
    expect(createFeedback).not.toHaveBeenCalled();
    // Whitespace only is still empty.
    await user.type(screen.getByPlaceholderText("Résumez en quelques mots…"), "   ");
    await user.click(screen.getByRole("button", { name: "Envoyer" }));
    expect(createFeedback).not.toHaveBeenCalled();
  });

  it("submits kind, trimmed title, description, logs, user agent and URL, then notifies and closes", async () => {
    let release: () => void = () => {};
    createFeedback.mockImplementation(() => new Promise<void>((r) => (release = r)));
    const { user, onClose, onSubmitted } = mount();
    await user.click(screen.getByText("Amélioration"));
    await user.type(screen.getByPlaceholderText("Résumez en quelques mots…"), "  Faster graph  ");
    await user.type(screen.getByPlaceholderText(/Décrivez ce qui s'est passé/), "It lags.");
    await user.click(screen.getByRole("button", { name: "Envoyer" }));

    // Busy state while the request is pending.
    expect(screen.getByRole("button", { name: "Envoi…" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Annuler" })).toBeDisabled();

    expect(createFeedback).toHaveBeenCalledTimes(1);
    const payload = createFeedback.mock.calls[0][0] as CreateFeedbackInput;
    expect(payload).toEqual({
      kind: "improvement",
      title: "Faster graph",
      description: "It lags.",
      screenshot: null,
      frontend_logs: "log line 1\nlog line 2",
      user_agent: navigator.userAgent,
      url: window.location.href,
    });
    expect(getLogTail).toHaveBeenCalledWith(300);

    release();
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
    expect(onSubmitted).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: "Envoyer" })).toBeEnabled();
  });

  it("works without onSubmitted", async () => {
    const { user, onClose } = mount({ onSubmitted: undefined });
    await user.type(screen.getByPlaceholderText("Résumez en quelques mots…"), "t");
    await user.click(screen.getByRole("button", { name: "Envoyer" }));
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
  });

  it("shows the API error and stays open; a non-Error rejection is stringified", async () => {
    createFeedback.mockRejectedValueOnce(new Error("503 Service Unavailable"));
    const { user, onClose } = mount();
    await user.type(screen.getByPlaceholderText("Résumez en quelques mots…"), "t");
    await user.click(screen.getByRole("button", { name: "Envoyer" }));
    expect(await screen.findByText("503 Service Unavailable")).toHaveClass("error-banner");
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Envoyer" })).toBeEnabled();

    createFeedback.mockRejectedValueOnce("plain");
    await user.click(screen.getByRole("button", { name: "Envoyer" }));
    expect(await screen.findByText("plain")).toBeInTheDocument();
  });

  it("closes from the × button, Annuler and the backdrop, but not from a click inside the dialog", async () => {
    const { user, onClose } = mount();
    await user.click(screen.getByTitle("Fermer"));
    expect(onClose).toHaveBeenCalledTimes(1);
    await user.click(screen.getByRole("button", { name: "Annuler" }));
    expect(onClose).toHaveBeenCalledTimes(2);
    await user.click(document.querySelector(".feedback-modal")!);
    expect(onClose).toHaveBeenCalledTimes(2);
    await user.click(document.querySelector(".feedback-backdrop")!);
    expect(onClose).toHaveBeenCalledTimes(3);
  });

  it("resets the form when closed (parent keeps it mounted)", async () => {
    const { user } = mount();
    await user.type(screen.getByPlaceholderText("Résumez en quelques mots…"), "draft");
    await user.click(screen.getByText("Erreur"));
    await user.click(screen.getByRole("button", { name: "Envoyer" })); // no-op: title set → request
    await user.click(screen.getByRole("button", { name: "Annuler" }));
    expect(screen.getByPlaceholderText("Résumez en quelques mots…")).toHaveValue("");
    expect(screen.getByText("Bug").closest("button")).toHaveStyle({ color: "rgb(255, 255, 255)" });
  });

  describe("screenshot import", () => {
    it("previews an imported image and lets the user remove it", async () => {
      const { user } = mount();
      const remove = screen.getByTitle("Supprimer");
      expect(remove).toBeDisabled();
      const input = document.querySelector<HTMLInputElement>('input[type="file"]')!;
      const clickSpy = vi.spyOn(input, "click");
      await user.click(screen.getByRole("button", { name: "Importer" }));
      expect(clickSpy).toHaveBeenCalledTimes(1);

      const png = new File([new Uint8Array([137, 80, 78, 71])], "shot.png", { type: "image/png" });
      fireEvent.change(input, { target: { files: [png] } });
      const img = await screen.findByAltText("capture");
      expect(img.getAttribute("src")).toMatch(/^data:image\/png;base64,/);
      expect(remove).toBeEnabled();
      await user.click(remove);
      expect(screen.queryByAltText("capture")).not.toBeInTheDocument();
    });

    it("ignores a change event without a file", () => {
      mount();
      const input = document.querySelector<HTMLInputElement>('input[type="file"]')!;
      fireEvent.change(input, { target: { files: [] } });
      expect(screen.queryByAltText("capture")).not.toBeInTheDocument();
    });

    it("reports a FileReader failure", async () => {
      class FailingReader {
        onload: (() => void) | null = null;
        onerror: (() => void) | null = null;
        result: string | null = null;
        readAsDataURL(): void {
          setTimeout(() => this.onerror?.(), 0);
        }
      }
      vi.stubGlobal("FileReader", FailingReader);
      mount();
      const input = document.querySelector<HTMLInputElement>('input[type="file"]')!;
      fireEvent.change(input, { target: { files: [new File(["x"], "x.png", { type: "image/png" })] } });
      expect(await screen.findByText("Lecture du fichier impossible.")).toBeInTheDocument();
    });
  });

  describe("screenshot capture", () => {
    it("explains when getDisplayMedia is unavailable", async () => {
      Object.defineProperty(navigator, "mediaDevices", { value: {}, configurable: true });
      const { user } = mount();
      await user.click(screen.getByRole("button", { name: /Capturer/ }));
      expect(await screen.findByText("Capture d'écran non supportée par ce navigateur.")).toBeInTheDocument();
    });

    it("captures one frame from the shared surface, stops the tracks and previews it", async () => {
      const stop = vi.fn();
      const track = { stop };
      const stream = { getVideoTracks: () => [track], getTracks: () => [track, { stop }] };
      const getDisplayMedia = vi.fn().mockResolvedValue(stream);
      Object.defineProperty(navigator, "mediaDevices", { value: { getDisplayMedia }, configurable: true });
      const play = vi.spyOn(HTMLMediaElement.prototype, "play").mockResolvedValue(undefined);
      vi.useFakeTimers();
      mount();
      fireEvent.click(screen.getByRole("button", { name: /Capturer/ }));
      await vi.advanceTimersByTimeAsync(250);
      vi.useRealTimers();
      const img = await screen.findByAltText("capture");
      expect(img).toHaveAttribute("src", "data:image/png;base64,");
      expect(getDisplayMedia).toHaveBeenCalledWith({ video: true, audio: false });
      expect(play).toHaveBeenCalledTimes(1);
      expect(stop).toHaveBeenCalledTimes(3);
    });

    it("reports a missing 2D canvas context", async () => {
      const stop = vi.fn();
      const track = { stop };
      const stream = { getVideoTracks: () => [track], getTracks: () => [track] };
      Object.defineProperty(navigator, "mediaDevices", {
        value: { getDisplayMedia: vi.fn().mockResolvedValue(stream) },
        configurable: true,
      });
      vi.spyOn(HTMLMediaElement.prototype, "play").mockResolvedValue(undefined);
      vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
      vi.useFakeTimers();
      mount();
      fireEvent.click(screen.getByRole("button", { name: /Capturer/ }));
      await vi.advanceTimersByTimeAsync(250);
      vi.useRealTimers();
      expect(await screen.findByText("Canvas 2D non disponible.")).toBeInTheDocument();
      expect(stop).not.toHaveBeenCalled();
    });

    it("shows the browser's refusal (a non-Error is stringified)", async () => {
      Object.defineProperty(navigator, "mediaDevices", {
        value: { getDisplayMedia: vi.fn().mockRejectedValue("NotAllowedError") },
        configurable: true,
      });
      const { user } = mount();
      await user.click(screen.getByRole("button", { name: /Capturer/ }));
      expect(await screen.findByText("NotAllowedError")).toBeInTheDocument();
    });
  });
});
