import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppActiveWorkSnapshot } from "../api/contracts";
import { ExitConfirmDialog } from "./ExitConfirmDialog";

const noWorkSnapshot: AppActiveWorkSnapshot = {
  queriedAt: "2026-08-23T00:00:00.000Z",
  workSetFingerprint: "none",
  downloads: { activeCount: 0 },
};

const allWorkSnapshot: AppActiveWorkSnapshot = {
  queriedAt: "2026-08-23T00:00:00.000Z",
  workSetFingerprint: "all-work",
  downloads: { activeCount: 2 },
  autoFind: {
    runId: "auto-1",
    completedFavorites: 3,
    totalFavorites: 7,
    candidatesFound: 128,
  },
  duplicateScan: {
    runId: "duplicate-1",
    hashedArtifacts: 12,
    totalArtifacts: 80,
    comparedPairs: 340,
    totalPairs: 3_160,
    candidatesFound: 4,
  },
  internalDuplicateScan: {
    runId: "internal-1",
    scannedArtifacts: 4,
    totalArtifacts: 20,
    skippedArtifacts: 1,
    groupsFound: 3,
  },
};

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

const props = {
  open: true,
  snapshot: noWorkSnapshot as AppActiveWorkSnapshot | null,
  statusError: false,
  actionPending: false,
  forceQuitArmed: false,
  onClose: vi.fn(),
  onMinimizeToTray: vi.fn(),
  onQuit: vi.fn(),
};

describe("ExitConfirmDialog", () => {
  it("renders loading and no-work states with a semantic status", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<ExitConfirmDialog {...props} snapshot={null} />));
      expect(container.querySelector("[role='status']")).toHaveTextContent("작업 상태 확인 중");
      expect(container.querySelector<HTMLButtonElement>(".quit-choice")).toBeDisabled();

      await act(async () => root.render(<ExitConfirmDialog {...props} />));
      expect(container.querySelector("[role='status']")).toHaveTextContent("진행 중인 작업 없음");
      expect(container.querySelector<HTMLButtonElement>(".quit-choice")).toHaveTextContent("종료");
      expect(container.querySelector<HTMLButtonElement>(".quit-choice")).not.toBeDisabled();
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("lists all four managed work kinds and uses the destructive active-work label", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<ExitConfirmDialog {...props} snapshot={allWorkSnapshot} />));
      const rows = [...container.querySelectorAll(".exit-work-list li")].map((row) => row.textContent);
      expect(rows).toEqual([
        "다운로드 2개",
        "Auto Find · 작가 3/7 · 후보 128개",
        "작품 중복 검사 · 아티팩트 12/80 · 비교 340/3160 · 후보 4개",
        "내부 중복 검사 · 앨범 4/20 · 제외 1개 · 검토 행 3개",
      ]);
      expect(container.querySelector(".exit-work-list")?.tagName).toBe("UL");
      expect(container).toHaveTextContent("종료하면 위 작업을 안전하게 취소한 뒤 앱을 닫습니다.");
      expect(container.querySelector<HTMLButtonElement>(".quit-choice")).toHaveTextContent("작업을 중단하고 종료");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("omits invalid zero totals and requires a separate force choice after repeated status failure", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const zeroTotals: AppActiveWorkSnapshot = {
      ...noWorkSnapshot,
      autoFind: { runId: "auto-2", completedFavorites: 0, totalFavorites: 0, candidatesFound: 0 },
    };
    try {
      await act(async () => root.render(<ExitConfirmDialog {...props} snapshot={zeroTotals} />));
      expect(container).toHaveTextContent("Auto Find · 후보 0개");
      expect(container).not.toHaveTextContent("0/0");

      await act(async () => root.render(<ExitConfirmDialog {...props} snapshot={null} statusError />));
      expect(container).toHaveTextContent("작업 상태를 확인할 수 없습니다.");
      expect(container.querySelector<HTMLButtonElement>(".quit-choice")).toHaveTextContent("다시 확인");
      expect(container.querySelector<HTMLButtonElement>(".primary-choice")).not.toBeDisabled();

      await act(async () => root.render(<ExitConfirmDialog {...props} snapshot={null} statusError forceQuitArmed />));
      expect(container.querySelector<HTMLButtonElement>(".quit-choice")).toHaveTextContent("상태 확인 없이 종료");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("keeps cancellation X and restores focus when the dialog closes", async () => {
    const opener = document.createElement("button");
    const container = document.createElement("div");
    document.body.append(opener, container);
    opener.focus();
    const root = createRoot(container);
    const onClose = vi.fn();
    try {
      await act(async () => root.render(<ExitConfirmDialog {...props} onClose={onClose} />));
      const cancel = container.querySelector<HTMLButtonElement>("[aria-label='종료 취소']");
      await act(async () => cancel?.click());
      expect(onClose).toHaveBeenCalledOnce();

      await act(async () => root.render(<ExitConfirmDialog {...props} open={false} onClose={onClose} />));
      await act(async () => { await new Promise((resolve) => window.setTimeout(resolve, 0)); });
      expect(document.activeElement).toBe(opener);
    } finally {
      await act(async () => root.unmount());
      opener.remove();
      container.remove();
    }
  });
});
