import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CloudLiveOperation } from '../../api/cloudRecording';
import type { CloudLiveReplayApi, LiveReplayDescriptor, LiveReplayPage, LiveReplaySnapshot } from '../../api/cloudLiveReplay';
import type { YouTubeVodClock } from '../../api/youtubeOriginal';
import { CloudLiveReplay, sameLivePageClock } from './CloudLiveReplay';
import { canReplayLiveArchive } from './cloudLiveReplayModel';
const seen = vi.hoisted(() => ({ player: null as any, soop: null as any, chzzk: null as any }));
vi.mock('../youtube/YouTubeOriginalPlayer', () => ({ YouTubeOriginalPlayer: (props: any) => { seen.player = props; return <div data-official-video={props.channelId} />; } }));
vi.mock('../platforms/PlatformReplayChat', () => ({ PlatformReplayChat: (props: any) => { seen.soop = props; return <div data-original-soop />; } }));
vi.mock('../streaming/RecordingReplayChat', () => ({ LiveReplayChzzkChat: (props: any) => { seen.chzzk = props; return <div data-original-chzzk />; } }));
const operation: CloudLiveOperation = { selectionId: 'selected', revision: 8, provider: 'soop', channelId: 'channel', broadcastId: 'abcdefghijk', state: 'joined',
  outcomeUncertain: false, receiptPersisted: true, stopRequested: true, interrupted: false, segmentAcks: 10, playlistAcks: 11, endListAcked: true,
  managerJoined: true, workerJoined: true, sidecarJoined: true, archiveReady: false, assetsIncomplete: false, timingIncomplete: false,
  archiveVerification: { state: 'metadata_verified', videoId: 'abcdefghijk', durationSeconds: 42, embeddable: true, deadlineAt: 9999, rounds: 2,
    processingInfoUnavailable: false, playbackVerified: false, remoteTimeOrigin: 'unknown', synchronizedReplayReady: false } };
const clock = (extra: Partial<YouTubeVodClock> = {}): YouTubeVodClock => ({ token: 'owned', videoId: 'abcdefghijk', documentEpoch: 1, sourceId: 'native-source', sourceGeneration: 1, seekGeneration: 1, sequence: 1,
  currentTime: 12, duration: 42, valid: true, adState: 'content', paused: false, seeking: false, buffering: false, ended: false, playbackRate: 1, ageMs: 0, ...extra });
const descriptor: LiveReplayDescriptor = { token: 'owned', selectionId: 'selected', provider: 'soop', videoId: 'abcdefghijk', durationSeconds: 42,
  player: { token: 'owned', channelId: 'abcdefghijk', broadcastId: 'abcdefghijk', state: 'playing', authenticated: true, chatVisible: false, captureReady: false, recordingJob: null, recordingStopping: false, fullscreen: false, error: null } };
const snapshot = (extra: Partial<LiveReplaySnapshot> = {}): LiveReplaySnapshot => ({ ...descriptor, clock: clock(), alignmentState: 'unknown', historyState: 'ready', historyItems: 3, untimedItems: 3, historyIncomplete: false, error: null, ...extra });
const page = (provider: 'soop' | 'chzzk' = 'soop'): LiveReplayPage => ({ provider, page: { items: [], state: 'unmapped', cursor: null, previousCursor: null, total: 3, timelineQuality: 'unknown', malformedLines: 0, unsupportedItems: 0, reordered: 0, duplicates: 0, partialFile: false },
  positions: [], nativeClock: null, alignmentState: 'unknown', cursorGeneration: null, untimedCount: 3, sourceCount: 3, incomplete: false } as LiveReplayPage);
