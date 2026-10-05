import type { BrowserRecording } from "../../api/officialBrowser";

const duration = (value: number) => {
  const seconds = Math.max(0, Math.floor(value));
  return `${Math.floor(seconds / 3600).toString().padStart(2, "0")}:${Math.floor(seconds / 60 % 60).toString().padStart(2, "0")}:${(seconds % 60).toString().padStart(2, "0")}`;
};

/** Display gate only. The native open command revalidates the owned file. */
export function hasCompletedMerge(recording: BrowserRecording): boolean {
  if (recording.mediaRemovedAt != null) return false;
  const merge = recording.merge;
  if (recording.status === "recording" || merge?.status !== "complete" ||
      !Number.isSafeInteger(recording.segmentCount) || recording.segmentCount <= 0 || merge.segmentCount !== recording.segmentCount) return false;
  const file = typeof merge.file === "string" ? /^merged-([a-f0-9]{32})\.(webm|mp4)$/.exec(merge.file) : null;
  return !!file && file[2] === (recording.mimeType.startsWith("video/webm") ? "webm" : "mp4") &&
    merge.timelineFile === `merged-${file[1]}.timeline.jsonl` &&
    typeof merge.bytes === "number" && Number.isSafeInteger(merge.bytes) && merge.bytes > 0 &&
    typeof merge.durationSeconds === "number" && Number.isFinite(merge.durationSeconds) && merge.durationSeconds > 0;
}

export function hasReplayableRanges(recording: BrowserRecording): boolean {
  if (recording.mediaRemovedAt != null) return false;
  const ranges = recording.progressive;
  return !!ranges && Number.isSafeInteger(ranges.partCount) && ranges.partCount > 0 &&
    Number.isSafeInteger(ranges.segmentCount) && ranges.segmentCount >= ranges.partCount && ranges.segmentCount <= recording.segmentCount &&
    Number.isFinite(ranges.durationSeconds) && ranges.durationSeconds > 0;
}


type RecordingPlaybackProps = {
  recording: BrowserRecording;
  disabled: boolean;
  privacyMode: boolean;
  retrying: boolean;
  onReplay: (id: string) => void;
  onRetryMerge: (id: string) => void;
};

/** The replay service separately validates the completed local derivative. */
export function RecordingPlayback({ recording, disabled, privacyMode, retrying, onReplay, onRetryMerge }: RecordingPlaybackProps) {
  if (recording.mediaRemovedAt != null) return <p className="official-browser-muted">영상이 삭제된 기록입니다.</p>;
  const completed = hasCompletedMerge(recording);
  const active = recording.status === "recording";
  const merge = recording.merge;
  const ranges = hasReplayableRanges(recording);
  const invalidMerge = !active && merge?.status === "complete" && !completed;
  const retryable = !active && recording.segmentCount > 0 && (!merge || merge.status === "blocked" || merge.status === "failed" || invalidMerge);
  const retryCleanup = completed && merge?.sourceCleanup?.status === "blocked";
  const problems = [...new Set([
    recording.archive?.lastError,
    !active && (merge?.status === "blocked" || merge?.status === "failed") ? merge.lastError || "재생 준비를 완료하지 못했습니다." : null,
    invalidMerge ? "저장 영상을 확인하지 못했습니다." : null,
    recording.progressive?.lastError,
    retryCleanup ? merge?.sourceCleanup?.lastError || "남은 파일을 정리하지 못했습니다. 재생은 가능합니다." : null,
  ].filter((value): value is string => !!value))];
  const canRetry = retryable || retryCleanup || !!recording.archive?.lastError || !!recording.progressive?.lastError;
  return <div className="official-browser-playback">
    <button type="button" className="official-browser-primary" disabled={disabled || privacyMode || !completed && !ranges} title={privacyMode ? "프라이버시 모드를 끄면 재생할 수 있습니다." : undefined} onClick={() => onReplay(recording.id)}>재생</button>
    {!completed ? <p className="official-browser-muted" role="status">{ranges ? `${duration(recording.progressive!.durationSeconds)}까지 재생 가능${active ? " · 녹화 중" : ""}` : !active && !recording.segmentCount ? "저장된 영상이 없습니다." : problems.length ? "재생 준비에 문제가 있습니다." : "재생 준비 중…"}</p> : null}
    {problems.length ? <details className="recording-playback-problems"><summary>문제 확인</summary>{problems.map(problem => <p className="official-browser-error" key={problem}>{privacyMode ? "저장 상태를 확인해 주세요." : problem}</p>)}</details> : null}
    {canRetry ? <button type="button" disabled={disabled || retrying} onClick={() => onRetryMerge(recording.id)}>{retrying ? "요청 중…" : "다시 시도"}</button> : null}
  </div>;
}
