// @vitest-environment node
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import bridgeSource from "../../../src-tauri/src/streaming/browser_capture.js?raw";

// Keep the fixture independent of DOM and browser codecs. A computed built-in
// import avoids adding Node ambient types to this browser-only application.
const vmModule = "node:vm";
const { runInNewContext } = await import(vmModule) as {
  runInNewContext(source: string, context: Record<string, unknown>): unknown;
};
const CHANNEL = "b3e262a2795f17734c149afc738ad250";
const REQUEST = "10000000-0000-4000-8000-000000000001";
const RECORDING = "20000000-0000-4000-8000-000000000001";
type Message = Record<string, unknown> & { id: string; kind: string };
type FixtureEvent = { type: string; detail?: unknown };
type Listener = (event: FixtureEvent) => void;

class Target {
  listeners = new Map<string, Set<Listener>>();
  addEventListener(name: string, listener: Listener) {
    if (!this.listeners.has(name)) this.listeners.set(name, new Set());
    this.listeners.get(name)!.add(listener);
  }
  removeEventListener(name: string, listener: Listener) {
    this.listeners.get(name)?.delete(listener);
  }
  dispatch(name: string, detail?: unknown) {
    for (const listener of [...(this.listeners.get(name) ?? [])]) listener({ type: name, detail });
  }
}
class Track extends Target {
  readyState = "live";
  stops = 0;
  constructor(public kind: string) { super(); }
  stop() { this.stops += 1; this.readyState = "ended"; }
}
class Stream {
  tracks: Track[];
  constructor(audio = true) { this.tracks = [new Track("video"), ...(audio ? [new Track("audio")] : [])]; }
  getTracks() { return this.tracks; }
  getVideoTracks() { return this.tracks.filter((track) => track.kind === "video"); }
  getAudioTracks() { return this.tracks.filter((track) => track.kind === "audio"); }
}
class Video extends Target {
  isConnected = true;
  readyState = 4;
  videoWidth = 1920;
  videoHeight = 1080;
  ended = false;
  paused = false;
  seeking = false;
  playbackRate = 1;
  currentTime = 100;
  width = 1920;
  height = 1080;
  captures = 0;
  stream = new Stream();
  buffered = { length: 1, end: () => 104 };
  getBoundingClientRect() { return { width: this.width, height: this.height }; }
  captureStream() { this.captures += 1; return this.stream; }
}
class Element {
  id = "";
  textContent = "";
  style = { cssText: "" };
  attributes = new Map<string, string>();
  setAttribute(name: string, value: string) { this.attributes.set(name, value); }
}
const flush = async (turns = 80) => {
  for (let index = 0; index < turns; index += 1) await Promise.resolve();
};

