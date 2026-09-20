import type { BrowserRecording } from "../../api/officialBrowser";

const duration = (value: number) => {
  const seconds = Math.max(0, Math.floor(value));
  return `${Math.floor(seconds / 3600).toString().padStart(2, "0")}:${Math.floor(seconds / 60 % 60).toString().padStart(2, "0")}:${(seconds % 60).toString().padStart(2, "0")}`;
};
const bytes = (value: number) => value >= 1024 ** 3 ? `${(value / 1024 ** 3).toFixed(2)} GiB` : `${(value / 1024 ** 2).toFixed(1)} MiB`;

/** Display gate only. The native open command revalidates the owned file. */
export function hasCompletedMerge(recording: BrowserRecording): boolean {
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
  const ranges = recording.progressive;
  return !!ranges && Number.isSafeInteger(ranges.partCount) && ranges.partCount > 0 &&
    Number.isSafeInteger(ranges.segmentCount) && ranges.segmentCount >= ranges.partCount && ranges.segmentCount <= recording.segmentCount &&
    Number.isFinite(ranges.durationSeconds) && ranges.durationSeconds > 0;
}

export function recordingMergeLabel(recording: BrowserRecording): string {
  if (recording.progressive && !hasCompletedMerge(recording) && (recording.status === "recording" || !recording.merge || recording.merge.status === "queued")) return recording.progressive.lastError ? "구간 병합 확인 필요" : `구간 병합 ${recording.progressive.partCount}개 완료`;
  if (recording.status === "recording") return "녹화 종료 후 병합";
  if (hasCompletedMerge(recording)) return "병합 완료";
  if (recording.segmentCount === 0) return "확정 조각 없음";
  switch (recording.merge?.status) {
    case "queued": return "병합 대기";
    case "merging": return "병합 중";
    case "blocked": return "병합 보류";
    case "failed": return "병합 실패";
    default: return "병합 상태 확인 필요";
  }
}

type RecordingPlaybackProps = {
  recording: BrowserRecording;
  disabled: boolean;
  privacyMode: boolean;
  retrying: boolean;
  opening: boolean;
  onOpenMerged: (id: string) => void;
  onReplay: (id: string) => void;
  onRetryMerge: (id: string) => void;
  onOpenSegment: (id: string, index: number) => void;
};

