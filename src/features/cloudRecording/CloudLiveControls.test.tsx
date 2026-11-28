import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CloudLiveApi, CloudLiveOperation, CloudLiveSnapshot } from '../../api/cloudRecording';
import { CloudLiveControls, liveStatusLabel, liveRecordWarnings, canCheckLiveArchive } from './CloudLiveControls';

const operation = (changes: Partial<CloudLiveOperation> = {}): CloudLiveOperation => ({ selectionId: 'native-selection', revision: 3,
  provider: 'soop', channelId: 'watching', state: 'selected', outcomeUncertain: false, stopRequested: false, interrupted: false,
  segmentAcks: 0, playlistAcks: 0, endListAcked: false, managerJoined: false, workerJoined: false, sidecarJoined: false, archiveReady: false, receiptPersisted: true, assetsIncomplete: false, timingIncomplete: false, ...changes });
const snapshot = (operations: CloudLiveOperation[] = []): CloudLiveSnapshot => ({ account: { configured: true, connected: true, authorizing: false, generation: 4, channelName: '보관 계정' }, operations });
function client(): CloudLiveApi {
  return { snapshot: vi.fn().mockResolvedValue(snapshot()), sources: vi.fn().mockResolvedValue(['watching']),
    importClient: vi.fn(), authorize: vi.fn().mockResolvedValue(snapshot()), disconnect: vi.fn().mockResolvedValue(snapshot()),
    select: vi.fn().mockResolvedValue(snapshot([operation()])), start: vi.fn().mockResolvedValue(snapshot([operation({ state: 'preparing', revision: 4 })])),
    stop: vi.fn().mockResolvedValue(snapshot([operation({ state: 'stopping', stopRequested: true, revision: 5 })])),
    verifyArchive: vi.fn().mockResolvedValue(snapshot()) };
}
let root: Root, container: HTMLDivElement;
const channels = [{ id: 'watching', name: '시청 방송' }, { id: 'offline', name: '즐겨찾기만 등록된 방송' }];
const button = (text: string) => [...container.querySelectorAll<HTMLButtonElement>('button')].find(value => value.textContent === text)!;
const render = async (api: CloudLiveApi, privacy = false, provider: 'soop' | 'chzzk' = 'soop') => {
  await act(async () => root.render(<CloudLiveControls provider={provider} channels={channels} active privacy={privacy} api={api} />));
};
async function expand(open = true) {
  await act(async () => { const details = container.querySelector('details')!; details.open = open; details.dispatchEvent(new Event('toggle')); });
}
async function choose(index: number, value: string) {
  await act(async () => { const select = container.querySelectorAll('select')[index]!; select.value = value; select.dispatchEvent(new Event('change', { bubbles: true })); });
}
beforeEach(() => { vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true); vi.useFakeTimers(); container = document.createElement('div'); document.body.append(container); root = createRoot(container); });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

