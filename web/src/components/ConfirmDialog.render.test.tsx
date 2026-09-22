// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Rendering tests of the confirm dialog: default and custom copy, the
// danger style, and every way to settle the promise — confirm / cancel
// buttons, Enter / Escape keys, backdrop click (the card itself does not
// close) — plus the guard in `useConfirm()` outside the provider.

import { describe, expect, it, vi } from "vitest";
import type { ComponentType, ReactNode } from "react";
import { render } from "@testing-library/react";
import { useState } from "react";
// @ts-expect-error JSX module
import { ConfirmProvider as Provider, useConfirm as useConfirmHook } from "./ConfirmDialog.jsx";
import { fireEvent, screen, waitFor } from "../test/render";

interface ConfirmOptions {
  title?: string;
  message?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  danger?: boolean;
}
const ConfirmProvider = Provider as ComponentType<{ children: ReactNode }>;
const useConfirm = useConfirmHook as () => (opts?: ConfirmOptions) => Promise<boolean>;

function Trigger({ opts }: { opts?: ConfirmOptions }) {
  const confirm = useConfirm();
  const [result, setResult] = useState("pending");
  return (
    <>
      <button onClick={() => void confirm(opts).then((r) => setResult(String(r)))}>ask</button>
      <output data-testid="result">{result}</output>
    </>
  );
}

function mount(opts?: ConfirmOptions) {
  const view = render(
    <ConfirmProvider>
      <Trigger opts={opts} />
    </ConfirmProvider>,
  );
  fireEvent.click(screen.getByText("ask"));
  return view;
}

const dialog = () => screen.getByRole("dialog");
const result = () => screen.getByTestId("result");

describe("ConfirmProvider", () => {
  it("opens a modal dialog with the default copy", () => {
    mount();
    expect(screen.queryByRole("dialog")).toBeInTheDocument();
    expect(dialog()).toHaveAttribute("aria-modal", "true");
    expect(screen.getByRole("heading", { name: "Confirm" })).toBeInTheDocument();
    expect(screen.getByText("Are you sure?")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
    const ok = screen.getByRole("button", { name: "Confirm" });
    expect(ok).toHaveFocus();
    expect(ok.style.background).toBe("");
    expect(result()).toHaveTextContent("pending");
  });

  it("uses the custom copy and paints the confirm button red when danger", () => {
    mount({ title: "Delete file?", message: "This cannot be undone.", confirmLabel: "Delete", cancelLabel: "Keep", danger: true });
    expect(screen.getByRole("heading", { name: "Delete file?" })).toBeInTheDocument();
    expect(screen.getByText("This cannot be undone.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Keep" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Delete" })).toHaveStyle({ background: "rgb(220, 38, 38)" });
  });

  it("resolves true on the confirm button and closes", async () => {
    mount();
    fireEvent.click(screen.getByRole("button", { name: "Confirm" }));
    await waitFor(() => expect(result()).toHaveTextContent("true"));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("resolves false on the cancel button", async () => {
    mount();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(result()).toHaveTextContent("false"));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("resolves true on Enter and false on Escape; other keys are ignored", async () => {
    mount();
    fireEvent.keyDown(window, { key: "a" });
    expect(dialog()).toBeInTheDocument();
    fireEvent.keyDown(window, { key: "Enter" });
    await waitFor(() => expect(result()).toHaveTextContent("true"));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    // Reopen: the key listener is re-attached for the new dialog.
    fireEvent.click(screen.getByText("ask"));
    expect(dialog()).toBeInTheDocument();
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(result()).toHaveTextContent("false"));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("detaches the key listener once closed", async () => {
    mount();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(result()).toHaveTextContent("false"));
    // A late Enter has no dialog to settle: nothing changes and nothing throws.
    fireEvent.keyDown(window, { key: "Enter" });
    expect(result()).toHaveTextContent("false");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("clicking the backdrop cancels, clicking inside the card does not", async () => {
    mount();
    fireEvent.click(screen.getByText("Are you sure?"));
    expect(dialog()).toBeInTheDocument();
    expect(result()).toHaveTextContent("pending");

    fireEvent.click(dialog());
    await waitFor(() => expect(result()).toHaveTextContent("false"));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("useConfirm() throws outside the provider", () => {
    // React re-dispatches render errors as window `error` events; a
    // default-prevented event keeps jsdom from echoing the stack to stderr.
    const swallow = (e: Event) => e.preventDefault();
    window.addEventListener("error", swallow);
    vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      expect(() => render(<Trigger />)).toThrow("useConfirm must be used inside <ConfirmProvider>");
    } finally {
      window.removeEventListener("error", swallow);
    }
  });
});
