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
  it("pulses the required target instead of rendering a button-like hint", async () => {
    const container = document.createElement("div");
    const target = document.createElement("button");
    target.id = "tutorial-fixture-target";
    target.textContent = "설정";
    const rect = new DOMRect(40, 40, 100, 40);
    target.getBoundingClientRect = () => rect;
    target.getClientRects = () => Object.assign([rect], { item: () => rect });
    document.body.append(target, container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<TutorialDialog open onClose={vi.fn()} steps={[
        { id: "fixture", section: "공통", target: "#tutorial-fixture-target", title: "저장 위치", description: "저장 위치 설정", action: "click", hint: "이 붉은 글씨를 누르세요" },
      ]} />));
      await act(async () => { await new Promise(resolve => window.setTimeout(resolve, 40)); });
      expect(container.querySelector(".tutorial-spotlight")).toHaveClass("is-action-required");
      expect(container).not.toHaveTextContent("이 붉은 글씨를 누르세요");
      expect(container.querySelector("[data-tour-next]")).toBeNull();
    } finally { await act(async () => root.unmount()); container.remove(); target.remove(); }
  });
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
