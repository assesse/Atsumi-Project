import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { FeedbackDialog } from "./FeedbackDialog";
import type { FeedbackApi } from "./api";
let root: Root, node: HTMLDivElement;
const draft = { id: "fixture-1", eventCount: 2, bytes: 512, createdAt: 1, payload: { events: [] } };
const api = { prepare: vi.fn<FeedbackApi["prepare"]>(), send: vi.fn<FeedbackApi["send"]>() };
const close = vi.fn();
const button = (text: string) => [...node.querySelectorAll<HTMLButtonElement>("button")].find(e => e.textContent === text)!;
beforeEach(() => { vi.clearAllMocks(); vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); HTMLDialogElement.prototype.showModal = function () { this.open = true; }; node = document.createElement("div"); document.body.append(node); root = createRoot(node); api.prepare.mockResolvedValue(draft); });
afterEach(async () => { await act(async () => root.unmount()); node.remove(); vi.unstubAllGlobals(); });
const render = () => act(async () => root.render(<FeedbackDialog source="hitomi" onClose={close} api={api} />));
it("prepares locally and never sends until explicit confirmation", async () => {
  await render(); expect(api.prepare).toHaveBeenCalledWith("hitomi"); expect(api.send).not.toHaveBeenCalled();
  expect(node.querySelector("details")).not.toHaveAttribute("open"); expect(node).toHaveTextContent("2건");
  api.send.mockResolvedValue({ receiptId: "receipt-1" });
  await act(async () => button("보내기").click());
  expect(api.send).toHaveBeenCalledExactlyOnceWith(draft.id, ""); expect(node).toHaveTextContent("receipt-1");
  expect(node.querySelectorAll("button")).toHaveLength(1);
  expect(node.querySelector('[aria-label="닫기"]')).toBeNull();
  await act(async () => button("완료").click()); expect(close).toHaveBeenCalledOnce();
});
it("blocks duplicate clicks and retains the same draft and comment on retry", async () => {
  await render(); let reject!: (e: Error) => void;
  api.send.mockReturnValueOnce(new Promise((_, fail) => { reject = fail; }));
  await act(async () => { button("보내기").click(); button("보내기")?.click(); });
  expect(api.send).toHaveBeenCalledOnce(); expect(button("보내는 중…")).toBeDisabled();
  expect(node.querySelector('button[aria-label="닫기"]')).toBeDisabled();
  await act(async () => reject(new Error("연결 실패"))); expect(node).toHaveTextContent("연결 실패");
  expect(node.querySelector("textarea")).toBeDisabled();
  api.send.mockResolvedValueOnce({ receiptId: "receipt-1" });
  await act(async () => button("보내기").click()); expect(api.send.mock.calls).toEqual([[draft.id, ""], [draft.id, ""]]);
});
it("does not send after a failed preparation and supports preparing again", async () => {
  api.prepare.mockRejectedValueOnce(new Error("진단 파일 오류")); await render();
  expect(button("보내기")).toBeDisabled(); expect(node).toHaveTextContent("진단 파일 오류");
  await act(async () => button("다시 준비").click()); expect(button("보내기")).toBeEnabled(); expect(api.send).not.toHaveBeenCalled();
});
