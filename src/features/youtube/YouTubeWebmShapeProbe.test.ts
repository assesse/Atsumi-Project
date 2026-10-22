// @ts-expect-error Node-only QA script harness.
import { runInNewContext } from 'node:vm';
import { expect, it } from 'vitest';
import probe from '../../../src-tauri/examples/youtube_webm_shape_probe.js?raw';

const join = (...parts: Array<Uint8Array | number[]>) => Uint8Array.from(parts.flatMap(p => Array.from(p)));
const ascii = (value: string) => Uint8Array.from([...value].map(c => c.charCodeAt(0)));
const hex = (value: number) => { const text = value.toString(16).padStart(value.toString(16).length + value.toString(16).length % 2, '0'); return Uint8Array.from(text.match(/../g)!.map(b => parseInt(b, 16))); };
const size = (value: number) => value < 127 ? Uint8Array.of(128 | value) : value < 16383 ? Uint8Array.of(64 | (value >> 8), value & 255) : Uint8Array.of(32 | (value >> 16), (value >> 8) & 255, value & 255);
const element = (id: number, ...parts: Array<Uint8Array | number[]>) => { const body = join(...parts); return join(hex(id), size(body.length), body); };
const uint = (id: number, value: number) => element(id, hex(value));
const unknown = (id: number, ...parts: Array<Uint8Array | number[]>) => join(hex(id), [255], ...parts);
function keyframe(width = 320, height = 180) {
  const bits = '10000011' + (0x498342).toString(2).padStart(24, '0') + '0010' + (width - 1).toString(2).padStart(16, '0') + (height - 1).toString(2).padStart(16, '0');
  return Uint8Array.from(bits.padEnd(Math.ceil(bits.length / 8) * 8, '0').match(/.{8}/g)!.map(b => parseInt(b, 2)));
}
const init = (options: { scale?: number; duration?: boolean; codec?: string; lace?: number } = {}) => join(
  element(0x1a45dfa3, element(0x4282, ascii('webm'))), unknown(0x18538067),
  element(0x1549a966, options.scale === undefined ? [] : uint(0x2ad7b1, options.scale)),
  element(0x1654ae6b, element(0xae, uint(0xd7, 1), uint(0x83, 1), element(0x86, ascii(options.codec ?? 'V_VP9')), uint(0x9c, options.lace ?? 1),
    options.duration === false ? [] : uint(0x23e383, 33333333), element(0xe0, uint(0xb0, 320), uint(0xba, 180)))));
const block = (pts = 0, payload = keyframe(), flags = 0x80, laceHeader: number[] = []) => element(0xa3, [0x81, (pts >> 8) & 255, pts & 255, flags], laceHeader, payload);
const cluster = (pts: number, ...blocks: Uint8Array[]) => unknown(0x1f43b675, uint(0xe7, pts), ...blocks);
const be32 = (n: number) => Uint8Array.of(n >>> 24, n >>> 16, n >>> 8, n);
const box = (id: string, ...parts: Array<Uint8Array | number[]>) => { const body = join(...parts); return join(be32(body.length + 8), ascii(id), body); };
const aacInit = () => { const tkhd = new Uint8Array(84); tkhd.set(be32(1), 12); return join(box('ftyp', ascii('isom'), be32(0)), box('moov', box('trak', box('tkhd', tkhd), box('mdia', box('mdhd', be32(0), be32(0), be32(0), be32(48000), be32(0), be32(0)))), box('mvex', box('trex', be32(0), be32(1), be32(1), be32(1024), be32(0), be32(0))))); };
const aacFragment = (pts = 48000) => join(box('moof', box('traf', box('tfhd', be32(8), be32(1), be32(1024)), box('tfdt', be32(0), be32(pts)), box('trun', be32(0), be32(2)))), box('mdat', new Uint8Array(300).fill(222)));

