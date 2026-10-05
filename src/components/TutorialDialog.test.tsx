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
  it("starts at the first step without an opt-in screen or replay preference", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const onClose = vi.fn();
    await act(async () => root.render(<TutorialDialog open onClose={onClose} />));
    expect(container.querySelector('[role="dialog"]')).toHaveAttribute("aria-modal", "true");
    expect(container.querySelector("#tutorial-title")).toHaveTextContent("1. 저장 위치 설정");
    expect(container.querySelector(".tutorial-progress")).toBeNull();
    expect(container.querySelector('input[type="checkbox"]')).toBeNull();
    expect(container).not.toHaveTextContent("함께 둘러보기");
    expect(container.querySelector("[data-tour-next]")).toBeNull();
    expect(container.querySelectorAll("button")).toHaveLength(1);
    expect(container).not.toHaveTextContent(/안내 종료|이전|조작 대기/);
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="튜토리얼 닫기"]')?.click());
    expect(onClose).toHaveBeenCalledWith();
    await act(async () => root.unmount());
    container.remove();
  });
});
