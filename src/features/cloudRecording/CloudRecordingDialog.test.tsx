import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CloudRecordingApi, CloudRecordingJob, CloudSnapshot, CloudSourceRef } from '../../api/cloudRecording';
import { CloudRecordingDialog } from './CloudRecordingDialog';

const replay = vi.hoisted(() => ({ describe: vi.fn() }));
vi.mock('../../api/cloudRecording', async importOriginal => ({ ...await importOriginal<typeof import('../../api/cloudRecording')>(), cloudRecordingReplayApi: replay }));
vi.mock('./CloudRecordingReplay', () => ({ CloudRecordingReplay: ({ jobs, initialJobId }: { jobs: CloudRecordingJob[]; initialJobId: string }) => <div data-remote-video={jobs.find(job => job.id === initialJobId)?.videoId}>원격 영상</div> }));
let root: Root, container: HTMLDivElement;
const source: CloudSourceRef = { provider: 'soop', recordingId: 'owned-recording' };
const state = (patch: Partial<CloudSnapshot> = {}): CloudSnapshot => ({ account: { configured: true, connected: true, authorizing: false, channelName: '업로드 채널' }, jobs: [], ...patch });
const job = (patch: Partial<CloudRecordingJob> = {}): CloudRecordingJob => ({ id: 'job', source, title: 'Synthetic', state: 'uploading', totalBytes: 100, uploadedBytes: 70, durationSeconds: 44, mediaOffsetSeconds: 0, createdAt: 1, updatedAt: 1, ...patch });
const api = (snapshot = state()): CloudRecordingApi => ({
  snapshot: vi.fn().mockResolvedValue(snapshot),
  describe: vi.fn().mockResolvedValue({ source, title: 'Synthetic', parts: [{ partIndex: null, durationSeconds: 44, mediaOffsetSeconds: 0, bytes: 100 }] }),
  getPlan: vi.fn().mockResolvedValue({ source, title: 'Synthetic', parts: [{ partIndex: null, durationSeconds: 44, mediaOffsetSeconds: 0, bytes: 100 }] }),
  importClient: vi.fn().mockResolvedValue(snapshot), authorize: vi.fn().mockResolvedValue(snapshot), disconnect: vi.fn().mockResolvedValue(snapshot),
  enqueue: vi.fn().mockResolvedValue(snapshot), enqueueRecording: vi.fn().mockResolvedValue(snapshot), pause: vi.fn().mockResolvedValue(snapshot), resume: vi.fn().mockResolvedValue(snapshot),
  setAutomatic: vi.fn().mockResolvedValue(snapshot),
});
const button = (text: string) => [...document.querySelectorAll<HTMLButtonElement>('button')].find(item => item.textContent === text)!;
const render = async (client: CloudRecordingApi, privacyMode = false) => { await act(async () => root.render(<CloudRecordingDialog source={source} title="Synthetic" privacyMode={privacyMode} onClose={() => {}} api={client} />)); };
const click = async (text: string) => { await act(async () => button(text).click()); };
beforeEach(() => { replay.describe.mockReset().mockResolvedValue({ source, title: 'Synthetic', parts: [{ partIndex: null, durationSeconds: 44, mediaOffsetSeconds: 0, bytes: 100 }] }); vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true); vi.useFakeTimers(); container = document.createElement('div'); document.body.append(container); root = createRoot(container); });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.useRealTimers(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });

