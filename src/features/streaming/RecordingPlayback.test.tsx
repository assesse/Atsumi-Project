import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { BrowserMerge, BrowserRecording } from "../../api/officialBrowser";
import { hasCompletedMerge, RecordingPlayback } from "./RecordingPlayback";

const token = "a".repeat(32);
const merge = (patch: Partial<BrowserMerge> = {}): BrowserMerge => ({ status: "complete", segmentCount: 2, updatedAt: 1, file: `merged-${token}.webm`, timelineFile: `merged-${token}.timeline.jsonl`, bytes: 2000, durationSeconds: 30, ...patch });
const recording = (patch: Partial<BrowserRecording> = {}): BrowserRecording => ({ id: "selected-recording", channelId: "b".repeat(32), title: "저장한 방송", startedAt: 0, updatedAt: 1, status: "stopped", mimeType: "video/webm", outputDir: "C:\\SyntheticOnly", segmentCount: 2, bytesWritten: 2000, durationSeconds: 30, lastError: null, segments: [{ index: 0, file: "segment-000000000000.webm", bytes: 1000, durationSeconds: 15 }, { index: 1, file: "segment-000000000001.webm", bytes: 1000, durationSeconds: 15 }], ...patch });
let root: Root;
let container: HTMLDivElement;
const callbacks = { onOpenMerged: vi.fn(), onReplay: vi.fn(), onRetryMerge: vi.fn(), onOpenSegment: vi.fn() };
const render = async (value: BrowserRecording, patch: { disabled?: boolean; privacyMode?: boolean; retrying?: boolean; opening?: boolean } = {}) => {
  await act(async () => root.render(<RecordingPlayback key={value.id} recording={value} disabled={false} privacyMode={false} retrying={false} opening={false} {...callbacks} {...patch} />));
};
const button = (label: string) => [...container.querySelectorAll<HTMLButtonElement>("button")].find((item) => item.textContent === label);
beforeEach(() => { vi.clearAllMocks(); vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); container = document.createElement("div"); document.body.append(container); root = createRoot(container); });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.unstubAllGlobals(); });

