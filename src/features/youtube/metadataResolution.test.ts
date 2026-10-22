import { describe, expect, it, vi } from 'vitest';
import { createYouTubeMetadataResolver, verifiedYouTubeMetadata } from './metadataResolution';
import { parseYouTubeTarget, type YouTubeMetadata } from './model';

const target = parseYouTubeTarget('cswFQYam8Ic')!;
const metadata: YouTubeMetadata = { ...target, title: 'LIVE title is not evidence', author: 'fixture', isLive: true };
describe('exact YouTube action metadata', () => {
  it('requires exact kind/id/canonical URL and a boolean live state', () => {
    expect(verifiedYouTubeMetadata(target, metadata).isLive).toBe(true);
    expect(verifiedYouTubeMetadata(target, { ...metadata, isLive: false }).isLive).toBe(false);
    for (const invalid of [
      { ...metadata, isLive: undefined }, { ...metadata, isLive: null }, { ...metadata, isLive: 'false' },
      { ...metadata, id: 'jNQXAC9IVRw' }, { ...metadata, url: 'https://www.youtube.com/watch?v=jNQXAC9IVRw' },
      { ...metadata, kind: 'channel' }, { ...metadata, url: 'https://evil.invalid/watch?v=cswFQYam8Ic' },
    ]) expect(() => verifiedYouTubeMetadata(target, invalid as YouTubeMetadata)).toThrow();
  });
  it('limits physical metadata calls to two and never starts an already cancelled queued action', async () => {
    const finish: Array<(value: YouTubeMetadata) => void> = [];
    const api = { metadata: vi.fn(() => new Promise<YouTubeMetadata>(resolve => finish.push(resolve))), openExternal: vi.fn() };
    const resolve = createYouTubeMetadataResolver(api), cancel = new AbortController();
    const first = resolve(target), second = resolve(target), third = resolve(target, cancel.signal).catch(error => error);
    expect(api.metadata).toHaveBeenCalledTimes(2); cancel.abort(); finish[0]!(metadata); await first; await third;
    expect(api.metadata).toHaveBeenCalledTimes(2); finish[1]!(metadata); await second;
  });
  it('rejects a late native response after cancellation', async () => {
    let finish!: (value: YouTubeMetadata) => void;
    const resolve = createYouTubeMetadataResolver({ metadata: () => new Promise<YouTubeMetadata>(done => { finish = done; }), openExternal: vi.fn() });
    const cancel = new AbortController(), result = resolve(target, cancel.signal).catch(error => error);
    cancel.abort(); finish(metadata); expect(await result).toMatchObject({ name: 'AbortError' });
  });
  it('bounds queue capacity and expires queued work without freeing running native slots', async () => {
    vi.useFakeTimers();
    try {
      const finish: Array<(value: YouTubeMetadata) => void> = [];
      const api = { metadata: vi.fn(() => new Promise<YouTubeMetadata>(resolve => finish.push(resolve))), openExternal: vi.fn() };
      const resolve = createYouTubeMetadataResolver(api);
      const active = [resolve(target), resolve(target)];
      const waiting = Array.from({ length: 100 }, () => resolve(target).catch(error => error));
      await expect(resolve(target)).rejects.toThrow('요청이 많습니다');
      await vi.advanceTimersByTimeAsync(12_001);
      expect((await Promise.all(waiting)).every(error => String(error).includes('지연'))).toBe(true);
      expect(api.metadata).toHaveBeenCalledTimes(2);
      const next = resolve(target); expect(api.metadata).toHaveBeenCalledTimes(2);
      finish[0]!(metadata); await active[0]; await Promise.resolve(); expect(api.metadata).toHaveBeenCalledTimes(3);
      finish[1]!(metadata); finish[2]!(metadata); await active[1]; await next;
    } finally { vi.useRealTimers(); }
  });
});