function fixture(options: { url?: string; iframe?: boolean; audio?: boolean; mimes?: string[]; recordingId?: string; captureChat?: boolean; originalOnly?: boolean } = {}) {
  const window = new Target() as Target & Record<string, unknown>;
  window.location = new URL(options.url ?? `https://chzzk.naver.com/live/${CHANNEL}`);
  window.top = options.iframe ? {} : window;
  const video = new Video();
  video.stream = new Stream(options.audio);
  const videos = [video];
  const elements: Element[] = [];
  const messages: Message[] = [];
  const rawMessages: string[] = [];
  const hold = new Set<string>();
  const reject = new Set<string>();
  let counter = 0;
  const reply = (message: Message, ok = true, data: unknown = message.kind === "begin" ? { id: options.recordingId ?? RECORDING, captureChat: options.captureChat === true } : {}) => {
    window.dispatch("atsumi-browser-reply", { id: message.id, ok, data });
  };
  window.chrome = { webview: { postMessage(payload: string) {
    if (typeof payload !== "string" || !payload.startsWith("ATSUMI_BROWSER_CAPTURE:")) {
      throw new Error("Wry requires the reserved string envelope");
    }
    rawMessages.push(payload);
    const message = JSON.parse(payload.slice("ATSUMI_BROWSER_CAPTURE:".length)) as Message;
    messages.push(message);
    if (message.kind !== "status" && !hold.has(message.kind)) {
      queueMicrotask(() => reply(message, !reject.has(message.kind)));
    }
  } } };
  const recorders: Recorder[] = [];
  class Recorder {
    static isTypeSupported(mime: string) {
      return (options.mimes ?? ["video/webm;codecs=vp8,opus"]).includes(mime);
    }
    state = "inactive";
    ondataavailable: ((event: { data: Blob | { size: number; arrayBuffer(): Promise<ArrayBuffer> } }) => void) | null = null;
    onstop: (() => void) | null = null;
    onerror: (() => void) | null = null;
    timeslice: number | undefined;
    stopBytes = 1;
    deferStop = false;
    constructor(public stream: Stream, public options: Record<string, unknown>) { recorders.push(this); }
    start(timeslice: number) { this.timeslice = timeslice; this.state = "recording"; }
    emit(data: Blob | { size: number; arrayBuffer(): Promise<ArrayBuffer> }) { this.ondataavailable?.({ data }); }
    stopped() {
      if (this.stopBytes) this.emit(new Blob([new Uint8Array(this.stopBytes)]));
      this.onstop?.();
    }
    stop() {
      if (this.state === "inactive") throw new Error("already stopped");
      this.state = "inactive";
      if (!this.deferStop) queueMicrotask(() => this.stopped());
    }
  }
  const document = {
    title: "Fixture channel - CHZZK",
    get cookie(): never { throw new Error("Must not read cookies"); },
    body: { appendChild: (element: Element) => elements.push(element) },
    createElement: (name: string) => {
      if (name !== "div") throw new Error("Do not replace the official player or create a canvas");
      return new Element();
    },
    querySelectorAll: (selector: string) => selector === "video" ? videos : [],
  };
  const context = {
    window, document, MediaRecorder: Recorder,
    crypto: { randomUUID: () => `30000000-0000-4000-8000-${String(++counter).padStart(12, "0")}` },
    performance: { now: () => Date.now() },
    setTimeout, clearTimeout, setInterval, clearInterval, btoa, Uint8Array,
  };
  // Older codec tests explicitly exercise the retired fallback in isolation.
  // Production-policy regressions below use the unchanged source.
  const script = options.originalOnly ? bridgeSource : bridgeSource.replace("const ALLOW_REENCODED_CAPTURE = false;", "const ALLOW_REENCODED_CAPTURE = true;");
  runInNewContext(script, context);
  const command = (kind: string, fields: Record<string, unknown> = {}) => {
    window.dispatch("atsumi-browser-command", {
      kind, channelId: CHANNEL, requestId: REQUEST, rightsAcknowledged: true, ...fields,
    });
  };
  return { window, video, videos, elements, messages, rawMessages, hold, reject, recorders, reply, command,
    injectAgain: () => runInNewContext(script, context),
    ofKind: (kind: string) => messages.filter((message) => message.kind === kind) };
}

beforeEach(() => { vi.useFakeTimers(); vi.setSystemTime(0); });
afterEach(() => { vi.clearAllTimers(); vi.useRealTimers(); });

