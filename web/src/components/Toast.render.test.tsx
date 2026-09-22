// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Rendering tests of the toast provider: the three kinds and their colours,
// stacking order, the 4 s auto-dismissal (fake timers) and the guard in
// `useToast()` outside the provider.

import { describe, expect, it, vi } from "vitest";
import type { ComponentType, ReactNode } from "react";
import { render } from "@testing-library/react";
// @ts-expect-error JSX module
import { ToastProvider as Provider, useToast as useToastHook } from "./Toast.jsx";
import { act, fireEvent, screen } from "../test/render";

interface ToastApi {
  info(m: string): void;
  success(m: string): void;
  error(m: string): void;
}
const ToastProvider = Provider as ComponentType<{ children: ReactNode }>;
const useToast = useToastHook as () => ToastApi;

function Buttons() {
  const toast = useToast();
  return (
    <>
      <button onClick={() => toast.info("FYI")}>info</button>
      <button onClick={() => toast.success("Saved")}>success</button>
      <button onClick={() => toast.error("Failed")}>error</button>
    </>
  );
}

function mount() {
  return render(
    <ToastProvider>
      <Buttons />
    </ToastProvider>,
  );
}

const statuses = () => screen.queryAllByRole("status");

describe("ToastProvider", () => {
  it("renders nothing until a toast is pushed, then a role=status per toast, coloured by kind", () => {
    mount();
    expect(statuses()).toHaveLength(0);

    fireEvent.click(screen.getByText("info"));
    fireEvent.click(screen.getByText("success"));
    fireEvent.click(screen.getByText("error"));

    const list = statuses();
    expect(list.map((el) => el.textContent)).toEqual(["FYI", "Saved", "Failed"]);
    expect(list[0]).toHaveStyle({ background: "rgb(31, 41, 55)" });
    expect(list[1]).toHaveStyle({ background: "rgb(21, 128, 61)" });
    expect(list[2]).toHaveStyle({ background: "rgb(185, 28, 28)" });
  });

  it("dismisses each toast 4 s after it was pushed, oldest first", () => {
    vi.useFakeTimers();
    mount();
    fireEvent.click(screen.getByText("success"));
    act(() => {
      vi.advanceTimersByTime(1500);
    });
    fireEvent.click(screen.getByText("error"));
    expect(statuses().map((el) => el.textContent)).toEqual(["Saved", "Failed"]);

    act(() => {
      vi.advanceTimersByTime(2499);
    });
    expect(statuses().map((el) => el.textContent)).toEqual(["Saved", "Failed"]);

    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(statuses().map((el) => el.textContent)).toEqual(["Failed"]);

    act(() => {
      vi.advanceTimersByTime(1500);
    });
    expect(statuses()).toHaveLength(0);
  });

  it("keeps two toasts with the same message apart", () => {
    vi.useFakeTimers();
    mount();
    fireEvent.click(screen.getByText("info"));
    fireEvent.click(screen.getByText("info"));
    expect(statuses()).toHaveLength(2);
    act(() => {
      vi.advanceTimersByTime(4000);
    });
    expect(statuses()).toHaveLength(0);
  });

  it("useToast() throws outside the provider", () => {
    // React re-dispatches render errors as window `error` events; a
    // default-prevented event keeps jsdom from echoing the stack to stderr.
    const swallow = (e: Event) => e.preventDefault();
    window.addEventListener("error", swallow);
    vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      expect(() => render(<Buttons />)).toThrow("useToast must be used inside <ToastProvider>");
    } finally {
      window.removeEventListener("error", swallow);
    }
  });
});
