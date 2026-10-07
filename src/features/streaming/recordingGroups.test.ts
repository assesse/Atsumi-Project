import { describe, expect, it } from "vitest";
import type { BrowserRecording } from "../../api/officialBrowser";
import { groupRecordings } from "./recordingGroups";
import { recordingEndDetail, recordingStatus } from "./recordingStatus";

const item = (id: string, patch: Partial<BrowserRecording> = {}): BrowserRecording => ({
  id, channelId: "channel", title: "같은 제목", startedAt: 1, updatedAt: 2, status: "stopped", mimeType: "video/mp4",
  outputDir: "", segmentCount: 1, bytesWritten: 100, durationSeconds: 10, lastError: null, segments: [], ...patch,
});
const merged = item("complete", { merge: { status: "complete", segmentCount: 1, updatedAt: 2,
  file: `merged-${"a".repeat(32)}.mp4`, timelineFile: `merged-${"a".repeat(32)}.timeline.jsonl`, bytes: 100, durationSeconds: 10 } });

describe("broadcast recording groups", () => {
  it("groups stopped/restarted/active files by channel and broadcast key, even when the title changes", () => {
    const records = [item("new", { broadcastKey: "id:42", title: "다른 제목", startedAt: 3, status: "recording" }),
      item("other", { broadcastKey: "id:43", startedAt: 2 }), item("old", { broadcastKey: "id:42" })];
    const groups = groupRecordings(records);
    expect(groups).toHaveLength(2);
    expect(groups[0]!.recordings.map(r => r.id)).toEqual(["old", "new"]);
    expect(groups[0]!.latest.id).toBe("new"); expect(groups[0]!.active).toBe(true);
    expect(records.map(r => r.id)).toEqual(["new", "other", "old"]);
  });
  it("never combines unknown broadcasts, titles, or different channels", () => {
    const groups = groupRecordings([item("a"), item("b", { broadcastKey: " " }), item("c", { broadcastKey: "id:42" }),
      item("d", { channelId: "other", broadcastKey: "id:42" })]);
    expect(groups).toHaveLength(4);
  });
});

describe("short recording status", () => {
  it.each(["app_shutdown", "user_stopped", "broadcast_ended", "broadcast_changed"])("labels a completed %s file concisely while retaining its reason", reason => {
    const recording = { ...merged, ending: { reason, trigger: reason, stoppedAt: 2, confirmedAt: null } };
    expect(recordingStatus(recording)).toBe("정상 파일");
    expect(recordingEndDetail(recording)).not.toBe("");
    expect(recordingStatus({ ...recording, storageCheckPending: true })).toBe("확인 중");
  });
  it("does not mistake playable partial broadcasts, unknown summaries, or invalid merge metadata for normal files", () => {
    expect(recordingStatus({ ...merged, status: "interrupted" })).toBe("중단된 파일");
    expect(recordingStatus({ ...merged, summaryPending: true })).toBe("확인 중");
    expect(recordingStatus({ ...merged, partial: { index: 2, file: "partial", bytes: 1 } })).toBe("저장 오류");
    expect(recordingStatus({ ...merged, merge: { ...merged.merge!, segmentCount: 2 } })).toBe("확인 필요");
    expect(recordingStatus({ ...merged, mediaRemovedAt: 1 })).toBe("영상 없음");
    expect(recordingStatus({ ...merged, status: "recording", storageCheckPending: false })).toBe("녹화 중");
  });
});
