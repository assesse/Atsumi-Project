import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppUpdateState } from "../update/useAppUpdater";
import { UpdateDialog } from "./UpdateDialog";

let previousShowModal: PropertyDescriptor | undefined;
let previousClose: PropertyDescriptor | undefined;

beforeEach(() => {
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => window.setTimeout(() => callback(0), 0));
  previousShowModal = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "showModal");
  previousClose = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "close");
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", {
    configurable: true,
    value() { this.setAttribute("open", ""); },
  });
  Object.defineProperty(HTMLDialogElement.prototype, "close", {
    configurable: true,
    value() { this.removeAttribute("open"); },
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
  if (previousShowModal) Object.defineProperty(HTMLDialogElement.prototype, "showModal", previousShowModal);
  else delete (HTMLDialogElement.prototype as unknown as { showModal?: unknown }).showModal;
  if (previousClose) Object.defineProperty(HTMLDialogElement.prototype, "close", previousClose);
  else delete (HTMLDialogElement.prototype as unknown as { close?: unknown }).close;
});

const availableState: AppUpdateState = {
  phase: "available",
  info: {
    currentVersion: "1.0.0",
    version: "1.1.0",
    date: "2026-08-27T00:00:00Z",
    notes: "중복 검토와 업데이트 기능을 개선했습니다.",
  },
  downloadedBytes: 0,
};

describe("UpdateDialog", () => {
  it("shows the version, release notes, verification notice and explicit choices", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const onLater = vi.fn();
    const onInstall = vi.fn();
    try {
      await act(async () => root.render(
        <UpdateDialog open state={availableState} onLater={onLater} onInstall={onInstall} />,
      ));
      expect(container).toHaveTextContent("현재 v1.0.0");
      expect(container).toHaveTextContent("새 버전 v1.1.0");
      expect(container).toHaveTextContent("중복 검토와 업데이트 기능을 개선했습니다.");
      expect(container).toHaveTextContent("전용 업데이트 키로 검증");

      const buttons = [...container.querySelectorAll<HTMLButtonElement>("button")];
      await act(async () => buttons.find((button) => button.textContent === "다운로드 및 설치")?.click());
      expect(onInstall).toHaveBeenCalledOnce();
      await act(async () => buttons.find((button) => button.textContent === "나중에")?.click());
      expect(onLater).toHaveBeenCalledOnce();
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("locks dismissal while download is active and reports progress", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const onLater = vi.fn();
    try {
      await act(async () => root.render(
        <UpdateDialog
          open
          state={{ ...availableState, phase: "downloading", downloadedBytes: 512, totalBytes: 1024 }}
          onLater={onLater}
          onInstall={vi.fn()}
        />,
      ));
      expect(container.querySelector("[role='status']")).toHaveTextContent("50%");
      expect(container.querySelector<HTMLProgressElement>("progress")?.value).toBe(50);
      expect([...container.querySelectorAll<HTMLButtonElement>("button")].every((button) => button.disabled)).toBe(true);

      const dialog = container.querySelector("dialog");
      await act(async () => dialog?.dispatchEvent(new Event("cancel", { bubbles: false, cancelable: true })));
      expect(onLater).not.toHaveBeenCalled();
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });
});
