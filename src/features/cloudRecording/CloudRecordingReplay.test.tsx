import { act, useImperativeHandle } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CloudRecordingJob, CloudReplayDescriptor } from '../../api/cloudRecording';
import type { YouTubeVodClock } from '../../api/youtubeOriginal';
import { CloudRecordingReplay } from './CloudRecordingReplay';
import type { CloudReplayExpectedPart } from './cloudRecordingModel';

const mocks = vi.hoisted(() => ({ open: vi.fn(), close: vi.fn(), page: vi.fn(), asset: vi.fn(), localOpen: vi.fn(), chzzkOpen: vi.fn(), chzzkClose: vi.fn(), native: null as any, chat: null as any, platformChat: null as any, seek: vi.fn(), pause: vi.fn(), resume: vi.fn(), setRate: vi.fn() }));
vi.mock('../../api/cloudRecording', async importOriginal => ({ ...await importOriginal<typeof import('../../api/cloudRecording')>(), cloudRecordingReplayApi: { open: mocks.open, close: mocks.close, page: mocks.page, asset: mocks.asset } }));
vi.mock('../platforms/platformMedia', () => ({ platformMediaApi: { open: mocks.localOpen } }));
vi.mock('../youtube/YouTubeOriginalPlayer', () => ({ YouTubeOriginalPlayer: (props: any) => {
  mocks.native = props; useImperativeHandle(props.ref, () => ({ seek: mocks.seek, pause: mocks.pause, resume: mocks.resume, setRate: mocks.setRate }));
  return <div data-official-video={props.channelId} data-private={String(props.privacyMode)} />;
} }));
vi.mock('../platforms/PlatformReplayChat', () => ({ PlatformReplayChat: (props: any) => { mocks.platformChat = props; return <div data-chat-time={props.currentTime} data-chat-provider={props.provider} />; } }));
vi.mock('../streaming/RecordingReplayChat', () => ({ RecordingReplayChat: (props: any) => { mocks.chat = props; return <div data-chat-time={props.time} data-chat-generation={props.seekVersion} />; } }));
vi.mock('../../api/replay', () => ({ createReplayApi: () => ({ open: mocks.chzzkOpen, close: mocks.chzzkClose }) }));