type Stats = Record<string, number | boolean | null>;
type Track = { reason: string; generation: number; mime: string; detached: boolean; bytes: number; appends: number; stats: Stats; clock: Array<{ startSeconds: number; endSeconds: number | null; offsetSeconds: number | null }>; offsetChanged: boolean };
type Snapshot = { totalBytes: number; stopped: boolean; observerError: boolean; sources: Array<{ source: number; selected: boolean; closed: boolean; tracks: Track[] }>; video: { selectedSource: number | null } };
function item<T>(items: readonly T[], index: number): T {
  if (!Number.isInteger(index) || index < 0 || index >= items.length) throw new Error('Expected bounded fixture item');
  const value = items[index];
  if (value === undefined) throw new Error('Missing fixture item');
  return value;
}
const getTrack = (snapshot: Snapshot, sourceIndex = 0, trackIndex = 0): Track => item(item(snapshot.sources, sourceIndex).tracks, trackIndex);
const lastClock = (track: Track) => item(track.clock, track.clock.length - 1);
function setup(options: { origin?: string; search?: string } = {}) {
  let now = 0, ad = false, serial = 0;
  const calls: Array<{ receiver: unknown; args: unknown[] }> = [];
  class SourceBuffer {
    mode = 'segments'; timestampOffset = 0; appendWindowStart = 0; appendWindowEnd = Infinity; fail = false;
    listeners = new Map<string, () => void>();
    appendBuffer(...args: unknown[]) { calls.push({ receiver: this, args }); if (this.fail) throw new Error('original_failure'); return 17; }
    changeType(...args: unknown[]) { calls.push({ receiver: this, args }); return 19; }
    addEventListener(name: string, callback: () => void) { this.listeners.set(name, callback); }
  }
  class MediaSource {
    sourceBuffers: SourceBuffer[] = []; listeners = new Map<string, () => void>();
    addSourceBuffer(_mime: string) { const buffer = new SourceBuffer(); this.sourceBuffers.push(buffer); return buffer; }
    removeSourceBuffer(buffer: SourceBuffer) { this.sourceBuffers = this.sourceBuffers.filter(item => item !== buffer); return 23; }
    addEventListener(name: string, callback: () => void) { this.listeners.set(name, callback); }
  }
  class ProbeURL extends URL { static createObjectURL(_object: unknown) { return `blob:private-${++serial}`; } static revokeObjectURL(_url: string) { return 29; } }
  const video = { currentSrc: '', src: '', currentTime: 1, readyState: 4, videoWidth: 320, videoHeight: 180, paused: false, ended: false, mediaKeys: null as unknown, srcObject: null as unknown, getVideoPlaybackQuality: () => ({ totalVideoFrames: 20 }) };
  const document = { querySelector: (selector: string) => selector === '#movie_player video' ? video : ad ? {} : null };
  const events = new Map<string, () => void>();
  const window: Record<string, unknown> = { __atsumiYouTubeWebmShapeProbeId: 'T4XZmMPQ9Kw', addEventListener: (name: string, callback: () => void) => events.set(name, callback) }; window.top = window;
  const location = { origin: options.origin ?? 'https://www.youtube.com', pathname: '/watch', search: options.search ?? '?v=T4XZmMPQ9Kw' };
  runInNewContext(probe, { window, document, location, MediaSource, SourceBuffer, URL: ProbeURL, URLSearchParams, performance: { now: () => now }, Uint8Array, ArrayBuffer, DataView, SharedArrayBuffer, BigInt });
  const api = window.__atsumiYouTubeWebmShapeProbe as { snapshot: () => Snapshot } | undefined;
  const source = new MediaSource(); video.currentSrc = ProbeURL.createObjectURL(source);
  const buffer = source.addSourceBuffer('video/webm; codecs="vp09.00.51.08.01.01.01.01.00"');
  const snapshot = () => { if (!api) throw new Error('Probe unavailable in fixture'); return api.snapshot(); };
  const track = () => getTrack(snapshot());
  return { api, snapshot, track, source, buffer, video, events, location, calls, MediaSource, URL: ProbeURL, setAd: (value: boolean) => { ad = value; }, setNow: (value: number) => { now = value; } };
}

