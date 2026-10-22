import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { BrowserMerge, BrowserRecording } from "../../api/officialBrowser";
import { hasCompletedMerge, RecordingPlayback } from "./RecordingPlayback";

const token = "a".repeat(32);
const merge = (patch: Partial<BrowserMerge> = {}): BrowserMerge => ({ status: "complete", segmentCount: 2, updatedAt: 1, file: `merged-${token}.webm`, timelineFile: `merged-${token}.timeline.jsonl`, bytes: 2000, durationSeconds: 30, ...patch });
const recording = (patch: Partial<BrowserRecording> = {}): BrowserRecording => ({ id: "selected-recording", channelId: "b".repeat(32), title: "저장한 방송", startedAt: 0, updatedAt: 1, status: "stopped", mimeType: "video/webm", outputDir: "C:/SyntheticOnly", segmentCount: 2, bytesWritten: 2000, durationSeconds: 30, lastError: null, segments: [{ index: 0, file: "segment-000000000000.webm", bytes: 1000, durationSeconds: 15 }], ...patch });
let root: Root, container: HTMLDivElement;
const callbacks = { onReplay: vi.fn(), onRetryMerge: vi.fn() };
const render = async (value: BrowserRecording, patch: { disabled?: boolean; privacyMode?: boolean; retrying?: boolean } = {}) => {
  await act(async () => root.render(<RecordingPlayback recording={value} disabled={false} privacyMode={false} retrying={false} {...callbacks} {...patch} />));
};
const button = (label: string) => [...container.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === label);
beforeEach(() => { vi.clearAllMocks(); vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); container = document.createElement("div"); document.body.append(container); root = createRoot(container); });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.unstubAllGlobals(); });

