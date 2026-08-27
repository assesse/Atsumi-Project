import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TutorialDialog } from "./TutorialDialog";

let previousShowModal: PropertyDescriptor | undefined;
let previousClose: PropertyDescriptor | undefined;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  previousShowModal = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "showModal");
  previousClose = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "close");
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", { configurable: true, value() { this.setAttribute("open", ""); } });
  Object.defineProperty(HTMLDialogElement.prototype, "close", { configurable: true, value() { this.removeAttribute("open"); } });
});

afterEach(() => {
  if (previousShowModal) Object.defineProperty(HTMLDialogElement.prototype, "showModal", previousShowModal);
  else delete (HTMLDialogElement.prototype as unknown as { showModal?: unknown }).showModal;
  if (previousClose) Object.defineProperty(HTMLDialogElement.prototype, "close", previousClose);
  else delete (HTMLDialogElement.prototype as unknown as { close?: unknown }).close;
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("TutorialDialog", () => {
  it("returns the explicit do-not-show-again choice", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const onClose = vi.fn();
    await act(async () => root.render(<TutorialDialog open onClose={onClose} />));
    expect(container.querySelector("dialog")).toHaveAttribute("open");
    expect(container).toHaveTextContent("artist:healthyman female:ahegao");
    await act(async () => container.querySelector<HTMLInputElement>('input[type="checkbox"]')?.click());
    await act(async () => [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Atsumi 시작")?.click());
    expect(onClose).toHaveBeenCalledWith(true);
    await act(async () => root.unmount());
    container.remove();
  });
});
