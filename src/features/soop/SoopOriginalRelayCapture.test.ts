import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import capture from '../../../src-tauri/src/soop/original_capture.js?raw';
import page from '../../../src-tauri/src/soop/original_page.js?raw';
const vmName = 'node:vm';
const { runInNewContext } = await import(vmName) as { runInNewContext(source: string, context: Record<string, unknown>): unknown };
type Message = { id: string; epoch: number; type: string; requestId?: string; runId?: string; trackIndex?: number; appendIndex?: number; chunkIndex?: number; data?: string; sourceId?: string; reason?: string; [key: string]: unknown };
type Adapter = {
  configure(value: unknown): boolean; start(id: string): Promise<unknown>; stop(): Promise<unknown>;
  relayStart(id: string, epoch: number): Promise<unknown>; relayStop(id: string, epoch: number): Promise<unknown>;
  reply(id: string, response: unknown): void; relayReply(id: string, response: unknown, epoch: number): boolean;
  relayClockProbe(nonce: string, runId: string, epoch: number): boolean;
};
type Capture = { getStatus(): { recording: boolean }; getRelayStatus(): { active: boolean; runId: string | null; queuedBytes: number; accepting: boolean; detail: string }; getDiagnostics(): unknown };
const recordId = '10000000-0000-4000-8000-000000000001';
const relayId = '20000000-0000-4000-8000-000000000001';
const secondRelayId = '20000000-0000-4000-8000-000000000002';
const concat = (...parts: Uint8Array[]) => { const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0)); let at = 0; for (const p of parts) { out.set(p, at); at += p.length; } return out; };
const box = (kind: string, size = 12, marker = 1) => { const b = new Uint8Array(size); b.fill(marker, 8); new DataView(b.buffer).setUint32(0, size); for (let i = 0; i < 4; i++) b[4 + i] = kind.charCodeAt(i); return b; };
const fragment = (marker = 1) => concat(box('moof', 12, marker), box('mdat', 24, marker));
const flush = async () => { for (let i = 0; i < 60; i++) await Promise.resolve(); };

