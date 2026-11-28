import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { CloudError, CloudLiveProvider } from './cloudRecording';
import type { ReplayMessage } from './replay';
import { youtubeOriginalApi, type YouTubeOriginalApi, type YouTubeOriginalSnapshot, type YouTubeVodClock, type YouTubeVodControlRequest } from './youtubeOriginal';
import type { NativeViewport } from '../features/platforms/nativePlayerGeometry';
import type { MediaReplayChatPage } from '../features/platforms/platformMedia';

export type LiveReplayAlignment = 'unknown' | 'measuring' | 'measured' | 'failed';
export type LiveReplayDescriptor = { token: string; selectionId: string; provider: CloudLiveProvider; videoId: string; durationSeconds: number; player: YouTubeOriginalSnapshot };
export type LiveReplaySnapshot = LiveReplayDescriptor & {
  clock: YouTubeVodClock | null; alignmentState: LiveReplayAlignment; historyState: 'preparing' | 'ready' | 'partial';
  historyItems: number; untimedItems: number; historyIncomplete: boolean; error: CloudError | null;
};
export type LiveReplayChzzkItem = Omit<ReplayMessage, 'mediaTimeSeconds' | 'syncQuality'> & { mediaTimeSeconds: number | null; syncQuality: 'observed_media' | 'unknown' };
export type LiveReplayPage = {
  positions: { id: string; remoteInterval: { lowerTicks90k: number; upperTicks90k: number; presentationUncertainty: string; measurement: string } | null; unmappedReason: string | null }[];
  nativeClock: YouTubeVodClock | null; alignmentState: LiveReplayAlignment;
  cursorGeneration: Pick<YouTubeVodClock, 'documentEpoch' | 'sourceId' | 'sourceGeneration' | 'seekGeneration'> | null;
  untimedCount: number; sourceCount: number; incomplete: boolean;
} & ({ provider: 'chzzk'; page: { items: LiveReplayChzzkItem[]; cursor?: string | null; previousCursor?: string | null } }
  | { provider: 'soop'; page: MediaReplayChatPage });
export interface CloudLiveReplayApi {
  open(selectionId: string, expectedRevision: number, requestId: string): Promise<LiveReplayDescriptor>;
  cancelOpen(selectionId: string, expectedRevision: number, requestId: string): Promise<void>;
  snapshot(token: string): Promise<LiveReplaySnapshot>;
  viewport(token: string, viewport: NativeViewport): Promise<void>;
  control(token: string, request: YouTubeVodControlRequest): Promise<void>;
  fullscreen(token: string, enabled: boolean): Promise<void>;
  controls(token: string, canClose: boolean): Promise<void>;
  close(token: string): Promise<void>;
  measure(token: string): Promise<LiveReplaySnapshot>;
  page(token: string, mode: 'current' | 'history' | 'untimed', cursor: string | null, limit: number): Promise<LiveReplayPage>;
  asset(token: string, assetId: string): Promise<string | null>;
}
export const cloudLiveReplayApi: CloudLiveReplayApi = {
  open: (selectionId, expectedRevision, requestId) => invoke('cloud_live_replay_open', { selectionId, expectedRevision, requestId }),
  cancelOpen: (selectionId, expectedRevision, requestId) => invoke('cloud_live_replay_cancel_open', { selectionId, expectedRevision, requestId }),
  snapshot: token => invoke('cloud_live_replay_snapshot', { token }),
  viewport: (token, viewport) => invoke('cloud_live_replay_viewport', { token, viewport }),
  control: (token, request) => invoke('cloud_live_replay_control', { token, request }),
  fullscreen: (token, enabled) => invoke('cloud_live_replay_fullscreen', { token, enabled }),
  controls: (token, canClose) => invoke('cloud_live_replay_controls', { token, canClose }),
  close: token => invoke('cloud_live_replay_close', { token }),
  measure: token => invoke('cloud_live_replay_measure', { token }),
  page: (token, mode, cursor, limit) => invoke('cloud_live_replay_page', { token, mode, cursor, limit }),
  asset: (token, assetId) => invoke('cloud_live_replay_asset', { token, assetId }),
};

