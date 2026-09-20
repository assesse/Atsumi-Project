import type { BrowserRecording } from "../../api/officialBrowser";

export function recordingStatus(recording: BrowserRecording): string {
  if (recording.status === "recording") return "녹화 중";
  if (recording.status === "failed" || recording.partial) return "저장 오류 · 확인 필요";
  const reasons: Record<string, string> = {
    broadcast_ended: "방송 종료 · 저장 완료", broadcast_changed: "다음 방송으로 전환 · 저장 완료",
    user_stopped: "직접 중지 · 저장 완료", app_shutdown: "앱 종료 · 저장 완료",
    checking: "방송 종료 확인 중", end_unconfirmed: "종료 원인 확인 불가",
    timeline_discontinuity: "수신 영상 시간축 불연속", start_failed: "녹화 시작 실패",
    storage_error: "저장 오류 · 확인 필요", app_interrupted: "앱·시청 창 중단", source_error: "영상 수신 중단",
  };
  return reasons[recording.ending?.reason ?? ""] ?? ({ stopped: "저장 완료", interrupted: "중단됨 · 종료 원인 미확인", failed: "실패" }[recording.status]);
}
export const emptyRecordingAttempt = (r: BrowserRecording) => r.status !== "recording" && r.segmentCount === 0 && r.bytesWritten === 0 && !r.partial && !r.progressive?.partCount && !r.merge?.bytes;
