// @vitest-environment node
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import source from "../../../src-tauri/src/streaming/browser_page_chat.js?raw";
const vmName = "node:vm";
const { runInNewContext } = await import(vmName) as { runInNewContext(source: string, context: Record<string, unknown>): unknown };
const CHANNEL = "b3e262a2795f17734c149afc738ad250";
const RECORDING = "10000000000040008000000000000001";
type EventValue = Record<string, unknown>;
type ChatApi = {
  start(options: { recordingId: string; channelId: string; getVideo?: () => unknown; sendBatch(events: EventValue[]): Promise<unknown>; onStatus(detail: string, dropped: number): void }): boolean;
  stop(recordingId: string): Promise<void>;
};
class Socket {
  static OPEN = 1;
  static CONNECTING = 0;
  static CLOSED = 3;
  readyState = 1;
  sent: unknown[] = [];
  listeners = new Map<string, Array<(event: { data?: unknown }) => void>>();
  onmessage: ((event: { data?: unknown }) => void) | null = null;
  constructor(public url: string, public protocols?: string[]) {}
  addEventListener(type: string, callback: (event: { data?: unknown }) => void) {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), callback]);
  }
  send(value: unknown) { this.sent.push(value); return "unchanged"; }
  emit(data: unknown) {
    const event = { data };
    for (const callback of this.listeners.get("message") ?? []) callback(event);
    this.onmessage?.(event);
  }
  close() { this.readyState = 3; for (const callback of this.listeners.get("close") ?? []) callback({}); }
}
const message = (value = "hello"): EventValue => ({ msgTypeCode: 1, msg: value, msgTime: 1720000000123, profile: { nickname: "viewer" } });
const envelope = (events: EventValue[], wrapped = false) => JSON.stringify({ cmd: 93101, bdy: wrapped ? { messageList: events } : events });
const flush = async () => { for (let index = 0; index < 80; index++) await Promise.resolve(); };
function fixture(url = `https://chzzk.naver.com/live/${CHANNEL}`) {
  const window = { location: new URL(url), top: null as unknown, WebSocket: Socket } as {
    location: URL; top: unknown; WebSocket: typeof Socket; __atsumiPageChat?: ChatApi;
    __atsumiEncodedCapture?: { getReplayClock(video: unknown): unknown };
  };
  window.top = window;
  const video = Object.assign(new EventTarget(), { readyState: 4, currentTime: 100, playbackRate: 1, seeking: false, ended: false, currentSrc: "blob:fixture" });
  const clock = { wall: 1720000000000, monotonic: 1000 };
  const context = { window, URL, TextEncoder, TextDecoder, ArrayBuffer, Blob, Uint8Array, crypto,
    Date: { now: () => clock.wall }, performance: { now: () => clock.monotonic }, setInterval, clearInterval };
  runInNewContext(source, context);
  const batches: EventValue[][] = [];
  const status: Array<{ detail: string; dropped: number }> = [];
  let transport = async (events: EventValue[]) => { batches.push(events); };
  const begin = () => window.__atsumiPageChat!.start({ recordingId: RECORDING, channelId: CHANNEL,
    getVideo: () => video,
    sendBatch: (events) => transport(events), onStatus: (detail, dropped) => status.push({ detail, dropped }) });
  return { window, batches, status, begin, video, clock,
    api: window.__atsumiPageChat!,
    setTransport: (next: (events: EventValue[]) => Promise<void>) => { transport = next; },
    again: () => runInNewContext(source, context),
    socket: (url = "wss://kr-ss1.chat.naver.com/chat") => new window.WebSocket(url, ["fixture"]),
  };
}
beforeEach(() => vi.useFakeTimers());
afterEach(() => { vi.clearAllTimers(); vi.useRealTimers(); });

