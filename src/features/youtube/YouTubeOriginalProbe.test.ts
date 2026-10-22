// Diagnostic only: never opens a website or reads the user's browser profile.
// @ts-expect-error Node-only test utility; production renderer intentionally has no Node globals.
import { readFileSync } from 'node:fs';
// @ts-expect-error Node-only test utility; production renderer intentionally has no Node globals.
import { runInNewContext } from 'node:vm';
import { describe, expect, it } from 'vitest';

const observer = readFileSync('src-tauri/examples/youtube_original_probe.js', 'utf8');
function environment(origin = 'https://www.youtube.com', path = '/watch', child = false) {
  const nativeError = new Error('native error');
  class SourceBufferMock {
    receiver?: unknown;
    appendBuffer(value: unknown) { this.receiver = this; if (value === 'throw') throw nativeError; return 'accepted'; }
  }
  class MediaSourceMock {
    receiver?: unknown;
    addSourceBuffer(type: string) { this.receiver = this; if (type === 'throw') throw nativeError; return new SourceBufferMock(); }
  }
  const nodes = new Map<string, unknown>();
  const context: Record<string, any> = {
    location: { origin, pathname: path }, MediaSource: MediaSourceMock, SourceBuffer: SourceBufferMock,
    document: { readyState: 'complete', querySelector: (selector: string) => nodes.get(selector) ?? null },
    performance: { now: () => 1000 }, getComputedStyle: () => ({ opacity: '1', display: 'block', visibility: 'visible' }),
  };
  context.window = context; context.top = child ? {} : context;
  runInNewContext(observer, context);
  return { context, SourceBufferMock, MediaSourceMock, nativeError, nodes, snapshot: () => context.__atsumiYouTubeOriginalProbe?.snapshot() };
}
describe('disposable original YouTube feasibility observer', () => {
  it('installs only in official top-level watch/chat documents', () => {
    expect(environment().snapshot()).toBeDefined();
    expect(environment('https://www.youtube.com', '/live_chat').snapshot()).toBeDefined();
    expect(environment('https://accounts.google.com').snapshot()).toBeUndefined();
    expect(environment('https://www.youtube.com.attacker.invalid').snapshot()).toBeUndefined();
    expect(environment('https://www.youtube.com', '/').snapshot()).toBeUndefined();
    expect(environment('https://www.youtube.com', '/watch', true).snapshot()).toBeUndefined();
  });
  it('preserves native receiver, return values and exceptions; counts accepted buffers without retaining bytes', () => {
    const e = environment(); const media = new e.MediaSourceMock();
    const buffer = media.addSourceBuffer('video/mp4; codecs="avc1.64002a"');
    expect(media.receiver).toBe(media);
    expect(buffer.appendBuffer(new Uint8Array([12, 34, 56]))).toBe('accepted');
    expect(buffer.receiver).toBe(buffer);
    expect(() => buffer.appendBuffer('throw')).toThrow(e.nativeError);
    expect(() => media.addSourceBuffer('throw')).toThrow(e.nativeError);
    expect(e.snapshot().tracks).toEqual([{ mime: 'video/mp4; codecs="avc1.64002a"', appends: 1, bytes: 3, lastAppendMs: 0 }]);
    expect(JSON.stringify(e.snapshot())).not.toContain('12,34,56');
  });
  it('never lets diagnostics throw after the native append succeeds', () => {
    const e = environment(); const buffer = new e.MediaSourceMock().addSourceBuffer('audio/mp4; codecs="mp4a.40.2"');
    const value = { get byteLength() { throw new Error('observer getter'); } };
    expect(buffer.appendBuffer(value)).toBe('accepted');
    expect(e.snapshot().observerError).toBe(true);
  });
  it('bounds tracks and sanitizes unexpected MIME without revealing supplied strings', () => {
    const e = environment(); const media = new e.MediaSourceMock();
    for (let n = 0; n < 30; n++) media.addSourceBuffer(n === 0 ? 'video/webm; codecs="vp9,opus"' : 'https://secret.invalid/?token=abc');
    const result = e.snapshot(); expect(result.tracks).toHaveLength(16);
    expect(result.tracks[0].mime).toBe('video/webm; codecs="vp9,opus"');
    expect(result.tracks[1].mime).toBe('other');
    expect(JSON.stringify(result)).not.toContain('secret');
  });
  it('reports original controls and media capability facts, not page text or URLs', () => {
    const e = environment();
    const node = { textContent: 'private chat message', href: 'https://secret.invalid/', getBoundingClientRect: () => ({ x: 1, y: 2, width: 960, height: 540 }) };
    e.nodes.set('#movie_player video', { ...node, currentTime: 8.5, duration: Infinity, paused: false, ended: false, readyState: 4, videoWidth: 1920, videoHeight: 1080, mediaKeys: {}, srcObject: null });
    e.nodes.set('#movie_player .ytp-chrome-bottom', node);
    e.nodes.set('#movie_player', { ...node, getPlayerResponse: () => ({ playabilityStatus: { status: 'OK', reason: 'sensitive' } }) });
    const result = e.snapshot();
    expect(result.video.currentTime).toBe(8.5); expect(result.video.protected).toBe(true); expect(result.video.duration).toBeNull();
    expect(result.controls.width).toBe(960); expect(result.playability).toBe('OK');
    expect(JSON.stringify(result)).not.toMatch(/private|secret|sensitive/);
  });
  it('installing twice does not duplicate wrappers', () => {
    const e = environment(); const first = e.MediaSourceMock.prototype.addSourceBuffer;
    runInNewContext(observer, e.context);
    expect(e.MediaSourceMock.prototype.addSourceBuffer).toBe(first);
  });
});
