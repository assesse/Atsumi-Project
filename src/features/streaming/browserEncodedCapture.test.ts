import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import source from "../../../src-tauri/src/streaming/browser_encoded_capture.js?raw";

const vmName = "node:vm";
const { runInNewContext } = await import(vmName) as { runInNewContext(source: string, context: Record<string, unknown>): unknown };
const CHANNEL = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const REQUEST = "30000000-0000-4000-8000-000000000001";
const RECORDING = "40000000-0000-4000-8000-000000000001";
type Message = { kind: string; [key: string]: unknown };
type Status = { active: boolean; starting: boolean; stopping: boolean; recordingId: string | null; ready: boolean; detail: string; rateControlAllowed: boolean };
type Bridge = { canStart(video: unknown): boolean; canChangePlaybackRate(video: unknown): boolean; getStatus(): Status;
  getDiagnostics(): { reason: string; appendCount: number; sources: unknown[] };
  getReplayClock(video: unknown): null | { clock: string; sourceId: string; sourceTimeSeconds: number };
  start(command: Record<string, unknown>, options: Record<string, unknown>): Promise<unknown>; stop(reason?: string, interrupted?: boolean): Promise<unknown> };
class Target {
  listeners = new Map<string, Set<(event: Record<string, unknown>) => void>>();
  addEventListener(name: string, callback: (event: Record<string, unknown>) => void) { if (!this.listeners.has(name)) this.listeners.set(name, new Set()); this.listeners.get(name)!.add(callback); }
  removeEventListener(name: string, callback: (event: Record<string, unknown>) => void) { this.listeners.get(name)?.delete(callback); }
  dispatch(name: string, fields: Record<string, unknown> = {}) { for (const callback of [...this.listeners.get(name) ?? []]) callback({ target: this, ...fields }); }
}
const concat = (...parts: Uint8Array[]) => { const result = new Uint8Array(parts.reduce((n, p) => n + p.length, 0)); let at = 0; for (const part of parts) { result.set(part, at); at += part.length; } return result; };
const box = (kind: string, payload = new Uint8Array()) => {
  const result = new Uint8Array(payload.length + 8); new DataView(result.buffer).setUint32(0, result.length);
  for (let i = 0; i < 4; i++) result[4 + i] = kind.charCodeAt(i); result.set(payload, 8); return result;
};
const init = () => concat(box("ftyp", new Uint8Array([1,2,3,4])), box("moov", new Uint8Array([5,6,7,8])));
const media = (size = 16) => concat(box("moof", new Uint8Array([1,2,3,4])), box("mdat", new Uint8Array(size).fill(9)));
const flush = async () => { for (let i = 0; i < 80; i++) await Promise.resolve(); };

