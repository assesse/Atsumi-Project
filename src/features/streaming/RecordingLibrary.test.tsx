import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { BrowserRecording } from "../../api/officialBrowser";
import { RecordingLibrary, recordingChatNote } from "./RecordingLibrary";

const token = "a".repeat(32);
const item = (id: string, patch: Partial<BrowserRecording> = {}): BrowserRecording => ({ id, channelId: `channel-${id}`, title: `방송 ${id}`, startedAt: 1800000000000, updatedAt: 1800000001000, status: "stopped", mimeType: "video/webm", outputDir: "C:\\SyntheticOnly", segmentCount: 2, bytesWritten: 2000, durationSeconds: 30, lastError: null, segments: [{ index: 0, file: "segment-000000000000.webm", bytes: 1000, durationSeconds: 15 }], ...patch });
const merged = (id: string) => item(id, { merge: { status: "complete", segmentCount: 2, updatedAt: 1, file: `merged-${token}.webm`, timelineFile: `merged-${token}.timeline.jsonl`, bytes: 1500, durationSeconds: 30 } });
let root: Root, container: HTMLDivElement;
const callbacks = { onSelect: vi.fn(), onFolder: vi.fn(), onReplay: vi.fn(), onOpenMerged: vi.fn(), onRetryMerge: vi.fn(), onOpenSegment: vi.fn(), onDelete: vi.fn<Parameters<typeof RecordingLibrary>[0]["onDelete"]>() };
const render = async (recordings: BrowserRecording[], selectedId: string | null = null, privacy = false, disabled = false) => {
  await act(async () => root.render(<RecordingLibrary recordings={recordings} selectedId={selectedId} privacy={privacy} disabled={disabled} retrying={false} opening={false} {...callbacks} />));
};
const button = (text: string) => [...container.querySelectorAll<HTMLButtonElement>("button")].find(element => element.textContent === text)!;
const cards = () => [...container.querySelectorAll<HTMLButtonElement>(".recording-library-card")];
const search = async (value: string) => { await act(async () => { const input = container.querySelector<HTMLInputElement>('input[type="search"]')!; Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value); input.dispatchEvent(new Event("input", { bubbles: true })); }); };
beforeEach(() => { vi.clearAllMocks(); callbacks.onDelete.mockResolvedValue({ deletedIds: [], failures: [] }); vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); HTMLDialogElement.prototype.showModal = function () { this.open = true; }; HTMLDialogElement.prototype.close = function () { this.open = false; }; container = document.createElement("div"); document.body.append(container); root = createRoot(container); });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.unstubAllGlobals(); });

