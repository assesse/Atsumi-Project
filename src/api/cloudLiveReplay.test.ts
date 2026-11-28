import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cloudLiveReplayApi, liveReplayPlayerApi, type CloudLiveReplayApi, type LiveReplayDescriptor, type LiveReplaySnapshot } from './cloudLiveReplay';
import type { YouTubeVodClock } from './youtubeOriginal';

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), listeners: new Map<string, (event: { payload: any }) => void>() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async (event, receive) => { mocks.listeners.set(event, receive); return () => mocks.listeners.delete(event); }) }));
const descriptor = (patch: Partial<LiveReplayDescriptor> = {}): LiveReplayDescriptor => ({
  token: 'owned-live', selectionId: 'selection', provider: 'chzzk', videoId: 'abcdefghijk', durationSeconds: 59,
  player: { token: 'owned-live', channelId: 'abcdefghijk', broadcastId: 'abcdefghijk', state: 'open', authenticated: null, chatVisible: false, captureReady: false, recordingJob: null, recordingStopping: false, fullscreen: false, error: null }, ...patch,
});
const clock = (patch: Partial<YouTubeVodClock> = {}): YouTubeVodClock => ({ token: 'owned-live', videoId: 'abcdefghijk', documentEpoch: 1, sourceId: 'source', sourceGeneration: 1, sequence: 1, seekGeneration: 1, currentTime: null, duration: null, paused: true, seeking: false, playbackRate: 1, buffering: false, ended: false, adState: 'unknown', valid: false, ageMs: 0, ...patch });
const snapshot = (patch: Partial<LiveReplaySnapshot> = {}): LiveReplaySnapshot => ({ ...descriptor(), clock: null, alignmentState: 'unknown', historyState: 'ready', historyItems: 8, untimedItems: 8, historyIncomplete: false, error: null, ...patch });
const fixture = () => {
  const api = { open: vi.fn().mockResolvedValue(descriptor()), cancelOpen: vi.fn().mockResolvedValue(undefined), snapshot: vi.fn().mockResolvedValue(snapshot()), viewport: vi.fn(), control: vi.fn(), fullscreen: vi.fn(), controls: vi.fn(), close: vi.fn().mockResolvedValue(undefined), measure: vi.fn(), page: vi.fn(), asset: vi.fn() } satisfies CloudLiveReplayApi;
  const observed = vi.fn();
  return { api, observed, player: liveReplayPlayerApi(api, 'selection', 7, observed) };
};
beforeEach(() => { vi.clearAllMocks(); mocks.listeners.clear(); });
afterEach(() => { vi.useRealTimers(); });