let root: Root, container: HTMLDivElement;
const job: CloudRecordingJob = { id: 'job', source: { provider: 'soop', recordingId: 'owned-record' }, title: 'Synthetic', state: 'ready', uploadedBytes: 40, totalBytes: 40, videoId: 'abcdefghijk', durationSeconds: 44, mediaOffsetSeconds: 120, createdAt: 1, updatedAt: 1 };
const clock = (patch: Partial<YouTubeVodClock> = {}): YouTubeVodClock => ({ token: 'viewer', videoId: job.videoId!, documentEpoch: 1, sourceId: 'content', sourceGeneration: 1, sequence: 1, seekGeneration: 1, currentTime: 10, duration: 44, valid: true, adState: 'content', buffering: false, seeking: false, ageMs: 20, paused: false, playbackRate: 1, ended: false, ...patch });
const part = (index: number, patch: Partial<CloudRecordingJob> = {}): CloudRecordingJob => ({ ...job, id: `part-${index}`, source: { provider: 'chzzk', recordingId: 'owned-record', planId: 'plan', unitId: `unit-${index}` }, replayCohortId: 'planned-cohort', durationSeconds: 60, mediaOffsetSeconds: index * 60, videoId: ['abcdefghijk', 'ABCDEFGHIJK', '12345678901'][index]!, ...patch });
const parts = [part(0), part(1), part(2)];
const manifest = (values = parts): CloudReplayExpectedPart[] => values.map(value => ({ source: value.source, partIndex: value.source.partIndex ?? null, unitIndex: value.source.planId ? value.mediaOffsetSeconds / 60 : undefined, durationSeconds: value.durationSeconds, mediaOffsetSeconds: value.mediaOffsetSeconds, bytes: value.totalBytes }));
const admissions = new Map<string, CloudReplayDescriptor>();
const admission = (value: CloudRecordingJob, expectedParts = manifest([value]), coverage: { state: 'complete' | 'incomplete'; reason?: string } = { state: 'complete' }): CloudReplayDescriptor => {
  const token = value.source.provider === 'chzzk' ? 'chat-lease' : 'sidecar-lease';
  return { provider: value.source.provider, token, cohortId: value.replayCohortId ?? value.id,
    description: { source: value.source, title: value.title, planId: value.source.planId, parts: expectedParts, coverage },
    ...(value.source.provider === 'chzzk' ? { session: { token, recordingId: value.source.recordingId, title: value.title, durationSeconds: 180, mimeType: '', chatStatus: 'ready', indexState: 'ready', syncQuality: 'observed_media', manualOffsetSeconds: 0, warnings: [] } } : {}),
  };
};
const frame = (value: CloudRecordingJob, patch: Partial<YouTubeVodClock> = {}) => clock({ token: `viewer-${value.id}`, videoId: value.videoId!, duration: value.durationSeconds, ...patch });
const render = async (value = job, privacy = false) => { admissions.set(value.id, admission(value)); await act(async () => root.render(<CloudRecordingReplay jobs={[value]} initialJobId={value.id} active privacyMode={privacy} onClose={() => {}} />)); };
const renderParts = async (values = parts, expectedParts = manifest(), initialJobId = parts[0]!.id, coverage: {state:'complete'|'incomplete';reason?:string} = { state: 'complete' }) => {
  for (const value of values) admissions.set(value.id, admission(value, expectedParts, coverage));
  await act(async () => root.render(<CloudRecordingReplay jobs={values} initialJobId={initialJobId} expectedParts={expectedParts} coverage={coverage} active privacyMode={false} onClose={() => {}} />));
};
const observe = async (value: YouTubeVodClock | null) => { await act(async () => mocks.native.onClock(value)); };
const chatTime = () => document.querySelector('[data-chat-time]')?.getAttribute('data-chat-time');
const selectedVideo = () => document.querySelector('[data-official-video]')?.getAttribute('data-official-video');
beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true); vi.resetAllMocks();
  mocks.native = null; mocks.chat = null; mocks.platformChat = null; admissions.clear();
  mocks.open.mockImplementation(async id => { const value = admissions.get(id); if (!value) throw new Error('missing test receipt'); return value; }); mocks.close.mockResolvedValue(undefined);
  for (const method of [mocks.seek, mocks.pause, mocks.resume, mocks.setRate]) method.mockResolvedValue(undefined);
  container = document.createElement('div'); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); expect(mocks.localOpen).not.toHaveBeenCalled(); expect(mocks.chzzkOpen).not.toHaveBeenCalled(); container.remove(); vi.useRealTimers(); vi.unstubAllGlobals(); });