function fixture(options: { url?: string; iframe?: boolean; queueBytes?: number; supported?: boolean } = {}) {
  const nativeCalls: unknown[] = [];
  class Buffer extends Target {
    mode = "segments"; timestampOffset = 0; appendWindowStart = 0; appendWindowEnd = Infinity;
    throwNext: Error | null = null;
    appendBuffer(bytes: ArrayBuffer | ArrayBufferView) { if (this.throwNext) { const error = this.throwNext; this.throwNext = null; throw error; } nativeCalls.push({ buffer: this, bytes }); return "native-result"; }
    changeType(_mime: string) { return "native-change"; }
    abort() { return "native-abort"; }
    remove(_start: number, _end: number) { return "native-remove"; }
  }
  class MediaSource extends Target {
    sourceBuffers: Buffer[] = [];
    addSourceBuffer(_mime: string) { const buffer = new Buffer(); this.sourceBuffers.push(buffer); return buffer; }
    removeSourceBuffer(buffer: Buffer) { this.sourceBuffers = this.sourceBuffers.filter((b) => b !== buffer); }
  }
  class URLFactory { static createObjectURL(_value: unknown) { return `blob:fixture-${nativeCalls.length}`; } }
  const video = Object.assign(new Target(), { isConnected: true, readyState: 4, ended: false, paused: false, seeking: false,
    videoWidth: 1280, videoHeight: 720, mediaKeys: null as unknown, srcObject: null as unknown,
    src: "", currentSrc: "", currentTime: 100, playbackRate: 1, getBoundingClientRect: () => ({ width: 960, height: 540 }) });
  const videos = [video];
  const window = Object.assign(new Target(), { location: new URL(options.url ?? `https://chzzk.naver.com/live/${CHANNEL}`), top: null as unknown,
    URL: URLFactory, MediaSource: options.supported === false ? undefined : MediaSource, SourceBuffer: Buffer, __atsumiEncodedCapture: undefined as Bridge | undefined });
  window.top = options.iframe ? {} : window;
  const document = { title: "Synthetic fixture", querySelectorAll: (name: string) => name === "video" ? videos : [], get cookie(): never { throw new Error("No cookie reads"); } };
  let counter = 0;
  const context = { window, document, crypto: { randomUUID: () => `10000000-0000-4000-8000-${String(++counter).padStart(12, "0")}` },
    Uint8Array, ArrayBuffer, DataView, btoa, SharedArrayBuffer, Date, setInterval, clearInterval, setTimeout };
  const script = options.queueBytes ? source.replace("const MAX_QUEUE = 64 * 1024 * 1024;", `const MAX_QUEUE = ${options.queueBytes};`) : source;
  runInNewContext(script, context);
  const ms = new MediaSource(); video.src = URLFactory.createObjectURL(ms); video.currentSrc = video.src;
  const v = ms.addSourceBuffer("video/mp4; codecs=\"avc1.42E01E\"");
  const a = ms.addSourceBuffer("audio/mp4; codecs=\"mp4a.40.2\"");
  const messages: Message[] = []; const pending: Array<{ message: Message; resolve(value: unknown): void; reject(error: Error): void }> = [];
  const hold = new Set<string>(); const reject = new Set<string>(); const notices: string[] = [];
  const response = (message: Message) => message.kind === "encoded_begin" ? { id: RECORDING, mode: "encoded", nativeApproved: true, captureChat: true } :
    message.kind === "encoded_finish" ? { stopped: true, interrupted: message.interrupted } : {};
  const request = (kind: string, fields: Record<string, unknown>) => {
    const message = { kind, ...fields }; messages.push(message);
    if (reject.has(kind)) return Promise.reject(new Error("native failure"));
    if (hold.has(kind)) return new Promise((resolve, reject) => pending.push({ message, resolve, reject }));
    return Promise.resolve(response(message));
  };
  const bridge = () => window.__atsumiEncodedCapture!;
  const load = () => { v.appendBuffer(init()); a.appendBuffer(init()); };
  const start = (fields: Record<string, unknown> = {}, extras: Record<string, unknown> = {}) => bridge().start({ channelId: CHANNEL, rightsAcknowledged: true, requestId: REQUEST, ...fields },
    { request, post: vi.fn(), onStatus: (detail: string) => notices.push(detail), ...extras });
  const release = () => { for (const item of pending.splice(0)) item.resolve(response(item.message)); };
  return { window, video, ms, v, a, bridge, messages, nativeCalls, notices, hold, reject, pending, release, load, start,
    ofKind: (kind: string) => messages.filter((message) => message.kind === kind), again: () => runInNewContext(script, context) };
}
beforeEach(() => vi.useFakeTimers());
afterEach(() => { vi.clearAllTimers(); vi.useRealTimers(); vi.restoreAllMocks(); });