describe('explicit selected live transfer', () => {
  it('does no work while collapsed and never creates/authenticates when viewing or polling', async () => {
    const api = client(); await render(api);
    expect(api.snapshot).not.toHaveBeenCalled(); expect(api.sources).not.toHaveBeenCalled();
    await expand(); await act(async () => vi.advanceTimersByTimeAsync(6100));
    expect(api.snapshot).toHaveBeenCalledTimes(3); expect(api.sources).toHaveBeenCalledWith('soop');
    expect(api.select).not.toHaveBeenCalled(); expect(api.start).not.toHaveBeenCalled(); expect(api.authorize).not.toHaveBeenCalled();
    expect(container.textContent).not.toContain('즐겨찾기만 등록된 방송');
    expect(button('전송 시작')).toBeDisabled();
  });
  it('requires an explicit audience and binds start to the selected native receipt/revision', async () => {
    const api = client(); await render(api); await expand(); await choose(0, 'watching');
    expect(button('전송 시작')).toBeDisabled();
    await choose(1, 'no'); expect(button('전송 시작')).not.toBeDisabled();
    await act(async () => button('전송 시작').click());
    expect(api.select).toHaveBeenCalledExactlyOnceWith('soop', 'watching');
    expect(api.start).toHaveBeenCalledExactlyOnceWith('native-selection', 3, { title: '시청 방송', scheduledStartMs: expect.any(Number), madeForKids: false });
    expect(container.textContent).toContain('YouTube 전송 준비 중');
  });
  it('does not continue a compound start when the panel closes during native selection', async () => {
    const api = client(); let resolve!: (value: CloudLiveSnapshot) => void;
    vi.mocked(api.select).mockImplementation(() => new Promise(done => { resolve = done; }));
    await render(api); await expand(); await choose(0, 'watching'); await choose(1, 'no');
    await act(async () => button('전송 시작').click()); await expand(false);
    await act(async () => resolve(snapshot([operation()])));
    expect(api.start).not.toHaveBeenCalled(); expect(api.stop).not.toHaveBeenCalled();
  });
  it('does not start if the destination owner changed during source selection', async () => {
    const api = client(); const changed = snapshot([operation()]); changed.account.generation = 5;
    vi.mocked(api.select).mockResolvedValue(changed);
    await render(api); await expand(); await choose(0, 'watching'); await choose(1, 'no');
    await act(async () => button('전송 시작').click());
    expect(api.start).not.toHaveBeenCalled(); expect(container.textContent).toContain('연결 계정이 변경됐습니다');
  });
  it('keeps an uncertain result visible without retrying or calling it archived', async () => {
    const api = client(); await render(api); await expand(); await choose(0, 'watching'); await choose(1, 'no');
    vi.mocked(api.start).mockRejectedValue({ message: '응답을 확인하지 못했습니다.' });
    vi.mocked(api.snapshot).mockResolvedValue(snapshot([operation({ state: 'recovery_required', outcomeUncertain: true, endListAcked: true, managerJoined: true })]));
    await act(async () => button('전송 시작').click()); await act(async () => vi.advanceTimersByTimeAsync(6000));
    expect(api.start).toHaveBeenCalledTimes(1); expect(container.textContent).toContain('전송 결과 확인 필요');
    expect(button('전송 시작')).toBeDisabled();
    expect(container.textContent).not.toContain('보관됨');
  });
  it('disables a source that disappears from the native view list', async () => {
    const api = client(); await render(api); await expand(); await choose(0, 'watching'); await choose(1, 'yes');
    vi.mocked(api.sources).mockResolvedValue([]); await act(async () => vi.advanceTimersByTimeAsync(3000));
    expect(button('전송 시작')).toBeDisabled(); expect(api.start).not.toHaveBeenCalled();
  });
  it('privacy hides identity/status and cancels a pending compound start', async () => {
    const api = client(); let resolve!: (value: CloudLiveSnapshot) => void;
    vi.mocked(api.select).mockImplementation(() => new Promise(done => { resolve = done; }));
    await render(api); await expand(); await choose(0, 'watching'); await choose(1, 'no');
    await act(async () => button('전송 시작').click()); await render(api, true);
    await act(async () => resolve(snapshot([operation()])));
    expect(api.start).not.toHaveBeenCalled(); expect(container.textContent).not.toContain('시청 방송');
    expect(container.textContent).not.toContain('보관 계정'); expect(button('전송 시작')).toBeDisabled();
  });
  it('preserves keyboard focus through polling and stops only the explicit receipt', async () => {
    const api = client(); vi.mocked(api.snapshot).mockResolvedValue(snapshot([operation({ state: 'streaming', revision: 11 })]));
    await render(api); await expand(); await choose(0, 'watching');
    const title = container.querySelector('input')!; title.focus();
    await act(async () => vi.advanceTimersByTimeAsync(3000)); expect(document.activeElement).toBe(title);
    await act(async () => button('전송 중지').click());
    expect(api.stop).toHaveBeenCalledExactlyOnceWith('native-selection', 11);
    await expand(false); expect(api.stop).toHaveBeenCalledTimes(1);
  });
  it('keeps provider operation states separate and never upgrades an upload account implicitly', async () => {
    const api = client(); const value = snapshot([operation()]); value.account.connected = false;
    vi.mocked(api.snapshot).mockResolvedValue(value);
    await render(api, false, 'chzzk'); await expand();
    expect(container.querySelector('.cloud-live-operations')).toBeNull(); expect(api.sources).toHaveBeenCalledWith('chzzk');
    expect(button('연결 해제')).toBeDisabled(); expect(button('실시간 전송 연결')).not.toBeDisabled();
    expect(api.authorize).not.toHaveBeenCalled();
    await act(async () => button('실시간 전송 연결').click()); expect(api.authorize).toHaveBeenCalledTimes(1);
  });
  it('never treats local segment/end-list ACKs as a verified remote archive', () => {
    expect(liveStatusLabel(operation({ state: 'joined', segmentAcks: 30, playlistAcks: 31, endListAcked: true, managerJoined: true, workerJoined: true, sidecarJoined: true }))).toBe('전송 종료 · 보관 확인 전');
  });
  it('keeps missing historical assets and timing separate from video transfer success', async () => {
    const partial = operation({ state: 'joined', managerJoined: true, sidecarJoined: true, sidecarTextState: 'partial', assetsIncomplete: true, timingIncomplete: true });
    const api = client(); vi.mocked(api.snapshot).mockResolvedValue(snapshot([partial]));
    await render(api); await expand();
    expect(container.textContent).toContain('전송 종료 · 보관 확인 전');
    for (const warning of ['채팅 일부 누락', '이미지 일부 누락', '동기화 정보 일부 누락']) expect(container.textContent).toContain(warning);
    expect(api.start).not.toHaveBeenCalled(); expect(api.stop).not.toHaveBeenCalled();
    expect(liveRecordWarnings(operation({ managerJoined: true, sidecarTextState: 'unknown' }))).toEqual(['채팅 기록 확인 필요']);
    expect(liveRecordWarnings(operation({ sidecarTextState: 'complete' }))).toEqual([]);
  });
  it('shows metadata processing separately and never offers a fabricated replay', async () => {
    const api = client(); const row = operation({ state: 'joined', managerJoined: true, workerJoined: true, sidecarJoined: true,
      sidecarTextState: 'partial', assetsIncomplete: true, archiveVerification: { state: 'metadata_verified', deadlineAt: 1234, rounds: 2,
        processingInfoUnavailable: false, playbackVerified: false, remoteTimeOrigin: 'unknown', synchronizedReplayReady: false } });
    vi.mocked(api.snapshot).mockResolvedValue(snapshot([row]));
    await render(api); await expand();
    expect(container.textContent).toContain('영상 처리 완료 · 재생·동기화 확인 전');
    expect(container.textContent).toContain('채팅 일부 누락'); expect(container.textContent).toContain('이미지 일부 누락');
    expect(container.textContent).not.toContain('보관됨'); expect(container.textContent).not.toContain('아츠미에서 재생');
    expect(api.verifyArchive).not.toHaveBeenCalled();
  });
  it('only rechecks an explicit joined receipt and does not retry from polling or reopening', async () => {
    const api = client(); const row = operation({ state: 'joined', revision: 18, managerJoined: true, workerJoined: true, sidecarJoined: true,
      runId: 'run', destinationChannelId: 'owner', broadcastId: 'video', archiveVerification: { state: 'needs_check', deadlineAt: 1234, rounds: 2,
        processingInfoUnavailable: false, playbackVerified: false, remoteTimeOrigin: 'unknown', synchronizedReplayReady: false } });
    const value = snapshot([row]); value.account.channelId = 'owner';
    vi.mocked(api.snapshot).mockResolvedValue(value); vi.mocked(api.verifyArchive).mockResolvedValue(value);
    await render(api); await expand(); await act(async () => vi.advanceTimersByTimeAsync(6000));
    expect(api.verifyArchive).not.toHaveBeenCalled();
    await act(async () => button('보관 확인').click());
    expect(api.verifyArchive).toHaveBeenCalledExactlyOnceWith('native-selection', 18);
    await expand(false); await expand(); expect(api.verifyArchive).toHaveBeenCalledTimes(1);
    expect(canCheckLiveArchive(row, { ...value.account, channelId: 'different-owner' })).toBe(false);
    expect(canCheckLiveArchive({ ...row, receiptPersisted: false }, value.account)).toBe(false);
    expect(canCheckLiveArchive({ ...row, outcomeUncertain: true }, value.account)).toBe(false);
    expect(canCheckLiveArchive({ ...row, workerJoined: false }, value.account)).toBe(false);
    for (const state of ['checking', 'pending'] as const) expect(canCheckLiveArchive({ ...row, archiveVerification: { ...row.archiveVerification!, state } }, value.account)).toBe(false);
    await render(api, true); expect(container.textContent).not.toContain('보관 확인');
  });
});