describe("recording library", () => {
  it("separates empty startup failures from saved broadcasts without deleting their diagnostics", async () => {
    await render([item("retry", { segmentCount: 0, bytesWritten: 0, durationSeconds: 0, status: "interrupted" }), merged("success")]);
    expect(container).toHaveTextContent("저장 영상 1개");
    const attempts = container.querySelector<HTMLDetailsElement>(".recording-library-attempts")!;
    expect(attempts.open).toBe(false);
    expect(attempts).toHaveTextContent("시작 실패 · 영상 저장 없음");
    expect(callbacks.onDelete).not.toHaveBeenCalled();
  });
  it("shows confirmed broadcast completion separately from manual stopping", async () => {
    await render([item("ended", { ending: { reason: "broadcast_ended", trigger: "video_ended", stoppedAt: 1, confirmedAt: 2 } }), item("manual", { ending: { reason: "user_stopped", trigger: "user_stop", stoppedAt: 1, confirmedAt: null } })]);
    expect(container).toHaveTextContent("방송 종료 · 저장 완료");
    expect(container).toHaveTextContent("직접 중지 · 저장 완료");
  });
  it("allows explicit selection of an empty attempt without including hidden attempts in select-all", async () => {
    await render([item("empty", { segmentCount: 0, bytesWritten: 0, durationSeconds: 0, status: "interrupted" })]);
    expect(button("선택")).toBeEnabled();
    await act(async () => button("선택").click());
    expect(button("전체 선택")).toBeDisabled();
    await act(async () => cards()[0]!.click());
    expect(container).toHaveTextContent("1개 선택");
    expect(callbacks.onDelete).not.toHaveBeenCalled();
  });
  it("includes unfinished but replayable ranges in the ready filter", async () => {
    await render([item("ranges", { progressive: { partCount: 1, segmentCount: 1, durationSeconds: 15, lastError: null } }), item("waiting")]);
    await act(async () => button("재생 가능").click());
    expect(cards()).toHaveLength(1); expect(cards()[0]).toHaveTextContent("방송 ranges");
    expect(button("앱에서 다시보기")).toBeEnabled();
  });
  it("selects multiple saved recordings, confirms exact IDs, and never deletes on cancel", async () => {
    await render([item("live", { status: "recording" }), merged("a"), merged("b")]);
    await act(async () => button("선택").click());
    expect(container.querySelectorAll('input[type="checkbox"]')).toHaveLength(2);
    await act(async () => { cards()[1]!.click(); cards()[2]!.click(); });
    expect(container).toHaveTextContent("2개 선택");
    await act(async () => button("선택 삭제").click());
    expect(container.querySelector("dialog")).toHaveAttribute("open");
    expect(container.querySelector("dialog")).toHaveTextContent("복구할 수 없습니다");
    expect(document.activeElement).toBe(button("취소"));
    await act(async () => button("취소").click());
    expect(callbacks.onDelete).not.toHaveBeenCalled();
    callbacks.onDelete.mockResolvedValueOnce({ deletedIds: ["a", "b"], failures: [] });
    await act(async () => button("선택 삭제").click());
    await act(async () => button("삭제").click());
    expect(callbacks.onDelete).toHaveBeenCalledExactlyOnceWith(["a", "b"]);
    expect(container.querySelector("dialog")).not.toHaveAttribute("open");
    expect(container).toHaveTextContent("2개 녹화를 삭제했습니다.");
  });

  it("select-all includes filtered later pages but not active, merging or cleaning records", async () => {
    const records = Array.from({ length: 30 }, (_, index) => merged(`a${index}`));
    await render([item("live", { status: "recording" }), item("busy", { merge: { status: "merging", segmentCount: 2, updatedAt: 1 } }), ...records]);
    await act(async () => button("선택").click());
    expect(container.querySelector<HTMLInputElement>('input[aria-label="방송 busy 삭제 선택"]')).toBeDisabled();
    await act(async () => button("전체 선택").click());
    expect(container).toHaveTextContent("30개 선택");
    await search("방송 a2");
    expect(container).toHaveTextContent("0개 선택");
    await act(async () => button("전체 선택").click());
    expect(container).toHaveTextContent("11개 선택");
  });

  it("retains only failed selections and reports partial success without hiding failures", async () => {
    await render([merged("a"), merged("b")]);
    callbacks.onDelete.mockResolvedValueOnce({ deletedIds: ["a"], failures: [{ id: "b", error: { code: "BUSY", message: "파일 사용 중", retryable: true } }] });
    await act(async () => button("선택").click());
    await act(async () => button("전체 선택").click());
    await act(async () => button("선택 삭제").click());
    await act(async () => button("삭제").click());
    await render([merged("b")]);
    expect(container).toHaveTextContent("1개 녹화를 삭제했습니다.");
    expect(container.querySelector('[role="alert"]')).toHaveTextContent("방송 b — 파일 사용 중");
    expect(container).toHaveTextContent("1개 선택");
    expect(container.querySelector<HTMLInputElement>('input[type="checkbox"]')!.checked).toBe(true);
  });

  it("fences repeated submits and keeps a transport failure retryable", async () => {
    let reject!: (error: Error) => void;
    callbacks.onDelete.mockReturnValueOnce(new Promise((_, fail) => { reject = fail; }));
    await render([merged("a")]);
    await act(async () => button("선택").click());
    await act(async () => button("전체 선택").click());
    await act(async () => button("선택 삭제").click());
    await act(async () => { button("삭제").click(); button("삭제")?.click(); });
    expect(button("삭제 중…")).toBeDisabled(); expect(button("취소")).toBeDisabled();
    expect(callbacks.onDelete).toHaveBeenCalledTimes(1);
    await act(async () => reject(new Error("연결 확인 필요")));
    expect(container.querySelector("dialog")).toHaveAttribute("open");
    expect(container.querySelector("dialog")).toHaveTextContent("연결 확인 필요");
    expect(button("삭제")).toBeEnabled();
  });

  it("drops newly protected selections on polling and hides private titles in confirmation", async () => {
    await render([merged("secret"), merged("busy")]);
    await act(async () => button("선택").click());
    await act(async () => button("전체 선택").click());
    await render([merged("secret"), item("busy", { merge: { status: "merging", segmentCount: 2, updatedAt: 1 } })]);
    expect(container).toHaveTextContent("1개 선택");
    await act(async () => button("선택 삭제").click());
    await render([merged("secret")], null, true);
    expect(container.querySelector("dialog")).not.toHaveTextContent("secret");
    expect(container.querySelector("dialog")).toHaveTextContent("녹화 영상");
  });

  it("offers retry deletion, not playback or merge, for interrupted deletion", async () => {
    await render([item("a", { deletionPending: true })]);
    expect(container).toHaveTextContent("삭제 미완료");
    expect(button("앱에서 다시보기")).toBeUndefined();
    await act(async () => button("선택").click());
    await act(async () => button("전체 선택").click());
    expect(container).toHaveTextContent("1개 선택");
  });
  it("separates active recordings and offers saved playback only on explicit action", async () => {
    await render([item("live", { status: "recording" }), merged("ready")]);
    expect(container.querySelector('.recording-library-active')).toHaveTextContent("방송 live");
    expect(container.querySelector('.recording-library-grid')).not.toHaveTextContent("방송 live");
    expect(container.querySelector('.recording-library-detail')).toHaveTextContent("방송 ready");
    expect(container.querySelector("video,iframe,img")).toBeNull();
    expect(callbacks.onReplay).not.toHaveBeenCalled();
    await act(async () => button("앱에서 다시보기").click());
    expect(callbacks.onReplay).toHaveBeenCalledExactlyOnceWith("ready");
    await act(async () => cards()[0]!.click());
    expect(callbacks.onSelect).toHaveBeenCalledExactlyOnceWith("live");
  });

  it("filters saved metadata while keeping live recordings visible", async () => {
    await render([item("live", { status: "recording" }), merged("ready"), item("failed", { lastError: "문제", status: "failed" }), item("merging", { merge: { status: "merging", segmentCount: 2, updatedAt: 1 } })]);
    await act(async () => button("재생 가능").click());
    expect(cards()).toHaveLength(2); expect(cards()[1]).toHaveTextContent("방송 ready");
    await act(async () => button("확인 필요").click());
    expect(cards()).toHaveLength(2); expect(cards()[1]).toHaveTextContent("방송 failed");
    await act(async () => button("전체").click());
    await search("CHANNEL-READY");
    expect(cards()).toHaveLength(2); expect(cards()[1]).toHaveTextContent("방송 ready");
    await search("일치하지않음");
    expect(cards()).toHaveLength(1); expect(container).toHaveTextContent("조건에 맞는 녹화가 없습니다");
  });

  it("renders 24 metadata cards at a time without opening media and retains more across polling", async () => {
    const recordings = Array.from({ length: 55 }, (_, index) => merged(String(index)));
    await render(recordings);
    expect(cards()).toHaveLength(24);
    await act(async () => button("더 보기 · 31개").click());
    expect(cards()).toHaveLength(48);
    await render(recordings.map(recording => ({ ...recording, updatedAt: recording.updatedAt + 1 })));
    expect(cards()).toHaveLength(48);
    await search("방송 54"); expect(cards()).toHaveLength(1);
    await search(""); expect(cards()).toHaveLength(24);
    expect(callbacks.onOpenSegment).not.toHaveBeenCalled();
    expect(callbacks.onOpenMerged).not.toHaveBeenCalled();
  });

  it("preserves selection across snapshot replacement and routes file actions only to it", async () => {
    await render([merged("a"), merged("b")], "b");
    expect(cards()[1]).toHaveAttribute("aria-pressed", "true");
    await render([merged("new"), merged("a"), merged("b")], "b");
    expect(container.querySelector('.recording-library-detail')).toHaveTextContent("방송 b");
    await act(async () => button("녹화 폴더 열기").click());
    await act(async () => button("외부 플레이어로 열기").click());
    expect(callbacks.onFolder).toHaveBeenCalledExactlyOnceWith("b");
    expect(callbacks.onOpenMerged).toHaveBeenCalledExactlyOnceWith("b");
    await render([merged("b")], "b", false, true);
    expect(button("녹화 폴더 열기")).toBeDisabled(); expect(button("앱에서 다시보기")).toBeDisabled();
  });

  it("clears private queries and hides broadcast titles, paths and filenames", async () => {
    const values = [merged("secret")]; await render(values); await search("secret");
    await render(values, null, true);
    expect(container).not.toHaveTextContent("secret"); expect(container).not.toHaveTextContent("SyntheticOnly");
    expect(container.querySelector<HTMLInputElement>('input[type="search"]')).toBeDisabled();
    expect(container.querySelector<HTMLInputElement>('input[type="search"]')?.value).toBe("");
    expect(container.querySelector("video,iframe,img")).toBeNull();
  });

  it("does not crash the library when legacy metadata contains an out-of-range date", async () => {
    await render([item("legacy", { startedAt: 1e20 })]);
    expect(cards()).toHaveLength(1);
    expect(cards()[0]?.querySelector("time")).not.toHaveAttribute("datetime");
  });

  it("does not infer saved chat for legacy or explicitly disabled recording data", () => {
    expect(recordingChatNote(item("legacy")).text).toBe("이 기록의 채팅 저장 상태는 확인되지 않았습니다.");
    expect(recordingChatNote(item("off", { captureChat: false, chatCount: 12 })).text).toBe("이 기록은 채팅 저장을 사용하지 않았습니다.");
    expect(recordingChatNote(item("bad", { captureChat: true, chatStatus: "storage_failed", chatCount: 42 }))).toEqual({ warning: true, text: "채팅 저장 실패 · 일부 기록 누락 가능 · 42개 기록" });
  });
});