/** The replay service separately validates the completed local derivative. */
export function RecordingPlayback({ recording, disabled, privacyMode, retrying, opening, onOpenMerged, onReplay, onRetryMerge, onOpenSegment }: RecordingPlaybackProps) {
  const completed = hasCompletedMerge(recording);
  const active = recording.status === "recording";
  const merge = recording.merge;
  const retryable = !active && recording.segmentCount > 0 && (!merge || merge.status === "blocked" || merge.status === "failed");
  const cleanup = merge?.sourceCleanup;
  const retryCleanup = completed && cleanup?.status === "blocked";
  const failed = !active && (merge?.status === "blocked" || merge?.status === "failed" || merge?.status === "complete" && !completed);
  return <div className="official-browser-playback">
    <p className={failed ? "official-browser-warning" : "official-browser-note"} role="status">
      <strong>{recordingMergeLabel(recording)}</strong>
      {active ? recording.progressive ? " · 약 3분 또는 48 MiB마다 확정된 구간을 병합합니다. 완료 구간부터 재생할 수 있습니다." : " · 녹화를 끝낸 뒤 확정된 조각을 하나의 파일로 병합합니다." : null}
      {!active && (merge?.status === "queued" || merge?.status === "merging") ? " · 병합·전체 재생 검증 중에는 원본 조각을 보존합니다. 긴 영상은 시간이 더 걸릴 수 있습니다." : null}
      {completed ? ` · ${duration(merge!.durationSeconds!)} · ${bytes(merge!.bytes!)}` : null}
    </p>
    {failed && merge?.lastError ? <p className="official-browser-error" role="alert">{merge.lastError}</p> : null}
    {!completed && !active && merge?.status === "complete" ? <p className="official-browser-error" role="alert">병합 결과 정보를 확인하지 못했습니다. 원본 조각을 이용해 주세요.</p> : null}
    {!completed && hasReplayableRanges(recording) ? <>
      <button type="button" className="official-browser-primary" disabled={disabled || privacyMode} onClick={() => onReplay(recording.id)}>앱에서 다시보기</button>
      <p className="official-browser-muted">{duration(recording.progressive!.durationSeconds)}까지 준비됐습니다. {active ? "구간은 자동으로 이어서 재생하며 영상·채팅 녹화는 계속됩니다." : "전체 파일 병합을 기다리지 않고 완료 구간부터 볼 수 있습니다."}</p>
    </> : null}
    {recording.progressive?.lastError ? <p className="official-browser-error" role="alert">{recording.progressive.lastError}</p> : null}
    {recording.progressive?.lastError && !retryable ? <button type="button" disabled={disabled || retrying} onClick={() => onRetryMerge(recording.id)}>{retrying ? "요청 중…" : "구간 병합 다시 시도"}</button> : null}
    {completed ? <>
      <button type="button" className="official-browser-primary" disabled={disabled || privacyMode} onClick={() => onReplay(recording.id)}>앱에서 다시보기</button>{" "}
      <button type="button" disabled={disabled} onClick={() => onOpenMerged(recording.id)}>{opening ? "여는 중…" : "외부 플레이어로 열기"}</button>
      <p className="official-browser-muted">저장 영상과 채팅을 앱에서 함께 봅니다. 외부 플레이어는 영상만 엽니다. {cleanup ? "채팅 로그와 시간표는 보존됩니다." : "원본 조각과 채팅 로그는 보존됩니다."}</p>
    </> : null}
    {completed && cleanup ? <p className={retryCleanup ? "official-browser-warning" : "official-browser-muted"} role="status">
      {cleanup.status === "complete" ? `원본 조각 ${cleanup.deletedSegments}개 정리 완료 · 병합본으로 재생합니다.` : cleanup.status === "blocked" ? (cleanup.lastError || "병합본은 저장되었습니다. 남은 원본 조각의 정리를 다시 시도할 수 있습니다.") : "병합본 검증 완료 · 원본 조각을 정리 중입니다. 병합본으로 재생할 수 있습니다."}
    </p> : null}
    {retryCleanup ? <button type="button" disabled={disabled} onClick={() => onRetryMerge(recording.id)}>{retrying ? "요청 중…" : "원본 정리 다시 시도"}</button> : null}
    {retryable ? <button type="button" disabled={disabled} onClick={() => onRetryMerge(recording.id)}>{retrying ? "요청 중…" : "병합 다시 시도"}</button> : null}
    {recording.status === "interrupted" || recording.status === "failed" ? <p className="official-browser-muted">중단 전 확정된 분량만 대상입니다. 병합 성공이 녹화·채팅의 누락을 복구하지는 않습니다.</p> : null}
    <details className="official-browser-originals">
      <summary>{cleanup ? "원본 조각 기록" : "원본 조각"} <span>{recording.segmentCount}개</span></summary>
      {cleanup ? <p className="official-browser-muted">이 목록은 녹화 이력입니다. 원본 정리를 시작한 녹화는 병합본으로 재생합니다.</p> : null}
      {recording.segmentCount > recording.segments.length ? <p className="official-browser-muted">최신 {recording.segments.length}개 파일을 표시합니다. 전체 기록은 녹화 폴더에서 확인할 수 있습니다.</p> : null}
      {recording.segments.length ? <ol className="official-browser-segments">{recording.segments.map((segment) => <li key={segment.index}>
        <div><strong>{privacyMode ? `영상 파일 ${segment.index + 1}` : segment.file}</strong><span>{duration(segment.durationSeconds)} · {bytes(segment.bytes)}</span></div>
        {!cleanup ? <button type="button" disabled={disabled} onClick={() => onOpenSegment(recording.id, segment.index)}>파일 열기</button> : null}
      </li>)}</ol> : <p className="official-browser-muted">아직 마무리된 영상 파일이 없습니다.</p>}
    </details>
  </div>;
}
