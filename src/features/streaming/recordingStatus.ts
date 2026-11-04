import type { BrowserRecording } from "../../api/officialBrowser";
import { hasCompletedMerge } from "./RecordingPlayback";

export function recordingWorkLabel(recording: BrowserRecording): string | null {
  const work = recording.workProgress;
  if (!work) return null;
  const labels: Record<string, string> = {
    preparing: "처리 준비", source_verification: "원본 확인", archive_verification: "저장 파일 확인",
    archiving: "파일 이동", merging: "병합", verifying: "재생 검증", final_verification: "최종 파일 확인", finalizing: "검증 결과 저장",
  };
  const phase = labels[work.phase] ?? "후처리";
  const measured = Number.isFinite(work.completed) && work.completed >= 0 && work.total != null && Number.isFinite(work.total) && work.total > 0;
  return measured ? `${phase} ${Math.min(100, Math.floor(work.completed / work.total! * 100))}%` : `${phase} 중`;
}

export function recordingStatus(recording: BrowserRecording): string {
  if (recording.mediaRemovedAt != null) return "영상 없음";
  // Cached, settled history defers disk validation until playback. That is not
  // a running task: retain its saved outcome without waking every archive disk.
  if (recording.summaryPending) return "기록 불러오는 중";
  if (recording.status === "recording") return "녹화 중";
  if (recording.ending?.reason === "checking") return "종료 확인 중";
  const progress = recordingWorkLabel(recording);
  if (progress) return progress;
  if (recording.archive?.status === "copying") return "파일 이동 중";
  if (recording.archive?.status === "cleanup_pending") return "저장 마무리 대기";
  if (recording.merge?.status === "merging") return "병합·검증 중";
  if (recording.merge?.status === "failed") return "처리 실패";
  if (recording.archive?.status === "blocked" || recording.archive?.lastError) return "이동 재시도 대기";
  if (recording.merge?.sourceCleanup?.status === "blocked") return "원본 정리 확인 필요";
  if (recording.merge?.sourceCleanup?.status === "pending") return "원본 정리 대기";
  if (recording.archive?.status === "pending" && hasCompletedMerge(recording)) return "파일 이동 대기";
  if (hasCompletedMerge(recording)) return recording.status === "failed" || recording.status === "interrupted" || recording.partial ? "저장된 구간 재생 가능" : "정상";
  if (recording.merge?.status === "blocked") return "처리 대기";
  if (recording.merge?.status === "queued") return "병합 대기";
  if (recording.status === "failed" || recording.partial) return "저장 오류";
  if (recording.status === "interrupted") return "중단됨";
  if (emptyRecordingAttempt(recording)) return "영상 없음";
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
