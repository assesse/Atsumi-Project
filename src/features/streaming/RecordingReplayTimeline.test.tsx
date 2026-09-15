import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import type { ReplayTimeline } from "../../api/replay";
import { RecordingReplayTimeline, replayBucketAt, replayMetricPaths } from "./RecordingReplayTimeline";
import { visibleReplayWarnings } from "./RecordingReplayModel";

const timeline = (values: (number | null)[]): ReplayTimeline => ({
  bucketSeconds: 2, indexState: "ready", viewerMetricStatus: "not_recorded",
  buckets: values.map((value, index) => ({ startSeconds: index * 2, chatCount: index + 2, uniqueSenderCount: value, viewerCount: value })),
});
describe("replay activity overlay", () => {
  it("uses continuous bounded curves for a four-minute recording, not wide per-bucket buttons", () => {
    const value = timeline(Array.from({ length: 120 }, (_, i) => i % 13));
    const paths = replayMetricPaths(value, 240, "uniqueSenderCount");
    expect(paths).toHaveLength(1); expect(paths[0]).toContain(" C");
    expect(paths[0]).toContain("L1000.00,32");
    expect(paths[0]).not.toMatch(/NaN|Infinity/);
    expect(replayMetricPaths(value, 0, "viewerCount")).toEqual([]);
    expect(replayMetricPaths({ ...value, bucketSeconds: NaN }, 240, "viewerCount")).toEqual([]);
  });
  it("never joins a missing observation with an invented zero or interpolates through a gap", () => {
    const value = timeline([8, 6, null, 0, 4]);
    const paths = replayMetricPaths(value, 10, "viewerCount");
    expect(paths).toHaveLength(2);
    expect(paths[0]).toContain("L400.00,32"); expect(paths[1]).toMatch(/^M600.00,32.00/);
    expect(replayMetricPaths(timeline([null, null]), 4, "viewerCount")).toEqual([]);
    expect(replayBucketAt(value, 4)?.viewerCount).toBeNull();
    expect(replayBucketAt(value, 10)).toBeUndefined();
  });
  it("retains the end of a long bounded timeline instead of dropping later buckets", () => {
    const value = timeline(Array.from({ length: 2000 }, (_, i) => i % 11));
    expect(replayMetricPaths(value, 4000, "viewerCount")[0]).toContain("L1000.00,32");
  });
  it("does not draw partial viewer coverage as a fully measured continuous interval", () => {
    const value = timeline([5, 8, 6]); value.buckets[1]!.viewerCoverageSeconds = .5;
    const paths = replayMetricPaths(value, 6, "viewerCount");
    expect(paths).toHaveLength(2);
    expect(paths[0]).toContain("L333.33,32"); expect(paths[1]).toMatch(/^M666.67,/);
  });
  it("keeps the original bucket counts for precise inspection and preserves zero measurements", () => {
    const value = timeline([0, 400]);
    const before = JSON.stringify(value);
    replayMetricPaths(value, 4, "viewerCount");
    expect(JSON.stringify(value)).toBe(before);
    expect(replayBucketAt(value, 1.999)?.viewerCount).toBe(0);
    expect(replayBucketAt(value, 2)?.viewerCount).toBe(400);
  });
  it("removes only the requested correction advice, not corruption or missing-chat warnings", () => {
    expect(visibleReplayWarnings(["수신 시각 기준의 근사 동기화가 포함됩니다. 채팅 보정값으로 조정할 수 있습니다.", "채팅 4개가 손상되어 제외됐습니다.", "영상 일부 누락 가능"])).toEqual(["채팅 4개가 손상되어 제외됐습니다.", "영상 일부 누락 가능"]);
  });
  it("renders one seek slider and no bucket buttons, with accessible measured/missing counts", async () => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    const element = document.createElement("div"); document.body.append(element);
    const root = createRoot(element);
    const value = timeline([7, 9]); value.buckets[0]!.viewerCount = null;
    try {
      await act(async () => root.render(<RecordingReplayTimeline timeline={value} time={0} duration={4} onSeek={vi.fn()} />));
      expect(element.querySelectorAll('input[type="range"]')).toHaveLength(1);
      expect(element.querySelector('button')).toBeNull();
      expect(element).toHaveTextContent("채팅 참여자 7명"); expect(element).toHaveTextContent("시청자 기록 없음");
      expect(element.querySelectorAll('.replay-chat-curve')).toHaveLength(1);
      await act(async () => root.render(<RecordingReplayTimeline timeline={timeline([null, null])} time={0} duration={4} onSeek={vi.fn()} />));
      expect(element.querySelector('svg')).toHaveAttribute('data-chat-metric', 'messages');
      expect(element).toHaveTextContent('곡선: 채팅 수');
      expect(element.querySelector('.replay-viewer-curve')).toBeNull();
    } finally { await act(async () => root.unmount()); element.remove(); vi.unstubAllGlobals(); }
  });
});
