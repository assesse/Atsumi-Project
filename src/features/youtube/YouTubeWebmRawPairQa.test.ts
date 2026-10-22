// @ts-expect-error Node-only QA script harness.
import { runInNewContext } from 'node:vm';
import { expect, it } from 'vitest';
import probe from '../../../src-tauri/examples/youtube_webm_raw_pair_qa.js?raw';

const join = (...parts: Array<Uint8Array | number[]>) => Uint8Array.from(parts.flatMap(p => Array.from(p)));
const ascii = (value: string) => Uint8Array.from([...value].map(c => c.charCodeAt(0)));
const hex = (value: number) => { const text = value.toString(16).padStart(value.toString(16).length + value.toString(16).length % 2, '0'); return Uint8Array.from(text.match(/../g)!.map(b => parseInt(b, 16))); };
const size = (value: number) => value < 127 ? Uint8Array.of(128 | value) : value < 16383 ? Uint8Array.of(64 | (value >> 8), value & 255) : Uint8Array.of(32 | (value >> 16), (value >> 8) & 255, value & 255);
const element = (id: number, ...parts: Array<Uint8Array | number[]>) => { const body = join(...parts); return join(hex(id), size(body.length), body); };
const uint = (id: number, value: number) => element(id, hex(value));
const float64 = (value: number) => { const bytes = new Uint8Array(8); new DataView(bytes.buffer).setFloat64(0, value); return bytes; };
const unknown = (id: number, ...parts: Array<Uint8Array | number[]>) => join(hex(id), [255], ...parts);
function keyframe(width = 320, height = 180) {
  const bits = '10000011' + (0x498342).toString(2).padStart(24, '0') + '0010' + (width - 1).toString(2).padStart(16, '0') + (height - 1).toString(2).padStart(16, '0');
  return Uint8Array.from(bits.padEnd(Math.ceil(bits.length / 8) * 8, '0').match(/.{8}/g)!.map(b => parseInt(b, 2)));
}
const init = (options: { scale?: number; duration?: boolean; codec?: string; lace?: number; width?: number; height?: number; trackScale?: number; codecDelayNs?: number } = {}) => join(
  element(0x1a45dfa3, element(0x4282, ascii('webm'))), unknown(0x18538067),
  element(0x1549a966, options.scale === undefined ? [] : uint(0x2ad7b1, options.scale)),
  element(0x1654ae6b, element(0xae, uint(0xd7, 1), uint(0x83, 1), element(0x86, ascii(options.codec ?? 'V_VP9')), uint(0x9c, options.lace ?? 1),
    options.trackScale === undefined ? [] : element(0x23314f, float64(options.trackScale)), options.codecDelayNs === undefined ? [] : uint(0x56aa, options.codecDelayNs),
    options.duration === false ? [] : uint(0x23e383, 33333333), element(0xe0, uint(0xb0, options.width ?? 320), uint(0xba, options.height ?? 180)))));
const block = (pts = 0, payload = keyframe(), flags = 0x80, laceHeader: number[] = []) => element(0xa3, [0x81, (pts >> 8) & 255, pts & 255, flags], laceHeader, payload);
const cluster = (pts: number, ...blocks: Uint8Array[]) => unknown(0x1f43b675, uint(0xe7, pts), ...blocks);
const be32 = (n: number) => Uint8Array.of(n >>> 24, n >>> 16, n >>> 8, n);
const box = (id: string, ...parts: Array<Uint8Array | number[]>) => { const body = join(...parts); return join(be32(body.length + 8), ascii(id), body); };
const aacInit = () => { const tkhd = new Uint8Array(84); tkhd.set(be32(1), 12); return join(box('ftyp', ascii('isom'), be32(0)), box('moov', box('trak', box('tkhd', tkhd), box('mdia', box('mdhd', be32(0), be32(0), be32(0), be32(48000), be32(0), be32(0)))), box('mvex', box('trex', be32(0), be32(1), be32(1), be32(1024), be32(0), be32(0))))); };
const aacFragment = (pts = 48000, duration = 1024) => join(box('moof', box('traf', box('tfhd', be32(8), be32(1), be32(duration)), box('tfdt', be32(0), be32(pts)), box('trun', be32(0), be32(2)))), box('mdat', new Uint8Array(300).fill(222)));


