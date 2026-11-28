import type { CloudJobState, CloudRecordingJob, CloudSourceCoverage, CloudSourcePart, CloudSourceRef } from '../../api/cloudRecording';

export type CloudReplayExpectedPart = CloudSourcePart;

export const cloudStateLabel: Record<CloudJobState, string> = {
  queued: '업로드 대기', validating: '원본 확인 중', uploading: '업로드 중', paused: '일시정지',
  waiting_auth: '계정 연결 필요', retry_wait: '재시도 대기', processing: 'YouTube 처리 중',
  ready: '처리 완료 · 재생 확인 필요', failed: '확인 필요',
};
export function cloudVisibilityLabel(job: Pick<CloudRecordingJob, 'visibility'>): string {
  // Catalogs written before visibility was persisted contain private uploads.
  if (job.visibility === undefined || job.visibility === 'private') return '비공개';
  return job.visibility === 'unlisted' ? '일부 공개 · 링크로 시청' : '공개 범위 확인 필요';
}
export function sameCloudRecording(a: CloudSourceRef, b: CloudSourceRef): boolean {
  return a.provider === b.provider && a.recordingId === b.recordingId;
}
export function sameCloudPart(a: CloudSourceRef, b: CloudSourceRef): boolean {
  return sameCloudRecording(a, b) && (a.partIndex ?? null) === (b.partIndex ?? null)
    && (a.planId ?? null) === (b.planId ?? null) && (a.unitId ?? null) === (b.unitId ?? null);
}
export function cloudJobCanPause(job: CloudRecordingJob): boolean {
  return ['queued', 'validating', 'uploading', 'retry_wait', 'processing'].includes(job.state);
}
export function cloudJobCanResume(job: CloudRecordingJob): boolean {
  return job.state === 'paused' || job.state === 'waiting_auth' || job.state === 'failed' && !!job.error?.retryable;
}
export function cloudJobPlayable(job: CloudRecordingJob): boolean {
  return job.state === 'ready' && typeof job.videoId === 'string' && /^[A-Za-z0-9_-]{11}$/.test(job.videoId);
}
export function cloudProgress(job: CloudRecordingJob): number | undefined {
  if (!Number.isFinite(job.totalBytes) || job.totalBytes <= 0 || !Number.isFinite(job.uploadedBytes) || job.uploadedBytes < 0) return undefined;
  return Math.min(100, job.uploadedBytes / job.totalBytes * 100);
}
export function cloudDuration(seconds: number): string {
  const s = Math.max(0, Math.floor(Number.isFinite(seconds) ? seconds : 0));
  return [Math.floor(s / 3600), Math.floor(s / 60) % 60, s % 60].map(n => String(n).padStart(2, '0')).join(':');
}
export function cloudSize(bytes: number): string {
  return bytes >= 1e9 ? `${(bytes / 1e9).toFixed(2)} GB` : `${(Math.max(0, bytes) / 1e6).toFixed(1)} MB`;
}

const RANGE_EPSILON = .005;
export type CloudReplayUnit = { source: CloudSourceRef; start: number; end: number; index: number; job: CloudRecordingJob | null; issue: string | null };
export type CloudReplayTimeline = { units: CloudReplayUnit[]; issue: string | null; end: number; tailIssue: string | null };
export type CloudReplayTarget = { kind: 'ready'; unit: CloudReplayUnit; job: CloudRecordingJob; seconds: number } | { kind: 'blocked'; reason: string } | { kind: 'end' };
const validRange = (start: number, duration: number) => Number.isFinite(start) && start >= 0 && Number.isFinite(duration) && duration > 0 && Number.isFinite(start + duration);

