import { describe, expect, it } from 'vitest';
import type { CloudRecordingJob } from '../../api/cloudRecording';
import { acceptCloudClock, cloudJobPlayable, cloudProgress, cloudReplayTimeline, cloudReplayTarget, cloudReplayNeighbor, sameCloudPart, type CloudContentClock, type CloudReplayExpectedPart } from './cloudRecordingModel';

const job: CloudRecordingJob = { id: 'job', source: { provider: 'soop', recordingId: 'record' }, title: 'Synthetic', state: 'ready', uploadedBytes: 40, totalBytes: 40, videoId: 'abcdefghijk', durationSeconds: 44, mediaOffsetSeconds: 120, createdAt: 1, updatedAt: 1 };
const clock = (patch: Partial<CloudContentClock> = {}): CloudContentClock => ({ token: 'viewer', videoId: job.videoId!, documentEpoch: 1, sourceId: 'content', sourceGeneration: 1, sequence: 1, seekGeneration: 1, currentTime: 10, duration: 44, valid: true, adState: 'content', buffering: false, seeking: false, ageMs: 20, ...patch });

describe('cloud archive evidence and time mapping', () => {
  it('requires processed remote identity, never byte completion alone', () => {
    expect(cloudProgress(job)).toBe(100);
    expect(cloudJobPlayable({ ...job, state: 'processing' })).toBe(false);
    expect(cloudJobPlayable({ ...job, videoId: 'https://untrusted/video' })).toBe(false);
    expect(cloudJobPlayable(job)).toBe(true);
    expect(cloudProgress({ ...job, totalBytes: 0 })).toBeUndefined();
  });
  it('keeps platforms and every progressive part distinct', () => {
    expect(sameCloudPart(job.source, { ...job.source, partIndex: null })).toBe(true);
    expect(sameCloudPart(job.source, { ...job.source, partIndex: 0 })).toBe(false);
    expect(sameCloudPart(job.source, { ...job.source, provider: 'youtube' })).toBe(false);
  });
  it('maps actual hosted content position to the original recording interval', () => {
    const accepted = acceptCloudClock(null, clock(), job)!;
    expect(accepted.time).toBe(130);
    const repeated = acceptCloudClock(accepted, clock({ sequence: 2 }), job)!;
    expect(repeated.time).toBe(130); // A pause does not extrapolate elapsed wall time.
    expect(repeated.generation).toBe(accepted.generation);
  });
  it('rejects ad, unknown, buffering, in-flight seeks and stale observations', () => {
    for (const patch of [{ adState: 'ad' }, { adState: 'unknown' }, { buffering: true }, { seeking: true }, { valid: false }, { ageMs: 2001 }, { currentTime: NaN }, { currentTime: 90 }, { duration: null }, { duration: 10 }, { videoId: 'wrong' }]) {
      expect(acceptCloudClock(null, clock(patch), job)).toBeNull();
    }
  });
  it('does not let old sequence/document/source observations overwrite a newer position', () => {
    const accepted = acceptCloudClock(null, clock({ documentEpoch: 3, sourceGeneration: 4, sequence: 10 }), job)!;
    for (const patch of [{ documentEpoch: 2, sequence: 11 }, { documentEpoch: 3, sourceGeneration: 3, sequence: 11 }, { documentEpoch: 3, sourceGeneration: 4, sequence: 9 }]) {
      expect(acceptCloudClock(accepted, clock(patch), job)).toBeNull();
    }
  });
  it('invalidates chat response generations after backseek and source replacement', () => {
    const accepted = acceptCloudClock(null, clock(), job)!;
    const sought = acceptCloudClock(accepted, clock({ sequence: 2, currentTime: 3, seekGeneration: 2 }), job)!;
    expect(sought.time).toBe(123);
    expect(sought.generation).toBe(accepted.generation + 1);
    const replaced = acceptCloudClock(sought, clock({ sequence: 3, sourceId: 'new-content', sourceGeneration: 2 }), job)!;
    expect(replaced.generation).toBe(sought.generation + 1);
  });
});