describe("already-received encoded MSE capture", () => {
  it("observes init metadata without starting requests, downloads or a decoder", () => {
    const f = fixture(); f.load(); expect(f.bridge().canStart(f.video)).toBe(true);
    expect(f.messages).toEqual([]); expect(f.nativeCalls).toHaveLength(2);
    for (const forbidden of ["fetch(", "XMLHttpRequest", "MediaRecorder", "captureStream(", "document.cookie", "localStorage", "new Worker", "createElement("]) expect(source).not.toContain(forbidden);
  });
  it.each(["https://evil.example/live/" + CHANNEL, `https://chzzk.naver.com/live/${CHANNEL}/chat`])("does not install on %s", (url) => { const f = fixture({ url }); expect(f.window.__atsumiEncodedCapture).toBeUndefined(); });
  it("does not hook subframes and fails closed without MSE", () => {
    expect(fixture({ iframe: true }).window.__atsumiEncodedCapture).toBeUndefined();
    const f = fixture({ supported: false }); f.load(); expect(f.bridge().canStart(f.video)).toBe(false);
  });
  it("preserves original append return, exact input view and original exception", () => {
    const f = fixture(); const backing = concat(new Uint8Array(7), init(), new Uint8Array(3));
    const view = new Uint8Array(backing.buffer, 7, init().length);
    expect(f.v.appendBuffer(view)).toBe("native-result");
    expect((f.nativeCalls[0] as { bytes: unknown }).bytes).toBe(view);
    const failure = new Error("quota"); f.a.throwNext = failure; expect(() => f.a.appendBuffer(init())).toThrow(failure);
    f.a.appendBuffer(init()); expect(f.bridge().canStart(f.video)).toBe(true);
  });
  it("reassembles split init headers and matches the actual selected MSE only", () => {
    const f = fixture(); for (const bytes of [init().subarray(0, 3), init().subarray(3, 17), init().subarray(17)]) f.v.appendBuffer(bytes);
    f.a.appendBuffer(init()); expect(f.bridge().canStart(f.video)).toBe(true);
    f.video.srcObject = {}; expect(f.bridge().canStart(f.video)).toBe(false); f.video.srcObject = null;
    f.video.mediaKeys = {}; expect(f.bridge().canStart(f.video)).toBe(false); f.video.mediaKeys = null;
    f.video.currentSrc = "https://cdn.invalid/never-requested.mp4"; expect(f.bridge().canStart(f.video)).toBe(false);
  });
  it("does not permit recording speed until native validates the exact source", async () => {
    const f = fixture(); f.load(); f.hold.add("encoded_begin"); const started = f.start(); await flush();
    expect(f.bridge().canChangePlaybackRate(f.video)).toBe(false);
    f.release(); await started; expect(f.bridge().canChangePlaybackRate(f.video)).toBe(true);
    expect(f.bridge().canChangePlaybackRate({ ...f.video })).toBe(false);
    f.video.playbackRate = 1.2; vi.advanceTimersByTime(500); await flush(); expect(f.bridge().getStatus().active).toBe(true);
    await f.bridge().stop();
  });
  it("exposes presentation anchors only after native approval with the exact unchanged MSE timeline", async () => {
    const f = fixture(); f.load(); expect(f.bridge().getReplayClock(f.video)).toBeNull(); await f.start();
    const sourceId = f.ofKind("encoded_begin")[0]!.sourceId;
    expect(f.bridge().getReplayClock(f.video)).toEqual({ clock: "mse_presentation_v1", sourceId, sourceTimeSeconds: 100 });
    for (const rate of [.5, .75, 1, 1.25, 1.5, 2]) {
      f.video.playbackRate = rate; f.video.currentTime += rate;
      expect(f.bridge().getReplayClock(f.video)?.sourceTimeSeconds).toBe(f.video.currentTime);
    }
    expect(f.ofKind("encoded_append")).toHaveLength(0);
    for (const offset of [1, -13745.920976833331, 30]) {
      f.v.timestampOffset = offset;
      expect(f.bridge().getReplayClock(f.video)?.sourceTimeSeconds).toBeCloseTo(f.video.currentTime - offset, 8);
      expect(f.bridge().canChangePlaybackRate(f.video)).toBe(true);
    }
    f.v.timestampOffset = 0;
    f.video.seeking = true; expect(f.bridge().getReplayClock(f.video)).toBeNull(); f.video.seeking = false;
    f.video.currentSrc = "blob:another-source"; expect(f.bridge().getReplayClock(f.video)).toBeNull();
    await f.bridge().stop(); expect(f.bridge().getReplayClock(f.video)).toBeNull();
  });
  it("copies bytes before caller mutation and drains append ACKs before finish", async () => {
    const f = fixture(); f.load(); await f.start(); f.hold.add("encoded_append");
    const bytes = media(); const expected = Array.from(bytes); f.v.appendBuffer(bytes); bytes.fill(0); await flush();
    const stopped = f.bridge().stop(); await flush(); expect(f.ofKind("encoded_finish")).toHaveLength(0);
    const appended = f.ofKind("encoded_append")[0]!;
    expect(Array.from(Uint8Array.from(atob(appended.data as string), (c) => c.charCodeAt(0)))).toEqual(expected);
    expect(appended).toMatchObject({ trackIndex: 0, appendIndex: 0, chunkIndex: 0, finalChunk: true });
    f.release(); await stopped; expect(f.ofKind("encoded_finish")).toHaveLength(1);
  });
  it("splits transport at 128 KiB without changing SourceBuffer append calls", async () => {
    const f = fixture(); f.load(); await f.start(); const before = f.nativeCalls.length;
    f.v.appendBuffer(media(300000)); await flush();
    const messages = f.ofKind("encoded_append"); expect(messages).toHaveLength(3);
    expect(messages.map((m) => m.chunkIndex)).toEqual([0,1,2]); expect(messages.map((m) => m.finalChunk)).toEqual([false,false,true]);
    expect(messages.every((m) => atob(m.data as string).length <= 128 * 1024)).toBe(true);
    expect(f.nativeCalls).toHaveLength(before + 1); await f.bridge().stop();
  });
  it("skips an old partial fragment until the next whole moof boundary", async () => {
    const f = fixture(); f.load(); const old = media(50); f.v.appendBuffer(old.subarray(0, 25)); await f.start();
    f.v.appendBuffer(old.subarray(25)); f.v.appendBuffer(media()); await flush();
    expect(f.ofKind("encoded_append")).toHaveLength(1); await f.bridge().stop();
  });
  it("stops honestly on bounded queue overflow while original appends continue", async () => {
    const f = fixture({ queueBytes: 1024 }); f.load(); await f.start(); f.hold.add("encoded_append");
    for (let i = 0; i < 4; i++) expect(f.v.appendBuffer(media(400))).toBe("native-result");
    await flush(); expect(f.bridge().getStatus().stopping).toBe(true);
    f.hold.delete("encoded_append"); f.release(); await flush();
    expect(f.ofKind("encoded_finish")[0]).toMatchObject({ interrupted: true, reason: "queue_overflow" });
    expect(f.notices).not.toContain("encoded_saved");
  });
  it("a delayed begin ACK after stop is finalized and never grants speed", async () => {
    const f = fixture(); f.load(); f.hold.add("encoded_begin"); const started = f.start(); await flush();
    const stopped = f.bridge().stop(); f.release(); await started; await stopped;
    expect(f.ofKind("encoded_finish")).toHaveLength(1); expect(f.bridge().canChangePlaybackRate(f.video)).toBe(false);
    expect(f.bridge().getStatus().active).toBe(false);
  });
  it.each(["encrypted", "ended"])("fails closed on %s without changing playback", async (event) => {
    const f = fixture(); f.load(); await f.start(); f.video.dispatch(event); await flush();
    expect(f.ofKind("encoded_finish")[0]?.interrupted).toBe(true); expect(f.video.currentTime).toBe(100); expect(f.video.playbackRate).toBe(1);
  });
  it("does not confuse decoder buffering or emptied with a replaced compressed source", async () => {
    const f=fixture(); f.load(); await f.start();
    f.video.playbackRate=2; f.video.readyState=1; f.video.dispatch("emptied");
    vi.advanceTimersByTime(1000); f.v.appendBuffer(media()); await flush();
    expect(f.bridge().getStatus().active).toBe(true); expect(f.ofKind("encoded_finish")).toHaveLength(0);
    f.video.readyState=4; expect(f.bridge().canChangePlaybackRate(f.video)).toBe(true);
    f.video.currentSrc="blob:really-replaced"; vi.advanceTimersByTime(500); await flush();
    expect(f.ofKind("encoded_finish")[0]).toMatchObject({interrupted:true,reason:"source_changed"});
  });
  it("ignores visibility/minimize and finite MSE offset changes, but rejects an unsupported append window", async () => {
    const f = fixture(); f.load(); await f.start(); f.window.dispatch("visibilitychange"); f.v.remove(0, 1); await flush();
    expect(f.bridge().getStatus().active).toBe(true);
    f.v.timestampOffset = 5; f.v.appendBuffer(media()); await flush(); expect(f.bridge().getStatus().active).toBe(true);
    f.v.appendWindowStart = 5; f.v.appendBuffer(media()); await flush(); expect(f.ofKind("encoded_finish")[0]).toMatchObject({ interrupted: true, reason: "timeline_changed" });
  });
  it("keeps original samples and clocks independent of speed, pause and seek events", async () => {
    const f = fixture(); f.v.timestampOffset = -13745.920976833331; f.load(); await f.start();
    for (const rate of [1.000001, 1.03, 1.2, 2, .75]) {
      f.video.playbackRate = rate; f.video.dispatch("ratechange");
      f.v.appendBuffer(media()); await flush();
      expect(f.bridge().getStatus().active).toBe(true);
    }
    f.video.paused = true; f.video.dispatch("pause"); f.video.seeking = true; f.video.dispatch("seeking");
    f.v.abort(); f.v.dispatch("abort"); vi.advanceTimersByTime(16000); await flush();
    expect(f.bridge().getStatus()).toMatchObject({ active: true, detail: "encoded_waiting" });
    expect(f.bridge().getReplayClock(f.video)).toBeNull();
    f.video.seeking = false; f.v.appendBuffer(media()); await flush();
    expect(f.bridge().getStatus().detail).toBe("encoded_recording");
    expect(f.ofKind("encoded_append")).toHaveLength(6);
    for (const message of f.ofKind("encoded_append")) expect(atob(message.data as string)).toBe(String.fromCharCode(...media()));
    await f.bridge().stop(); expect(f.ofKind("encoded_finish")[0]?.interrupted).toBe(false);
  });
  it("accepts identical init repetitions but rejects changed codec headers", async () => {
    const f = fixture(); f.load(); await f.start(); f.v.appendBuffer(init()); f.v.appendBuffer(media()); await flush();
    expect(f.bridge().getStatus().active).toBe(true); expect(f.ofKind("encoded_append")).toHaveLength(1);
    const changed = init(); changed[changed.length - 1] = 42; f.v.appendBuffer(changed); await flush();
    expect(f.ofKind("encoded_finish")[0]).toMatchObject({ interrupted: true, reason: "init_changed" });
  });
  it("reports bounded source facts without URLs, cookies or source identifiers", () => {
    const f = fixture(); f.v.timestampOffset = -13745.920976833331; f.load();
    const report = f.bridge().getDiagnostics(); expect(report.reason).toBe("ready"); expect(report.appendCount).toBe(2);
    expect(JSON.stringify(report)).not.toMatch(/blob:|https:|cookie|sourceId/);
    f.video.srcObject = {}; expect(f.bridge().getDiagnostics().reason).toBe("source_object_unsupported");
  });
  it("native failure cannot produce a successful saved status or legacy fallback", async () => {
    const f = fixture(); f.load(); await f.start(); f.reject.add("encoded_append"); f.v.appendBuffer(media()); await flush();
    expect(f.ofKind("encoded_finish")[0]?.interrupted).toBe(true); expect(f.notices).not.toContain("encoded_saved");
  });
  it.each(["BRIDGE_BUSY", "BROWSER_ACK_TIMEOUT"])("retries an identical append after %s without advancing or stopping", async code => {
    const f = fixture(); f.load(); await f.start(); f.hold.add("encoded_append");
    f.v.appendBuffer(media(140000)); await flush();
    const first = f.pending.shift()!;
    first.reject(Object.assign(new Error("transient"), { code })); await flush();
    expect(f.ofKind("encoded_append")).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(100); await flush();
    expect(f.ofKind("encoded_append")[1]).toEqual(first.message);
    expect(f.ofKind("encoded_finish")).toHaveLength(0);
    f.hold.delete("encoded_append"); f.release(); await flush();
    expect(f.ofKind("encoded_append")[2]).toMatchObject({ chunkIndex: 1 });
    expect(f.bridge().getStatus().active).toBe(true);
    await f.bridge().stop();
    expect(f.ofKind("encoded_finish")[0]).toMatchObject({ interrupted: false });
  });
  it("bounds transient retries and never retries an unsupported source", async () => {
    const f = fixture(); f.load(); await f.start(); f.hold.add("encoded_append");
    f.v.appendBuffer(media()); await flush();
    for (const delay of [100, 200, 400, 800, 1600, 2000]) {
      f.pending.shift()!.reject(Object.assign(new Error("busy"), { code: "BRIDGE_BUSY" }));
      await flush(); await vi.advanceTimersByTimeAsync(delay); await flush();
    }
    expect(f.bridge().getStatus().active).toBe(true);
    await vi.advanceTimersByTimeAsync(60_000);
    f.pending.shift()!.reject(Object.assign(new Error("busy"), { code: "BRIDGE_BUSY" })); await flush();
    expect(f.ofKind("encoded_append")).toHaveLength(7);
    expect(f.ofKind("encoded_finish")[0]).toMatchObject({ interrupted: true, reason:"bridge_busy" });
    const hard = fixture(); hard.load(); await hard.start(); hard.hold.add("encoded_append");
    hard.v.appendBuffer(media()); await flush();
    hard.pending.shift()!.reject(Object.assign(new Error("real media gap"), { code: "ENCODED_UNSUPPORTED" })); await flush();
    await vi.advanceTimersByTimeAsync(2000);
    expect(hard.ofKind("encoded_append")).toHaveLength(1);
    expect(hard.ofKind("encoded_finish")[0]).toMatchObject({ interrupted: true });
  });
  it("survives the observed seven-second storage congestion without advancing the chunk", async () => {
    const f=fixture(); f.load(); await f.start(); f.hold.add("encoded_append");
    f.v.appendBuffer(media()); await flush(); const first=f.ofKind("encoded_append")[0];
    for (const delay of [100,200,400,800,1600,2000,2000,2000]) {
      f.pending.shift()!.reject(Object.assign(new Error("busy"),{code:"BRIDGE_BUSY"}));
      await flush(); await vi.advanceTimersByTimeAsync(delay); await flush();
      expect(f.bridge().getStatus().active).toBe(true);
      expect(f.ofKind("encoded_append").at(-1)).toEqual(first);
      expect(f.ofKind("encoded_finish")).toHaveLength(0);
    }
    f.hold.delete("encoded_append"); f.release(); await flush(); await f.bridge().stop();
    expect(f.ofKind("encoded_finish")[0]).toMatchObject({interrupted:false});
  });
  it("awaits chat drain before native finish and reports native actual interruption", async () => {
    const f = fixture(); f.load(); let drain: (() => void) | undefined;
    await f.start({}, { beforeFinish: () => new Promise<void>((resolve) => { drain = resolve; }) });
    const stopped = f.bridge().stop(); await flush(); expect(f.ofKind("encoded_finish")).toHaveLength(0);
    drain!(); await stopped; expect(f.ofKind("encoded_finish")).toHaveLength(1);
  });
  it("rejects malformed start before native work, and rejects unapproved begin", async () => {
    const f = fixture(); f.load(); await expect(f.start({ requestId: "bad" })).rejects.toThrow("encoded_unavailable"); expect(f.messages).toHaveLength(0);
    await expect(f.start({}, { request: () => Promise.resolve({ id: RECORDING }) })).rejects.toMatchObject({ nativeAttempted: true });
    expect(f.bridge().getStatus().active).toBe(false);
  });
  it("allows legacy only after explicit pre-arm native unsupported init rejection", async () => {
    const f = fixture(); f.load();
    await expect(f.start({}, { request: () => Promise.reject(Object.assign(new Error("unsupported codec"), { code: "ENCODED_UNSUPPORTED" })) }))
      .rejects.toMatchObject({ nativeAttempted: true, allowLegacyFallback: true });
    expect(f.bridge().canStart(f.video)).toBe(false); expect(f.bridge().getStatus().active).toBe(false);
  });
  it("does not permit legacy fallback for lost ACK, other native errors or invalid approval", async () => {
    for (const reply of [() => Promise.reject(new Error("lost ACK")), () => Promise.reject(Object.assign(new Error("unavailable"), { code: "BROWSER_UNAVAILABLE" })),
      () => Promise.resolve({ id: RECORDING, mode: "reencoded", nativeApproved: false })]) {
      const f = fixture(); f.load();
      const failure = await f.start({}, { request: reply }).catch((error: unknown) => error) as { nativeAttempted: boolean; allowLegacyFallback?: boolean };
      expect(failure.nativeAttempted).toBe(true); expect(failure.allowLegacyFallback).toBeUndefined();
    }
  });
});