it('installs only at the configured exact canonical public watch document', () => {
  expect(setup({ origin: 'https://www.youtube.com.evil.invalid' }).api).toBeUndefined();
  expect(setup({ search: '?v=T4XZmMPQ9Kw&v=abcdefghijk' }).api).toBeUndefined();
  expect(setup({ search: '?v=abcdefghijk' }).api).toBeUndefined();
});
it('reads split EBML and Block headers at every append boundary without owning the caller buffer', () => {
  const bytes = join(init(), cluster(1000, block()));
  for (let cut = 0; cut <= bytes.length; cut++) {
    const s = setup(), first = bytes.slice(0, cut), last = bytes.slice(cut);
    s.buffer.appendBuffer(first); first.fill(0); s.buffer.appendBuffer(last); last.fill(255);
    const t = s.track(); expect(t.reason, `split ${cut}`).toBe('observing');
    expect(t.stats).toMatchObject({ completeInit: true, timestampScale: 1000000, trackNumber: 1, width: 320, height: 180, clusters: 1, blocks: 1, vp9Keys: 1, vp9Width: 320, vp9Height: 180, vp9BitDepth: 8, firstPtsSeconds: 1, openUnknownCluster: true, partialBlock: false });
    expect(item(t.clock, 0).endSeconds).toBeCloseTo(1.033333333, 8);
  }
});
it('handles a known Segment and Cluster without classifying a legal unknown final Cluster as malformed', () => {
  const s = setup();
  const header = element(0x1a45dfa3, element(0x4282, ascii('webm')));
  const rest = init().subarray(header.length + 5);
  s.buffer.appendBuffer(join(header, element(0x18538067, rest, element(0x1f43b675, uint(0xe7, 1000), block()))));
  expect(s.track().stats).toMatchObject({ completeInit: true, closedClusters: 1, segmentUnknown: 0 });
  const open = setup(); open.buffer.appendBuffer(join(init(), cluster(1000, block()))); open.setNow(55000);
  expect(open.track()).toMatchObject({ reason: 'done', stats: { openUnknownCluster: true, blocks: 1 } });
});
it('keeps absent duration unknown and uses BlockGroup duration/reference independently', () => {
  const s = setup(); s.buffer.appendBuffer(join(init({ duration: false }), cluster(1000, block())));
  expect(item(s.track().clock, 0).endSeconds).toBeNull();
  const group = element(0xa0, element(0xa1, [0x81, 0, 40, 0], keyframe()), uint(0x9b, 25), element(0xfb, [255]));
  s.buffer.appendBuffer(group);
  expect(s.track().stats).toMatchObject({ blockGroups: 1, blockDurations: 1, referenceBlocks: 1, containerKeys: 1 });
  expect(lastClock(s.track()).endSeconds).toBeCloseTo(1.065, 8);
});
it('closes an unknown Cluster at the next Cluster and preserves signed relative times', () => {
  const s = setup(); s.buffer.appendBuffer(join(init(), cluster(1000, block(-10)), cluster(2000, block(30))));
  expect(s.track().stats).toMatchObject({ clusters: 2, closedClusters: 1, firstPtsSeconds: .99, lastPtsSeconds: 2.03 });
});
it('recognizes lacing topology without claiming VP9 frame header validity for laced payload', () => {
  for (const [flags, header] of [[0x82, [1, 1]], [0x84, [1]], [0x86, [1, 0x81]]] as const) {
    const s = setup(); s.buffer.appendBuffer(join(init(), cluster(1000, block(0, Uint8Array.of(0x86, 0x86), flags, [...header]))));
    expect(s.track().reason).toBe('observing'); expect(s.track().stats).toMatchObject({ lacedFrames: 2, vp9HeaderUnknown: 1, vp9Keys: 0 });
  }
  const bad = setup(); bad.buffer.appendBuffer(join(init(), cluster(1000, block(0, Uint8Array.of(1, 2), 0x82, [1, 3]))));
  expect(bad.track().reason).toBe('malformed');
});
it('distinguishes first-frame inter/show-existing from validated superframe tail only', () => {
  const s = setup(); const key = keyframe(), inter = Uint8Array.of(0x86), marker = 0xc1;
  const superframe = join(key, inter, [marker, key.length, inter.length, marker]);
  s.buffer.appendBuffer(join(init(), cluster(1000, block(0, superframe), block(33, Uint8Array.of(0x88)), block(66, Uint8Array.of(0x84)), block(99, Uint8Array.of(0x86)))));
  expect(s.track().stats).toMatchObject({ superframes: 1, superframeFrames: 2, vp9Keys: 1, vp9ShowExisting: 1, vp9Invisible: 1, vp9Inter: 2 });
  s.buffer.appendBuffer(block(132, join(key, [marker, 250, 1, marker]))); expect(s.track().stats.superframeInvalid).toBe(1);
});
it('stops malformed/overlarge/deep structures with fixed reasons without throwing into playback', () => {
  for (const data of [Uint8Array.of(0), join(init(), unknown(0x1f43b675, block())), join(init({ scale: 0 }), cluster(0, block()))]) {
    const s = setup(); expect(s.buffer.appendBuffer(data)).toBe(17); expect(s.track().reason).toBe('malformed');
  }
  const s = setup(); s.buffer.appendBuffer(new Uint8Array(16 * 1024 * 1024 + 1)); expect(s.track().reason).toBe('limit'); expect(s.track().bytes).toBe(0);
  const depth = setup(); let nested = new Uint8Array(); for (let i = 0; i < 14; i++) nested = element(0xae, nested); depth.buffer.appendBuffer(nested); expect(depth.track().reason).toBe('limit');
});
it('projects AAC metadata clocks while skipping mdat bytes and retaining no caller buffer', () => {
  const data = join(aacInit(), aacFragment());
  for (const cut of [0, 1, 7, 16, 31, 70, 130, data.length - 5, data.length]) {
    const s = setup(), a = s.source.addSourceBuffer('audio/mp4; codecs="mp4a.40.2"');
    const first = data.slice(0, cut), last = data.slice(cut); a.appendBuffer(first); first.fill(255); a.appendBuffer(last); last.fill(0);
    const audio = getTrack(s.snapshot(), 0, 1);
    expect(audio).toMatchObject({ reason: 'observing', stats: { completeInit: true, audioTimescale: 48000, audioSamples: 2, firstDtsSeconds: 1, mdats: 1 } });
    expect(item(audio.clock, 0).endSeconds).toBeCloseTo(1 + 2048 / 48000, 8);
  }
});
it('does not read AAC clock fields across atom boundaries, duplicate tfdt or a different track', () => {
  const invalid = [
    box('moof', box('traf', box('tfhd', be32(8), be32(1), be32(1024)), box('tfdt', be32(0)), box('trun', be32(0), be32(2)))),
    box('moof', box('traf', box('tfhd', be32(8), be32(2), be32(1024)), box('tfdt', be32(0), be32(48000)), box('trun', be32(0), be32(2)))),
    box('moof', box('traf', box('tfhd', be32(8), be32(1), be32(1024)), box('tfdt', be32(0), be32(48000)), box('tfdt', be32(0), be32(48000)), box('trun', be32(0), be32(2)))),
    box('moof', box('traf', box('tfhd', be32(4), be32(1)), box('tfdt', be32(0), be32(48000)), box('trun', be32(0), be32(2)))),
    box('moof', box('traf', box('tfhd', be32(8), be32(1)), box('tfdt', be32(0), be32(48000)), box('trun', be32(0), be32(2)))),
  ];
  for (const bytes of invalid) {
    const s = setup(), a = s.source.addSourceBuffer('audio/mp4;codecs=mp4a.40.2'); a.appendBuffer(aacInit()); a.appendBuffer(bytes);
    expect(getTrack(s.snapshot(), 0, 1)).toMatchObject({ reason: 'malformed', clock: [], stats: { firstDtsSeconds: null } });
  }
  const s = setup(), a = s.source.addSourceBuffer('audio/mp4;codecs=mp4a.40.2');
  a.appendBuffer(join(aacInit(), aacFragment(48000), aacInit(), aacFragment(240000)));
  expect(getTrack(s.snapshot(), 0, 1)).toMatchObject({ generation: 2, stats: { firstDtsSeconds: 5, audioSamples: 2 } });
  expect(getTrack(s.snapshot(), 0, 1).clock).toHaveLength(1);
});
it('reports contemporaneous clock offsets and changes without zeroing tracks', () => {
  const s = setup(); s.buffer.timestampOffset = -1; s.buffer.appendBuffer(join(init(), cluster(1000, block())));
  s.buffer.timestampOffset = -2; s.buffer.appendBuffer(block(33));
  expect(s.track().clock.map(x => x.offsetSeconds)).toEqual([-1, -2]); expect(s.track().offsetChanged).toBe(true);
  expect(s.track().stats.firstPtsSeconds).toBe(1);
  s.buffer.timestampOffset = 1e9 + 1; s.buffer.appendBuffer(block(66)); expect(lastClock(s.track()).offsetSeconds).toBeNull();
});
it('starts a fresh bounded structural generation only at a full new initialization boundary', () => {
  const s = setup(); s.buffer.appendBuffer(join(init(), cluster(1000, block()), init({ duration: false }), cluster(4000, block())));
  expect(s.track()).toMatchObject({ reason: 'observing', generation: 2, stats: { completeInit: true, firstPtsSeconds: 4, blocks: 1 } });
  expect(s.track().clock).toHaveLength(1); expect(item(s.track().clock, 0).endSeconds).toBeNull();
});
it('does not mix source replacement, unselected tracks, removed buffers or excluded protected/ad intervals', () => {
  const s = setup(); s.buffer.appendBuffer(join(init(), cluster(1000, block())));
  const other = new s.MediaSource(), b = other.addSourceBuffer('video/webm;codecs=vp9'); b.appendBuffer(init());
  expect(s.snapshot().sources.map(x => x.selected)).toEqual([true, false]);
  s.video.currentSrc = s.URL.createObjectURL(other); expect(s.snapshot().video.selectedSource).toBe(2);
  expect(s.source.removeSourceBuffer(s.buffer)).toBe(23); expect(s.track().reason).toBe('detached');
  s.setAd(true); b.appendBuffer(cluster(2000, block())); expect(getTrack(s.snapshot(), 1).reason).toBe('excluded');
  s.setAd(false); b.appendBuffer(cluster(3000, block())); expect(getTrack(s.snapshot(), 1).stats.blocks).toBe(0);
  const protectedState = setup(); protectedState.video.mediaKeys = {}; protectedState.buffer.appendBuffer(init()); expect(protectedState.track().reason).toBe('excluded'); expect(protectedState.snapshot().video.selectedSource).toBeNull();
});
it('preserves method results/receivers/throws, bounds generations and emits no raw bytes or URLs', () => {
  const s = setup(), payload = join(init(), cluster(1000, block()));
  expect(s.buffer.appendBuffer(payload)).toBe(17); expect(item(s.calls, 0)).toEqual({ receiver: s.buffer, args: [payload] });
  s.buffer.fail = true; expect(() => s.buffer.appendBuffer(payload)).toThrow('original_failure'); expect(s.track().appends).toBe(1); s.buffer.fail = false;
  for (let i = 0; i < 8; i++) expect(s.buffer.changeType('video/webm;codecs=vp9')).toBe(19);
  expect(s.track()).toMatchObject({ generation: 8, reason: 'limit' });
  const text = JSON.stringify(s.snapshot()); expect(text).not.toMatch(/blob:|private-|https:|T4XZmMPQ9Kw|V_VP9/);
  expect(s.URL.revokeObjectURL(s.video.currentSrc)).toBe(29); expect(s.snapshot().video.selectedSource).toBeNull();
});
it('bounds observation count, source count and elapsed deadline without later rearming', () => {
  const s = setup(); for (let i = 0; i < 130; i++) s.buffer.appendBuffer(new Uint8Array());
  expect(s.track()).toMatchObject({ appends: 128, reason: 'limit' });
  for (let i = 0; i < 12; i++) new s.MediaSource().addSourceBuffer('video/webm;codecs=vp9'); expect(s.snapshot().sources).toHaveLength(8);
  const pagehide = s.events.get('pagehide'); if (!pagehide) throw new Error('Missing pagehide fixture handler');
  pagehide(); const count = s.snapshot().totalBytes; s.buffer.appendBuffer(init()); expect(s.snapshot()).toMatchObject({ stopped: true, totalBytes: count });
});