function api(): CloudLiveReplayApi { return { open: vi.fn().mockResolvedValue(descriptor), cancelOpen: vi.fn().mockResolvedValue(undefined), snapshot: vi.fn().mockResolvedValue(snapshot()), viewport: vi.fn(), control: vi.fn(), fullscreen: vi.fn(), controls: vi.fn(), close: vi.fn().mockResolvedValue(undefined), measure: vi.fn().mockResolvedValue(snapshot({ alignmentState: 'measuring' })), page: vi.fn().mockResolvedValue(page()), asset: vi.fn().mockResolvedValue(null) }; }
let root: Root, container: HTMLDivElement;
beforeEach(() => { vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true); vi.useFakeTimers(); seen.player = null; seen.chzzk = null; seen.soop = null; container = document.createElement('div'); document.body.append(container); root = createRoot(container); });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.useRealTimers(); vi.unstubAllGlobals(); });
const render = async (client: CloudLiveReplayApi, value = operation, privacyMode = false) => act(async () => root.render(<CloudLiveReplay operation={value} title="시험 방송" active privacyMode={privacyMode} onClose={() => {}} api={client} />));
const open = async () => act(async () => { await seen.player.api.open('abcdefghijk', 'request', 'vod'); });
const button = (text: string) => [...document.querySelectorAll('button')].find(value => value.textContent === text)!;
describe('native Live archive replay admission and history', () => {
  it('requires exact completed processing and joined original records, without calling a source or creating another upload', async () => {
    expect(canReplayLiveArchive(operation)).toBe(true);
    for (const changes of [{ receiptPersisted: false }, { outcomeUncertain: true }, { sidecarJoined: false }, { broadcastId: 'other' },
      { archiveVerification: { ...operation.archiveVerification!, state: 'pending' as const } }, { archiveVerification: { ...operation.archiveVerification!, durationSeconds: NaN } }]) {
      expect(canReplayLiveArchive({ ...operation, ...changes })).toBe(false);
    }
    const client = api(); await render(client, { ...operation, archiveVerification: { ...operation.archiveVerification!, state: 'pending' } });
    expect(seen.player).toBeNull(); expect(client.open).not.toHaveBeenCalled(); expect(client.page).not.toHaveBeenCalled(); expect(client.measure).not.toHaveBeenCalled();
  });
  it('retains original SOOP history and nullable timing before explicit measurement', async () => {
    const client = api(); await render(client); await open();
    await act(async () => seen.soop.api.page('owned', 999999, null, 120));
    expect(client.page).toHaveBeenLastCalledWith('owned', 'history', null, 120);
    expect(seen.soop.provider).toBe('soop'); expect(client.measure).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain('동기화 확인 전');
    await act(async () => button('동기화 확인').click());
    expect(client.measure).toHaveBeenCalledExactlyOnceWith('owned'); expect(document.body.textContent).toContain('재생 장면 비교 중');
    expect(client.control).not.toHaveBeenCalled();
  });
  it('current pages use native time even if a child supplies a wildly different renderer time', async () => {
    const client = api(); vi.mocked(client.snapshot).mockResolvedValue(snapshot({ alignmentState: 'measured', untimedItems: 1 }));
    await render(client); await open(); await act(async () => { await seen.player.api.snapshot('owned'); seen.player.onClock(clock()); });
    await act(async () => button('재생 위치').click());
    await act(async () => seen.soop.api.page('owned', 999999, 'old-history', 120));
    expect(client.page).toHaveBeenLastCalledWith('owned', 'current', null, 120);
    expect(document.body.textContent).toContain('장면 기준 동기화');
    await act(async () => button('시각 미확인').click()); await act(async () => seen.soop.api.page('owned', 999999, 'r:120', 120));
    expect(client.page).toHaveBeenLastCalledWith('owned', 'untimed', 'r:120', 120);
  });
  it('a stale or buffering clock does not rewind the chat or expose all future history', async () => {
    const client = api(); await render(client); await open(); await act(async () => { seen.player.onClock(clock()); button('재생 위치').click(); });
    expect(seen.soop.currentTime).toBe(12);
    await act(async () => seen.player.onClock(null));
    expect(seen.soop.currentTime).toBe(12);
    await act(async () => seen.soop.api.page('owned', 12_000, null, 120));
    expect(client.page).toHaveBeenLastCalledWith('owned', 'current', null, 120);
  });
  it('rejects a response crossing a seek or a provider boundary', async () => {
    const client = api(); await render(client); await open(); await act(async () => seen.player.onClock(clock({ seekGeneration: 2 })));
    vi.mocked(client.page).mockResolvedValue({ ...page(), cursorGeneration: { documentEpoch: 1, sourceId: 'native-source', sourceGeneration: 1, seekGeneration: 1 } });
    await expect(seen.soop.api.page('owned', 5, null, 120)).rejects.toThrow('재생 위치');
    vi.mocked(client.page).mockResolvedValue(page('chzzk'));
    await expect(seen.soop.api.page('owned', 5, null, 120)).rejects.toThrow('재생 위치');
    expect(sameLivePageClock(page(), null)).toBe(true);
  });
  it('hides stored rows and controls in privacy without sending pause or measurement commands', async () => {
    const client = api(); await render(client); await open(); await render(client, operation, true);
    expect(document.body.textContent).not.toContain('시험 방송'); expect(document.body.textContent).not.toContain('동기화 확인');
    expect(document.querySelector('[data-original-soop]')).toBeNull(); expect(client.control).not.toHaveBeenCalled(); expect(client.measure).not.toHaveBeenCalled();
    expect(seen.player.privacyMode).toBe(true);
  });
});