describe("encoded capture integration", () => {
  it("uses an approved encoded session without creating a second MediaRecorder and drains chat before finish", async () => {
    const h = fixture({ originalOnly: true }); const order: string[] = [];
    const state = { active: false, starting: false, stopping: false, recordingId: RECORDING, channelId: CHANNEL, detail: "recording" };
    let hooks: { beforeFinish(id: string): Promise<void>; onStatus(detail: string): void } | undefined;
    const encoded = {
      canStart: (video: unknown) => video === h.video,
      getStatus: () => state,
      start: vi.fn(async (_command: unknown, supplied: typeof hooks) => { hooks = supplied; state.active = true; return { id: RECORDING, mode: "encoded", nativeApproved: true, captureChat: true }; }),
      stop: vi.fn(async () => { state.stopping = true; await hooks!.beforeFinish(RECORDING); order.push("finish"); state.active = false; hooks!.onStatus("saved"); return { stopped: true }; }),
    };
    h.window.__atsumiEncodedCapture = encoded;
    h.window.__atsumiPageChat = { start: vi.fn(() => true), stop: vi.fn(async () => { order.push("chat-drained"); }) };
    h.video.playbackRate = 1.2;
    h.command("start"); await flush();
    expect(encoded.start).toHaveBeenCalledOnce(); expect(h.video.captures).toBe(0); expect(h.recorders).toHaveLength(0);
    expect(h.ofKind("begin")).toHaveLength(0);
    h.command("start"); await flush(); expect(encoded.start).toHaveBeenCalledOnce();
    h.command("stop"); await flush(); expect(order).toEqual(["chat-drained", "finish"]);
    expect(h.ofKind("status").at(-1)).toMatchObject({ recording: false, detail: "saved" });
  });
  it("does not silently create a legacy recorder after an encoded begin failure", async () => {
    const h = fixture();
    h.window.__atsumiEncodedCapture = { canStart: () => true, getStatus: () => ({ active: false }), start: vi.fn(async () => { throw new Error("lost begin ACK"); }) };
    h.command("start"); await flush();
    expect(h.video.captures).toBe(0); expect(h.recorders).toHaveLength(0); expect(h.ofKind("begin")).toHaveLength(0);
    expect(h.ofKind("status").at(-1)).toMatchObject({ recording: false, detail: "native_rejected" });
  });
  it("ends our viewing catch-up before falling back to normal 1x recording", async () => {
    const h = fixture(); h.video.playbackRate = 1.2;
    h.window.__atsumiPlayerUI = { update: () => {}, stopCatchup: () => { h.video.playbackRate = 1; } };
    h.command("start"); await flush();
    expect(h.video.playbackRate).toBe(1); expect(h.recorders).toHaveLength(1);
  });
  it("falls back only on the observer's explicit pre-arm unsupported result", async () => {
    const h = fixture(); h.video.playbackRate = 1.2;
    h.window.__atsumiPlayerUI = { update: () => {}, stopCatchup: () => { h.video.playbackRate = 1; } };
    h.window.__atsumiEncodedCapture = { canStart: () => true, getStatus: () => ({ active: false }),
      start: async () => { throw Object.assign(new Error("unsupported init"), { allowLegacyFallback: true }); } };
    h.command("start"); await flush();
    expect(h.video.playbackRate).toBe(1); expect(h.recorders).toHaveLength(1);
    expect(h.ofKind("begin")).toHaveLength(1);
  });
  it("still finalizes encoded video when chat drain throws synchronously", async () => {
    const h = fixture(); const state = { active: false, stopping: false, recordingId: RECORDING, channelId: CHANNEL, detail: "encoded_recording" };
    let hooks: { beforeFinish(id: string): Promise<void>; onStatus(detail: string): void };
    const finish = vi.fn();
    h.window.__atsumiEncodedCapture = { canStart: () => true, getStatus: () => state,
      start: async (_command: unknown, supplied: typeof hooks) => { hooks = supplied; state.active = true; return { id: RECORDING, mode: "encoded", nativeApproved: true, captureChat: true }; },
      stop: async () => { state.stopping = true; await hooks.beforeFinish(RECORDING); finish(); state.active = false; hooks.onStatus("encoded_saved"); } };
    h.window.__atsumiPageChat = { start: () => true, stop: () => { throw new Error("chat failure"); } };
    h.command("start"); await flush();
    expect(h.ofKind("status").at(-1)).toMatchObject({ recording: true, detail: "recording" });
    h.command("stop"); await flush();
    expect(finish).toHaveBeenCalledOnce();
    expect(h.ofKind("chat_status").at(-1)).toMatchObject({ recordingId: RECORDING, detail: "storage_failed" });
    expect(h.ofKind("status").at(-1)).toMatchObject({ recording: false, detail: "saved" });
  });
  it("bounds a stuck encoded chat drain so native exit can still finish the video", async () => {
    const h = fixture(); const state = { active: false, recordingId: RECORDING, channelId: CHANNEL, detail: "encoded_recording" };
    let hooks: { beforeFinish(id: string): Promise<void>; onStatus(detail: string): void };
    const finish = vi.fn();
    h.window.__atsumiEncodedCapture = { canStart: () => true, getStatus: () => state,
      start: async (_command: unknown, supplied: typeof hooks) => { hooks = supplied; state.active = true; return { id: RECORDING, mode: "encoded", nativeApproved: true, captureChat: true }; },
      stop: async () => { await hooks.beforeFinish(RECORDING); finish(); state.active = false; hooks.onStatus("encoded_saved"); } };
    h.window.__atsumiPageChat = { start: () => true, stop: () => new Promise(() => {}) };
    h.command("start"); await flush(); h.command("stop"); await flush();
    expect(finish).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(2001); await flush();
    expect(finish).toHaveBeenCalledOnce();
    expect(h.ofKind("chat_status").at(-1)).toMatchObject({ detail: "storage_failed" });
  });
});