// These boxes exercise the page observer/IPC, not the native codec admission.
function fixture(splitTracks = false) {
  class Events {
    listeners = new Map<string, Set<() => void>>();
    addEventListener(name: string, fn: () => void) { const set = this.listeners.get(name) ?? new Set(); set.add(fn); this.listeners.set(name, set); }
    removeEventListener(name: string, fn: () => void) { this.listeners.get(name)?.delete(fn); }
    emit(name: string) { for (const fn of [...this.listeners.get(name) ?? []]) fn(); }
  }
  const originalCalls: unknown[] = [];
  class SourceBuffer extends Events {
    mode = 'segments'; timestampOffset = 0; appendWindowStart = 0; appendWindowEnd = Infinity; failure: Error | null = null;
    appendBuffer(value: unknown) { if (this.failure) throw this.failure; originalCalls.push(value); return 'accepted-once'; }
    changeType(_type: string) { return 'changed'; }
  }
  class MediaSource extends Events {
    sourceBuffers: SourceBuffer[] = [];
    addSourceBuffer(_type: string) { const buffer = new SourceBuffer(); this.sourceBuffers.push(buffer); return buffer; }
    removeSourceBuffer(buffer: SourceBuffer) { this.sourceBuffers = this.sourceBuffers.filter(b => b !== buffer); }
  }
  class Url { static createObjectURL(_source: unknown) { return 'blob:fixture'; } }
  const video = Object.assign(new Events(), { tagName: 'VIDEO', readyState: 4, videoWidth: 960, videoHeight: 540, isConnected: true,
    currentSrc: '', src: '', mediaKeys: null as unknown, srcObject: null as unknown, ended: false, paused: false, seeking: false,
    currentTime: 12, playbackRate: 1, play: vi.fn() });
  const messages: Message[] = [], posts: unknown[] = [], nodes = new Map<string, unknown>([['livePlayer', video]]);
  const document = { title: 'Synthetic fixture', head: null, documentElement: { appendChild: (node: { id: string }) => nodes.set(node.id, node), style: { setProperty: vi.fn() } },
    getElementById: (id: string) => nodes.get(id), createElement: () => ({ id: '', textContent: '' }), addEventListener: vi.fn(),
    get cookie(): never { throw Error('No credential reads'); } };
  let hold: (m: Message) => boolean = () => false;
  const location = new URL('https://play.sooplive.com/fixture/12345/embed?fromApi=1');
  const window = Object.assign(new Events(), { top: null as unknown, location, MediaSource, SourceBuffer, URL: Url,
    livePlayer: { liveAPI: { getResponse: () => ({ CHANNEL: { BJID: 'fixture', BNO: 12345 } }) } }, liveView: { isLoaded: () => Promise.resolve() },
    postMessage: (value: unknown) => posts.push(value), chrome: { webview: { postMessage: (raw: string) => { const m = JSON.parse(raw.slice('ATSUMI_SOOP_V1:'.length)) as Message; messages.push(m); if (!hold(m)) queueMicrotask(() => reply(m)); } } },
    __atsumiSoopOriginal: undefined as Adapter | undefined, __atsumiSoopEncodedCapture: undefined as Capture | undefined });
  window.top = window;
  const value = (m: Message) => m.type === 'encoded_begin' ? { id: recordId, mode: 'encoded', nativeApproved: true, captureChat: false } :
    m.type === 'encoded_finish' ? { stopped: true, interrupted: false } : m.type === 'relay_begin' ? { runId: m.requestId, mode: 'relay', nativeApproved: true, acceptedInMemory: true, captureChat: false } :
      m.type === 'relay_finish' ? { acceptedInMemory: true, stopRequested: true, interrupted: m.interrupted } : { acceptedInMemory: true };
  function reply(m: Message, result: unknown = { ok: true, value: value(m) }, replyEpoch = m.epoch) {
    if (m.type.startsWith('relay_')) return window.__atsumiSoopOriginal!.relayReply(m.id, result, replyEpoch);
    window.__atsumiSoopOriginal!.reply(m.id, result); return true;
  }
  const context = { window, document, location, crypto: { randomUUID: () => crypto.randomUUID() }, Uint8Array, ArrayBuffer, DataView,
    btoa, setTimeout, clearTimeout, setInterval, clearInterval, Date, queueMicrotask, performance: { now: () => 250.5 } };
  runInNewContext(capture, context); runInNewContext(page, context);
  const source = new MediaSource(); video.currentSrc = video.src = Url.createObjectURL(source);
  const buffers = [source.addSourceBuffer(splitTracks ? 'video/mp4;codecs="avc1.42E01E"' : 'video/mp4;codecs="avc1.42E01E,mp4a.40.2"')];
  if (splitTracks) buffers.push(source.addSourceBuffer('audio/mp4;codecs="mp4a.40.2"'));
  for (const buffer of buffers) buffer.appendBuffer(concat(box('ftyp'), box('moov')));
  const adapter = window.__atsumiSoopOriginal!, observer = window.__atsumiSoopEncodedCapture!;
  adapter.configure({ epoch: 1, channel: 'fixture' });
  const ofType = (type: string) => messages.filter(m => m.type === type);
  const payload = (type: string, track = 0) => concat(...ofType(type).filter(m => m.trackIndex === track).map(m => Uint8Array.from(atob(m.data!), c => c.charCodeAt(0))));
  return { window, adapter, observer, source, buffers, video, messages, posts, originalCalls, ofType, payload, reply, hold: (fn: typeof hold) => { hold = fn; },
    rerunCapture: () => runInNewContext(capture, context) };
}
beforeEach(() => vi.useFakeTimers());
afterEach(() => { vi.clearAllTimers(); vi.useRealTimers(); });