/** Bind one immutable archive, never a mixture of old parts and a newer plan. */
export function cloudReplayTimeline(jobs: CloudRecordingJob[], initial: CloudRecordingJob, expectedParts?: CloudReplayExpectedPart[], coverage?: CloudSourceCoverage): CloudReplayTimeline {
  const identity = initial.source;
  const invalid = (issue: string): CloudReplayTimeline => ({ units: [], issue, end: 0, tailIssue: null });
  if (!!identity.planId !== !!identity.unitId) return invalid('보관 구간 정보를 확인하지 못했습니다.');
  const planned = !!identity.planId, fragmented = initial.source.partIndex != null;
  const cohort = jobs.filter(job => {
    if (!sameCloudRecording(job.source, initial.source)) return false;
    if (initial.replayCohortId && job.replayCohortId !== initial.replayCohortId) return false;
    const candidate = job.source;
    return planned ? candidate.planId === identity.planId
      : fragmented ? !candidate.planId && !candidate.unitId && candidate.partIndex != null : job.id === initial.id;
  });
  if (!expectedParts?.length && (planned || fragmented)) return invalid('전체 보관 구간을 확인한 뒤 다시 재생해 주세요.');
  const parts: CloudReplayExpectedPart[] = expectedParts?.length ? expectedParts : [{ partIndex: null, durationSeconds: initial.durationSeconds, mediaOffsetSeconds: initial.mediaOffsetSeconds, bytes: initial.totalBytes }];
  const keys = new Set<string>();
  const units: CloudReplayUnit[] = [];
  for (const [index, part] of parts.entries()) {
    if (!validRange(part.mediaOffsetSeconds, part.durationSeconds)) return invalid('보관 구간의 길이와 위치를 확인하지 못했습니다.');
    const supplied = part.source;
    const planId = supplied?.planId, unitId = supplied?.unitId;
    if (supplied && !sameCloudRecording(supplied, initial.source)) return invalid('서로 다른 녹화의 보관 구간이 포함되어 있습니다.');
    if (planned && (planId !== identity.planId || !unitId || supplied?.partIndex != null || part.partIndex != null || !Number.isSafeInteger(part.unitIndex) || part.unitIndex! < 0) || !planned && (planId || unitId)
      || fragmented && part.partIndex == null || !planned && !fragmented && parts.length !== 1) return invalid('서로 다른 보관 구간을 함께 재생할 수 없습니다.');
    const source: CloudSourceRef = { provider: initial.source.provider, recordingId: initial.source.recordingId,
      partIndex: planned ? null : supplied?.partIndex ?? part.partIndex, ...(planned ? { planId, unitId } : {}) };
    const key = JSON.stringify([source.partIndex ?? null, planId ?? null, unitId ?? null]);
    if (keys.has(key)) return invalid('중복된 보관 구간이 있습니다.');
    keys.add(key);
    const matches = cohort.filter(job => sameCloudPart(job.source, source));
    const job = matches.length === 1 ? matches[0]! : null;
    let issue = matches.length > 1 ? '이 구간의 보관 영상이 중복되어 자동으로 선택할 수 없습니다.' : !job ? '이 구간은 아직 업로드되지 않았습니다.' : null;
    if (job && cloudJobPlayable(job) && (!validRange(job.mediaOffsetSeconds, job.durationSeconds)
      || Math.abs(job.mediaOffsetSeconds - part.mediaOffsetSeconds) > RANGE_EPSILON || Math.abs(job.durationSeconds - part.durationSeconds) > RANGE_EPSILON)) issue = '보관 영상과 원본 구간의 시간 정보가 일치하지 않습니다.';
    if (job && !cloudJobPlayable(job)) issue = job.error?.message || `${cloudStateLabel[job.state]} · 이 구간이 준비되면 이어서 재생합니다.`;
    units.push({ source, start: part.mediaOffsetSeconds, end: part.mediaOffsetSeconds + part.durationSeconds, index: part.unitIndex ?? index, job, issue });
  }
  units.sort((a, b) => a.start - b.start || a.index - b.index);
  if (planned && (units[0]!.start > RANGE_EPSILON || units.some((unit, index) => unit.index !== index))) return invalid('보관 계획의 시작 구간이나 순서가 누락되었습니다.');
  for (let index = 1; index < units.length; index++) {
    if (units[index]!.start < units[index - 1]!.end - RANGE_EPSILON) return invalid('보관 구간이 겹쳐 재생 순서를 확인하지 못했습니다.');
  }
  if (!units.some(unit => unit.job?.id === initial.id)) return invalid('선택한 영상이 현재 보관 구간에 없습니다.');
  const tailIssue = coverage?.state === 'incomplete' ? coverage.reason || '녹화 끝부분의 보관 구간이 완전하지 않습니다.'
    : !coverage && (planned || fragmented) ? '녹화 전체 구간의 보관 여부를 확인하지 못했습니다.' : null;
  return { units, issue: null, end: units.at(-1)?.end ?? 0, tailIssue };
}
function unitTarget(unit: CloudReplayUnit, seconds: number): CloudReplayTarget {
  if (unit.issue) return { kind: 'blocked', reason: unit.issue };
  if (!unit.job || !cloudJobPlayable(unit.job)) return { kind: 'blocked', reason: '이 구간의 영상이 아직 준비되지 않았습니다.' };
  return { kind: 'ready', unit, job: unit.job, seconds };
}
export function cloudReplayTarget(timeline: CloudReplayTimeline, target: number): CloudReplayTarget {
  if (timeline.issue) return { kind: 'blocked', reason: timeline.issue };
  if (!Number.isFinite(target) || target < 0) return { kind: 'blocked', reason: '이동할 영상 시각이 올바르지 않습니다.' };
  // Half-open ranges select the next unit at a shared boundary. Only the final
  // real EOF belongs to the preceding unit; a missing interval is never clamped.
  const unit = timeline.units.find(value => target >= value.start && target < value.end)
    ?? (Math.abs(target - timeline.end) <= RANGE_EPSILON ? timeline.units.at(-1) : undefined);
  if (!unit) return { kind: 'blocked', reason: target > timeline.end && timeline.tailIssue ? timeline.tailIssue : '해당 시각의 보관 구간이 없습니다.' };
  return unitTarget(unit, Math.max(0, Math.min(unit.end - unit.start, target - unit.start)));
}
export function cloudReplayNeighbor(timeline: CloudReplayTimeline, jobId: string, direction: -1 | 1): CloudReplayTarget {
  if (timeline.issue) return { kind: 'blocked', reason: timeline.issue };
  const index = timeline.units.findIndex(unit => unit.job?.id === jobId), current = timeline.units[index];
  if (!current) return { kind: 'blocked', reason: '현재 보관 구간을 확인하지 못했습니다.' };
  const next = timeline.units[index + direction];
  if (!next) return direction === 1 && timeline.tailIssue ? { kind: 'blocked', reason: timeline.tailIssue } : { kind: 'end' };
  if (Math.abs(direction === 1 ? next.start - current.end : current.start - next.end) > RANGE_EPSILON) return { kind: 'blocked', reason: '사이에 저장되지 않은 영상 구간이 있어 이어서 재생할 수 없습니다.' };
  return unitTarget(next, 0);
}

