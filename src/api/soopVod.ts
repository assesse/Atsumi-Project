import { invoke } from '@tauri-apps/api/core';
import type { NativeViewport } from '../features/platforms/nativePlayerGeometry';

/** Watch-only SOOP VOD lease. It cannot grant live capture or account authority. */
export type SoopVodSnapshot = {
  token: string;
  videoId: string;
  state: string;
  chatState: 'pending' | 'available' | 'unavailable';
  fullscreen: boolean;
  error: string | null;
};
export const soopVodApi = {
  open: (videoId: string, requestId: string) => invoke<SoopVodSnapshot>('soop_vod_open', { videoId, requestId }),
  snapshot: (token: string) => invoke<SoopVodSnapshot>('soop_vod_snapshot', { token }),
  viewport: (token: string, viewport: NativeViewport) => invoke<void>('soop_vod_viewport', { token, viewport }),
  close: (token: string) => invoke<void>('soop_vod_close', { token }),
  fullscreen: (token: string, enabled: boolean) => invoke<void>('soop_vod_fullscreen', { token, enabled }),
};
export type SoopVodApi = typeof soopVodApi;
