import { invoke } from '@tauri-apps/api/core';
import type { MediaProvider, MediaRef } from './platformMedia';

export interface PlatformScreenshotItem {
  id: string; provider: MediaProvider; reference: MediaRef; title: string; channelName: string;
  /** createdAt is Unix epoch milliseconds, matching the screenshot collection. */
  fileName: string; createdAt: number; bytes: number; width: number; height: number;
}
export interface PlatformScreenshotPage { items: PlatformScreenshotItem[]; cursor: string | null }
export interface PlatformScreenshotDeleteReport { deleted: string[]; failed: { id: string; error: string }[] }
export interface PlatformScreenshotsApi {
  begin(token: string, requestId: string, size: number, width: number, height: number): Promise<{ captureId: string }>;
  chunk(token: string, captureId: string, index: number, data: string): Promise<void>;
  finish(token: string, captureId: string): Promise<PlatformScreenshotItem>;
  abort(token: string, captureId: string): Promise<void>;
  undo(token: string, id: string): Promise<void>;
  list(provider: MediaProvider, cursor?: string | null, limit?: number): Promise<PlatformScreenshotPage>;
  image(provider: MediaProvider, id: string, full: boolean): Promise<string>;
  delete(provider: MediaProvider, ids: string[]): Promise<PlatformScreenshotDeleteReport>;
  folder(provider: MediaProvider, id: string): Promise<void>;
}
export const platformScreenshotsApi: PlatformScreenshotsApi = {
  begin: (token, requestId, size, width, height) => invoke('platform_media_screenshot_begin', { token, requestId, size, width, height }),
  chunk: (token, captureId, index, data) => invoke('platform_media_screenshot_chunk', { token, captureId, index, data }),
  finish: (token, captureId) => invoke('platform_media_screenshot_finish', { token, captureId }),
  abort: (token, captureId) => invoke('platform_media_screenshot_abort', { token, captureId }),
  undo: (token, id) => invoke('platform_media_screenshot_undo', { token, id }),
  list: (provider, cursor = null, limit = 24) => invoke('platform_media_screenshot_list', { provider, cursor, limit }),
  image: (provider, id, full) => invoke('platform_media_screenshot_image', { provider, id, full }),
  delete: (provider, ids) => invoke('platform_media_screenshot_delete', { provider, ids }),
  folder: (provider, id) => invoke('platform_media_screenshot_folder', { provider, id }),
};