describe('native Live replay adapter ownership', () => {
  it('keeps the selected receipt/revision and unknown clock intact without synthesizing a media job or zero time', async () => {
    const { api, player, observed } = fixture();
    await expect(player.open('abcdefghijk', 'request', 'vod')).resolves.toEqual(descriptor().player);
    expect(api.open).toHaveBeenCalledExactlyOnceWith('selection', 7, 'request');
    await expect(player.vodClock!('owned-live')).resolves.toBeNull();
    expect(observed.mock.calls.at(-1)![0]).toEqual(snapshot());
    api.snapshot.mockResolvedValue(snapshot({ clock: clock() }));
    await expect(player.vodClock!('owned-live')).resolves.toMatchObject({ valid: false, currentTime: null, duration: null });
    expect(player.login).toBeDefined(); // Private playback retains the normal account recovery path.
    await expect(player.record('owned-live', true)).rejects.toThrow();
    expect(api.control).not.toHaveBeenCalled();
  });
  it.each([
    { selectionId: 'another-selection' }, { provider: 'soop' as const },
    { durationSeconds: 61 }, { clock: clock({ token: 'another-session' }) },
    { clock: clock({ videoId: 'ABCDEFGHIJK' }) },
    { videoId: 'ABCDEFGHIJK', player: { ...descriptor().player, broadcastId: 'ABCDEFGHIJK', channelId: 'ABCDEFGHIJK' } },
  ])('rejects a retargeted snapshot and does not publish it: %j', async patch => {
    const { api, player, observed } = fixture();
    await player.open('abcdefghijk', 'request', 'vod'); observed.mockClear();
    api.snapshot.mockResolvedValue(snapshot(patch));
    await expect(player.snapshot('owned-live')).rejects.toThrow();
    expect(observed).not.toHaveBeenCalled();
  });
  it('closes a rejected open, and never uses generic Live open for an archive', async () => {
    const { api, player, observed } = fixture();
    await expect(player.open('abcdefghijk', 'request')).rejects.toThrow();
    expect(api.open).not.toHaveBeenCalled();
    api.open.mockResolvedValue(descriptor({ selectionId: 'wrong-selection' }));
    await expect(player.open('abcdefghijk', 'request', 'vod')).rejects.toThrow();
    expect(api.close).toHaveBeenCalledExactlyOnceWith('owned-live'); expect(observed).not.toHaveBeenCalled();
  });
  it('drops a snapshot that finishes after close and blocks later controls', async () => {
    const { api, player, observed } = fixture();
    await player.open('abcdefghijk', 'request', 'vod'); observed.mockClear();
    let resolve!: (value: LiveReplaySnapshot) => void;
    api.snapshot.mockReturnValue(new Promise(value => { resolve = value; }));
    const pending = player.snapshot('owned-live');
    const rejected = expect(pending).rejects.toThrow();
    await player.close('owned-live'); resolve(snapshot()); await rejected;
    expect(observed).not.toHaveBeenCalled();
    await expect(player.fullscreen('owned-live', true)).rejects.toThrow();
    expect(api.fullscreen).not.toHaveBeenCalled();
  });
  it('isolates same-video ordinary and other Live events, including late events after close', async () => {
    const { player } = fixture(); const action = vi.fn(), fullscreen = vi.fn();
    await player.events!.action(action); await player.events!.fullscreen(fullscreen);
    await player.open('abcdefghijk', 'request', 'vod');
    const emit = (kind: string, payload: unknown) => mocks.listeners.get(`cloud-live-replay:${kind}`)!({ payload });
    emit('action', { token: 'ordinary-same-video', action: 'viewer_close' });
    emit('fullscreen', { token: 'another-live', enabled: true });
    expect(action).not.toHaveBeenCalled(); expect(fullscreen).not.toHaveBeenCalled();
    emit('action', { token: 'owned-live', action: 'viewer_close' });
    emit('fullscreen', { token: 'owned-live', enabled: true });
    expect(action).toHaveBeenCalledOnce(); expect(fullscreen).toHaveBeenCalledOnce();
    await player.close('owned-live'); emit('fullscreen', { token: 'owned-live', enabled: false });
    expect(fullscreen).toHaveBeenCalledOnce();
  });
  it('closes a delayed older open without replacing the newer descriptor or token', async () => {
    const { api, player, observed } = fixture();
    let resolve!: (value: LiveReplayDescriptor) => void;
    api.open.mockReturnValueOnce(new Promise(value => { resolve = value; }));
    const first = player.open('abcdefghijk', 'first', 'vod'); const rejected = expect(first).rejects.toThrow();
    await player.open('abcdefghijk', 'second', 'vod');
    resolve(descriptor({ token: 'old-live', player: { ...descriptor().player, token: 'old-live' } })); await rejected;
    expect(observed).toHaveBeenCalledExactlyOnceWith(descriptor()); expect(api.close).toHaveBeenCalledExactlyOnceWith('old-live');
    await player.snapshot('owned-live'); expect(observed).toHaveBeenCalledTimes(2);
  });
  it('keeps pull-only native clocks advancing and shares each in-flight read with status polling', async () => {
    vi.useFakeTimers(); const { api, player } = fixture(); const receive = vi.fn();
    const stop = await player.events!.clock(receive);
    await player.open('abcdefghijk', 'request', 'vod');
    let resolve!: (value: LiveReplaySnapshot) => void;
    api.snapshot.mockReturnValueOnce(new Promise(value => { resolve = value; }));
    await vi.advanceTimersByTimeAsync(400);
    const status = player.snapshot('owned-live'); const tick = player.vodClock!('owned-live');
    await vi.advanceTimersByTimeAsync(2000); expect(api.snapshot).toHaveBeenCalledOnce();
    resolve(snapshot({ clock: clock({ currentTime: 2, sequence: 1 }) })); await status; await tick;
    expect(receive).toHaveBeenLastCalledWith(expect.objectContaining({ currentTime: 2 }));
    api.snapshot.mockResolvedValue(snapshot({ clock: clock({ currentTime: 3, sequence: 2 }) }));
    await vi.advanceTimersByTimeAsync(400);
    expect(receive).toHaveBeenLastCalledWith(expect.objectContaining({ currentTime: 3, sequence: 2 }));
    stop(); const reads = api.snapshot.mock.calls.length;
    await vi.advanceTimersByTimeAsync(2000); expect(api.snapshot).toHaveBeenCalledTimes(reads);
  });
  it('does not deliver a pending clock after unsubscribe or close', async () => {
    vi.useFakeTimers(); const { api, player } = fixture(); const receive = vi.fn();
    await player.open('abcdefghijk', 'request', 'vod');
    let resolve!: (value: LiveReplaySnapshot) => void;
    api.snapshot.mockReturnValueOnce(new Promise(value => { resolve = value; }));
    const stop = await player.events!.clock(receive);
    stop(); await player.close('owned-live');
    resolve(snapshot({ clock: clock({ currentTime: 2 }) })); await vi.advanceTimersByTimeAsync(2000);
    expect(receive).not.toHaveBeenCalled(); expect(api.snapshot).toHaveBeenCalledOnce();
  });
  it('forwards opaque paging and asset requests without a local media route', async () => {
    await cloudLiveReplayApi.page('token', 'untimed', null, 120);
    await cloudLiveReplayApi.asset('token', 'a'.repeat(64));
    expect(mocks.invoke.mock.calls).toEqual([
      ['cloud_live_replay_page', { token: 'token', mode: 'untimed', cursor: null, limit: 120 }],
      ['cloud_live_replay_asset', { token: 'token', assetId: 'a'.repeat(64) }],
    ]);
  });
  it('cancels only its unresolved request once and closes a late token without publishing', async () => {
    const { api, player, observed } = fixture();
    let finish!: (value: LiveReplayDescriptor) => void;
    api.open.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
    const pending = player.open('abcdefghijk', 'pending', 'vod');
    const rejected = expect(pending).rejects.toThrow();
    await player.cancelOpen!('foreign'); expect(api.cancelOpen).not.toHaveBeenCalled();
    await player.cancelOpen!('pending'); await player.cancelOpen!('pending');
    expect(api.cancelOpen).toHaveBeenCalledExactlyOnceWith('selection', 7, 'pending');
    finish(descriptor()); await rejected;
    expect(observed).not.toHaveBeenCalled(); expect(api.close).toHaveBeenCalledExactlyOnceWith('owned-live');
  });
  it('keeps cancellation fenced when native cancellation fails and never cancels a settled request', async () => {
    const { api, player, observed } = fixture();
    let finish!: (value: LiveReplayDescriptor) => void;
    api.open.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
    api.cancelOpen.mockRejectedValueOnce(new Error('native busy'));
    const pending = player.open('abcdefghijk', 'pending', 'vod'); const rejected = expect(pending).rejects.toThrow();
    await expect(player.cancelOpen!('pending')).rejects.toThrow('native busy');
    finish(descriptor()); await rejected; expect(observed).not.toHaveBeenCalled();
    await player.open('abcdefghijk', 'settled', 'vod'); await player.cancelOpen!('settled');
    expect(api.cancelOpen).toHaveBeenCalledOnce(); await expect(player.snapshot('owned-live')).resolves.toEqual(descriptor().player);
    api.open.mockRejectedValueOnce(new Error('open failed'));
    await expect(player.open('abcdefghijk', 'failed', 'vod')).rejects.toThrow('open failed');
    await player.cancelOpen!('failed'); expect(api.cancelOpen).toHaveBeenCalledOnce();
  });
  it('does not invalidate a newer same-video token when an older request is cancelled', async () => {
    const { api, player, observed } = fixture();
    let finish!: (value: LiveReplayDescriptor) => void;
    api.open.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
    const pending = player.open('abcdefghijk', 'old', 'vod'); const rejected = expect(pending).rejects.toThrow();
    await player.open('abcdefghijk', 'new', 'vod'); await player.cancelOpen!('old');
    finish(descriptor({ token: 'old-live', player: { ...descriptor().player, token: 'old-live' } })); await rejected;
    expect(observed).toHaveBeenCalledExactlyOnceWith(descriptor());
    expect(api.close).toHaveBeenCalledExactlyOnceWith('old-live');
    await expect(player.snapshot('owned-live')).resolves.toEqual(descriptor().player);
  });
  it('forwards only the exact selected request identity to native cancellation', async () => {
    await cloudLiveReplayApi.cancelOpen('selection', 7, 'request');
    expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith('cloud_live_replay_cancel_open', { selectionId: 'selection', expectedRevision: 7, requestId: 'request' });
  });
});