describe('cloud replay source separation', () => {
  const captureWarning = '녹화 도중 수신이 중단되었습니다. 저장된 구간만 보관되어 있습니다.';
  it('retains an admitted interruption warning through pause, backward seek and EOF without disabling the valid video', async () => {
    const selected = { ...job, durationSeconds: 108.229, mediaOffsetSeconds: 0 };
    const receipt = admission(selected);
    receipt.description.coverage = { state: 'complete', captureWarning };
    mocks.open.mockResolvedValueOnce(receipt);
    await render(selected);
    expect(selectedVideo()).toBe(selected.videoId);
    expect(document.querySelector('.cloud-recording-capture-warning')).toHaveTextContent(captureWarning);
    await observe(clock({ currentTime: 41.805, duration: 108.229 })); expect(chatTime()).toBe('41.805');
    await observe(clock({ sequence: 2, currentTime: 41.805, duration: 108.229, paused: true }));
    expect(mocks.platformChat.paused).toBe(true);
    await observe(clock({ sequence: 3, currentTime: 4.995, duration: 108.229, paused: true, seekGeneration: 2 }));
    expect(chatTime()).toBe('4.995');
    await observe(clock({ sequence: 4, currentTime: 108.229, duration: 108.229, paused: true, ended: true, seekGeneration: 3 }));
    expect(chatTime()).toBe('108.229'); expect(mocks.platformChat.ended).toBe(true);
    expect(document.querySelector('.cloud-recording-capture-warning')).toHaveTextContent(captureWarning);
    expect(mocks.open).toHaveBeenCalledOnce(); expect(mocks.close).not.toHaveBeenCalled();
  });
  it('hides and restores an admitted capture warning in privacy without reacquiring the lease', async () => {
    const receipt = admission(job); receipt.description.coverage = { state: 'complete', captureWarning };
    mocks.open.mockResolvedValueOnce(receipt);
    await render(); await observe(clock());
    expect(document.body.textContent).toContain(captureWarning);
    await render(job, true); expect(document.body.textContent).not.toContain(captureWarning);
    await render(job, false); expect(document.body.textContent).toContain(captureWarning);
    expect(mocks.open).toHaveBeenCalledOnce(); expect(mocks.close).not.toHaveBeenCalled();
    expect(selectedVideo()).toBe(job.videoId);
  });
  it.each([undefined, { state: 'complete' as const }])('does not invent capture warnings in a complete or legacy native descriptor %j', async coverage => {
    const receipt = admission(job); receipt.description.coverage = coverage;
    mocks.open.mockResolvedValueOnce(receipt);
    await render(); await observe(clock());
    expect(document.querySelector('.cloud-recording-capture-warning')).toBeNull();
    expect(selectedVideo()).toBe(job.videoId); expect(chatTime()).toBe('130');
  });
  it('does not expose a warning or chat from a mismatched source admission', async () => {
    const receipt = admission(job);
    receipt.description.source = { ...job.source, recordingId: 'other-record' };
    receipt.description.coverage = { state: 'complete', captureWarning: 'foreign-source-warning' };
    mocks.open.mockResolvedValueOnce(receipt);
    await render(); await observe(clock());
    expect(document.body.textContent).not.toContain('foreign-source-warning');
    expect(chatTime()).toBeUndefined(); expect(selectedVideo()).toBe(job.videoId);
    expect(mocks.close).toHaveBeenCalledExactlyOnceWith('soop', 'sidecar-lease');
  });
  it('shows the selected video visibility without replacing the account-recovery player', async () => {
    await render();
    expect(document.querySelector('[aria-label="재생 영상 공개 범위"]')).toHaveTextContent('비공개');
    await render({ ...job, visibility: 'unlisted' });
    expect(document.querySelector('[aria-label="재생 영상 공개 범위"]')).toHaveTextContent('일부 공개 · 링크로 시청');
    expect(mocks.native.allowAccountRecovery).toBe(true);
    expect(mocks.open).toHaveBeenCalledOnce(); expect(mocks.close).not.toHaveBeenCalled();
  });
  it('uses the official hosted video and keeps local video entirely out of the media DOM', async () => {
    await render();
    expect(mocks.open).toHaveBeenCalledWith(job.id);
    expect(mocks.native.allowAccountRecovery).toBe(true);
    expect(selectedVideo()).toBe('abcdefghijk'); expect(document.querySelector('video,audio,source')).toBeNull();
    expect(document.body.innerHTML).not.toContain('atsumi-media://'); expect(chatTime()).toBeUndefined();
    await observe(clock()); expect(chatTime()).toBe('130');
  });
  it('freezes chat for ads/buffering and advances only from a later actual content position', async () => {
    await render(); await observe(clock());
    await observe(clock({ sequence: 2, adState: 'ad', valid: false, currentTime: null })); expect(chatTime()).toBe('130');
    await observe(clock({ sequence: 3, buffering: true, currentTime: 25 })); expect(chatTime()).toBe('130');
    await observe(clock({ sequence: 4, currentTime: 4, seekGeneration: 2 })); expect(chatTime()).toBe('124');
  });
  it('flushes paused and settled observations within the same presentation bucket', async () => {
    await render({ ...job, mediaOffsetSeconds: 0 });
    await observe(clock({ currentTime: 15.71 }));
    await observe(clock({ sequence: 2, currentTime: 15.79, paused: true }));
    expect(chatTime()).toBe('15.79'); expect(mocks.platformChat.paused).toBe(true);
    await observe(clock({ sequence: 3, currentTime: 15.795, paused: true })); expect(chatTime()).toBe('15.795');
    const generation = mocks.platformChat.seekVersion;
    await observe(clock({ sequence: 4, currentTime: 15.75, paused: true, seekGeneration: 2 }));
    expect(chatTime()).toBe('15.75'); expect(mocks.platformChat.seekVersion).toBeGreaterThan(generation);
  });
  it('forwards fractional EOF and its final settled clock without requiring another playback bucket', async () => {
    await render({ ...job, mediaOffsetSeconds: 0, durationSeconds: 42.102 });
    await observe(clock({ currentTime: 42.01, duration: 42.102 }));
    await observe(clock({ sequence: 2, currentTime: 42.09, duration: 42.102, ended: true, paused: true }));
    expect(chatTime()).toBe('42.09'); expect(mocks.platformChat.ended).toBe(true);
    await observe(clock({ sequence: 3, currentTime: 42.102, duration: 42.102, ended: true, paused: true }));
    expect(chatTime()).toBe('42.102'); expect(mocks.resume).not.toHaveBeenCalled();
    await observe(clock({ sequence: 4, currentTime: 0, duration: 42.102, seekGeneration: 2 }));
    expect(chatTime()).toBe('0'); expect(mocks.platformChat.ended).toBe(false); expect(mocks.platformChat.paused).toBe(false);
  });
  it('does not release the source lease on privacy transitions and closes it on exit', async () => {
    await render(); await observe(clock()); await render(job, true);
    expect(mocks.open).toHaveBeenCalledTimes(1); expect(mocks.close).not.toHaveBeenCalled();
    expect(document.querySelector('[data-official-video]')?.getAttribute('data-private')).toBe('true');
    expect(document.querySelector('.cloud-recording-chat')?.hasAttribute('hidden')).toBe(true);
    await act(async () => root.render(null)); expect(mocks.close).toHaveBeenCalledWith('soop', 'sidecar-lease');
  });
  it('releases a late source lease after the replay has already closed', async () => {
    let resolve!: (value: unknown) => void;
    mocks.open.mockImplementation(() => new Promise(value => { resolve = value; }));
    await render(); await act(async () => root.render(null));
    await act(async () => resolve({ ...admission(job), token: 'late-lease' }));
    expect(mocks.close).toHaveBeenCalledWith('soop', 'late-lease');
  });
  it('uses the sidecar token for both saved chat and images without a local media fallback', async () => {
    mocks.localOpen.mockRejectedValue(new Error('local media missing'));
    await render(); await observe(clock());
    expect(mocks.platformChat.libraryId).toBe('sidecar-lease');
    await mocks.platformChat.api.page('ignored-recording', 130000, null, 120);
    await mocks.platformChat.api.asset('ignored-recording', 'saved-image');
    expect(mocks.page).toHaveBeenCalledWith('sidecar-lease', 130000, null, 120);
    expect(mocks.asset).toHaveBeenCalledWith('sidecar-lease', 'saved-image');
  });
  it('reacquires on an owner/source cohort change even when recording and job identifiers stay the same', async () => {
    await render({ ...job, replayCohortId: 'owner-one-generation' }); await observe(clock());
    await render({ ...job, replayCohortId: 'owner-two-generation' });
    expect(mocks.open).toHaveBeenCalledTimes(2);
    expect(mocks.close).toHaveBeenCalledExactlyOnceWith('soop', 'sidecar-lease');
    expect(chatTime()).toBeUndefined();
  });
  it('closes mismatched native admission and keeps the remote video available with an honest chat error', async () => {
    mocks.open.mockResolvedValueOnce({ ...admission(job), cohortId: 'another-owner' });
    await render({ ...job, replayCohortId: 'expected-owner' }); await observe(clock());
    expect(mocks.close).toHaveBeenCalledExactlyOnceWith('soop', 'sidecar-lease');
    expect(chatTime()).toBeUndefined(); expect(selectedVideo()).toBe(job.videoId);
    expect(document.body.textContent).toContain('채팅 기록을 확인하지 못했습니다');
  });
  it('releases an obsolete open after a new cohort has already acquired its own chat capability', async () => {
    let finish!: (value: CloudReplayDescriptor) => void;
    mocks.open.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    await render({ ...job, replayCohortId: 'old-cohort' });
    await render({ ...job, replayCohortId: 'new-cohort' }); await observe(clock());
    await act(async () => finish({ ...admission({ ...job, replayCohortId: 'old-cohort' }), token: 'old-token' }));
    expect(mocks.close).toHaveBeenCalledWith('soop', 'old-token');
    expect(mocks.platformChat.libraryId).toBe('sidecar-lease'); expect(chatTime()).toBe('130');
  });
});

