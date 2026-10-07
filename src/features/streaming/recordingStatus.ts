import type { BrowserRecording } from "../../api/officialBrowser";
import { hasCompletedMerge } from "./RecordingPlayback";

export function recordingStatus(recording: BrowserRecording): string {
  if (recording.mediaRemovedAt != null) return "영상 없음";
  if (recording.summaryPending || recording.storageCheckPending) return "확인 중";
  if (recording.status === "recording") return "녹화 중";
  if (recording.status === "failed" || recording.partial) return "저장 오류";
  if (recording.ending?.reason === "checking") return "종료 확인 중";
  if (recording.status === "interrupted") return "중단된 파일";
  if (emptyRecordingAttempt(recording)) return "영상 없음";
  if (recording.merge?.status === "failed") return "처리 실패";
  if (recording.merge?.status === "blocked") return "처리 대기";
  if (hasCompletedMerge(recording)) return "정상 파일";
  if (recording.merge?.status === "complete") return "확인 필요";
  return "재생 준비 중";
}

/** Keep the end reason available on demand, not repeated beside every file. */
export function recordingEndDetail(recording: BrowserRecording): string {
  const reasons: Record<string, string> = {
    broadcast_ended: "방송 종료", broadcast_changed: "다음 방송으로 전환",
    user_stopped: "직접 중지", app_shutdown: "앱 종료",
    checking: "방송 종료 확인 중", end_unconfirmed: "종료 원인 미확인",
    timeline_discontinuity: "영상 시간축 불연속", start_failed: "녹화 시작 실패",
    format_changed: "영상 형식 변경 · 파일 분리",
    storage_error: "저장 오류", app_interrupted: "앱·시청 창 중단", source_error: "영상 수신 중단",
  };
  return reasons[recording.ending?.reason ?? ""] ?? (recording.status === "interrupted" ? "종료 원인 미확인" : "");
}
export const emptyRecordingAttempt = (r: BrowserRecording) => r.mediaRemovedAt == null && !r.summaryPending && r.status !== "recording" && r.segmentCount === 0 && r.bytesWritten === 0 && !r.partial && !r.progressive?.partCount && !r.merge?.bytes;