describe('SOOP independent relay consumer', () => {
  it('observes the admitted relay source clock with recording off without changing playback', async () => {
    const f=fixture();await f.adapter.relayStart(relayId,1);f.buffers[0]!.timestampOffset=2;
    expect(f.adapter.relayClockProbe(recordId,relayId,1)).toBe(true);
    expect(f.ofType('relay_clock')[0]).toMatchObject({nonce:recordId,runId:relayId,sample:{sourceTimeSeconds:10,currentTimeSeconds:12,
      timestampOffsetSeconds:2,performanceNowMs:250.5,paused:false,seeking:false,readyState:4}});
    expect(f.observer.getStatus().recording).toBe(false);expect(f.ofType('encoded_begin')).toHaveLength(0);
    expect(f.video.currentTime).toBe(12);expect(f.video.playbackRate).toBe(1);expect(f.video.play).not.toHaveBeenCalled();
    f.video.seeking=true;f.video.paused=true;expect(f.adapter.relayClockProbe(secondRelayId,relayId,1)).toBe(true);
    expect(f.ofType('relay_clock')[1]).toMatchObject({sample:{seeking:true,paused:true}});
  });
  it('rejects foreign run epoch and source loss without fabricating a zero clock', async () => {
    const f=fixture();await f.adapter.relayStart(relayId,1);
    expect(f.adapter.relayClockProbe(recordId,secondRelayId,1)).toBe(false);
    expect(f.adapter.relayClockProbe(recordId,relayId,2)).toBe(false);
    f.video.currentSrc=f.video.src='blob:other';
    expect(f.adapter.relayClockProbe(recordId,relayId,1)).toBe(true);
    expect(f.ofType('relay_clock')[0]!.sample).toBeNull();
    await f.adapter.relayStop(relayId,1);expect(f.adapter.relayClockProbe(recordId,relayId,1)).toBe(false);
  });
  it('observes each accepted append once without changing native return, exception or caller bytes', async () => {
    const f = fixture(); await f.adapter.relayStart(relayId, 1); const hook = Object.getPrototypeOf(f.buffers[0]!).appendBuffer;
    f.rerunCapture(); expect(Object.getPrototypeOf(f.buffers[0]!).appendBuffer).toBe(hook);
    const bytes = fragment(7), expected = bytes.slice(); const before = f.originalCalls.length;
    expect(f.buffers[0]!.appendBuffer(bytes)).toBe('accepted-once'); expect(f.originalCalls).toHaveLength(before + 1); expect(f.originalCalls.at(-1)).toBe(bytes);
    bytes.fill(99); await flush(); expect(f.payload('relay_append')).toEqual(expected);
    const error = new Error('quota'); f.buffers[0]!.failure = error; expect(() => f.buffers[0]!.appendBuffer(fragment())).toThrow(error);
    expect(f.ofType('relay_append')).toHaveLength(1); expect(f.observer.getStatus().recording).toBe(false); expect(f.ofType('encoded_begin')).toHaveLength(0);
    expect(f.video.currentTime).toBe(12); expect(f.video.playbackRate).toBe(1); expect(f.video.play).not.toHaveBeenCalled(); expect(f.posts).toHaveLength(1);
  });

  it('fanouts the same bytes and stopping archive leaves relay forwarding and ordinals intact', async () => {
    const f = fixture(); await f.adapter.start(recordId); await f.adapter.relayStart(relayId, 1);
    const a = fragment(2), b = fragment(3); f.buffers[0]!.appendBuffer(a); await flush();
    expect(f.payload('encoded_append')).toEqual(a); expect(f.payload('relay_append')).toEqual(a);
    await f.adapter.stop(); f.buffers[0]!.appendBuffer(b); await flush();
    expect(f.payload('encoded_append')).toEqual(a); expect(f.payload('relay_append')).toEqual(concat(a, b));
    expect(f.ofType('relay_append').map(m => m.appendIndex)).toEqual([0, 1]); expect(f.observer.getRelayStatus().active).toBe(true);
    const reply = await f.adapter.relayStop(relayId, 1); expect(reply).toMatchObject({ stopRequested: true }); expect(reply).not.toHaveProperty('stopped');
  });

  it('archive joining in a relay mdat waits for the next moof and relay stop leaves archive running', async () => {
    const f = fixture(); await f.adapter.relayStart(relayId, 1);
    const a = fragment(4), b = fragment(5), c = fragment(6);
    f.buffers[0]!.appendBuffer(a.subarray(0, 24)); await flush(); await f.adapter.start(recordId);
    f.buffers[0]!.appendBuffer(concat(a.subarray(24), b)); await flush();
    expect(f.payload('relay_append')).toEqual(concat(a, b)); expect(f.payload('encoded_append')).toEqual(b);
    await f.adapter.relayStop(relayId, 1); f.buffers[0]!.appendBuffer(c); await flush();
    expect(f.payload('relay_append')).toEqual(concat(a, b)); expect(f.payload('encoded_append')).toEqual(concat(b, c));
    expect(f.observer.getStatus().recording).toBe(true);
  });

  it('preserves chunk framing and private bytes across shared header and payload parts', async () => {
    const f = fixture(); await f.adapter.start(recordId); await f.adapter.relayStart(relayId, 1);
    const bytes = concat(box('moof'), box('mdat', 128 * 1024 + 9, 73)), expected = bytes.slice();
    f.buffers[0]!.appendBuffer(bytes); bytes.fill(0); await flush();
    for (const type of ['encoded_append', 'relay_append']) {
      expect(f.payload(type)).toEqual(expected);
      expect(f.ofType(type).map(m => [m.appendIndex, m.chunkIndex, m.finalChunk])).toEqual([[0, 0, false], [0, 1, true]]);
    }
  });

  it('new relay skips an in-progress moof header independently on two tracks', async () => {
    const f = fixture(true); await f.adapter.start(recordId);
    const a = fragment(8), b = fragment(9);
    f.buffers[0]!.appendBuffer(a.subarray(0, 4)); f.buffers[1]!.appendBuffer(a.subarray(0, 23));
    await f.adapter.relayStart(relayId, 1);
    f.buffers[0]!.appendBuffer(concat(a.subarray(4), b)); f.buffers[1]!.appendBuffer(concat(a.subarray(23), b)); await flush();
    for (let track = 0; track < 2; track++) { expect(f.payload('encoded_append', track)).toEqual(concat(a, b)); expect(f.payload('relay_append', track)).toEqual(b); }
  });

  it('stalled relay ACK does not occupy archive replies and times out without retrying', async () => {
    const f = fixture(); await f.adapter.start(recordId); await f.adapter.relayStart(relayId, 1); f.hold(m => m.type === 'relay_append');
    f.buffers[0]!.appendBuffer(fragment()); await flush(); await f.adapter.stop();
    expect(f.ofType('encoded_finish')).toHaveLength(1); expect(f.observer.getRelayStatus().queuedBytes).toBeGreaterThan(0);
    const old = f.ofType('relay_append')[0]!; f.adapter.reply(old.id, { ok: true, value: { acceptedInMemory: true } }); await flush();
    expect(f.observer.getRelayStatus().queuedBytes).toBeGreaterThan(0);
    await vi.advanceTimersByTimeAsync(3001); await flush();
    expect(f.ofType('relay_append')).toHaveLength(1); expect(f.ofType('relay_finish')).toHaveLength(1); expect(f.ofType('relay_finish')[0]!.interrupted).toBe(true);
    expect(f.observer.getRelayStatus()).toMatchObject({ active: false, queuedBytes: 0 }); expect(f.reply(old)).toBe(false);
  });

  it('stalled archive ACK does not delay relay append or its finish control lane', async () => {
    const f = fixture(); await f.adapter.start(recordId); await f.adapter.relayStart(relayId, 1); f.hold(m => m.type === 'encoded_append');
    const bytes = fragment(); f.buffers[0]!.appendBuffer(bytes); await flush();
    expect(f.payload('relay_append')).toEqual(bytes); await f.adapter.relayStop(relayId, 1);
    expect(f.observer.getStatus().recording).toBe(true); expect(f.ofType('relay_finish')).toHaveLength(1);
    f.reply(f.ofType('encoded_append')[0]!); await flush(); await f.adapter.stop();
  });

  it('normal stop drains appends arriving around ACK completion before sending finish', async () => {
    const f = fixture(); await f.adapter.relayStart(relayId, 1); const parts: Uint8Array[] = [];
    for (let i = 0; i < 24; i++) { const bytes = fragment(i); parts.push(bytes); f.buffers[0]!.appendBuffer(bytes); await Promise.resolve(); }
    await f.adapter.relayStop(relayId, 1);
    expect(f.payload('relay_append')).toEqual(concat(...parts));
    const ordered = f.messages.filter(m => m.type === 'relay_append' || m.type === 'relay_finish');
    expect(ordered.at(-1)?.type).toBe('relay_finish'); expect(f.ofType('relay_append').map(m => m.appendIndex)).toEqual(parts.map((_, i) => i));
    expect(f.observer.getRelayStatus()).toMatchObject({ active: false, queuedBytes: 0 });
  });

  it('begin timeout cancels the known native run and rejects a late approval', async () => {
    const f = fixture(); f.hold(m => m.type === 'relay_begin'); const start = f.adapter.relayStart(relayId, 1).catch(error => error); await flush();
    f.buffers[0]!.appendBuffer(fragment()); await vi.advanceTimersByTimeAsync(3001); await start; await flush();
    expect(f.ofType('relay_finish')[0]!).toMatchObject({ runId: relayId, interrupted: true, reason: 'begin_ack_timeout' });
    expect(f.ofType('relay_append')).toHaveLength(0); expect(f.observer.getRelayStatus()).toMatchObject({ active: false, queuedBytes: 0 });
    expect(f.reply(f.ofType('relay_begin')[0]!)).toBe(false);
  });

  it('epoch replacement fences late begin and finish replies without reviving an old run', async () => {
    const f = fixture(); f.hold(m => m.type === 'relay_begin'); const oldStart = f.adapter.relayStart(relayId, 1).catch(error => error); await flush();
    const oldBegin = f.ofType('relay_begin')[0]!; f.adapter.configure({ epoch: 2, channel: 'fixture' }); await oldStart; await flush();
    f.hold(() => false); await f.adapter.relayStart(secondRelayId, 2); expect(f.reply(oldBegin)).toBe(false); expect(f.reply(oldBegin, undefined, 2)).toBe(false);
    expect(f.observer.getRelayStatus()).toMatchObject({ active: true, runId: secondRelayId }); await expect(f.adapter.relayStop(relayId, 1)).rejects.toThrow('relay_not_active');
    f.buffers[0]!.appendBuffer(fragment()); await flush(); expect(f.ofType('relay_append').every(m => m.runId === secondRelayId && m.epoch === 2)).toBe(true);
  });

  it('source replacement sends the interrupted finish using the original source binding', async () => {
    const f = fixture(); await f.adapter.start(recordId); await f.adapter.relayStart(relayId, 1); const begin = f.ofType('relay_begin')[0]!;
    f.window.livePlayer.liveAPI.getResponse = () => ({ CHANNEL: { BJID: 'fixture', BNO: 99999 } });
    f.buffers[0]!.appendBuffer(fragment()); await flush();
    expect(f.ofType('relay_append')).toHaveLength(0);
    expect(f.ofType('relay_finish')[0]!).toMatchObject({ actualChannel: 'fixture', actualBroadcast: '12345', sourceId: begin.sourceId, interrupted: true });
    expect(f.observer.getStatus().recording).toBe(true);
  });

  it('late append and finish replies from a stopped run cannot settle its successor', async () => {
    const f = fixture(); await f.adapter.relayStart(relayId, 1); f.hold(m => m.type === 'relay_append' || m.type === 'relay_finish');
    f.buffers[0]!.appendBuffer(fragment()); await flush(); const append = f.ofType('relay_append')[0]!;
    const stopped = f.adapter.relayStop(relayId, 1).catch(error => error); await vi.advanceTimersByTimeAsync(6001); await stopped; await flush();
    const finish = f.ofType('relay_finish')[0]!; f.hold(() => false); await f.adapter.relayStart(secondRelayId, 1);
    expect(f.reply(append)).toBe(false); expect(f.reply(finish)).toBe(false);
    expect(f.observer.getRelayStatus()).toMatchObject({ active: true, runId: secondRelayId });
    f.buffers[0]!.appendBuffer(fragment(8)); await flush(); expect(f.ofType('relay_append').at(-1)?.runId).toBe(secondRelayId);
  });

  it('native relay rejection does not poison recording or recreate a receiver', async () => {
    const f = fixture(); await f.adapter.start(recordId); await f.adapter.relayStart(relayId, 1); f.hold(m => m.type === 'relay_append');
    const a = fragment(1), b = fragment(2); f.buffers[0]!.appendBuffer(a); await flush();
    f.reply(f.ofType('relay_append')[0]!, { ok: false, code: 'OUTPUT_FULL', error: 'relay full' }); await flush();
    f.buffers[0]!.appendBuffer(b); await flush(); expect(f.payload('encoded_append')).toEqual(concat(a, b));
    expect(f.observer.getStatus().recording).toBe(true); expect(f.observer.getRelayStatus().active).toBe(false); expect(f.posts).toHaveLength(1);
  });

  it('relay queue overflow releases queued references while accepted original appends return', async () => {
    const f = fixture(); f.hold(m => m.type === 'relay_begin'); const start = f.adapter.relayStart(relayId, 1).catch(error => error); await flush();
    const bytes = concat(box('moof'), box('mdat', 12 * 1024 * 1024));
    for (let i = 0; i < 3; i++) expect(f.buffers[0]!.appendBuffer(bytes)).toBe('accepted-once');
    await start; await flush(); expect(f.observer.getRelayStatus()).toMatchObject({ active: false, queuedBytes: 0 });
    expect(f.ofType('relay_finish')[0]!).toMatchObject({ reason: 'queue_overflow', interrupted: true }); expect(f.ofType('relay_append')).toHaveLength(0);
    f.hold(() => false); await f.adapter.start(recordId); f.buffers[0]!.appendBuffer(fragment()); await flush(); expect(f.ofType('encoded_append')).toHaveLength(1);
  });

  it('pagehide closes relay admission without starting recording or touching playback controls', async () => {
    const f = fixture(); await f.adapter.relayStart(relayId, 1); f.hold(m => m.type === 'relay_append'); f.buffers[0]!.appendBuffer(fragment()); await flush();
    f.window.emit('pagehide'); await flush(); expect(f.observer.getRelayStatus()).toMatchObject({ active: false, queuedBytes: 0 });
    expect(f.ofType('encoded_begin')).toHaveLength(0); expect(f.video.currentTime).toBe(12); expect(f.video.playbackRate).toBe(1); expect(f.video.play).not.toHaveBeenCalled();
  });
});