/** No generic player token or local recording identifier is fabricated here. */
export function liveReplayPlayerApi(api: CloudLiveReplayApi, selectionId: string, revision: number,
  observed: (value: LiveReplayDescriptor | LiveReplaySnapshot) => void): YouTubeOriginalApi {
  // A same-video ordinary player or another Live selection never owns this token.
  const opened = new Map<string, { videoId: string; provider: CloudLiveProvider; durationSeconds: number }>();
  let openEpoch = 0;
  const pendingOpens = new Map<string, { epoch: number; cancelled: boolean }>();
  let currentToken: string | null = null;
  const mismatch = () => new Error('선택한 보관 영상의 재생 연결을 확인하지 못했습니다.');
  const owner = (token: string) => {
    const value = opened.get(token);
    if (!value || token !== currentToken) throw mismatch();
    return value;
  };
  const validate = <T extends LiveReplayDescriptor>(value: T, token?: string) => {
    if (value.selectionId !== selectionId || !value.token || token !== undefined && token !== value.token
      || !['chzzk', 'soop'].includes(value.provider) || !Number.isFinite(value.durationSeconds) || value.durationSeconds <= 0
      || value.player.token !== value.token || value.player.broadcastId !== value.videoId || value.player.channelId !== value.videoId) {
      throw mismatch();
    }
    return value;
  };
  const pendingSnapshots = new Map<string, Promise<LiveReplaySnapshot>>();
  const snapshot = async (token: string): Promise<LiveReplaySnapshot> => {
    const expected = owner(token), epoch = openEpoch;
    const pending = pendingSnapshots.get(token);
    if (pending) return pending;
    const request = (async () => {
      const value = validate(await api.snapshot(token), token);
      if (currentToken !== token || openEpoch !== epoch || opened.get(token) !== expected || value.videoId !== expected.videoId || value.provider !== expected.provider
        || value.durationSeconds !== expected.durationSeconds
        || value.clock && (value.clock.token !== token || value.clock.videoId !== expected.videoId)) throw mismatch();
      observed(value); return value;
    })();
    pendingSnapshots.set(token, request);
    try { return await request; }
    finally { if (pendingSnapshots.get(token) === request) pendingSnapshots.delete(token); }
  };
  // The native owned route emits close/fullscreen, but clock samples are pull-only.
  // Share one snapshot with the player's slower status poll and never queue ticks.
  const clockListeners = new Set<(clock: YouTubeVodClock) => void>();
  let clockEpoch = 0;
  let clockTimer: ReturnType<typeof setTimeout> | undefined;
  const pollClock = async (epoch: number) => {
    if (epoch !== clockEpoch || !clockListeners.size) return;
    const token = currentToken;
    try {
      if (token) {
        const value = await snapshot(token);
        if (epoch === clockEpoch && currentToken === token && value.clock) {
          for (const receive of clockListeners) receive(value.clock);
        }
      }
    } catch { /* A later tick retries; invalid/stale clocks expire in the player. */ }
    if (epoch === clockEpoch && clockListeners.size) clockTimer = setTimeout(() => { void pollClock(epoch); }, 400);
  };
  const owned = async <T>(token: string, action: () => Promise<T>) => { owner(token); return action(); };
  const unavailable = async (): Promise<never> => { throw new Error('이 보관 영상에서는 사용할 수 없는 기능입니다.'); };
  return {
    ...youtubeOriginalApi,
    open: async (videoId, requestId, kind) => {
      if (kind !== 'vod') throw new Error('보관 영상 재생만 지원합니다.');
      if (pendingOpens.has(requestId)) throw mismatch();
      const pending = { epoch: ++openEpoch, cancelled: false };
      pendingOpens.set(requestId, pending);
      currentToken = null;
      try {
        const value = await api.open(selectionId, revision, requestId);
        try { validate(value); if (pending.cancelled || pending.epoch !== openEpoch || value.videoId !== videoId) throw new Error('선택한 보관 영상이 변경됐습니다.'); }
        catch (error) { if (value.token !== currentToken) await api.close(value.token).catch(() => {}); throw error; }
        opened.set(value.token, { videoId: value.videoId, provider: value.provider, durationSeconds: value.durationSeconds });
        currentToken = value.token;
        observed(value); return value.player;
      } finally { if (pendingOpens.get(requestId) === pending) pendingOpens.delete(requestId); }
    },
    cancelOpen: async requestId => {
      const pending = pendingOpens.get(requestId);
      if (!pending) return;
      pendingOpens.delete(requestId); pending.cancelled = true;
      if (pending.epoch === openEpoch) ++openEpoch;
      // Fence synchronously, even if native cancellation fails or arrives late.
      await api.cancelOpen(selectionId, revision, requestId);
    },
    snapshot: async token => (await snapshot(token)).player,
    vodClock: async token => (await snapshot(token)).clock,
    vodControl: (token, request) => owned(token, () => api.control(token, request)),
    viewport: (token, viewport) => owned(token, () => api.viewport(token, viewport)),
    close: token => { opened.delete(token); if (currentToken === token) { currentToken = null; ++openEpoch; } return api.close(token); },
    fullscreen: (token, enabled) => owned(token, () => api.fullscreen(token, enabled)),
    controls: (token, canClose) => owned(token, () => api.controls(token, canClose)),
    resume: unavailable, record: unavailable, screenshot: unavailable, undoScreenshot: unavailable,
    events: {
      action: receive => listen<{ token: string; action: string }>('cloud-live-replay:action', event => { if (event.payload.token === currentToken) receive(event.payload); }),
      clock: async receive => {
        clockListeners.add(receive);
        if (clockListeners.size === 1) void pollClock(++clockEpoch);
        return () => { clockListeners.delete(receive); if (!clockListeners.size) { ++clockEpoch; clearTimeout(clockTimer); clockTimer = undefined; } };
      },
      fullscreen: receive => listen<{ token: string; enabled: boolean }>('cloud-live-replay:fullscreen', event => { if (event.payload.token === currentToken) receive(event.payload); }),
      screenshot: async () => () => {},
    },
  };
}
