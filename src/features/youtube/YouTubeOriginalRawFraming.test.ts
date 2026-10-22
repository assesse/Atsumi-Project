import { expect, it } from 'vitest';
import script from '../../../src-tauri/src/youtube/original_raw_framing.js?raw';
const vmName = 'node:vm', fsName = 'node:fs', cryptoName = 'node:crypto';
const { runInNewContext } = await import(vmName);
const { existsSync } = await import(fsName);
const fixedRawFolder = '.runtime/youtube-webm-raw-pair-qa/c26c9147-735f-4ed3-821b-31e92cef9afd/';
const hasFixedRawEvidence = existsSync(fixedRawFolder + 'manifest.json');
const join = (...parts: Uint8Array[]) => Uint8Array.from(parts.flatMap(part => [...part]));
const id = (value: number) => Uint8Array.from(value.toString(16).padStart(Math.ceil(value.toString(16).length / 2) * 2, '0').match(/../g)!.map(v => parseInt(v, 16)));
const element = (value: number, payload = new Uint8Array()) => join(id(value), payload.length < 127 ? Uint8Array.of(0x80 | payload.length) : Uint8Array.of(0x40 | payload.length >> 8, payload.length & 255), payload);
const unknown = (value: number) => join(id(value), Uint8Array.of(255));
const init = (padding = 0) => join(element(0x1a45dfa3, Uint8Array.of(1)), unknown(0x18538067), element(0xec, new Uint8Array(padding)), element(0x1549a966), element(0x1654ae6b, Uint8Array.of(2)));
const media = (payload = Uint8Array.of(3, 4, 5), finite = false) => {
  const body = element(0xa3, payload);
  return finite ? element(0x1f43b675, body) : join(unknown(0x1f43b675), body);
};
const box = (kind: string, payload = new Uint8Array()) => {
  const out = new Uint8Array(8 + payload.length); new DataView(out.buffer).setUint32(0, out.length);
  out.set([...kind].map(c => c.charCodeAt(0)), 4); out.set(payload, 8); return out;
};
const audioInit = () => join(box('ftyp', Uint8Array.of(1)), box('moov', Uint8Array.of(2)));
const audioMedia = () => join(box('skip', Uint8Array.of(7, 8)), box('moof', Uint8Array.of(3)), box('mdat', Uint8Array.of(4, 5)));
type Event = { kind: 'init' | 'data'; init?: Uint8Array; bytes?: Uint8Array; unitIndex?: number; startOffset?: number; rawFirstUnitEnd?: number };
type Framer = { feed(bytes: Uint8Array, emit?: (event: Event) => void): void; arm(): void; disarm(): void; init: Uint8Array | null; completed: number; failed: boolean; offset: number };
function setup(role: 'video' | 'audio') {
  const window: Record<string, unknown> = {};
  runInNewContext(script, { window, Uint8Array, ArrayBuffer, DataView, BigInt, Date });
  const api = window.__atsumiYouTubeRawFraming as { create(role: string): Framer; role(mime: string): string | null };
  const f = api.create(role), events: Event[] = [];
  const feed = (bytes: Uint8Array) => f.feed(bytes, event => events.push({ ...event, ...(event.bytes ? { bytes: event.bytes.slice() } : {}), ...(event.init ? { init: event.init.slice() } : {}) }));
  const bytes = () => join(...events.filter(e => e.kind === 'data').map(e => e.bytes!));
  return { f, events, feed, bytes, api };
}
it('retains initialization only before approval, then begins at the next exact structural init', () => {
  const t = setup('video'), first = join(init(), media()); t.feed(first);
  expect(t.f.init).toEqual(init()); expect(t.events).toHaveLength(0);
  t.f.arm(); t.feed(element(0xa3, new Uint8Array(10)));
  expect(t.events).toHaveLength(0);
  const retained = join(init(3), media(), init(1), media()); t.feed(retained);
  expect(t.bytes()).toEqual(retained);
  expect(t.events.filter(e => e.kind === 'init').map(e => [e.unitIndex, e.startOffset])).toEqual([[0, 0], [1, init(3).length + media().length]]);
  expect(t.events.filter(e => e.rawFirstUnitEnd !== undefined).map(e => e.rawFirstUnitEnd)).toEqual([init(3).length + media().length]);
});
it.each(['video', 'audio'] as const)('preserves every %s byte across one-byte header and payload splits', (role: 'video' | 'audio') => {
  const t = setup(role), unit = role === 'video' ? join(init(), media()) : join(audioInit(), audioMedia());
  t.feed(unit); t.f.arm();
  for (const byte of join(unit, unit)) t.feed(Uint8Array.of(byte));
  expect(t.bytes()).toEqual(join(unit, unit)); expect(t.events.filter(e => e.kind === 'init')).toHaveLength(2);
});
it('excludes an initialization whose first header bytes arrived before approval', () => {
  const t = setup('video'), unit = join(init(), media()); t.feed(unit); t.feed(unit.subarray(0, 2)); t.f.arm();
  t.feed(unit.subarray(2)); expect(t.events).toHaveLength(0);
  t.feed(unit); expect(t.bytes()).toEqual(unit);
});
it('does not mistake EBML signatures inside finite payloads for another initialization', () => {
  const t = setup('video'), unit = join(init(), media(join(init(), media()), true));
  t.feed(unit); t.f.arm(); t.feed(unit);
  expect(t.events.filter(e => e.kind === 'init')).toHaveLength(1); expect(t.bytes()).toEqual(unit);
});
it('preserves repeated AAC skip/init bytes and resets only at a parsed ftyp boundary', () => {
  const t = setup('audio'), unit = join(audioInit(), audioMedia()); t.feed(unit); t.f.arm(); t.feed(join(unit, unit));
  expect(t.bytes()).toEqual(join(unit, unit)); expect(t.events.filter(e => e.kind === 'init').map(e => e.startOffset)).toEqual([0, unit.length]);
});
it('disarm stops storage immediately and a fresh arm does not inherit old media', () => {
  const t = setup('video'), unit = join(init(), media()); t.feed(unit); t.f.arm(); t.feed(unit); t.f.disarm(); t.feed(unit);
  expect(t.bytes()).toEqual(unit); t.events.length = 0; t.f.arm(); t.feed(unit);
  expect(t.events.find(e => e.kind === 'init')?.unitIndex).toBe(0);
});
it('rejects unsupported parents and malformed sizes without searching for another header', () => {
  const t = setup('video'); expect(() => t.feed(join(element(0x1a45dfa3), element(0x18538067)))).toThrow();
  expect(t.f.failed).toBe(true); expect(() => t.feed(init())).toThrow();
  const a = setup('audio'); expect(() => a.feed(Uint8Array.of(0, 0, 0, 1, 102, 116, 121, 112))).toThrow();
});
it('rejects missing media before repeated init, metadata over the cap, and unsupported unknown leaves', () => {
  const t = setup('video'); t.feed(init()); expect(() => t.feed(init())).toThrow();
  const big = setup('audio'); expect(() => big.feed(box('ftyp', new Uint8Array(65536)))).toThrow();
  const leaf = setup('video'); leaf.feed(init()); leaf.feed(unknown(0x1f43b675)); expect(() => leaf.feed(unknown(0xa3))).toThrow();
});
it('classifies only the closed original MIME subset', () => {
  const t = setup('video'); expect(t.api.role('video/webm; codecs="vp09.00.51.08.01.01.01.01.00"')).toBe('video');
  expect(t.api.role('audio/mp4; codecs="mp4a.40.2"')).toBe('audio');
  for (const mime of ['video/webm;codecs=vp8', 'video/webm;codecs=vp9,opus', 'audio/webm;codecs=opus', 'video/mp4;codecs=avc1.640028', 'video/webm;codecs=\u00a0vp9', ' '.repeat(121)]) expect(t.api.role(mime)).toBeNull();
});
it.skipIf(!hasFixedRawEvidence)('read-only exact-hash original four-unit evidence survives split framing byte-for-byte', async () => {
  const { readFileSync } = await import(fsName), { createHash } = await import(cryptoName);
  const folder = fixedRawFolder;
  const files = [
    ['video-1.webm', 95635, '9be82861467e27ed66fa8220db1a7c4f1e2872996755257cbeafe149035070a4'],
    ['video-2.webm', 76753, '7bb5f6e6f3fcd435042dc5b7227ca023799014f7f3e7886e4a99bc732451a4b6'],
    ['audio-1.mp4', 81810, 'c349dc8672a8ade3d961d19c7d0b30f09473b08b96c7eb58641cbaceb385779a'],
    ['audio-2.mp4', 82185, 'b9fa4f91f7dc48bee2a9f7128188f27b3b041b6faf16f91e19b7a3e17daf7457'],
  ] as const;
  const original = files.map(([name, length, hash]) => { const bytes = new Uint8Array(readFileSync(folder + name)); expect(bytes.length).toBe(length); expect(createHash('sha256').update(bytes).digest('hex')).toBe(hash); return bytes; });
  for (const [role, first, second] of [['video', original[0]!, original[1]!], ['audio', original[2]!, original[3]!]] as const) {
    const t = setup(role); t.feed(first); t.f.arm(); const selected = join(first, second);
    for (let at = 0; at < selected.length; at += 137) t.feed(selected.subarray(at, at + 137));
    expect(t.bytes()).toEqual(selected); expect(t.events.filter(e => e.kind === 'init')).toHaveLength(2);
  }
  files.forEach(([name, , hash]) => expect(createHash('sha256').update(readFileSync(folder + name)).digest('hex')).toBe(hash));
});