describe("compact recording playback", () => {
  it("shows only Play for a healthy recording without technical metadata or external actions", async () => {
    await render(recording({ merge: merge(), archive: { status: "complete", sourceDir: "C:/private", destinationRoot: "D:/archive", retryAt: 0, lastError: null } }));
    expect(container.textContent).toBe("재생");
    expect(container.querySelector("video,iframe,details")).toBeNull();
    await act(async () => button("재생")!.click());
    expect(callbacks.onReplay).toHaveBeenCalledExactlyOnceWith("selected-recording");
    expect(callbacks.onRetryMerge).not.toHaveBeenCalled();
  });
  it("replays saved ranges while recording without requesting another capture or final merge", async () => {
    await render(recording({ status: "recording", progressive: { partCount: 1, segmentCount: 1, durationSeconds: 15, lastError: null } }));
    expect(container).toHaveTextContent("00:00:15까지 재생 가능 · 녹화 중");
    await act(async () => button("재생")!.click());
    expect(callbacks.onReplay).toHaveBeenCalledExactlyOnceWith("selected-recording");
    expect(callbacks.onRetryMerge).not.toHaveBeenCalled();
    await render(recording({ status: "recording", progressive: { partCount: 1, segmentCount: 3, durationSeconds: 15, lastError: "디스크 확인 필요" } }));
    expect(button("재생")).toBeDisabled();
    expect(container.querySelector("details")).not.toHaveAttribute("open");
    await act(async () => button("다시 시도")!.click());
    expect(callbacks.onRetryMerge).toHaveBeenCalledExactlyOnceWith("selected-recording");
  });
  it.each(["pending", "complete", "blocked"] as const)("retains playback during %s cleanup without original-file controls", async status => {
    await render(recording({ merge: merge({ sourceCleanup: { status, deletedSegments: 0, proofFile: `merged-${token}.cleanup.json`, proofSha256: "a".repeat(64), lastError: status === "blocked" ? "사용 중인 파일입니다." : null } }) }));
    expect(button("재생")).toBeEnabled();
    expect(container).not.toHaveTextContent("원본 조각");
    if (status === "blocked") {
      expect(container).toHaveTextContent("사용 중인 파일입니다.");
      await act(async () => button("다시 시도")!.click());
      expect(callbacks.onRetryMerge).toHaveBeenCalledExactlyOnceWith("selected-recording");
    } else expect(button("다시 시도")).toBeUndefined();
  });
  it("retains archive errors and retry, but never exposes paths in privacy mode", async () => {
    const value = recording({ merge: merge(), archive: { status: "blocked", sourceDir: "C:/private-ssd", destinationRoot: "D:/private-archive", retryAt: 1, lastError: "C:/private-ssd 드라이브 확인 필요" } });
    await render(value);
    expect(button("재생")).toBeEnabled();
    expect(container.querySelector("details")).not.toHaveAttribute("open");
    await act(async () => button("다시 시도")!.click());
    expect(callbacks.onRetryMerge).toHaveBeenCalledExactlyOnceWith(value.id);
    await render(value, { privacyMode: true });
    expect(button("재생")).toBeDisabled();
    expect(container.innerHTML).not.toContain("private-ssd");
    expect(container.innerHTML).not.toContain("private-archive");
  });
  it.each(["queued", "merging"] as const)("does not offer a duplicate retry during %s", async status => {
    await render(recording({ merge: merge({ status }) }));
    expect(container).toHaveTextContent("재생 준비 중…");
    expect(button("재생")).toBeDisabled();
    expect(button("다시 시도")).toBeUndefined();
  });
  it.each(["blocked", "failed"] as const)("preserves %s reasons and retries only the selected recording", async status => {
    const value = recording({ status: "interrupted", merge: merge({ status, lastError: "저장 공간을 확인해 주세요." }) });
    await render(value);
    expect(container).toHaveTextContent("저장 공간을 확인해 주세요.");
    expect(container.querySelector("details")).not.toHaveAttribute("open");
    await act(async () => button("다시 시도")!.click());
    expect(callbacks.onRetryMerge).toHaveBeenCalledExactlyOnceWith(value.id);
    expect(value.status).toBe("interrupted");
  });
  it("waits for live data without arming a final merge; retries older records but not empty attempts", async () => {
    await render(recording({ status: "recording", merge: merge() }));
    expect(button("재생")).toBeDisabled();
    expect(button("다시 시도")).toBeUndefined();
    expect(callbacks.onRetryMerge).not.toHaveBeenCalled();
    await render(recording());
    expect(button("다시 시도")).toBeEnabled();
    await render(recording({ segmentCount: 0, segments: [] }));
    expect(container).toHaveTextContent("저장된 영상이 없습니다.");
    expect(button("재생")).toBeDisabled();
    expect(button("다시 시도")).toBeUndefined();
  });
  it.each([
    { file: "https://example.test/video.webm" }, { file: "../video.webm" },
    { file: `merged-${token}.mp4` }, { timelineFile: `merged-${"b".repeat(32)}.timeline.jsonl` },
    { segmentCount: 1 }, { bytes: 0 }, { bytes: Number.POSITIVE_INFINITY },
    { durationSeconds: Number.NaN }, { durationSeconds: 0 },
  ])("rejects inconsistent completed metadata %j", async patch => {
    const value = recording({ merge: merge(patch) });
    expect(hasCompletedMerge(value)).toBe(false);
    await render(value);
    expect(button("재생")).toBeDisabled();
    expect(container).toHaveTextContent("저장 영상을 확인하지 못했습니다.");
  });
  it("accepts MP4 and respects privacy and pending actions", async () => {
    const value = recording({ mimeType: "video/mp4;codecs=avc1.640028,mp4a.40.2", merge: merge({ file: `merged-${token}.mp4` }) });
    expect(hasCompletedMerge(value)).toBe(true);
    await render(value, { privacyMode: true });
    expect(button("재생")).toBeDisabled();
    expect(container).not.toHaveTextContent("merged-");
    await render(value, { disabled: true });
    await act(async () => button("재생")!.click());
    expect(callbacks.onReplay).not.toHaveBeenCalled();
    await render(recording({ merge: merge({ status: "failed" }) }), { retrying: true });
    expect(button("요청 중…")).toBeDisabled();
  });
});
