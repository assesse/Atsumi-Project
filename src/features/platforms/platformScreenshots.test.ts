import { expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { platformScreenshotsApi as api } from './platformScreenshots';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));
it('uses the exact native bounded screenshot contract without renderer paths or URLs', async () => {
  await api.begin('token', 'request', 3, 1, 1); await api.chunk('token', 'capture', 0, 'AAAA'); await api.finish('token', 'capture'); await api.abort('token', 'capture'); await api.undo('token', 'shot');
  await api.list('youtube'); await api.image('soop', 'shot', true); await api.delete('soop', ['shot']); await api.folder('youtube', 'shot');
  expect(vi.mocked(invoke).mock.calls).toEqual([
    ['platform_media_screenshot_begin', { token: 'token', requestId: 'request', size: 3, width: 1, height: 1 }],
    ['platform_media_screenshot_chunk', { token: 'token', captureId: 'capture', index: 0, data: 'AAAA' }],
    ['platform_media_screenshot_finish', { token: 'token', captureId: 'capture' }],
    ['platform_media_screenshot_abort', { token: 'token', captureId: 'capture' }],
    ['platform_media_screenshot_undo', { token: 'token', id: 'shot' }],
    ['platform_media_screenshot_list', { provider: 'youtube', cursor: null, limit: 24 }],
    ['platform_media_screenshot_image', { provider: 'soop', id: 'shot', full: true }],
    ['platform_media_screenshot_delete', { provider: 'soop', ids: ['shot'] }],
    ['platform_media_screenshot_folder', { provider: 'youtube', id: 'shot' }],
  ]);
});