type Unit = { unit: number; track: 'video' | 'audio'; generation: number; startOffset: number; endOffset: number; byteLength: number; initLength: number; startSeconds: number; endSeconds: number; configuration: Record<string, number | string | null> };
type TrackDiagnostics = { units: number; adjacentPairs: number; adjacentTriples: number; initDifferences: number; configurationDifferences: number; byteGenerationBreaks: number; clockOrderBreaks: number; gaps: number };
type Snapshot = { version: number; captureId: string; documentId: string; state: string; reason: string; sourceOrdinal: number | null; elapsedMs: number; totalBytes: number; trackBytes: { video: number; audio: number }; excludedPartialUnits: number; requiresAudioJoin: boolean; diagnostics: { video: TrackDiagnostics; audio: TrackDiagnostics }; units: Unit[] };
type Chunk = { version: number; captureId: string; documentId: string; unit: number; generation: number; offset: number; byteLength: number; final: boolean; base64: string };
type Api = { snapshot: () => Snapshot; readChunk: (capture: string, document: string, unit: number, offset: number) => Chunk | null };
const captureId = 'f029bf78-d276-445d-b3fd-171387690f4f', documentId = '88428559-d8f3-49ec-981c-14f8df1c05da';
function item<T>(values: readonly T[], at: number): T { const value = values[at]; if (!Number.isInteger(at) || at < 0 || value === undefined) throw new Error('Missing bounded fixture item'); return value; }
function setup(options: { origin?: string; search?: string; capture?: string } = {}) {
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
    removeSourceBuffer(buffer: SourceBuffer) { this.sourceBuffers = this.sourceBuffers.filter(value => value !== buffer); return 23; }
    addEventListener(name: string, callback: () => void) { this.listeners.set(name, callback); }
  }
  class ProbeURL extends URL { static createObjectURL(_object: unknown) { return 'blob:private-' + (++serial); } static revokeObjectURL(_url: string) { return 29; } }
  const video = { currentSrc: '', src: '', mediaKeys: null as unknown, srcObject: null as unknown };
  const document = { querySelector: (selector: string) => selector === '#movie_player video' ? video : ad ? {} : null };
  const events = new Map<string, () => void>(), location = { origin: options.origin ?? 'https://www.youtube.com', pathname: '/watch', search: options.search ?? '?v=T4XZmMPQ9Kw' };
  const window: Record<string, unknown> = { __atsumiYouTubeRawPairQaCaptureId: options.capture ?? captureId, addEventListener: (name: string, callback: () => void) => events.set(name, callback) }; window.top = window;
  runInNewContext(probe, { window, document, location, MediaSource, SourceBuffer, URL: ProbeURL, URLSearchParams, performance: { now: () => now },
    crypto: { randomUUID: () => documentId }, btoa, Uint8Array, ArrayBuffer, DataView, SharedArrayBuffer, BigInt });
  const api = window.__atsumiYouTubeRawPairQa as Api | undefined;
  const source = new MediaSource(); video.currentSrc = ProbeURL.createObjectURL(source);
  const buffer = source.addSourceBuffer('video/webm;codecs="vp09.00.51.08.01.01.01.01.00"'), audio = source.addSourceBuffer('audio/mp4; codecs="mp4a.40.2"');
  const snapshot = () => { if (!api) throw new Error('Missing fixture API'); return api.snapshot(); };
  const read = (unit: number, offset = 0) => { if (!api) throw new Error('Missing fixture API'); return api.readChunk(captureId, documentId, unit, offset); };
  return { api, snapshot, read, source, buffer, audio, video, events, location, calls, MediaSource, URL: ProbeURL, setAd: (value: boolean) => { ad = value; }, setNow: (value: number) => { now = value; } };
}
const videoUnit = (start = 1000, payload = keyframe()) => join(init(), cluster(start, block(0, payload)));
const audioUnit = (start = 48000, duration = 1024) => join(aacInit(), aacFragment(start, duration));
const videoRun = (count = 3, start = 1000) => join(...Array.from({ length: count }, (_, i) => videoUnit(start + i * 33)), init());
const audioRun = (count = 3, start = 48000, duration = 1024) => join(...Array.from({ length: count }, (_, i) => audioUnit(start + i * duration * 2, duration)), aacInit());
function ready() { const s = setup(); s.buffer.appendBuffer(videoRun()); s.audio.appendBuffer(audioRun()); return s; }
const decode = (chunk: Chunk | null) => { if (!chunk) throw new Error('Missing fixture chunk'); return Uint8Array.from(atob(chunk.base64), value => value.charCodeAt(0)); };
const zeroDiagnostics = { units: 0, adjacentPairs: 0, adjacentTriples: 0, initDifferences: 0, configurationDifferences: 0, byteGenerationBreaks: 0, clockOrderBreaks: 0, gaps: 0 };

