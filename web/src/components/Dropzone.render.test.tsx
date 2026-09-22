// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Rendering tests of the Dropzone: hint text, drag highlight, drop and
// hidden-input selection, and the `disabled` guard on every entry point.

import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import Dropzone from "./Dropzone";
import { fireEvent, makeFile, screen } from "../test/render";

function zone(): HTMLElement {
  return screen.getByText("Drop files here or click to upload").closest(".dropzone") as HTMLElement;
}

function input(): HTMLInputElement {
  return zone().querySelector("input[type=file]") as HTMLInputElement;
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("Dropzone", () => {
  it("shows the default hint, or the one given", () => {
    const { unmount } = render(<Dropzone onFile={() => {}} />);
    expect(screen.getByText("JSONL, CSV, XLSX, triples, text or ontology JSON")).toBeInTheDocument();
    unmount();
    render(<Dropzone onFile={() => {}} hint="PDF only" accept=".pdf" />);
    expect(screen.getByText("PDF only")).toBeInTheDocument();
    expect(input()).toHaveAttribute("accept", ".pdf");
  });

  it("highlights while a file is dragged over and drops the first file", () => {
    const onFile = vi.fn();
    render(<Dropzone onFile={onFile} />);
    const dz = zone();
    expect(dz).not.toHaveClass("active");
    fireEvent.dragOver(dz);
    expect(dz).toHaveClass("active");
    fireEvent.dragLeave(dz);
    expect(dz).not.toHaveClass("active");

    fireEvent.dragOver(dz);
    const a = makeFile("a.jsonl", "{}");
    const b = makeFile("b.jsonl", "{}");
    fireEvent.drop(dz, { dataTransfer: { files: [a, b] } });
    expect(dz).not.toHaveClass("active");
    expect(onFile).toHaveBeenCalledTimes(1);
    expect(onFile).toHaveBeenCalledWith(a);
  });

  it("ignores a drop that carries no file", () => {
    const onFile = vi.fn();
    render(<Dropzone onFile={onFile} />);
    fireEvent.drop(zone(), { dataTransfer: { files: [] } });
    expect(onFile).not.toHaveBeenCalled();
  });

  it("opens the hidden file input on click and forwards the chosen file", async () => {
    const user = userEvent.setup();
    const onFile = vi.fn();
    render(<Dropzone onFile={onFile} />);
    const click = vi.spyOn(HTMLInputElement.prototype, "click");
    await user.click(zone());
    // The programmatic click bubbles from the input back to the zone; the
    // browser's click-in-progress guard stops the recursion after one hop.
    expect(click).toHaveBeenCalled();

    const file = makeFile("notes.txt", "hello");
    await user.upload(input(), file);
    expect(onFile).toHaveBeenCalledWith(file);
    // The input is reset so the same file can be picked again.
    expect(input().value).toBe("");
  });

  it("does nothing while disabled: no click-through, no highlight, no drop", async () => {
    const user = userEvent.setup();
    const onFile = vi.fn();
    render(<Dropzone onFile={onFile} disabled />);
    const click = vi.spyOn(HTMLInputElement.prototype, "click");
    const dz = zone();
    await user.click(dz);
    expect(click).not.toHaveBeenCalled();
    fireEvent.dragOver(dz);
    expect(dz).not.toHaveClass("active");
    fireEvent.drop(dz, { dataTransfer: { files: [makeFile("a.csv", "x")] } });
    expect(onFile).not.toHaveBeenCalled();
  });

  it("ignores an input change without a file", () => {
    const onFile = vi.fn();
    render(<Dropzone onFile={onFile} />);
    fireEvent.change(input(), { target: { files: [] } });
    expect(onFile).not.toHaveBeenCalled();
  });
});