describe("original-only production policy", () => {
  it("never captures rendered output when the original observer is unavailable", async () => {
    const h = fixture({ originalOnly: true }); h.video.playbackRate = 1.03;
    h.command("start"); await flush();
    expect(h.video.captures).toBe(0); expect(h.recorders).toHaveLength(0); expect(h.ofKind("begin")).toHaveLength(0);
    expect(h.video.playbackRate).toBe(1.03);
    expect(h.ofKind("status").at(-1)).toMatchObject({ ready: false, recording: false, detail: "original_unavailable" });
  });
  it("rejects the old fallback hint without resetting speed or creating a recorder", async () => {
    const h = fixture({ originalOnly: true }); h.video.playbackRate = 2;
    const stopCatchup = vi.fn(); h.window.__atsumiPlayerUI = { update: () => {}, stopCatchup };
    h.window.__atsumiEncodedCapture = { canStart: () => true, getStatus: () => ({ active: false }),
      start: async () => { throw Object.assign(new Error("unsupported init"), { code: "ENCODED_UNSUPPORTED", allowLegacyFallback: true }); } };
    h.command("start"); await flush();
    expect(h.recorders).toHaveLength(0); expect(h.video.captures).toBe(0); expect(h.ofKind("begin")).toHaveLength(0);
    expect(stopCatchup).not.toHaveBeenCalled(); expect(h.video.playbackRate).toBe(2);
    expect(h.ofKind("status").at(-1)?.detail).toBe("original_unavailable");
  });
  it("reports waiting for input as an active recording, not a successful stop", async () => {
    const h = fixture({ originalOnly: true }); const diagnostics = { reason: "ready", installed: true };
    h.video.paused = true;
    h.window.__atsumiEncodedCapture = { canStart: () => true, getDiagnostics: () => diagnostics,
      getStatus: () => ({ active: true, recordingId: RECORDING, detail: "encoded_waiting" }) };
    await vi.advanceTimersByTimeAsync(1000);
    expect(h.ofKind("status").at(-1)).toMatchObject({ recording: true, detail: "waiting_source", captureMode: "encoded", captureDiagnostics: diagnostics });
    expect(h.elements[0]?.textContent).toBe("수신 대기"); expect(h.recorders).toHaveLength(0);
  });
});