it('installs only in the exact public top-level video with a canonical native UUID', () => {
  expect(setup({ origin: 'https://www.youtube.com.evil.invalid' }).api).toBeUndefined();
  expect(setup({ search: '?v=abcdefghijk' }).api).toBeUndefined();
  expect(setup({ search: '?v=T4XZmMPQ9Kw&v=T4XZmMPQ9Kw' }).api).toBeUndefined();
  expect(setup({ capture: '../../wrong' }).api).toBeUndefined();
});
it('projects only closed bounded diagnostics, including zero and immutable terminal counters', () => {
  expect(setup().snapshot().diagnostics).toEqual({ video: zeroDiagnostics, audio: zeroDiagnostics });
  const s = ready(), counters = { ...zeroDiagnostics, units: 3, adjacentPairs: 2, adjacentTriples: 1 };
  const before = s.snapshot().diagnostics;
  expect(before).toEqual({ video: counters, audio: counters });
  expect(Object.keys(before).sort()).toEqual(['audio','video']);
  for (const value of Object.values(before)) {
    expect(Object.keys(value).sort()).toEqual(Object.keys(zeroDiagnostics).sort());
    for (const count of Object.values(value)) expect(Number.isInteger(count) && count >= 0 && count <= 16).toBe(true);
    expect(Object.isFrozen(value)).toBe(true);
  }
  expect(Object.isFrozen(before)).toBe(true);
  s.setNow(15000); s.setAd(true); s.buffer.appendBuffer(videoRun());
  expect(s.snapshot().diagnostics).toBe(before);
  expect(JSON.stringify(before)).not.toMatch(/base64|blob:|https:|T4XZmMPQ9Kw|"generation"|"startOffset"|"startSeconds"|"trackNumber"|"width"|"height"|"profile"/i);
});
it('exports three consecutive videos and AAC units early with original offsets and closed v3 metadata', () => {
  const s = ready(), result = s.snapshot();
  expect(result).toMatchObject({ version: 3, captureId, documentId, state: 'ready', reason: 'complete', sourceOrdinal: 1, requiresAudioJoin: true });
  expect(result.units).toHaveLength(6);
  expect(item(result.units, 0)).toMatchObject({ unit: 1, track: 'video', generation: 1, startOffset: 0, endOffset: videoUnit().length, initLength: init().length, startSeconds: 1 });
  expect(item(result.units, 3)).toMatchObject({ unit: 4, track: 'audio', generation: 1, startOffset: 0, endOffset: audioUnit().length, initLength: aacInit().length, startSeconds: 1 });
  for (let i = 0; i < 3; i++) {
    expect(item(result.units, i)).toMatchObject({ unit: i + 1, track: 'video', generation: i + 1, startOffset: videoUnit().length * i });
    expect(item(result.units, i + 3)).toMatchObject({ unit: i + 4, track: 'audio', generation: i + 1, startOffset: audioUnit().length * i });
    expect(decode(s.read(i + 1))).toEqual(videoUnit(1000 + i * 33));
    expect(decode(s.read(i + 4))).toEqual(audioUnit(48000 + i * 2048));
  }
  expect(result.totalBytes).toBe(result.trackBytes.video + result.trackBytes.audio);
  expect(JSON.stringify(result)).not.toMatch(/base64|private-|blob:|https:|T4XZmMPQ9Kw/);
});
it('recognizes structural repeated-init boundaries at every split rather than payload magic', () => {
  const bytes = videoUnit(1033);
  for (let cut = 0; cut <= bytes.length; cut++) {
    const s = setup(), first = bytes.slice(0, cut), last = bytes.slice(cut); s.buffer.appendBuffer(videoUnit());
    s.buffer.appendBuffer(first); first.fill(0); s.buffer.appendBuffer(last); last.fill(255);
    s.buffer.appendBuffer(join(videoUnit(1066), init())); s.audio.appendBuffer(audioRun());
    expect(s.snapshot().state, 'split ' + cut).toBe('ready'); expect(decode(s.read(2))).toEqual(bytes);
  }
  const s = setup(), magicPayload = join(keyframe(), [0x1a,0x45,0xdf,0xa3,0x66,0x74,0x79,0x70]);
  const raw = videoUnit(1000, magicPayload); s.buffer.appendBuffer(join(raw, videoUnit(1033), videoUnit(1066), init())); s.audio.appendBuffer(audioRun());
  expect(s.snapshot().state).toBe('ready'); expect(decode(s.read(1))).toEqual(raw);
});
it('recognizes AAC init/media boundaries at every split and never exports a partial mdat', () => {
  const bytes = audioUnit(50048);
  for (let cut = 0; cut <= bytes.length; cut++) {
    const s = setup(); s.buffer.appendBuffer(videoRun()); s.audio.appendBuffer(audioUnit());
    const first = bytes.slice(0, cut); s.audio.appendBuffer(first); first.fill(1); s.audio.appendBuffer(bytes.slice(cut));
    s.audio.appendBuffer(join(audioUnit(52096), aacInit()));
    expect(s.snapshot().state, 'audio split ' + cut).toBe('ready'); expect(decode(s.read(5))).toEqual(bytes);
  }
  const partial = setup(); partial.buffer.appendBuffer(videoRun()); partial.audio.appendBuffer(join(audioUnit(), audioUnit(50048).subarray(0, audioUnit().length - 1)));
  partial.setNow(15000); expect(partial.snapshot()).toMatchObject({ state: 'stopped', reason: 'no_candidate', units: [] });
});
it('does not promote an unknown terminal Cluster or absent duration to a complete exported unit', () => {
  const s = setup(); s.buffer.appendBuffer(join(videoUnit(), videoUnit(1033))); s.audio.appendBuffer(audioRun()); s.setNow(15000);
  expect(s.snapshot()).toMatchObject({ state: 'stopped', reason: 'no_candidate', units: [] });
  expect(s.snapshot().diagnostics.video).toEqual({ ...zeroDiagnostics, units: 1 });
  const unknownDuration = setup(); unknownDuration.buffer.appendBuffer(join(init({ duration: false }), cluster(1000, block()), init()));
  unknownDuration.audio.appendBuffer(audioRun()); unknownDuration.setNow(15000);
  expect(unknownDuration.snapshot().state).toBe('stopped');
  expect(unknownDuration.snapshot().diagnostics.video).toEqual(zeroDiagnostics);
});
it('preserves BlockGroup duration and checks first actual key-frame random access', () => {
  const group = element(0xa0, element(0xa1, [0x81,0,0,0], keyframe()), uint(0x9b, 25));
  const s = setup(); s.buffer.appendBuffer(join(...[1000,1025,1050].map(start => join(init({ duration: false }), cluster(start, group))), init({ duration: false })));
  s.audio.appendBuffer(audioRun()); expect(s.snapshot().state).toBe('ready');
  expect(item(s.snapshot().units, 0).endSeconds).toBe(1.025);
  const nonKey = setup(); nonKey.buffer.appendBuffer(join(videoUnit(), videoUnit(1033, Uint8Array.of(0x86)), videoUnit(1066), init())); nonKey.audio.appendBuffer(audioRun()); nonKey.setNow(15000);
  expect(nonKey.snapshot().state).toBe('stopped');
});
it('selects by full video coverage rather than equal generation and prefers two AAC candidates', () => {
  const s = setup(); s.buffer.appendBuffer(videoRun()); s.audio.appendBuffer(audioRun(3, 48000, 1280));
  expect(s.snapshot()).toMatchObject({ state: 'ready', requiresAudioJoin: true });
  expect(s.snapshot().units).toHaveLength(5);
  expect(s.snapshot().units.filter(unit => unit.track === 'audio')).toHaveLength(2);
  const different = setup(); different.buffer.appendBuffer(videoRun()); different.audio.appendBuffer(audioRun(4, 45952));
  expect(item(different.snapshot().units, 0).generation).toBe(1);
  expect(item(different.snapshot().units, 3).generation).toBe(2);
  const short = setup(); short.buffer.appendBuffer(videoRun()); short.audio.appendBuffer(audioRun(3, 45000)); short.setNow(15000);
  expect(short.snapshot()).toMatchObject({ state: 'stopped', reason: 'no_candidate', units: [] });
});
it('retains two videos and two original AAC files only at the deadline, without single-pair fallback', () => {
  const s = setup(); s.buffer.appendBuffer(videoRun(2)); s.audio.appendBuffer(audioRun(2));
  s.setNow(14999); expect(s.snapshot()).toMatchObject({ version: 3, state: 'collecting', units: [], requiresAudioJoin: false });
  s.setNow(15000); expect(s.snapshot()).toMatchObject({ state: 'ready', requiresAudioJoin: true, elapsedMs: 15000 });
  expect(s.snapshot().units).toHaveLength(4);
  expect(decode(s.read(1))).toEqual(videoUnit()); expect(decode(s.read(2))).toEqual(videoUnit(1033));
  expect(decode(s.read(3))).toEqual(audioUnit()); expect(decode(s.read(4))).toEqual(audioUnit(50048));
  const single = setup(); single.buffer.appendBuffer(join(videoUnit(), init())); single.audio.appendBuffer(audioRun()); single.setNow(15000);
  expect(single.snapshot()).toMatchObject({ state: 'stopped', reason: 'no_candidate', units: [], requiresAudioJoin: false });
});
it('allows two-video deadline coverage by three AAC units and observed audio overlap without declaring a join', () => {
  const s = setup(); s.buffer.appendBuffer(videoRun(2)); s.audio.appendBuffer(audioRun(3, 46976));
  expect(s.snapshot().state).toBe('collecting'); s.setNow(15000);
  expect(s.snapshot().units.map(unit => unit.track)).toEqual(['video','video','audio','audio','audio']);
  const overlap = setup(); overlap.buffer.appendBuffer(videoRun());
  overlap.audio.appendBuffer(join(...[48000,49500,51000].map(start => audioUnit(start)), aacInit()));
  expect(overlap.snapshot()).toMatchObject({ state: 'ready', requiresAudioJoin: true });
  expect(overlap.snapshot().units).toHaveLength(6);
  for (const key of ['mediaVerified','recordingVerified','joined','gapless']) expect(overlap.snapshot()).not.toHaveProperty(key);
});
it('does not skip a rejected middle generation or accept duplicate and reversed unit times', () => {
  const missing = setup();
  missing.buffer.appendBuffer(join(videoUnit(), videoUnit(1033, Uint8Array.of(0x86)), videoUnit(1066), init()));
  missing.audio.appendBuffer(audioRun()); missing.setNow(15000);
  expect(missing.snapshot()).toMatchObject({ state: 'stopped', reason: 'no_candidate', units: [] });
  expect(missing.snapshot().diagnostics.video).toEqual({ ...zeroDiagnostics, units: 2, byteGenerationBreaks: 1, gaps: 1 });
  for (const starts of [[1000,1000,1000], [1066,1033,1000]]) {
    const s = setup(); s.buffer.appendBuffer(join(...starts.map(start => videoUnit(start)), init()));
    s.audio.appendBuffer(audioRun()); s.setNow(15000); expect(s.snapshot().reason).toBe('no_candidate');
    expect(s.snapshot().diagnostics.video).toEqual({ ...zeroDiagnostics, units: 3, clockOrderBreaks: 2 });
  }
  for (const starts of [[48000,48000,48000], [52096,50048,48000]]) {
    const s = setup(); s.buffer.appendBuffer(videoRun());
    s.audio.appendBuffer(join(...starts.map(start => audioUnit(start)), aacInit())); s.setNow(15000);
    expect(s.snapshot().reason).toBe('no_candidate');
    expect(s.snapshot().diagnostics.audio).toEqual({ ...zeroDiagnostics, units: 3, clockOrderBreaks: 2 });
  }
});
it('preserves independent video units with different init bytes or lengths without claiming strict adjacency', () => {
  const header = element(0x1a45dfa3, element(0x4282, ascii('webm')));
  const differentLength = join(element(0x1a45dfa3, element(0x4282, ascii('webm')), element(0xec, [1])), init().subarray(header.length));
  const sameLength = init({ lace: 0 }); expect(sameLength.length).toBe(init().length); expect(sameLength).not.toEqual(init());
  expect(differentLength.length).not.toBe(init().length);
  for (const alternate of [sameLength, differentLength]) {
    const s = setup(), middle = join(alternate, cluster(1033, block()));
    s.buffer.appendBuffer(join(videoUnit(), middle, videoUnit(1066), init())); s.audio.appendBuffer(audioRun());
    const result = s.snapshot(); expect(result).toMatchObject({ version: 3, state: 'ready', reason: 'complete' });
    expect(result.units).toHaveLength(6);
    expect(result.diagnostics.video).toEqual({ ...zeroDiagnostics, units: 3, initDifferences: 2 });
    expect(item(result.units, 1).initLength).toBe(alternate.length);
    expect(item(result.units, 2).startOffset).toBe(videoUnit().length + middle.length);
    expect(decode(s.read(1))).toEqual(videoUnit()); expect(decode(s.read(2))).toEqual(middle); expect(decode(s.read(3))).toEqual(videoUnit(1066));
    for (const key of ['initializationsIdentical','codecEquivalent','joined','mediaVerified','recordingVerified']) expect(result).not.toHaveProperty(key);
  }
});
it('excludes an unfinished third unit and never takes new bytes at the exact deadline', () => {
  const s = setup(), third = videoUnit(1066);
  s.buffer.appendBuffer(join(videoUnit(), videoUnit(1033), third.subarray(0, third.length - 1))); s.audio.appendBuffer(audioRun());
  const before = s.snapshot().trackBytes.video; s.setNow(15000);
  s.buffer.appendBuffer(join(third.subarray(third.length - 1), init()));
  const result = s.snapshot(); expect(result).toMatchObject({ state: 'ready', elapsedMs: 15000, trackBytes: { video: before } });
  expect(result.units.filter(unit => unit.track === 'video')).toHaveLength(2);
  expect(result.excludedPartialUnits).toBeGreaterThanOrEqual(1);
  expect(decode(s.read(2))).toEqual(videoUnit(1033));
});
it('keeps different complete video configurations independent and selects only the covered unit', () => {
  const s = setup(), changedInit = init({ width: 640, height: 360 });
  const changedUnit = join(changedInit, cluster(2000, block(0, keyframe(640, 360))));
  s.buffer.appendBuffer(join(videoUnit(), ...[2000,2033,2066].map(start => join(changedInit, cluster(start, block(0, keyframe(640, 360))))), changedInit));
  s.audio.appendBuffer(audioRun(3, 96000));
  expect(s.snapshot().state).toBe('ready');
  expect(item(s.snapshot().units, 0)).toMatchObject({ generation: 2, startOffset: videoUnit().length, startSeconds: 2, configuration: { width: 640, height: 360 } });
  expect(decode(s.read(1))).toEqual(changedUnit);
  expect(s.snapshot().diagnostics.video).toEqual({ ...zeroDiagnostics, units: 4, adjacentPairs: 2, adjacentTriples: 1, initDifferences: 1, configurationDifferences: 1, gaps: 1 });
});
it('still rejects a raw evidence run across differing observed video configurations', () => {
  const s = setup(), changed = join(init({ width: 640, height: 360 }), cluster(1033, block(0, keyframe(640, 360))));
  s.buffer.appendBuffer(join(videoUnit(), changed, videoUnit(1066), init())); s.audio.appendBuffer(audioRun()); s.setNow(15000);
  expect(s.snapshot()).toMatchObject({ state: 'stopped', reason: 'no_candidate', units: [] });
  expect(s.snapshot().diagnostics.video).toEqual({ ...zeroDiagnostics, units: 3, initDifferences: 2, configurationDifferences: 2 });
  expect(s.read(1)).toBeNull();
});
it('rejects an observed AAC gap while preserving differing original audio initializations as separate evidence', () => {
  const s = setup(); s.buffer.appendBuffer(videoRun()); s.audio.appendBuffer(join(audioUnit(), audioUnit(50049), audioUnit(52098), aacInit())); s.setNow(15000);
  expect(s.snapshot().state).toBe('stopped');
  expect(s.snapshot().diagnostics.audio).toEqual({ ...zeroDiagnostics, units: 3, adjacentPairs: 2, adjacentTriples: 1, gaps: 2 });
  const newInit = aacInit(); newInit[15] = 1;
  const longerInit = join(aacInit().subarray(0, 16), box('free', [1]), aacInit().subarray(16));
  for (const alternate of [newInit, longerInit]) {
    const changed = setup(), middle = join(alternate, aacFragment(50048)); changed.buffer.appendBuffer(videoRun());
    changed.audio.appendBuffer(join(audioUnit(), middle, audioUnit(52096), aacInit()));
    expect(changed.snapshot().state).toBe('ready');
    expect(changed.snapshot().diagnostics.audio).toEqual({ ...zeroDiagnostics, units: 3, initDifferences: 2 });
    expect(item(changed.snapshot().units, 4).initLength).toBe(alternate.length);
    expect(decode(changed.read(4))).toEqual(audioUnit()); expect(decode(changed.read(5))).toEqual(middle); expect(decode(changed.read(6))).toEqual(audioUnit(52096));
  }
});
it('reports video gaps without adding a new selection gate and bounds a full track to 16 units', () => {
  const gapped = setup();
  gapped.buffer.appendBuffer(join(...[1000,1034,1068].map(start => videoUnit(start)), init()));
  gapped.audio.appendBuffer(audioRun(3, 48000, 1280));
  expect(gapped.snapshot().state).toBe('ready');
  expect(gapped.snapshot().diagnostics.video).toEqual({ ...zeroDiagnostics, units: 3, adjacentPairs: 2, adjacentTriples: 1, gaps: 2 });
  const bounded = setup(); bounded.buffer.appendBuffer(videoRun(17));
  expect(bounded.snapshot()).toMatchObject({ state: 'stopped', reason: 'limit' });
  expect(bounded.snapshot().diagnostics.video).toEqual({ ...zeroDiagnostics, units: 16, adjacentPairs: 15, adjacentTriples: 14 });
});
it('validates exact transfer identity and bounds while allowing deterministic repeat reads', () => {
  const s = ready(); if (!s.api) throw new Error('Missing API');
  expect(s.api.readChunk('00000000-0000-4000-8000-000000000000', documentId, 1, 0)).toBeNull();
  expect(s.api.readChunk(captureId, '00000000-0000-4000-8000-000000000000', 1, 0)).toBeNull();
  for (const [unit, offset] of [[0,0],[7,0],[1,-1],[1,.5],[1,videoUnit().length]]) expect(s.read(unit ?? -1, offset ?? -1)).toBeNull();
  expect(s.read(1)).toEqual(s.read(1)); expect(s.read(1)).toMatchObject({ version: 3, offset: 0, byteLength: videoUnit().length, final: true });
  const before = setup(); expect(before.read(1)).toBeNull();
  s.location.search = '?v=abcdefghijk'; expect(s.read(1)).toBeNull();
});
it('caps chunks at 128KiB, keeps exact byte order and leaves immutable terminal metadata', () => {
  const s = setup(), raw = videoUnit(1000, join(keyframe(), new Uint8Array(180000).fill(7)));
  s.buffer.appendBuffer(join(raw, videoUnit(1033), videoUnit(1066), init())); s.audio.appendBuffer(audioRun());
  const first = s.read(1), second = s.read(1, 131072); expect(first).toMatchObject({ byteLength: 131072, final: false });
  expect(join(decode(first), decode(second))).toEqual(raw);
  const before = JSON.stringify(s.snapshot()); s.setAd(true); s.video.currentSrc = 'blob:changed'; s.setNow(100000); s.buffer.appendBuffer(new Uint8Array(10));
  expect(JSON.stringify(s.snapshot())).toBe(before); expect(s.read(1)).toEqual(first);
});
it('does not alter native method return, receiver, arguments, exceptions or caller buffers', () => {
  const s = setup(), input = videoUnit(), original = input.slice();
  expect(s.buffer.appendBuffer(input)).toBe(17); expect(input).toEqual(original); expect(item(s.calls, 0)).toEqual({ receiver: s.buffer, args: [input] });
  input.fill(0); s.buffer.fail = true; expect(() => s.buffer.appendBuffer(init())).toThrow('original_failure');
  s.buffer.fail = false; s.buffer.appendBuffer(join(videoUnit(1033), videoUnit(1066), init())); s.audio.appendBuffer(audioRun());
  expect(decode(s.read(1))).toEqual(original);
});
it('fails closed on source, ad, protected-media and SourceBuffer configuration changes', () => {
  const cases = [
    (s: ReturnType<typeof setup>) => s.setAd(true),
    (s: ReturnType<typeof setup>) => { s.video.mediaKeys = {}; },
    (s: ReturnType<typeof setup>) => { s.video.srcObject = {}; },
    (s: ReturnType<typeof setup>) => { s.buffer.timestampOffset = 1; },
    (s: ReturnType<typeof setup>) => { s.audio.mode = 'sequence'; },
    (s: ReturnType<typeof setup>) => { s.buffer.appendWindowEnd = 30; },
    (s: ReturnType<typeof setup>) => { s.video.currentSrc = s.URL.createObjectURL(new s.MediaSource()); },
  ];
  for (const change of cases) { const s = setup(); s.buffer.appendBuffer(videoUnit()); change(s); s.audio.appendBuffer(audioUnit()); expect(s.snapshot().state).toBe('stopped'); expect(s.read(1)).toBeNull(); }
  const changed = setup(); changed.buffer.appendBuffer(videoUnit()); expect(changed.buffer.changeType('video/webm;codecs=vp9')).toBe(19); expect(changed.snapshot().reason).toBe('configuration_changed');
});
it('checks source, exclusion and buffer configuration before selecting a waiting candidate at 15000ms', () => {
  const cases: Array<[string, (s: ReturnType<typeof setup>) => void]> = [
    ['excluded', s => s.setAd(true)],
    ['excluded', s => { s.video.mediaKeys = {}; }],
    ['excluded', s => { s.video.srcObject = {}; }],
    ['source_changed', s => { s.video.currentSrc = s.URL.createObjectURL(new s.MediaSource()); }],
    ['source_changed', s => { s.location.search = '?v=abcdefghijk'; }],
    ['configuration_changed', s => { s.buffer.timestampOffset = 1; }],
    ['configuration_changed', s => { s.audio.mode = 'sequence'; }],
    ['configuration_changed', s => { s.buffer.appendWindowEnd = 30; }],
  ];
  for (const [reason, change] of cases) {
    const s = setup(); s.buffer.appendBuffer(videoRun(2)); s.audio.appendBuffer(audioRun(2));
    expect(s.snapshot().state).toBe('collecting'); s.setNow(15000); change(s);
    expect(s.snapshot()).toMatchObject({ state: 'stopped', reason, units: [], requiresAudioJoin: false });
    expect(s.read(1)).toBeNull();
  }
});
it('ignores unselected source bytes and rejects SharedArrayBuffer observation without changing playback', () => {
  const s = setup(), other = new s.MediaSource(), b = other.addSourceBuffer('video/webm;codecs=vp9');
  expect(b.appendBuffer(videoUnit())).toBe(17); expect(s.snapshot().totalBytes).toBe(0);
  expect(s.buffer.appendBuffer(new SharedArrayBuffer(16))).toBe(17); expect(s.snapshot()).toMatchObject({ state: 'stopped', reason: 'unsupported', totalBytes: 0 });
});
it('keeps malformed headers and in-unit dimension changes explicit without parser resynchronization', () => {
  const malformed = setup(); expect(malformed.buffer.appendBuffer(Uint8Array.of(0))).toBe(17); expect(malformed.snapshot().reason).toBe('malformed');
  const changed = setup(); changed.buffer.appendBuffer(join(init(), cluster(1000, block(0, keyframe(640, 360))), init()));
  expect(changed.snapshot().reason).toBe('configuration_changed');
  const timebase = setup(); timebase.buffer.appendBuffer(join(videoUnit(), element(0x1549a966, uint(0x2ad7b1, 2000000)), init()));
  expect(timebase.snapshot().reason).toBe('configuration_changed');
  const fakeBoundary = setup(); fakeBoundary.buffer.appendBuffer(join(Uint8Array.of(0), videoUnit(), init())); expect(fakeBoundary.snapshot().state).toBe('stopped');
});
it('rejects non-unit TrackTimestampScale and nonzero CodecDelay instead of silently ignoring them', () => {
  for (const options of [{ trackScale: 2 }, { codecDelayNs: 1 }]) {
    const s = setup(); s.buffer.appendBuffer(join(init(options), cluster(1000, block()), init()));
    expect(s.snapshot()).toMatchObject({ state: 'stopped', reason: 'unsupported', units: [] });
  }
  const s = setup(), explicit = init({ trackScale: 1, codecDelayNs: 0 });
  s.buffer.appendBuffer(join(...[1000,1033,1066].map(start => join(explicit, cluster(start, block()))), explicit));
  s.audio.appendBuffer(audioRun()); expect(s.snapshot().state).toBe('ready');
});
it('enforces byte/call/deadline caps and never rearms after terminal state', () => {
  const sizeCap = setup(); sizeCap.buffer.appendBuffer(new Uint8Array(16777217)); expect(sizeCap.snapshot()).toMatchObject({ state: 'stopped', reason: 'limit', totalBytes: 0 });
  const calls = setup(); for (let i = 0; i < 4097; i++) calls.buffer.appendBuffer(new Uint8Array()); expect(calls.snapshot().reason).toBe('limit');
  const deadline = setup(); deadline.buffer.appendBuffer(videoUnit()); deadline.setNow(15000); deadline.audio.appendBuffer(join(audioUnit(), aacInit()));
  expect(deadline.snapshot()).toMatchObject({ state: 'stopped', reason: 'no_candidate', elapsedMs: 15000, trackBytes: { audio: 0 } });
  const closed = setup(); const handler = closed.events.get('pagehide'); if (!handler) throw new Error('Missing handler'); handler();
  closed.buffer.appendBuffer(videoUnit()); expect(closed.snapshot()).toMatchObject({ state: 'stopped', reason: 'closed', totalBytes: 0 });
});