describe("recorded file playback and merge status", () => {
  it.each(["pending", "complete", "blocked"] as const)("keeps merged replay available during %s source cleanup and hides stale source actions", async (status) => {
    await render(recording({ merge: merge({ sourceCleanup: { status, deletedSegments: status === "complete" ? 2 : 0, proofFile: `merged-${token}.cleanup.json`, proofSha256: "a".repeat(64), lastError: status === "blocked" ? "사용 중인 원본은 보존했습니다." : null } }) }));
    expect(button("앱에서 다시보기")).toBeEnabled();
    expect(button("외부 플레이어로 열기")).toBeEnabled();
    expect(button("파일 열기")).toBeUndefined();
    expect(container).toHaveTextContent("원본 조각 기록");
    expect(container).toHaveTextContent("채팅 로그와 시간표는 보존됩니다");
    expect(container).not.toHaveTextContent("원본 조각과 채팅 로그는 보존됩니다");
    expect(button("병합 다시 시도")).toBeUndefined();
    if (status === "blocked") {
      expect(container).toHaveTextContent("사용 중인 원본은 보존했습니다");
      await act(async () => button("원본 정리 다시 시도")!.click());
      expect(callbacks.onRetryMerge).toHaveBeenCalledExactlyOnceWith("selected-recording");
    } else expect(button("원본 정리 다시 시도")).toBeUndefined();
    if (status === "complete") expect(container).toHaveTextContent("원본 조각 2개 정리 완료");
  });

  it("opens only the selected native-verified derivative by recording ID and keeps originals folded", async () => {
    await render(recording({ merge: merge() }));
    expect(container).toHaveTextContent("병합 완료");
    expect(container).toHaveTextContent("앱에서 다시보기");
    expect(container).toHaveTextContent("원본 조각과 채팅 로그는 보존됩니다");
    expect(container.querySelector("video,iframe")).toBeNull();
    expect(container.querySelector("details")).not.toHaveAttribute("open");
    expect(button("병합 다시 시도")).toBeUndefined();
    await act(async () => button("앱에서 다시보기")!.click());
    expect(callbacks.onReplay).toHaveBeenCalledExactlyOnceWith("selected-recording");
    expect(callbacks.onOpenMerged).not.toHaveBeenCalled();
    await act(async () => button("외부 플레이어로 열기")!.click());
    expect(callbacks.onOpenMerged).toHaveBeenCalledExactlyOnceWith("selected-recording");
    await act(async () => container.querySelector("summary")!.click());
    expect(container.querySelector("details")).toHaveAttribute("open");
    await act(async () => button("파일 열기")!.click());
    expect(callbacks.onOpenSegment).toHaveBeenCalledExactlyOnceWith("selected-recording", 0);
  });

  it.each(["queued", "merging"] as const)("shows %s without claiming playback or offering a duplicate merge", async (status) => {
    await render(recording({ merge: merge({ status }) }));
    expect(container).toHaveTextContent(status === "queued" ? "병합 대기" : "병합 중");
    expect(button("외부 플레이어로 열기")).toBeUndefined();
    expect(button("병합 다시 시도")).toBeUndefined();
    expect(container.querySelector("details")).not.toHaveAttribute("open");
  });

  it.each(["blocked", "failed"] as const)("retains %s reasons and retries only the selected recording", async (status) => {
    const value = recording({ status: "interrupted", merge: merge({ status, lastError: "도구를 준비한 뒤 다시 시도해 주세요." }) });
    await render(value);
    expect(container).toHaveTextContent("도구를 준비한 뒤 다시 시도해 주세요.");
    expect(container).toHaveTextContent("누락을 복구하지는 않습니다");
    expect(button("외부 플레이어로 열기")).toBeUndefined();
    await act(async () => button("병합 다시 시도")!.click());
    expect(callbacks.onRetryMerge).toHaveBeenCalledExactlyOnceWith(value.id);
    expect(value.status).toBe("interrupted");
    expect(value.segments).toHaveLength(2);
  });

  it("explains end-of-recording merging without starting it while recording is active", async () => {
    await render(recording({ status: "recording", merge: merge() }));
    expect(container).toHaveTextContent("녹화를 끝낸 뒤 확정된 조각");
    expect(button("외부 플레이어로 열기")).toBeUndefined();
    expect(button("병합 다시 시도")).toBeUndefined();
    expect(callbacks.onRetryMerge).not.toHaveBeenCalled();
  });

  it("distinguishes older unmerged records from an archive with no finalized segments", async () => {
    await render(recording());
    expect(container).toHaveTextContent("병합 상태 확인 필요");
    expect(button("병합 다시 시도")).toBeEnabled();
    await render(recording({ segmentCount: 0, segments: [] }));
    expect(container).toHaveTextContent("확정 조각 없음");
    expect(button("병합 다시 시도")).toBeUndefined();
    expect(button("파일 열기")).toBeUndefined();
  });

  it.each([
    { file: "https://example.test/video.webm" }, { file: "../video.webm" },
    { file: `merged-${token}.mp4` }, { timelineFile: `merged-${"b".repeat(32)}.timeline.jsonl` },
    { segmentCount: 1 }, { bytes: 0 }, { bytes: Number.POSITIVE_INFINITY },
    { durationSeconds: Number.NaN }, { durationSeconds: 0 },
  ])("does not expose incomplete or inconsistent complete metadata %j", async (patch) => {
    const value = recording({ merge: merge(patch) });
    expect(hasCompletedMerge(value)).toBe(false);
    await render(value);
    expect(button("외부 플레이어로 열기")).toBeUndefined();
    expect(container).toHaveTextContent("병합 결과 정보를 확인하지 못했습니다");
    expect(callbacks.onOpenMerged).not.toHaveBeenCalled();
  });

  it("accepts the MP4 derivative contract and never leaks filenames in privacy mode", async () => {
    const value = recording({ mimeType: "video/mp4;codecs=avc1.640028,mp4a.40.2", merge: merge({ file: `merged-${token}.mp4` }) });
    expect(hasCompletedMerge(value)).toBe(true);
    await render(value, { privacyMode: true });
    expect(button("외부 플레이어로 열기")).toBeEnabled();
    expect(button("앱에서 다시보기")).toBeDisabled();
    expect(container).not.toHaveTextContent("merged-");
    expect(container).not.toHaveTextContent("segment-");
    expect(container).not.toHaveTextContent("SyntheticOnly");
    expect(container).toHaveTextContent("영상 파일 1");
  });

  it("disables all file actions while a native action is pending", async () => {
    await render(recording({ merge: merge() }), { disabled: true, opening: true });
    expect(button("여는 중…")).toBeDisabled();
    expect(button("파일 열기")).toBeDisabled();
    await act(async () => { button("여는 중…")!.click(); button("파일 열기")!.click(); });
    expect(callbacks.onOpenMerged).not.toHaveBeenCalled();
    expect(callbacks.onOpenSegment).not.toHaveBeenCalled();
    await render(recording({ merge: merge({ status: "failed" }) }), { disabled: true, retrying: true });
    expect(button("요청 중…")).toBeDisabled();
  });
});
