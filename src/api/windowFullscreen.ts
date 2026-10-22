import { invoke } from '@tauri-apps/api/core';

/** Capability for the shared native window, not a media/account session. */
export type WindowFullscreenGrant = { owner: string; documentEpoch: number; revision: number; windowKey: string };
export type WindowFullscreenOutcome = { grant: WindowFullscreenGrant | null; error: string | null };
export const windowFullscreenApi = {
  acquire: (owner: string, documentEpoch: number) => invoke<WindowFullscreenOutcome>('platform_window_fullscreen_acquire', { owner, documentEpoch }),
  release: (grant: WindowFullscreenGrant) => invoke<boolean>('platform_window_fullscreen_release', { grant }),
  current: (grant: WindowFullscreenGrant) => invoke<boolean>('platform_window_fullscreen_current', { grant }),
};
export const sameFullscreenGrant = (a: WindowFullscreenGrant | null, b: WindowFullscreenGrant | null) => !!a && !!b
  && a.owner === b.owner && a.documentEpoch === b.documentEpoch && a.revision === b.revision && a.windowKey === b.windowKey;