describe('continuous recording replay', () => {
  it('advances once at real content EOF, observes restored rate/position, and retains one chat lease', async () => {
    await renderParts(parts.slice(0, 2), manifest(parts.slice(0, 2)));
    await observe(frame(parts[0]!, { currentTime: 55, playbackRate: 1.5 }));
    const obsolete = mocks.native.onClock;
    await observe(frame(parts[0]!, { sequence: 2, currentTime: 60, ended: true, paused: true, playbackRate: 1.5 }));
    expect(selectedVideo()).toBe(parts[1]!.videoId); expect(chatTime()).toBe('60');
    await act(async () => obsolete(frame(parts[0]!, { sequence: 3, currentTime: 60, ended: true })));
    expect(selectedVideo()).toBe(parts[1]!.videoId);
    await observe(frame(parts[1]!, { currentTime: 0 }));
    expect(mocks.setRate).toHaveBeenCalledWith(1.5); expect(mocks.seek).toHaveBeenCalledWith(0); expect(mocks.resume).not.toHaveBeenCalled();
    await observe(frame(parts[1]!, { sequence: 2, currentTime: 0, paused: true, playbackRate: 1.5 }));
    expect(mocks.resume).toHaveBeenCalledOnce(); expect(mocks.open).toHaveBeenCalledOnce(); expect(mocks.close).not.toHaveBeenCalled();
    await observe(frame(parts[1]!, { sequence: 3, currentTime: 60, ended: true, paused: true, playbackRate: 1.5 }));
    expect(selectedVideo()).toBe(parts[1]!.videoId); expect(chatTime()).toBe('120');
    await observe(frame(parts[1]!, { sequence: 4, currentTime: 0, seekGeneration: 2 })); expect(chatTime()).toBe('60');
    await act(async () => root.render(null)); expect(mocks.close).toHaveBeenCalledWith('chzzk', 'chat-lease');
  });
  it('seeks into the correct earlier/later unit without clamping chat targets or accepting command-only progress', async () => {
    await renderParts(); await observe(frame(parts[0]!, { currentTime: 30 }));
    await act(async () => mocks.chat.onSeek(130)); expect(selectedVideo()).toBe(parts[2]!.videoId); expect(chatTime()).toBe('30');
    await observe(frame(parts[2]!, { currentTime: 0, paused: true })); expect(mocks.seek).toHaveBeenLastCalledWith(10); expect(chatTime()).toBe('30');
    await observe(frame(parts[2]!, { sequence: 2, currentTime: 10, paused: true, seekGeneration: 2 })); expect(chatTime()).toBe('130');
    await act(async () => mocks.chat.onSeek(10)); expect(selectedVideo()).toBe(parts[0]!.videoId);
    await observe(frame(parts[0]!, { currentTime: 0, paused: true })); expect(mocks.seek).toHaveBeenLastCalledWith(10);
    await observe(frame(parts[0]!, { sequence: 2, currentTime: 10, paused: true, seekGeneration: 2 })); expect(chatTime()).toBe('10');
    expect(mocks.open).toHaveBeenCalledOnce(); expect(mocks.close).not.toHaveBeenCalled();
  });
  it('preserves a paused viewing intent across a cross-unit seek', async () => {
    await renderParts(); await observe(frame(parts[0]!, { currentTime: 25, paused: true }));
    await act(async () => mocks.chat.onSeek(70));
    await observe(frame(parts[1]!, { currentTime: 0, paused: false }));
    await observe(frame(parts[1]!, { sequence: 2, currentTime: 10, paused: true, seekGeneration: 2 }));
    expect(chatTime()).toBe('70'); expect(mocks.pause).toHaveBeenCalled(); expect(mocks.resume).not.toHaveBeenCalled();
  });
  it('waits for a known pending unit and continues after its ready snapshot without reopening chat', async () => {
    await renderParts([parts[0]!, { ...parts[1]!, state: 'processing' }, parts[2]!]);
    await observe(frame(parts[0]!, { currentTime: 59 })); await observe(frame(parts[0]!, { sequence: 2, currentTime: 60, ended: true, paused: true }));
    expect(selectedVideo()).toBe(parts[0]!.videoId); expect(document.body.textContent).toContain('YouTube 처리 중');
    await renderParts(); expect(selectedVideo()).toBe(parts[1]!.videoId); expect(mocks.open).toHaveBeenCalledOnce();
  });
  it('does not skip missing units or pretend an incomplete tail is final success', async () => {
    await renderParts([parts[0]!, parts[2]!]);
    await observe(frame(parts[0]!, { currentTime: 59 })); await observe(frame(parts[0]!, { sequence: 2, currentTime: 60, ended: true }));
    expect(selectedVideo()).toBe(parts[0]!.videoId); expect(document.body.textContent).toContain('아직 업로드되지');
    await act(async () => root.render(null));
    await renderParts([parts[0]!], manifest([parts[0]!]), parts[0]!.id, { state: 'incomplete', reason: '끝부분 확인 필요' });
    await observe(frame(parts[0]!, { currentTime: 60, ended: true }));
    expect(selectedVideo()).toBe(parts[0]!.videoId); expect(document.body.textContent).toContain('끝부분 확인 필요');
  });
  it('leaves an unavailable cross-unit seek on the actual current video and explains the missing range', async () => {
    await renderParts([parts[0]!, parts[2]!]); await observe(frame(parts[0]!, { currentTime: 20 }));
    await act(async () => mocks.chat.onSeek(70));
    expect(selectedVideo()).toBe(parts[0]!.videoId); expect(chatTime()).toBe('20'); expect(mocks.seek).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain('아직 업로드되지');
  });
  it('does not advance on ad, stale-video or buffering end observations', async () => {
    await renderParts(); await observe(frame(parts[0]!, { currentTime: 20 }));
    const observations: Partial<YouTubeVodClock>[] = [{ valid: false, adState: 'ad', currentTime: null }, { buffering: true, currentTime: 60 }, { videoId: parts[2]!.videoId!, currentTime: 60 }];
    for (const patch of observations) {
      await observe(frame(parts[0]!, { sequence: 2, ended: true, ...patch })); expect(selectedVideo()).toBe(parts[0]!.videoId); expect(chatTime()).toBe('20');
    }
  });
  it('rejects late control acknowledgements after another unit was selected', async () => {
    await renderParts(); await observe(frame(parts[0]!, { currentTime: 20 }));
    let finish!: () => void; mocks.seek.mockImplementationOnce(() => new Promise<void>(resolve => { finish = resolve; }));
    await act(async () => mocks.chat.onSeek(70)); await observe(frame(parts[1]!, { currentTime: 0, paused: true }));
    const obsolete = mocks.native.onClock;
    await act(async () => mocks.chat.onSeek(130)); await act(async () => finish());
    await act(async () => obsolete(frame(parts[1]!, { sequence: 2, currentTime: 10, paused: true })));
    expect(selectedVideo()).toBe(parts[2]!.videoId); expect(chatTime()).toBe('20'); expect(mocks.resume).not.toHaveBeenCalled();
    await observe(frame(parts[2]!, { currentTime: 0, paused: true })); await observe(frame(parts[2]!, { sequence: 2, currentTime: 10, paused: true }));
    expect(chatTime()).toBe('130'); expect(mocks.resume).toHaveBeenCalledOnce();
  });
  it('stops and reports a rejected playback rate instead of silently resuming at the wrong speed', async () => {
    await renderParts(); await observe(frame(parts[0]!, { currentTime: 59, playbackRate: 1.5 }));
    await observe(frame(parts[0]!, { sequence: 2, currentTime: 60, ended: true, playbackRate: 1.5 }));
    mocks.setRate.mockRejectedValueOnce(new Error('unsupported'));
    await observe(frame(parts[1]!, { currentTime: 0 }));
    expect(mocks.resume).not.toHaveBeenCalled(); expect(mocks.seek).not.toHaveBeenCalled(); expect(document.body.textContent).toContain('재생속도를 적용하지');
  });
  it('does not reset selected units or chat when ordinary snapshot polling replaces job objects', async () => {
    await renderParts(); await observe(frame(parts[0]!, { currentTime: 20 }));
    await act(async () => mocks.chat.onSeek(130)); await observe(frame(parts[2]!, { currentTime: 0, paused: true })); await observe(frame(parts[2]!, { sequence: 2, currentTime: 10, paused: true }));
    await renderParts(parts.map(value => ({ ...value, updatedAt: 10 })));
    expect(selectedVideo()).toBe(parts[2]!.videoId); expect(chatTime()).toBe('130'); expect(mocks.open).toHaveBeenCalledOnce();
  });
  it('waits through an ad before starting bounded position/rate confirmation', async () => {
    vi.useFakeTimers();
    await renderParts(); await observe(frame(parts[0]!, { currentTime: 59 }));
    await observe(frame(parts[0]!, { sequence: 2, currentTime: 60, ended: true }));
    await observe(frame(parts[1]!, { valid: false, adState: 'ad', currentTime: null, duration: null }));
    await act(async () => vi.advanceTimersByTimeAsync(30000));
    expect(mocks.seek).not.toHaveBeenCalled(); expect(mocks.resume).not.toHaveBeenCalled(); expect(document.body.textContent).not.toContain('확인하지 못했습니다');
    await observe(frame(parts[1]!, { sequence: 2, currentTime: 0, paused: true }));
    await observe(frame(parts[1]!, { sequence: 3, currentTime: 0, paused: true }));
    expect(mocks.resume).toHaveBeenCalledOnce();
  });
  it('does not equate an accepted rate request with the actually applied speed', async () => {
    vi.useFakeTimers();
    await renderParts(); await observe(frame(parts[0]!, { currentTime: 59, playbackRate: 1.5 }));
    await observe(frame(parts[0]!, { sequence: 2, currentTime: 60, ended: true, playbackRate: 1.5 }));
    await observe(frame(parts[1]!, { currentTime: 0, paused: true }));
    await observe(frame(parts[1]!, { sequence: 2, currentTime: 0, paused: true, playbackRate: 1 }));
    expect(mocks.setRate).toHaveBeenCalledWith(1.5); expect(mocks.resume).not.toHaveBeenCalled();
    await act(async () => vi.advanceTimersByTimeAsync(15001));
    expect(mocks.resume).not.toHaveBeenCalled(); expect(document.body.textContent).toContain('재생속도를 확인하지 못했습니다');
  });
});
