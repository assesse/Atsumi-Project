import { invoke } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createYouTubeApi, YouTubeThumbnailError } from './api';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
beforeEach(() => vi.mocked(invoke).mockReset());
describe('YouTube thumbnail error normalization', () => {
  it('retains the closed native busy code but never retries localized or unknown errors', async () => {
    for (const [input, expected] of [[{ code: 'busy', message: 'safe' }, 'busy'], [{ code: 'unavailable' }, 'unavailable'], ['미리보기를 준비 중입니다.', 'unavailable'], [{ code: 'BUSY' }, 'unavailable'], [null, 'unavailable']] as const) {
      vi.mocked(invoke).mockRejectedValueOnce(input);
      const error = await createYouTubeApi('tauri').thumbnail!('id').catch(value => value);
      expect(error).toBeInstanceOf(YouTubeThumbnailError); expect(error.code).toBe(expected);
    }
  });
  it('keeps successful and missing images unchanged and browser mock offline', async () => {
    const api = createYouTubeApi('tauri'); vi.mocked(invoke).mockResolvedValueOnce('data:image/png;base64,AQ==').mockResolvedValueOnce(null);
    expect(await api.thumbnail!('first')).toBe('data:image/png;base64,AQ=='); expect(await api.thumbnail!('second')).toBeNull();
    expect(await createYouTubeApi('browser-mock').thumbnail!('third')).toBeNull(); expect(invoke).toHaveBeenCalledTimes(2);
  });
});