describe('selected archive entry', () => {
  it('requires an explicit unlisted choice and resets the next recording to private', async () => {
    const client = api(); await render(client);
    const select = document.querySelector<HTMLSelectElement>('[aria-label="이번 업로드 공개 범위"]')!;
    expect(select.value).toBe('private');
    await act(async () => { select.value = 'unlisted'; select.dispatchEvent(new Event('change', { bubbles: true })); });
    expect(client.enqueueRecording).not.toHaveBeenCalled();
    await click('일부 공개 업로드');
    expect(client.enqueueRecording).toHaveBeenCalledExactlyOnceWith(source, 'unlisted');
    const next = { ...source, recordingId: 'next-recording' };
    vi.mocked(client.describe).mockResolvedValue({ source: next, title: 'Next', parts: [{ partIndex: null, durationSeconds: 44, mediaOffsetSeconds: 0, bytes: 100 }] });
    await act(async () => root.render(<CloudRecordingDialog source={next} title="Next" privacyMode={false} onClose={() => {}} api={client} />));
    expect(document.querySelector<HTMLSelectElement>('[aria-label="이번 업로드 공개 범위"]')?.value).toBe('private');
    expect(client.enqueueRecording).toHaveBeenCalledTimes(1);
  });
  const captureWarning = '녹화 도중 수신이 중단되었습니다. 저장된 구간만 보관되어 있습니다.';
  it('keeps interruption visible while allowing the verified selected file to upload once', async () => {
    const client = api();
    vi.mocked(client.describe).mockResolvedValue({ source, title: 'Synthetic', coverage: { state: 'complete', captureWarning }, parts: [{ source, partIndex: null, durationSeconds: 108.229, mediaOffsetSeconds: 0, bytes: 100 }] });
    await render(client);
    expect(document.querySelector('.cloud-recording-capture-warning')).toHaveTextContent(captureWarning);
    expect(button('비공개 업로드').disabled).toBe(false);
    await click('비공개 업로드');
    expect(client.enqueueRecording).toHaveBeenCalledExactlyOnceWith(source, 'private');
    expect(client.enqueue).not.toHaveBeenCalled(); expect(client.setAutomatic).not.toHaveBeenCalled();
    expect(document.querySelector('.cloud-recording-capture-warning')).toHaveTextContent(captureWarning);
  });
  it('hides a loaded capture warning during privacy and restores it without another source action', async () => {
    const client = api();
    vi.mocked(client.describe).mockResolvedValue({ source, title: 'Synthetic', coverage: { state: 'complete', captureWarning }, parts: [{ source, partIndex: null, durationSeconds: 44, mediaOffsetSeconds: 0, bytes: 100 }] });
    await render(client);
    expect(document.body.textContent).toContain(captureWarning);
    await render(client, true);
    expect(document.body.textContent).not.toContain(captureWarning);
    expect(button('비공개 업로드').disabled).toBe(true);
    await render(client, false);
    expect(document.body.textContent).toContain(captureWarning);
    expect(button('비공개 업로드').disabled).toBe(false);
    expect(client.describe).toHaveBeenCalledOnce(); expect(client.enqueueRecording).not.toHaveBeenCalled();
  });
  it.each([undefined, { state: 'complete' as const }])('does not invent capture warnings for complete or legacy description %j', async coverage => {
    const client = api();
    vi.mocked(client.describe).mockResolvedValue({ source, title: 'Synthetic', coverage, parts: [{ source, partIndex: null, durationSeconds: 44, mediaOffsetSeconds: 0, bytes: 100 }] });
    await render(client);
    expect(document.querySelector('.cloud-recording-capture-warning')).toBeNull();
    expect(button('비공개 업로드').disabled).toBe(false);
  });
  it('rejects a warning-bearing response for another source instead of allowing its upload', async () => {
    const client = api();
    vi.mocked(client.describe).mockResolvedValue({ source: { ...source, recordingId: 'another-recording' }, title: 'Other', coverage: { state: 'complete', captureWarning: 'other-source-warning' }, parts: [{ source, partIndex: null, durationSeconds: 44, mediaOffsetSeconds: 0, bytes: 100 }] });
    await render(client);
    expect(button('비공개 업로드').disabled).toBe(true);
    expect(document.body.textContent).not.toContain('other-source-warning');
    await click('비공개 업로드'); expect(client.enqueueRecording).not.toHaveBeenCalled();
  });
  it('retains a planned capture warning while opening the ready archive with upload account disconnected', async () => {
    const plannedSource = { ...source, planId: 'selected-plan', unitId: 'unit' };
    const description = { source, title: 'Synthetic', planId: 'selected-plan', coverage: { state: 'complete' as const, captureWarning }, parts: [{ source: plannedSource, partIndex: null, durationSeconds: 44, mediaOffsetSeconds: 0, bytes: 100 }] };
    const client = api(state({ account: { configured: true, connected: false, authorizing: false }, jobs: [job({ source: plannedSource, state: 'ready', videoId: 'abcdefghijk' })] }));
    vi.mocked(client.getPlan).mockResolvedValue(description); replay.describe.mockResolvedValue(description);
    await render(client);
    expect(document.body.textContent).toContain(captureWarning);
    await click('아츠미에서 재생');
    expect(document.querySelector('[data-remote-video="abcdefghijk"]')).not.toBeNull();
    expect(client.enqueueRecording).not.toHaveBeenCalled(); expect(client.authorize).not.toHaveBeenCalled();
  });
  it('defaults new uploads to private while preserving every prior job visibility', async () => {
    const client = api(state({ jobs: [job({ id: 'legacy' }), job({ id: 'private', visibility: 'private' }), job({ id: 'unlisted', visibility: 'unlisted' })] }));
    await render(client);
    expect(button('비공개 업로드')).toBeDefined();
    const labels = [...document.querySelectorAll('[aria-label="업로드 공개 범위"]')].map(node => node.textContent);
    expect(labels).toEqual(['비공개', '비공개', '일부 공개 · 링크로 시청']);
    expect(document.querySelector<HTMLSelectElement>('[aria-label="이번 업로드 공개 범위"]')?.value).toBe('private');
    expect(client.enqueueRecording).not.toHaveBeenCalled(); expect(client.resume).not.toHaveBeenCalled();
  });
  it('offers exact automatic selection while recording without trying source preparation or manual upload', async () => {
    const current = state({ automatic: { revision: 7, rules: [] } });
    const client = api(current);
    await act(async () => root.render(<CloudRecordingDialog source={source} title="Synthetic" privacyMode={false} pendingRecording onClose={() => {}} api={client} />));
    expect(client.describe).not.toHaveBeenCalled(); expect(button('비공개 업로드').disabled).toBe(true);
    const choice = document.querySelector<HTMLInputElement>('[aria-label="Synthetic 이 녹화 자동 업로드"]')!;
    expect(choice.disabled).toBe(false);
    await act(async () => choice.click());
    expect(client.setAutomatic).toHaveBeenCalledExactlyOnceWith({ kind: 'recording', provider: 'soop', recordingId: source.recordingId }, true, 7, 'private');
    expect(client.enqueueRecording).not.toHaveBeenCalled(); expect(document.body.textContent).toContain('녹화 완료 대기');
    await act(async () => root.render(<CloudRecordingDialog source={source} title="Synthetic" privacyMode={false} pendingRecording={false} onClose={() => {}} api={client} />));
    expect(client.describe).toHaveBeenCalledExactlyOnceWith(source); expect(client.enqueueRecording).not.toHaveBeenCalled();
  });
  it('requires setup and account consent before allowing upload', async () => {
    const client = api(state({ account: { configured: false, connected: false, authorizing: false } }));
    await render(client);
    expect(button('비공개 업로드').disabled).toBe(true);
    expect(button('계정 연결').disabled).toBe(true);
    expect(button('설정 파일 선택').disabled).toBe(false);
    await click('설정 파일 선택'); expect(client.importClient).toHaveBeenCalledOnce();
    expect(client.enqueueRecording).not.toHaveBeenCalled();
  });
  it('queues the whole owned recording atomically with no renderer paths or partial enqueue loop', async () => {
    const client = api(); await render(client); await click('비공개 업로드');
    expect(client.enqueueRecording).toHaveBeenCalledExactlyOnceWith(source, 'private');
    expect(client.enqueue).not.toHaveBeenCalled();
    expect(button('계정 연결').disabled).toBe(true);
    expect(document.body.textContent).toContain('로컬 원본·채팅·후원 기록 유지');
  });
  it('allows a long recording to enter tracked preparation before upload units exist', async () => {
    const client = api();
    vi.mocked(client.describe).mockResolvedValue({ source, title: 'Synthetic', preparationRequired: true, durationSeconds: 46800, bytes: 1000, coverage: { state: 'complete' }, parts: [] });
    vi.mocked(client.enqueueRecording).mockResolvedValue(state({ jobs: [job({ recordingPlan: true, state: 'validating', preparation: { phase: 'checking', completedBytes: 10, totalBytes: 100 } })] }));
    await render(client);
    expect(document.body.textContent).toContain('13:00:00');
    expect(button('비공개 업로드').disabled).toBe(false);
    await click('비공개 업로드');
    expect(client.enqueueRecording).toHaveBeenCalledExactlyOnceWith(source, 'private');
    expect(client.enqueue).not.toHaveBeenCalled();
    expect(button('비공개 업로드').disabled).toBe(true);
    expect(document.body.textContent).toContain('저장 구간 확인 중');
  });
  it('does not upload a verified prefix as if an incomplete recording were whole', async () => {
    const client = api();
    vi.mocked(client.describe).mockResolvedValue({ source, title: 'Synthetic', coverage: { state: 'incomplete', expectedSegments: 12, coveredSegments: 10, missingFromSegment: 10, reason: '마지막 2개 구간을 확인하지 못했습니다.' }, parts: [{ partIndex: 0, durationSeconds: 30, mediaOffsetSeconds: 0, bytes: 100 }] });
    await render(client);
    expect(button('비공개 업로드').disabled).toBe(true);
    expect(document.body.textContent).toContain('마지막 2개 구간을 확인하지 못했습니다.');
    await click('비공개 업로드');
    expect(client.enqueueRecording).not.toHaveBeenCalled();
  });
  it('retries a failed preparation through its existing job instead of a duplicate upload', async () => {
    const client = api(state({ jobs: [job({ recordingPlan: true, state: 'failed', error: { code: 'PREPARATION_SPACE', message: '준비 공간 부족', retryable: true } })] }));
    vi.mocked(client.describe).mockResolvedValue({ source, title: 'Synthetic', preparationRequired: true, parts: [] });
    await render(client);
    expect(button('비공개 업로드').disabled).toBe(true);
    await click('다시 시도');
    expect(client.resume).toHaveBeenCalledExactlyOnceWith('job');
    expect(client.enqueueRecording).not.toHaveBeenCalled();
  });
  it('recovers an initial source lookup without closing the dialog', async () => {
    const client = api();
    vi.mocked(client.describe).mockRejectedValueOnce(new Error('저장 구간 조회 실패'));
    await render(client);
    expect(button('비공개 업로드').disabled).toBe(true);
    await click('다시 확인');
    expect(document.body.textContent).not.toContain('저장 구간 조회 실패');
    expect(button('비공개 업로드').disabled).toBe(false);
  });
  it('retries the immutable manifest after a temporary failure', async () => {
    const plannedSource = { ...source, planId: 'selected-plan', unitId: 'unit' };
    const client = api(state({ jobs: [job({ source: plannedSource, state: 'ready', videoId: 'abcdefghijk' })] }));
    vi.mocked(client.getPlan).mockRejectedValueOnce(new Error('구간 계획 조회 실패')).mockResolvedValue({ source, title: 'Synthetic', planId: 'selected-plan', parts: [{ source: plannedSource, partIndex: null, durationSeconds: 44, mediaOffsetSeconds: 0, bytes: 100 }] });
    replay.describe.mockResolvedValue({ source, title: 'Synthetic', planId: 'selected-plan', parts: [{ source: plannedSource, partIndex: null, durationSeconds: 44, mediaOffsetSeconds: 0, bytes: 100 }] });
    await render(client);
    expect(document.body.textContent).toContain('구간 계획 조회 실패');
    await click('다시 확인');
    expect(document.body.textContent).not.toContain('구간 계획 조회 실패');
    await click('아츠미에서 재생');
    expect(document.querySelector('[data-remote-video="abcdefghijk"]')).not.toBeNull();
  });
  it('refuses a different saved plan when opening an already uploaded recording', async () => {
    const selected = job({ state: 'ready', videoId: 'abcdefghijk', source: { ...source, planId: 'selected-plan', unitId: 'unit' } });
    const client = api(state({ jobs: [selected] }));
    vi.mocked(client.getPlan).mockResolvedValue({ source, title: 'Synthetic', planId: 'different-plan', parts: [] });
    replay.describe.mockResolvedValue({ source, title: 'Synthetic', planId: 'different-plan', parts: [] });
    await render(client); await click('아츠미에서 재생');
    expect(client.getPlan).toHaveBeenCalledWith('selected-plan');
    expect(document.querySelector('[data-remote-video]')).toBeNull();
    expect(document.body.textContent).toContain('업로드 구간을 확인하지 못했습니다.');
  });
  it('never offers remote playback for an uploaded but unprocessed video', async () => {
    const client = api(state({ jobs: [job({ state: 'processing', videoId: 'abcdefghijk', uploadedBytes: 100 })] }));
    await render(client); expect(button('아츠미에서 재생')).toBeUndefined();
    expect(document.body.textContent).toContain('YouTube 처리 중');
  });
  it('shows actual source preparation progress separately from upload acknowledgment', async () => {
    const client = api(state({ jobs: [job({ state: 'validating', preparation: { phase: 'preparing', completedSeconds: 30, totalSeconds: 120 } })] }));
    await render(client);
    expect(document.body.textContent).toContain('업로드 영상 준비 중');
    expect(document.body.textContent).toContain('00:00:30 / 00:02:00');
    expect(document.querySelector<HTMLProgressElement>('[aria-label="업로드 준비 진행"]')?.value).toBe(25);
    expect(document.querySelector('[aria-label="업로드 진행"]')).toBeNull();
    expect(button('아츠미에서 재생')).toBeUndefined();
  });
  it('keeps API readiness explicitly separate from playback verification', async () => {
    const client = api(state({ jobs: [job({ state: 'ready', videoId: 'abcdefghijk' })] }));
    await render(client); expect(document.body.textContent).toContain('처리 완료 · 재생 확인 필요');
    await click('아츠미에서 재생'); expect(document.querySelector('[data-remote-video="abcdefghijk"]')).not.toBeNull();
  });
  it('opens a verified legacy archive while local video is unavailable and upload OAuth is disconnected', async () => {
    const client = api(state({ account: { configured: true, connected: false, authorizing: false }, jobs: [job({ state: 'ready', videoId: 'abcdefghijk', visibility: 'unlisted' })] }));
    vi.mocked(client.describe).mockRejectedValue(new Error('local media missing'));
    await render(client);
    expect(replay.describe).toHaveBeenCalledWith('job');
    expect(client.describe).not.toHaveBeenCalled();
    expect(document.body.textContent).not.toContain('local media missing');
    expect(button('비공개 업로드').disabled).toBe(true);
    await click('아츠미에서 재생');
    expect(document.querySelector('[data-remote-video="abcdefghijk"]')).not.toBeNull();
    expect(client.authorize).not.toHaveBeenCalled(); expect(client.enqueueRecording).not.toHaveBeenCalled();
  });
  it('pauses queued work without closing the dialog or deleting local media', async () => {
    const client = api(state({ jobs: [job()] })); await render(client); await click('일시정지');
    expect(client.pause).toHaveBeenCalledWith('job'); expect(client.disconnect).not.toHaveBeenCalled();
    expect(document.querySelector('[aria-label="YouTube 보관 닫기"]')).not.toBeNull();
  });
  it('privacy hides channel/title and prevents new account or upload actions', async () => {
    const client = api(); await render(client, true);
    expect(document.body.textContent).not.toContain('업로드 채널'); expect(document.body.textContent).not.toContain('Synthetic');
    expect(button('비공개 업로드').disabled).toBe(true); expect(button('연결 해제').disabled).toBe(true);
  });
  it('filters jobs by provider and never sends another recording through the selected action', async () => {
    const client = api(state({ jobs: [job({ source: { ...source, provider: 'youtube' } })] })); await render(client);
    expect(button('일시정지')).toBeUndefined(); expect(button('비공개 업로드').disabled).toBe(false);
  });
  it('clears a recovered poll failure without erasing a failed upload action', async () => {
    const client = api();
    vi.mocked(client.snapshot).mockRejectedValueOnce(new Error('일시적 조회 실패'));
    await render(client); expect(document.body.textContent).toContain('일시적 조회 실패');
    await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
    expect(document.body.textContent).not.toContain('일시적 조회 실패');
    vi.mocked(client.enqueueRecording).mockRejectedValueOnce(new Error('업로드 요청 실패'));
    await click('비공개 업로드');
    await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
    expect(document.body.textContent).toContain('업로드 요청 실패');
  });
});