describe("official-page browser capture bridge", () => {
  it.each([
    "http://chzzk.naver.com/live/" + CHANNEL,
    "https://chzzk.naver.com.evil.example/live/" + CHANNEL,
    "https://chzzk.naver.com:444/live/" + CHANNEL,
    "https://chzzk.naver.com/video/" + CHANNEL,
    "https://chzzk.naver.com/live/" + CHANNEL + "/extra",
  ])("does not install outside the exact official live origin/path: %s", async (url) => {
    const harness = fixture({ url });
    harness.command("start");
    await flush();
    expect(harness.messages).toEqual([]);
    expect(harness.video.captures).toBe(0);
  });

  it("refuses frames and does not install twice", async () => {
    expect(fixture({ iframe: true }).messages).toEqual([]);
    const harness = fixture();
    harness.injectAgain();
    harness.command("start");
    await flush();
    expect(harness.ofKind("begin")).toHaveLength(1);
    expect(harness.recorders).toHaveLength(1);
  });

  it("requires explicit rights, a native UUID request and matching channel", async () => {
    const harness = fixture();
    harness.command("start", { rightsAcknowledged: false });
    harness.command("start", { requestId: "not-a-uuid" });
    harness.command("start", { channelId: "a".repeat(32) });
    await flush();
    expect(harness.ofKind("begin")).toEqual([]);
    expect(harness.video.captures).toBe(0);
    harness.command("start");
    await flush();
    expect(harness.ofKind("begin")[0]).toMatchObject({ requestId: REQUEST,
      channelId: CHANNEL, mimeType: "video/webm;codecs=vp8,opus" });
  });

  it("captures the largest ready source without reading credentials or replacing the player", async () => {
    const harness = fixture();
    const small = new Video();
    small.width = 10;
    small.height = 10;
    harness.videos.unshift(small);
    harness.command("start");
    await flush();
    expect(harness.video.captures).toBe(1);
    expect(small.captures).toBe(0);
    expect(harness.recorders[0]?.timeslice).toBe(1000);
    expect(harness.recorders[0]?.stream).toBe(harness.video.stream);
    expect(harness.ofKind("status")[0]).toMatchObject({ ready: true, bufferSeconds: 4, videoWidth: 1920, videoHeight: 1080, paused: false });
    expect(harness.elements[0]?.attributes.get("aria-label")).toContain("순서대로 저장");
    expect(harness.elements[0]?.textContent).toBe("녹화 중");
    expect(harness.elements[0]?.textContent).not.toMatch(/재인코딩|WebView|15초/);
    expect(harness.elements[0]?.textContent).not.toContain("\n");
    expect(harness.elements[0]?.style.cssText).toContain("top:46px");
    expect(harness.messages.every((message) => message.atsumiBrowserCapture === 1)).toBe(true);
  });

  it("uses the reserved Wry string envelope for every notification and acknowledged message", async () => {
    const harness = fixture();
    harness.command("start");
    await flush();
    await vi.advanceTimersByTimeAsync(1000);
    harness.command("stop");
    await flush();
    expect(harness.messages.map((message) => message.kind)).toEqual(
      expect.arrayContaining(["status", "begin", "chunk", "segment", "finish"]),
    );
    expect(harness.rawMessages).toHaveLength(harness.messages.length);
    for (const payload of harness.rawMessages) {
      expect(typeof payload).toBe("string");
      expect(payload.startsWith("ATSUMI_BROWSER_CAPTURE:")).toBe(true);
      expect(JSON.parse(payload.slice("ATSUMI_BROWSER_CAPTURE:".length))).toMatchObject({ atsumiBrowserCapture: 1 });
    }
  });

  it("accepts the store's compact recording UUID without relaxing native start request UUIDs", async () => {
    const recordingId = "20000000000040008000000000000001";
    const harness = fixture({ recordingId });
    harness.command("start", { requestId: REQUEST.replaceAll("-", "") });
    await flush();
    expect(harness.ofKind("begin")).toHaveLength(0);
    harness.command("start");
    await flush();
    await vi.advanceTimersByTimeAsync(1000);
    harness.command("stop");
    await flush();
    expect(harness.recorders).toHaveLength(1);
    expect(harness.ofKind("chunk")[0]?.recordingId).toBe(recordingId);
    expect(harness.ofKind("finish")[0]).toMatchObject({ recordingId, interrupted: false });
  });

  it("arms page chat only after native consent and waits for chat drain before final finish", async () => {
    const harness = fixture({ captureChat: true });
    harness.hold.add("begin");
    let resolveDrain!: () => void;
    const drain = new Promise<void>((resolve) => { resolveDrain = resolve; });
    const start = vi.fn(() => true);
    const stop = vi.fn(() => drain);
    harness.window.__atsumiPageChat = { start, stop };
    harness.command("start");
    await flush();
    expect(start).not.toHaveBeenCalled();
    harness.reply(harness.ofKind("begin")[0]!);
    await flush();
    expect(start).toHaveBeenCalledOnce();
    expect(start).toHaveBeenCalledWith(expect.objectContaining({ recordingId: RECORDING, channelId: CHANNEL }));
    await vi.advanceTimersByTimeAsync(1000);
    harness.command("stop");
    await flush();
    expect(stop).toHaveBeenCalledWith(RECORDING);
    expect(harness.ofKind("segment")).toHaveLength(1);
    expect(harness.ofKind("finish")).toHaveLength(0);
    resolveDrain();
    await flush();
    expect(harness.ofKind("finish")[0]?.interrupted).toBe(false);
  });

  it("does not observe chat without native captureChat permission and isolates chat startup failure", async () => {
    const disabled = fixture();
    const start = vi.fn(() => true);
    disabled.window.__atsumiPageChat = { start };
    disabled.command("start");
    await flush();
    expect(start).not.toHaveBeenCalled();
    const unsupported = fixture({ captureChat: true });
    unsupported.command("start");
    await flush();
    expect(unsupported.recorders[0]?.state).toBe("recording");
    expect(unsupported.ofKind("chat_status")[0]).toMatchObject({ recordingId: RECORDING, detail: "observer_unavailable" });
  });
  it("bounds a stuck legacy chat drain without discarding the finished video", async () => {
    const h = fixture({ captureChat: true });
    h.window.__atsumiPageChat = { start: () => true, stop: () => new Promise(() => {}) };
    h.command("start"); await flush(); await vi.advanceTimersByTimeAsync(1000);
    h.command("stop"); await flush(); expect(h.ofKind("finish")).toHaveLength(0);
    await vi.advanceTimersByTimeAsync(2001); await flush();
    expect(h.ofKind("finish")[0]).toMatchObject({ interrupted: false });
    expect(h.ofKind("chat_status").at(-1)).toMatchObject({ detail: "storage_failed" });
  });

  it("rejects missing audio before native begin and negotiates MP4 fallback", async () => {
    const missing = fixture({ audio: false });
    missing.command("start");
    await flush();
    expect(missing.ofKind("begin")).toHaveLength(0);
    expect(missing.ofKind("status").at(-1)?.detail).toBe("no_audio");
    expect(missing.video.stream.getTracks().every((track) => track.stops === 1)).toBe(true);
    const mp4 = fixture({ mimes: ["video/mp4;codecs=avc1.42E01E,mp4a.40.2"] });
    mp4.command("start");
    await flush();
    expect(mp4.ofKind("begin")[0]?.mimeType).toBe("video/mp4;codecs=avc1.42E01E,mp4a.40.2");
  });

  it("splits byte-exact blobs into 128 KiB chunks with monotonic indices and waits for the final blob", async () => {
    const harness = fixture();
    harness.command("start");
    await flush();
    const bytes = new Uint8Array(128 * 1024 * 2 + 3).fill(173);
    harness.recorders[0]!.emit(new Blob([bytes]));
    await flush();
    const chunks = harness.ofKind("chunk");
    expect(chunks.map((chunk) => chunk.chunkIndex)).toEqual([0, 1, 2]);
    expect(chunks.map((chunk) => atob(String(chunk.data)).length)).toEqual([131072, 131072, 3]);
    expect(chunks.map((chunk) => atob(String(chunk.data))).join(""))
      .toBe(String.fromCharCode(173).repeat(bytes.length));
    await vi.advanceTimersByTimeAsync(1000);
    harness.recorders[0]!.deferStop = true;
    harness.command("stop");
    await flush();
    expect(harness.ofKind("finish")).toHaveLength(0);
    await vi.advanceTimersByTimeAsync(2000);
    harness.recorders[0]!.stopped();
    await flush();
    expect(harness.ofKind("chunk").at(-1)).toMatchObject({ segmentIndex: 0, chunkIndex: 3 });
    expect(harness.ofKind("segment")[0]).toMatchObject({ segmentIndex: 0, durationSeconds: 1 });
    expect(harness.ofKind("finish")[0]).toMatchObject({ recordingId: RECORDING, interrupted: false });
    expect(harness.ofKind("status").at(-1)?.detail).toBe("saved");
  });

  it("requires ACK before sending the next chunk, boundary and finish", async () => {
    const harness = fixture();
    harness.hold.add("chunk");
    harness.command("start");
    await flush();
    harness.recorders[0]!.emit(new Blob([new Uint8Array(128 * 1024 + 1)]));
    await flush();
    expect(harness.ofKind("chunk")).toHaveLength(1);
    harness.reply(harness.ofKind("chunk")[0]!);
    await flush();
    expect(harness.ofKind("chunk")).toHaveLength(2);
    await vi.advanceTimersByTimeAsync(1000);
    harness.command("stop");
    await flush();
    expect(harness.ofKind("segment")).toHaveLength(0);
    harness.hold.delete("chunk");
    harness.hold.add("segment");
    harness.reply(harness.ofKind("chunk")[1]!);
    await flush();
    expect(harness.ofKind("segment")).toHaveLength(1);
    expect(harness.ofKind("finish")).toHaveLength(0);
    harness.reply(harness.ofKind("segment")[0]!);
    await flush();
    expect(harness.ofKind("finish")).toHaveLength(1);
  });

  it("rotates independent recorders every 15 seconds while preserving generation and queue order", async () => {
    const harness = fixture();
    harness.hold.add("segment");
    harness.command("start");
    await flush();
    await vi.advanceTimersByTimeAsync(15_000);
    await flush();
    expect(harness.recorders).toHaveLength(2);
    expect(harness.ofKind("segment")[0]).toMatchObject({ segmentIndex: 0, durationSeconds: 15 });
    harness.recorders[1]!.emit(new Blob([new Uint8Array([7, 8])]));
    await flush();
    expect(harness.ofKind("chunk").some((message) => message.segmentIndex === 1)).toBe(false);
    harness.hold.delete("segment");
    harness.reply(harness.ofKind("segment")[0]!);
    await flush();
    expect(harness.ofKind("chunk").at(-1)).toMatchObject({ segmentIndex: 1, chunkIndex: 0 });
    await vi.advanceTimersByTimeAsync(1000);
    harness.command("stop");
    await flush();
    expect(harness.ofKind("segment")[1]).toMatchObject({ segmentIndex: 1, durationSeconds: 1 });
    expect(harness.ofKind("finish")[0]?.interrupted).toBe(false);
  });

  it("does not orphan a native begin when stop arrives before its ACK", async () => {
    const harness = fixture();
    harness.hold.add("begin");
    harness.command("start");
    harness.command("stop");
    harness.command("start");
    expect(harness.ofKind("begin")).toHaveLength(1);
    harness.reply(harness.ofKind("begin")[0]!);
    await flush();
    expect(harness.recorders).toHaveLength(0);
    expect(harness.ofKind("finish")[0]).toMatchObject({ interrupted: true, reason: "empty_segment" });
  });

  it("ignores stale replies from a rejected generation when starting again", async () => {
    const harness = fixture();
    harness.reject.add("begin");
    harness.command("start");
    await flush();
    const old = harness.ofKind("begin")[0]!;
    harness.reject.delete("begin");
    harness.video.stream = new Stream();
    harness.command("start");
    await flush();
    harness.reply(old);
    await flush();
    expect(harness.recorders).toHaveLength(1);
    expect(harness.ofKind("finish")).toHaveLength(0);
    expect(harness.ofKind("status").at(-1)?.detail).toBe("recording");
  });

  it("starts a fresh generation at segment/chunk zero and ignores an old recorder callback", async () => {
    const harness = fixture();
    harness.command("start");
    await flush();
    await vi.advanceTimersByTimeAsync(1000);
    harness.command("stop");
    await flush();
    const previous = harness.recorders[0]!;
    harness.video.stream = new Stream();
    harness.command("start");
    await flush();
    previous.emit(new Blob([new Uint8Array([99])]));
    previous.onstop?.();
    previous.onerror?.();
    harness.recorders[1]!.emit(new Blob([new Uint8Array([7])]));
    await flush();
    expect(harness.ofKind("begin")).toHaveLength(2);
    expect(harness.ofKind("finish")).toHaveLength(1);
    expect(harness.ofKind("chunk")).toHaveLength(2);
    expect(harness.ofKind("chunk").at(-1)).toMatchObject({ segmentIndex: 0, chunkIndex: 0 });
    expect(harness.recorders[1]?.state).toBe("recording");
  });

  it("reserves queued Blob bytes before reading and interrupts at the 16 MiB bound", async () => {
    const harness = fixture();
    harness.hold.add("chunk");
    harness.command("start");
    await flush();
    await vi.advanceTimersByTimeAsync(1000);
    const first = { size: 9 * 1024 * 1024, arrayBuffer: vi.fn(async () => new ArrayBuffer(9 * 1024 * 1024)) };
    const second = { size: 9 * 1024 * 1024, arrayBuffer: vi.fn(async () => new ArrayBuffer(9 * 1024 * 1024)) };
    harness.recorders[0]!.emit(first);
    harness.recorders[0]!.emit(second);
    await flush();
    expect(first.arrayBuffer).toHaveBeenCalledOnce();
    expect(second.arrayBuffer).not.toHaveBeenCalled();
    harness.reply(harness.ofKind("chunk")[0]!, false);
    await flush();
    expect(harness.ofKind("segment")).toHaveLength(0);
    expect(harness.ofKind("finish")[0]).toMatchObject({ interrupted: true, reason: "queue_overflow" });
    expect(harness.ofKind("status").at(-1)?.detail).not.toBe("saved");
  });

  it("marks native write rejection as interrupted and never commits the invalid segment", async () => {
    const harness = fixture();
    harness.reject.add("chunk");
    harness.command("start");
    await flush();
    await vi.advanceTimersByTimeAsync(1000);
    harness.recorders[0]!.emit(new Blob([new Uint8Array([1])]));
    await flush();
    expect(harness.ofKind("chunk")).toHaveLength(1);
    expect(harness.ofKind("segment")).toHaveLength(0);
    expect(harness.ofKind("finish")[0]).toMatchObject({ interrupted: true, reason: "native_rejected" });
  });

  it("times out a missing ACK without silently reporting success", async () => {
    const harness = fixture();
    harness.hold.add("chunk");
    harness.command("start");
    await flush();
    harness.recorders[0]!.emit(new Blob([new Uint8Array([1])]));
    await flush();
    await vi.advanceTimersByTimeAsync(15_001);
    await flush();
    expect(harness.ofKind("finish")[0]).toMatchObject({ interrupted: true, reason: "native_rejected" });
    expect(harness.ofKind("status").at(-1)?.detail).toBe("native_rejected");
  });

  it("does not show saved if reading a Blob or final native completion fails", async () => {
    const unreadable = fixture();
    unreadable.command("start");
    await flush();
    await vi.advanceTimersByTimeAsync(1000);
    unreadable.recorders[0]!.emit({ size: 1, arrayBuffer: async () => { throw new Error("read failed"); } });
    await flush();
    expect(unreadable.ofKind("segment")).toHaveLength(0);
    expect(unreadable.ofKind("finish")[0]?.interrupted).toBe(true);
    const rejected = fixture();
    rejected.reject.add("finish");
    rejected.command("start");
    await flush();
    await vi.advanceTimersByTimeAsync(1000);
    rejected.command("stop");
    await flush();
    expect(rejected.ofKind("status").at(-1)?.detail).toBe("native_rejected");
  });

  it("does not label an empty MediaRecorder segment successful", async () => {
    const harness = fixture();
    harness.command("start");
    await flush();
    harness.recorders[0]!.stopBytes = 0;
    await vi.advanceTimersByTimeAsync(1000);
    harness.command("stop");
    await flush();
    expect(harness.ofKind("segment")).toHaveLength(0);
    expect(harness.ofKind("finish")[0]).toMatchObject({ interrupted: true, reason: "empty_segment" });
  });

  it("honors the native final interrupted state even when the finish request was graceful", async () => {
    const harness = fixture();
    harness.hold.add("finish");
    harness.command("start");
    await flush();
    await vi.advanceTimersByTimeAsync(1000);
    harness.command("stop");
    await flush();
    const completion = harness.ofKind("finish")[0]!;
    expect(completion.interrupted).toBe(false);
    harness.reply(completion, true, { stopped: true, interrupted: true, status: "Interrupted" });
    await flush();
    expect(harness.ofKind("status").at(-1)?.detail).toBe("native_rejected");
  });

  it.each(["seek", "rate_change", "video_changed", "channel_changed", "no_audio", "page_hidden"])(
    "safely interrupts on %s", async (reason) => {
      const harness = fixture();
      harness.command("start");
      await flush();
      await vi.advanceTimersByTimeAsync(1000);
      if (reason === "seek") { harness.video.seeking = true; harness.video.dispatch("seeking"); }
      if (reason === "rate_change") { harness.video.playbackRate = 2; harness.video.dispatch("ratechange"); }
      if (reason === "video_changed") { harness.videos.splice(0, 1, new Video()); }
      if (reason === "channel_changed") { harness.window.location = new URL("https://chzzk.naver.com/live/" + "a".repeat(32)); }
      if (reason === "no_audio") { harness.video.stream.getAudioTracks()[0]!.dispatch("ended"); }
      if (reason === "page_hidden") { harness.window.dispatch("pagehide"); }
      await vi.advanceTimersByTimeAsync(1000);
      await flush();
      expect(harness.ofKind("finish")[0]).toMatchObject({ interrupted: true, reason });
      expect(harness.video.stream.getTracks().every((track) => track.stops === 1)).toBe(true);
    },
  );

  it("ignores visibility/minimize and a redundant ratechange event, omitting nonfinite buffer data", async () => {
    const harness = fixture();
    harness.command("start");
    await flush();
    harness.window.dispatch("visibilitychange");
    harness.video.dispatch("ratechange");
    harness.video.buffered.end = () => Number.POSITIVE_INFINITY;
    await vi.advanceTimersByTimeAsync(1000);
    expect(harness.ofKind("finish")).toHaveLength(0);
    expect(harness.ofKind("status").at(-1)).not.toHaveProperty("bufferSeconds");
  });
});