/** An observation, never a requested seek position or a wall-clock estimate. */
export type CloudContentClock = {
  videoId: string; token: string; documentEpoch: number; sourceId: string; sourceGeneration: number;
  sequence: number; seekGeneration: number; currentTime: number | null; duration: number | null;
  valid: boolean; adState: string; buffering: boolean; seeking: boolean; ageMs: number;
};
export type CloudReplayPosition = { clock: CloudContentClock; time: number; generation: number };
export function acceptCloudClock(previous: CloudReplayPosition | null, clock: CloudContentClock | null, job: CloudRecordingJob): CloudReplayPosition | null {
  if (!clock || clock.videoId !== job.videoId || !clock.valid || clock.adState !== 'content' || clock.buffering || clock.seeking || !Number.isFinite(clock.ageMs) || clock.ageMs < 0 || clock.ageMs > 2000
    || clock.currentTime === null || !Number.isFinite(clock.currentTime) || clock.currentTime < 0 || !Number.isFinite(job.mediaOffsetSeconds)
    || clock.duration === null || !Number.isFinite(clock.duration) || Math.abs(clock.duration - job.durationSeconds) > 2
    || clock.currentTime > job.durationSeconds + 2) return null;
  const prior = previous?.clock;
  const sameLease = prior?.token === clock.token;
  if (sameLease && (clock.documentEpoch < prior.documentEpoch || clock.documentEpoch === prior.documentEpoch
    && (clock.sourceGeneration < prior.sourceGeneration || clock.sequence <= prior.sequence))) return null;
  const time = job.mediaOffsetSeconds + Math.min(clock.currentTime, job.durationSeconds);
  const discontinuity = !prior || !sameLease || prior.documentEpoch !== clock.documentEpoch
    || prior.sourceId !== clock.sourceId || prior.sourceGeneration !== clock.sourceGeneration
    || prior.seekGeneration !== clock.seekGeneration || time < previous!.time;
  return { clock, time, generation: (previous?.generation ?? 0) + (discontinuity ? 1 : 0) };
}