describe('native OAuth file picker return focus', () => {
  const disconnected = () => state({ account: { configured: false, connected: false, authorizing: false } });
  const pendingPicker = () => {
    const snapshot = disconnected(), client = api(snapshot);
    let resolve!: (value: CloudSnapshot) => void, reject!: (cause: Error) => void;
    vi.mocked(client.importClient).mockReturnValue(new Promise<CloudSnapshot>((done, fail) => { resolve = done; reject = fail; }));
    return { client, snapshot, resolve: () => resolve(snapshot), reject };
  };
  const openPicker = async () => {
    const trigger = button('설정 파일 선택');
    trigger.focus();
    await click('설정 파일 선택');
    expect(trigger.disabled).toBe(true);
    // jsdom keeps disabled controls focused and refuses blur(). Model the native
    // dialog taking focus, then disappearing, using a temporary focusable node.
    const nativeDialog = document.createElement('button');
    document.body.append(nativeDialog); nativeDialog.focus(); nativeDialog.remove();
    expect(document.activeElement).toBe(document.body);
    return trigger;
  };

  it('restores the enabled picker trigger after cancel so immediate Escape reaches the modal', async () => {
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    const pending = pendingPicker(), onClose = vi.fn();
    await act(async () => root.render(<CloudRecordingDialog source={source} title="Synthetic" privacyMode={false} onClose={onClose} api={pending.client} />));
    const trigger = await openPicker();
    await act(async () => pending.resolve());
    expect(trigger.disabled).toBe(false);
    expect(document.activeElement).toBe(trigger);
    await act(async () => document.activeElement?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
    expect(onClose).toHaveBeenCalledOnce();
    expect(pending.client.authorize).not.toHaveBeenCalled();
    expect(pending.client.enqueueRecording).not.toHaveBeenCalled();
  });

  it('preserves another control already focused when the native dialog returns', async () => {
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    const pending = pendingPicker(); await render(pending.client); await openPicker();
    const close = document.querySelector<HTMLButtonElement>('[aria-label="YouTube 보관 닫기"]')!;
    close.focus();
    await act(async () => pending.resolve());
    expect(document.activeElement).toBe(close);
  });

  it('does not restore focus while another app is active or defer it until the app returns', async () => {
    const foreground = vi.spyOn(document, 'hasFocus').mockReturnValue(false);
    const pending = pendingPicker(); await render(pending.client); await openPicker();
    await act(async () => pending.resolve());
    expect(document.activeElement).toBe(document.body);
    foreground.mockReturnValue(true);
    await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
    expect(document.activeElement).toBe(document.body);
  });

  it.each(['inactive', 'privacy', 'source'] as const)('discards restoration after a %s transition even when the original view returns before completion', async kind => {
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    const pending = pendingPicker(); await render(pending.client); await openPicker();
    await act(async () => root.render(<CloudRecordingDialog source={kind === 'source' ? { ...source, recordingId: 'another-recording' } : source} title="Synthetic" active={kind !== 'inactive'} privacyMode={kind === 'privacy'} onClose={() => {}} api={pending.client} />));
    await render(pending.client);
    (document.activeElement as HTMLElement | null)?.blur();
    await act(async () => pending.resolve());
    expect(document.activeElement).toBe(document.body);
  });

  it('does not focus a removed dialog after its picker completes', async () => {
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    const pending = pendingPicker(); await render(pending.client); const trigger = await openPicker();
    await act(async () => root.render(<div>다른 화면</div>));
    await act(async () => pending.resolve());
    expect(trigger.isConnected).toBe(false);
    expect(document.activeElement).toBe(document.body);
    expect(document.body.textContent).toContain('다른 화면');
  });

  it('restores orphaned focus after a picker failure while retaining the error', async () => {
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    const pending = pendingPicker(); await render(pending.client); const trigger = await openPicker();
    await act(async () => pending.reject(new Error('파일 선택 실패')));
    expect(trigger.disabled).toBe(false);
    expect(document.activeElement).toBe(trigger);
    expect(document.body.textContent).toContain('파일 선택 실패');
  });

  it('restores focus after an immediate cancel response without requiring a visible busy render', async () => {
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    const snapshot = disconnected(), client = api(snapshot); await render(client);
    const trigger = button('설정 파일 선택'); trigger.focus();
    vi.mocked(client.importClient).mockImplementation(async () => { trigger.blur(); return snapshot; });
    await click('설정 파일 선택');
    expect(trigger.disabled).toBe(false);
    expect(document.activeElement).toBe(trigger);
  });
});