describe("official page chat receive bridge", () => {
  it("does not install on another origin and preserves native construction, constants, send and page listeners", async () => {
    expect(fixture("https://example.com/live/" + CHANNEL).api).toBeUndefined();
    const harness = fixture();
    harness.again();
    const socket = harness.socket();
    expect(socket).toBeInstanceOf(Socket);
    expect(harness.window.WebSocket.prototype).toBe(Socket.prototype);
    expect(harness.window.WebSocket.OPEN).toBe(Socket.OPEN);
    expect(socket.protocols).toEqual(["fixture"]);
    expect(socket.send("AUTH_TOKEN_NOT_OBSERVED")).toBe("unchanged");
    const page = vi.fn();
    socket.onmessage = page;
    socket.emit(envelope([message("before consent")]));
    await flush();
    expect(page).toHaveBeenCalledOnce();
    expect(harness.batches).toEqual([]);
    expect(harness.status).toEqual([]);
    class Derived extends harness.window.WebSocket {}
    expect(new Derived("wss://kr-ss2.chat.naver.com/chat")).toBeInstanceOf(Derived);
  });

  it("observes only allowed chat hosts and ordinary non-hidden live messages", async () => {
    const harness = fixture();
    const ignored = ["wss://evil.test/chat", "wss://kr-ss1.chat.naver.com.evil.test/chat", "ws://kr-ss1.chat.naver.com/chat", "wss://kr-ss1.chat.naver.com/other"];
    const sockets = ignored.map((url) => harness.socket(url));
    const socket = harness.socket();
    harness.begin();
    for (const other of sockets) other.emit(envelope([message("wrong socket")]));
    socket.emit(JSON.stringify({ cmd: 10100, bdy: { accTkn: "SECRET" } }));
    socket.emit(JSON.stringify({ cmd: 15101, bdy: [message("history")] }));
    socket.emit(envelope([{ ...message("hidden"), msgStatusType: "HIDDEN" }, { ...message("donation"), msgTypeCode: 10 }, message("live")], true));
    await harness.api.stop(RECORDING);
    expect(harness.batches.flat().map((event) => event.msg)).toEqual(["live"]);
  });

  it("exports only selected display fields, not raw profile identifiers, tokens or unrelated extras", async () => {
    const harness = fixture();
    const socket = harness.socket();
    harness.begin();
    socket.emit(envelope([{ ...message("hi {:wave:}"), profile: JSON.stringify({ nickname: "viewer",
      userIdHash: "PRIVATE_ID", accessToken: "SECRET_TOKEN", email: "private@example.com",
      title: { name: "display title", color: "#aabbcc", token: "SECRET_TOKEN" },
      streamingProperty: { nicknameColor: { colorCode: "#112233" }, subscription: { accumulativeMonth: 3, badge: { imageUrl: "https://ssl.pstatic.net/badge.png?token=SECRET_TOKEN" } } },
    }), extras: JSON.stringify({ accessToken: "SECRET_TOKEN", emojis: { wave: "https://ssl.pstatic.net/wave.png?type=w80", unused: "https://evil.test/private.png" } }) }]));
    await harness.api.stop(RECORDING);
    const event = harness.batches.flat()[0]!;
    const serialized = JSON.stringify(event);
    expect(serialized).not.toMatch(/PRIVATE_ID|SECRET_TOKEN|email|accessToken|userIdHash|evil.test/);
    expect(event.profile).toMatchObject({ nickname: "viewer", title: { name: "display title", color: "#aabbcc" } });
    expect(event.extras).toEqual({ emojis: { wave: "https://ssl.pstatic.net/wave.png" } });
  });

  it("streams more than 200 messages without a history cap and bounds each UTF-8 batch", async () => {
    const harness = fixture();
    const socket = harness.socket();
    harness.begin();
    for (let frame = 0; frame < 9; frame++) {
      socket.emit(envelope(Array.from({ length: 25 }, (_, index) => message(`${frame * 25 + index} 😀`))));
    }
    await harness.api.stop(RECORDING);
    expect(harness.batches.flat()).toHaveLength(225);
    expect(harness.batches.flat().at(-1)?.msg).toBe("224 😀");
    for (const batch of harness.batches) {
      expect(batch.length).toBeLessThanOrEqual(32);
      expect(new TextEncoder().encode(JSON.stringify(batch)).byteLength).toBeLessThanOrEqual(128 * 1024);
    }
  });

  it("preserves Blob/string receive order and drains received messages before stop completes", async () => {
    const harness = fixture();
    const socket = harness.socket();
    harness.begin();
    let releaseBlob!: (value: string) => void;
    const blob = new Blob([envelope([message("first")])]);
    blob.text = () => new Promise<string>((resolve) => { releaseBlob = resolve; });
    socket.emit(blob);
    socket.emit(envelope([message("second")]));
    await flush();
    let releaseAck!: () => void;
    harness.setTransport(async (events) => {
      harness.batches.push(events);
      await new Promise<void>((resolve) => { releaseAck = resolve; });
    });
    let stopped = false;
    const draining = harness.api.stop(RECORDING).then(() => { stopped = true; });
    socket.emit(envelope([message("after stop")]));
    releaseBlob(envelope([message("first")]));
    await flush();
    expect(stopped).toBe(false);
    expect(harness.batches.flat().map((event) => event.msg)).toEqual(["first", "second"]);
    releaseAck();
    await draining;
    expect(harness.status.at(-1)?.detail).toBe("stopped");
  });

  it("samples socket receipt and video position before delayed Blob decode and keeps source generations", async () => {
    const h = fixture(), socket = h.socket(); h.begin();
    let release!: (value: string) => void;
    const blob = new Blob(["fixture"]); blob.text = () => new Promise<string>((resolve) => { release = resolve; });
    socket.emit(blob); await flush();
    h.clock.wall += 5000; h.clock.monotonic += 5000; h.video.currentTime = 108; h.video.playbackRate = 2;
    h.video.dispatchEvent(new Event("seeking")); h.video.currentSrc = "blob:replacement";
    socket.emit(envelope([message("second")]));
    release(envelope([message("first")])); await h.api.stop(RECORDING);
    const [first, second] = h.batches.flat().map((event) => event.replayClock as Record<string, number | string>);
    expect(first).toMatchObject({ version: 1, receivedAtMs: 1720000000000, observedMonotonicMs: 1000, mediaTimeSeconds: 100, playbackRate: 1, clock: "player_observation" });
    expect(second).toMatchObject({ receivedAtMs: 1720000005000, observedMonotonicMs: 6000, mediaTimeSeconds: 108, playbackRate: 2 });
    expect(second!.sourceGeneration).toBeGreaterThan(first!.sourceGeneration as number);
    expect(first).not.toHaveProperty("sourceTimeSeconds");
    expect(JSON.stringify(h.batches)).not.toContain("blob:");
  });

  it("keeps opaque sender digests stable across reconnects and changes the salt for a new recording", async () => {
    const h = fixture(); let socket = h.socket(); h.begin();
    const identified = { ...message(), profile: { nickname: "viewer", userIdHash: "opaque_account_123" } };
    socket.emit(envelope([identified])); await flush(); socket.close(); socket = h.socket();
    socket.emit(envelope([{ ...identified, profile: { ...identified.profile, nickname: "renamed" } }, message("unknown sender")]));
    await h.api.stop(RECORDING);
    const old = h.batches.flat(); expect(old[0]!.senderKey).toMatch(/^sha256:[a-f0-9]{64}$/);
    expect(old[1]!.senderKey).toBe(old[0]!.senderKey); expect(old[2]).not.toHaveProperty("senderKey");
    h.begin(); socket.emit(envelope([identified])); await h.api.stop(RECORDING);
    expect(h.batches.flat().at(-1)!.senderKey).not.toBe(old[0]!.senderKey);
    expect(JSON.stringify(h.batches)).not.toMatch(/opaque_account_123|userIdHash|salt/);
  });

  it("uses a single approved presentation sample even if currentTime advances between reads", async () => {
    const h = fixture(), socket = h.socket(); let media = 4000;
    Object.defineProperty(h.video, "currentTime", { get: () => media += .001 });
    h.window.__atsumiEncodedCapture = { getReplayClock: () => ({ clock: "mse_presentation_v1",
      sourceId: "40000000-0000-4000-8000-000000000001", sourceTimeSeconds: h.video.currentTime }) };
    h.begin(); socket.emit(envelope([message()])); await h.api.stop(RECORDING);
    const observation = h.batches.flat()[0]!.replayClock as Record<string, unknown>;
    expect(observation.clock).toBe("mse_presentation_v1");
    expect(observation.mediaTimeSeconds).toBe(observation.sourceTimeSeconds);
  });

  it("reports rejected storage and oversize frames without disrupting official socket listeners", async () => {
    const rejected = fixture();
    const socket = rejected.socket();
    rejected.setTransport(async () => { throw new Error("native rejection"); });
    rejected.begin();
    socket.emit(envelope([message()]));
    await rejected.api.stop(RECORDING);
    expect(rejected.status.at(-1)).toMatchObject({ detail: "storage_failed", dropped: 1 });
    const oversize = fixture();
    const other = oversize.socket();
    const original = vi.fn();
    other.onmessage = original;
    oversize.begin();
    other.emit("x".repeat(256 * 1024 + 1));
    await oversize.api.stop(RECORDING);
    expect(oversize.status.at(-1)).toMatchObject({ detail: "frame_too_large", dropped: 1 });
    expect(original).toHaveBeenCalledOnce();
    expect(oversize.batches).toHaveLength(0);
  });

  it("bounds raw decode backlog at 2 MiB and reports the loss instead of silently stopping", async () => {
    const harness = fixture();
    const socket = harness.socket();
    harness.begin();
    const raw = envelope([message("x".repeat(200_000))]);
    for (let index = 0; index < 12; index++) socket.emit(raw);
    await harness.api.stop(RECORDING);
    expect(harness.status.at(-1)?.detail).toBe("queue_overflow");
    expect(harness.status.at(-1)?.dropped).toBeGreaterThan(0);
  });

  it("marks connection gaps, does not forward another page's channel, and can begin a fresh recording", async () => {
    const harness = fixture();
    const socket = harness.socket();
    harness.begin();
    socket.close();
    harness.window.location = new URL("https://chzzk.naver.com/live/" + "a".repeat(32));
    socket.emit(envelope([message("other channel")]));
    await harness.api.stop(RECORDING);
    expect(harness.status.at(-1)?.detail).toBe("partial");
    expect(harness.batches).toHaveLength(0);
    harness.window.location = new URL(`https://chzzk.naver.com/live/${CHANNEL}`);
    expect(harness.begin()).toBe(true);
    harness.socket().emit(envelope([message("new recording")]));
    await harness.api.stop(RECORDING);
    expect(harness.batches.flat().map((event) => event.msg)).toEqual(["new recording"]);
  });

  it("never attributes a previous channel's still-open socket to a new channel recording", async () => {
    const harness = fixture();
    const previous = harness.socket();
    const nextChannel = "a".repeat(32);
    harness.window.location = new URL(`https://chzzk.naver.com/live/${nextChannel}`);
    expect(harness.api.start({ recordingId: RECORDING, channelId: nextChannel,
      sendBatch: async (events) => { harness.batches.push(events); },
      onStatus: (detail, dropped) => harness.status.push({ detail, dropped }),
    })).toBe(true);
    expect(harness.status.at(-1)?.detail).toBe("waiting_socket");
    previous.emit(envelope([message("old channel")]));
    previous.close();
    harness.socket().emit(envelope([message("current channel")]));
    await harness.api.stop(RECORDING);
    expect(harness.batches.flat().map((event) => event.msg)).toEqual(["current channel"]);
    expect(harness.status.at(-1)?.detail).toBe("stopped");
  });
});