const part = (index: number, patch: Partial<CloudRecordingJob> = {}): CloudRecordingJob => ({ ...job, id: `part-${index}`, source: { provider: 'chzzk', recordingId: 'record', partIndex: index }, durationSeconds: 60, mediaOffsetSeconds: index * 60, videoId: ['abcdefghijk', 'ABCDEFGHIJK', '12345678901'][index] ?? 'aaaaaaaaaaa', ...patch });
const expected = (jobs: CloudRecordingJob[]): CloudReplayExpectedPart[] => jobs.map(value => ({ source: value.source, partIndex: value.source.partIndex ?? null, durationSeconds: value.durationSeconds, mediaOffsetSeconds: value.mediaOffsetSeconds, bytes: value.totalBytes }));
const complete = { state: 'complete' as const };
describe('complete recording playback ranges', () => {
  it('finds the owning unit for forward/backward seeks and shared boundaries', () => {
    const jobs = [part(0), part(1), part(2)], timeline = cloudReplayTimeline(jobs, jobs[1]!, expected(jobs), complete);
    expect(cloudReplayTarget(timeline, 10)).toMatchObject({ kind: 'ready', job: { id: 'part-0' }, seconds: 10 });
    expect(cloudReplayTarget(timeline, 130)).toMatchObject({ kind: 'ready', job: { id: 'part-2' }, seconds: 10 });
    expect(cloudReplayTarget(timeline, 60)).toMatchObject({ kind: 'ready', job: { id: 'part-1' }, seconds: 0 });
    expect(cloudReplayTarget(timeline, 180)).toMatchObject({ kind: 'ready', job: { id: 'part-2' }, seconds: 60 });
    expect(cloudReplayTarget(timeline, 181).kind).toBe('blocked');
    expect(cloudReplayNeighbor(timeline, 'part-2', 1)).toEqual({ kind: 'end' });
  });
  it('never skips missing, uploading, failed or unknown-position units', () => {
    const jobs = [part(0), part(1), part(2)], manifest = expected(jobs);
    for (const middle of [null, part(1, { state: 'uploading' }), part(1, { state: 'failed' }), part(1, { state: 'queued', durationSeconds: 0, mediaOffsetSeconds: 0 })]) {
      const actual = [jobs[0]!, ...(middle ? [middle] : []), jobs[2]!];
      const timeline = cloudReplayTimeline(actual, actual[0]!, manifest, complete);
      expect(cloudReplayNeighbor(timeline, 'part-0', 1).kind).toBe('blocked');
      expect(cloudReplayTarget(timeline, 70).kind).toBe('blocked');
    }
    const missingTail = cloudReplayTimeline(jobs.slice(0, 2), jobs[0]!, manifest, complete);
    expect(cloudReplayNeighbor(missingTail, 'part-1', 1).kind).toBe('blocked');
  });
  it('rejects gaps, overlap, duplicate identities and ready-range drift', () => {
    const first = part(0), next = part(1, { mediaOffsetSeconds: 70 });
    const gap = cloudReplayTimeline([first, next], first, expected([first, next]), complete);
    expect(cloudReplayTarget(gap, 65).kind).toBe('blocked'); expect(cloudReplayNeighbor(gap, first.id, 1).kind).toBe('blocked');
    const overlap = part(1, { mediaOffsetSeconds: 55 });
    expect(cloudReplayTimeline([first, overlap], first, expected([first, overlap]), complete).issue).not.toBeNull();
    expect(cloudReplayTimeline([first], first, [...expected([first]), ...expected([first])], complete).issue).not.toBeNull();
    const manifest = expected([first, part(1)]);
    expect(cloudReplayTarget(cloudReplayTimeline([first, next], first, manifest, complete), 70).kind).toBe('blocked');
    expect(cloudReplayTimeline([first, { ...first, id: 'other-account' }], first, expected([first]), complete).issue).not.toBeNull();
  });
  it('does not call a verified prefix or missing manifest a complete recording', () => {
    const first = part(0);
    expect(cloudReplayTimeline([first], first).issue).not.toBeNull();
    const prefix = cloudReplayTimeline([first], first, expected([first]), { state: 'incomplete', reason: '끝부분 확인 필요' });
    expect(cloudReplayNeighbor(prefix, first.id, 1)).toEqual({ kind: 'blocked', reason: '끝부분 확인 필요' });
    expect(cloudReplayNeighbor(cloudReplayTimeline([first], first, expected([first])), first.id, 1).kind).toBe('blocked');
    expect(cloudReplayTimeline([job], job).issue).toBeNull(); // legacy whole-file compatibility
  });
  it('keeps native plans and legacy sources isolated while mapping long recordings', () => {
    const planned = [0, 1, 2].map(index => part(index, { source: { provider: 'chzzk', recordingId: 'record', planId: 'plan-a', unitId: `unit-${index}` }, durationSeconds: 21600, mediaOffsetSeconds: index * 21600 }));
    const manifest = expected(planned).map((value, unitIndex) => ({ ...value, unitIndex }));
    const mixed = [...planned, part(0), { ...planned[0]!, id: 'other-plan', source: { ...planned[0]!.source, planId: 'plan-b' } }];
    const timeline = cloudReplayTimeline(mixed, planned[0]!, manifest, complete);
    expect(timeline.units).toHaveLength(3); expect(timeline.end).toBe(64800);
    expect(cloudReplayTarget(timeline, 45000)).toMatchObject({ kind: 'ready', job: { id: 'part-2' }, seconds: 1800 });
    expect(sameCloudPart(planned[0]!.source, { ...planned[0]!.source, unitId: 'unit-1' })).toBe(false);
    expect(cloudReplayTimeline(mixed, planned[0]!, expected([part(0)]), complete).issue).not.toBeNull();
  });
  it('never combines another owner or source generation into the selected native cohort', () => {
    const first = part(0, { replayCohortId: 'owner-one-source-a' });
    const second = part(1, { replayCohortId: 'owner-one-source-a' });
    const other = { ...second, id: 'other-owner', replayCohortId: 'owner-two-source-a' };
    const manifest = expected([first, second]);
    expect(cloudReplayNeighbor(cloudReplayTimeline([first, other], first, manifest, complete), first.id, 1).kind).toBe('blocked');
    expect(cloudReplayNeighbor(cloudReplayTimeline([first, second, other], first, manifest, complete), first.id, 1)).toMatchObject({ kind: 'ready', job: { id: second.id } });
  });
});
