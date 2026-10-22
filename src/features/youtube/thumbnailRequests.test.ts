import { afterEach, describe, expect, it, vi } from 'vitest';
import { YouTubeThumbnailError } from './api';
import { subscribeYouTubeThumbnail as subscribe } from './thumbnailRequests';
const image = 'data:image/png;base64,AQ==';
const flush = async () => { for (let n = 0; n < 5; n++) await Promise.resolve(); };
afterEach(() => vi.useRealTimers());

describe('YouTube shared thumbnail transport', () => {
  it('shares four physical slots across shallow API copies and deduplicates an exact ID', async () => {
    const pending: Array<(value: string | null) => void> = [];
    const thumbnail = vi.fn(() => new Promise<string | null>(done => pending.push(done)));
    const api = { thumbnail }, actionApi = { ...api }, results = Array.from({ length: 6 }, () => vi.fn());
    const stops = results.map((done, n) => subscribe(n < 4 ? api.thumbnail : actionApi.thumbnail, String(n), done));
    const duplicate = vi.fn(); stops.push(subscribe(actionApi.thumbnail, '0', duplicate));
    expect(thumbnail).toHaveBeenCalledTimes(4);
    pending[0]!(image); await flush();
    expect(thumbnail).toHaveBeenCalledTimes(5); expect(duplicate).toHaveBeenCalledWith(image);
    pending[1]!(image); await flush(); expect(thumbnail).toHaveBeenCalledTimes(6);
    pending.slice(2).forEach(done => done(image)); await flush();
    results.forEach(done => expect(done).toHaveBeenCalledOnce()); stops.forEach(stop => stop());
  });
  it('unsubscribes queued work without prematurely releasing physical in-flight slots', async () => {
    const pending: Array<(value: string | null) => void> = [];
    const thumbnail = vi.fn(() => new Promise<string | null>(done => pending.push(done)));
    const stops = Array.from({ length: 8 }, (_, n) => subscribe(thumbnail, String(n), vi.fn()));
    stops.forEach(stop => stop()); const visible = vi.fn();
    const stop = subscribe(thumbnail, 'visible', visible);
    expect(thumbnail).toHaveBeenCalledTimes(4);
    pending[0]!(image); await flush(); expect(thumbnail).toHaveBeenCalledTimes(5);
    expect(thumbnail).toHaveBeenLastCalledWith('visible');
    pending.slice(1).forEach(done => done(image)); await flush();
    expect(visible).toHaveBeenCalledWith(image); stop();
  });
  it('retries only closed busy at250/750ms and terminates after three calls', async () => {
    vi.useFakeTimers(); const thumbnail = vi.fn().mockRejectedValue(new YouTubeThumbnailError('busy')), done = vi.fn();
    subscribe(thumbnail, 'busy', done); await flush();
    await vi.advanceTimersByTimeAsync(249); expect(thumbnail).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1); expect(thumbnail).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(749); expect(thumbnail).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(1); expect(thumbnail).toHaveBeenCalledTimes(3);
    expect(done).toHaveBeenCalledExactlyOnceWith(null);
    await vi.advanceTimersByTimeAsync(10000); expect(thumbnail).toHaveBeenCalledTimes(3);
  });
  it('cancels a pending busy retry when its last visible consumer leaves', async () => {
    vi.useFakeTimers(); const thumbnail = vi.fn().mockRejectedValue(new YouTubeThumbnailError('busy')), done = vi.fn();
    const stop = subscribe(thumbnail, 'busy', done); await flush(); stop();
    await vi.advanceTimersByTimeAsync(2000); expect(thumbnail).toHaveBeenCalledTimes(1); expect(done).not.toHaveBeenCalled();
  });
  it('does not retry null, malformed images, unavailable or locale-only errors', async () => {
    vi.useFakeTimers();
    for (const value of [null, 'https://example.invalid/image.png', new YouTubeThumbnailError('unavailable'), '미리보기를 준비 중입니다.', { code: 'busy' }]) {
      const thumbnail = value === null || typeof value === 'string' && value.startsWith('https') ? vi.fn().mockResolvedValue(value) : vi.fn().mockRejectedValue(value);
      const done = vi.fn(); subscribe(thumbnail, 'id', done); await flush(); await vi.advanceTimersByTimeAsync(2000);
      expect(thumbnail).toHaveBeenCalledTimes(1); expect(done).toHaveBeenCalledExactlyOnceWith(null);
    }
  });
  it('caps pending requests at128 and drops all unstarted requests on unsubscribe', async () => {
    const pending: Array<(value: string | null) => void> = [];
    const thumbnail = vi.fn(() => new Promise<string | null>(done => pending.push(done))), overflow = vi.fn();
    const stops = Array.from({ length: 128 }, (_, n) => subscribe(thumbnail, String(n), vi.fn()));
    subscribe(thumbnail, 'overflow', overflow); expect(overflow).toHaveBeenCalledWith(null);
    stops.forEach(stop => stop()); pending.forEach(done => done(null)); await flush(); expect(thumbnail).toHaveBeenCalledTimes(4);
  });
  it('expires success cache after10minutes and never caches a failure', async () => {
    vi.useFakeTimers(); const thumbnail = vi.fn().mockResolvedValueOnce(null).mockResolvedValue(image);
    subscribe(thumbnail, 'id', vi.fn()); await flush(); subscribe(thumbnail, 'id', vi.fn()); await flush();
    subscribe(thumbnail, 'id', vi.fn()); expect(thumbnail).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(600000); subscribe(thumbnail, 'id', vi.fn()); await flush(); expect(thumbnail).toHaveBeenCalledTimes(3);
  });
});
