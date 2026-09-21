// @vitest-environment node
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import source from "../../../src-tauri/src/streaming/browser_page_chat.js?raw";
const vmName = "node:vm";
const { runInNewContext } = await import(vmName) as { runInNewContext(source: string, context: Record<string, unknown>): unknown };
const CHANNEL = "b3e262a2795f17734c149afc738ad250";
const RECORDING = "10000000000040008000000000000001";
type EventValue = Record<string, unknown>;
type ChatApi = {
  start(options: { recordingId: string; channelId: string; getVideo?: () => unknown; sendBatch(events: EventValue[], batchId?: number): Promise<unknown>; onStatus(detail: string, dropped: number): void }): boolean;
  stop(recordingId: string): Promise<void>;
  pulse(): void;
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
const viewerResponse = (count: number, overrides: Record<string, unknown> = {}) => new Response(JSON.stringify({
  code: 200, content: { channelId: CHANNEL, openDate: "2024-07-03 09:00:00", status: "OPEN", cvExposure: true,
    concurrentUserCount: count, ...overrides },
}), { headers: { "content-type": "application/json;charset=UTF-8" } });
function fixture(url = `https://chzzk.naver.com/live/${CHANNEL}`) {
  const window = { location: new URL(url), top: null as unknown, WebSocket: Socket } as {
    location: URL; top: unknown; WebSocket: typeof Socket; __atsumiPageChat?: ChatApi;
    fetch?: typeof fetch;
    __atsumiEncodedCapture?: { getReplayClock(video: unknown): unknown };
    __atsumiChatEnhancements?: { readViewerCount(): number | null };
  };
  window.top = window;
  const video = Object.assign(new EventTarget(), { readyState: 4, currentTime: 100, playbackRate: 1, seeking: false, ended: false, currentSrc: "blob:fixture" });
  const clock = { wall: 1720000000000, monotonic: 1000 };
  const context = { window, URL, TextEncoder, TextDecoder, ArrayBuffer, Blob, Uint8Array, crypto,
    Date: { now: () => clock.wall, parse: Date.parse }, performance: { now: () => clock.monotonic },
    AbortController, setTimeout, clearTimeout, setInterval, clearInterval };
  runInNewContext(source, context);
  const batches: EventValue[][] = [];
  const status: Array<{ detail: string; dropped: number }> = [];
  let transport = async (events: EventValue[], _batchId?: number) => { batches.push(events); };
  const begin = () => window.__atsumiPageChat!.start({ recordingId: RECORDING, channelId: CHANNEL,
    getVideo: () => video,
    sendBatch: (events, batchId) => transport(events, batchId), onStatus: (detail, dropped) => status.push({ detail, dropped }) });
  return { window, batches, status, begin, video, clock,
    api: window.__atsumiPageChat!,
    setTransport: (next: (events: EventValue[], batchId?: number) => Promise<void>) => { transport = next; },
    again: () => runInNewContext(source, context),
    socket: (url = "wss://kr-ss1.chat.naver.com/chat") => new window.WebSocket(url, ["fixture"]),
  };
}
beforeEach(() => vi.useFakeTimers());
afterEach(() => { vi.restoreAllMocks(); vi.clearAllTimers(); vi.useRealTimers(); });

describe("official page chat receive bridge", () => {
  it("continues chat and viewer persistence from receive pulses after presentation timer suspension", async () => {
    const h = fixture(); h.window.fetch = vi.fn().mockResolvedValue(viewerResponse(17));
    const socket = h.socket(); h.begin(); await flush(); vi.clearAllTimers();
    h.clock.wall += 16000; h.clock.monotonic += 16000;
    socket.emit(envelope([message("after switching to live")])); await flush();
    h.api.pulse(); await flush(); h.api.pulse(); await flush();
    await h.api.stop(RECORDING);
    expect(h.batches.flat().some(event => event.msg === "after switching to live")).toBe(true);
    expect(h.window.fetch).toHaveBeenCalledTimes(2);
  });
  it("bounds an optional sender hash so a stalled digest cannot block subsequent chat or statistics", async () => {
    const digest = vi.spyOn(crypto.subtle, "digest").mockImplementation(() => new Promise(() => {}));
    const h = fixture(); const socket = h.socket(); h.begin();
    socket.emit(envelope([{ ...message("no hash"), profile: { nickname: "viewer", userIdHash: "opaque-id" } }, { ...message("following message"), profile: { nickname: "viewer2", userIdHash: "another-id" } }]));
    await flush(); await vi.advanceTimersByTimeAsync(1501); await flush(); await h.api.stop(RECORDING);
    expect(h.batches.flat().map(event => event.msg)).toEqual(["no hash", "following message"]);
    expect(digest).toHaveBeenCalledTimes(1);
  });
  it("reuses the same batch identity after a lost ACK and does not recreate the socket", async () => {
    const h = fixture(); const socket = h.socket(); const identities: (number | undefined)[] = [];
    h.setTransport(async (events, id) => { identities.push(id); if (identities.length === 1) throw Object.assign(new Error("lost ACK"),{code:"BROWSER_ACK_TIMEOUT"}); h.batches.push(events); });
    h.begin(); socket.emit(envelope([message()])); const stopped = h.api.stop(RECORDING);
    await flush(); await vi.advanceTimersByTimeAsync(100); await stopped;
    expect(identities).toEqual([1, 1]); expect(h.batches.flat()).toHaveLength(1); expect(socket.sent).toEqual([]);
  });
  it("waits out busy storage instead of exhausting all chat retries in milliseconds", async () => {
    const h=fixture(); const socket=h.socket(); const identities: (number|undefined)[]=[];
    h.setTransport(async (events,id)=>{identities.push(id); if(identities.length<=8) throw Object.assign(new Error("busy"),{code:"BRIDGE_BUSY"}); h.batches.push(events);});
    h.begin(); socket.emit(envelope([message()])); const stopped=h.api.stop(RECORDING); await flush();
    for(const delay of [100,200,400,800,1600,2000,2000,2000]) {
      await vi.advanceTimersByTimeAsync(delay); await flush();
      expect(h.status.some(s=>s.detail==="storage_failed")).toBe(false);
    }
    await stopped; expect(identities).toEqual(Array(9).fill(1)); expect(h.batches.flat()).toHaveLength(1);
  });
  it("exports only a bounded official default-color seed, never a raw opaque identifier", async () => {
    const h = fixture(); const socket = h.socket(); h.begin();
    socket.emit(envelope([
      { ...message(), profile: { nickname: "viewer", userIdHash: "opaque_ABC-123" } },
      { ...message(), profile: { nickname: "viewer", userIdHash: "a".repeat(129) } },
      { ...message(), profile: { nickname: "viewer", userIdHash: "../private" } },
    ]));
    await h.api.stop(RECORDING);
    const profiles = h.batches.flat().map((event) => event.profile as Record<string, unknown>);
    expect(profiles[0]!.nicknameColorSeed).toBe(Array.from("opaque_ABC-123").reduce((sum, c) => sum + c.charCodeAt(0), 0) % 40);
    expect(profiles[1]!.nicknameColorSeed).toBeUndefined();
    expect(profiles[2]!.nicknameColorSeed).toBeUndefined();
    expect(JSON.stringify(h.batches)).not.toMatch(/opaque_ABC-123|userIdHash|\.\.\/private/);
    expect(socket.sent).toEqual([]);
  });
  it("stores public profile destinations and verified symbolic colors without raw credentials", async () => {
    const h = fixture(); const socket = h.socket(); h.begin();
    socket.emit(envelope([{ ...message(), profile: { nickname: "viewer", userIdHash: CHANNEL.toUpperCase(), accessToken: "PRIVATE_TOKEN", streamingProperty: { nicknameColor: { colorCode: "CD001" } }, title: { color: "#abcdef" } } }]));
    await h.api.stop(RECORDING);
    expect(h.batches.flat()[0]!.profile).toMatchObject({ publicProfileUrl: `https://chzzk.naver.com/${CHANNEL}`, title: { color: "#abcdef" }, streamingProperty: { nicknameColor: { colorCode: "#EEA05D" } } });
    expect(JSON.stringify(h.batches)).not.toMatch(/PRIVATE_TOKEN|userIdHash|accessToken/);
  });
  it("requests fresh public viewers only after opt-in at 10s, shares the queue and stops", async () => {
    const h = fixture(); const request = vi.fn<typeof fetch>().mockResolvedValue(viewerResponse(321));
    h.window.fetch = request;
    const readViewerCount = vi.fn(() => 999); h.window.__atsumiChatEnhancements = { readViewerCount };
    vi.advanceTimersByTime(4000); expect(request).not.toHaveBeenCalled();
    const socket = h.socket(); h.begin(); await flush();
    h.clock.wall += 100; h.clock.monotonic += 100; socket.emit(envelope([message("ordered chat")])); await flush();
    vi.advanceTimersByTime(9999); await flush(); expect(request).toHaveBeenCalledTimes(1);
    h.clock.wall += 10000; h.clock.monotonic += 10000;
    request.mockRejectedValue(new Error("network unavailable")); vi.advanceTimersByTime(1); await flush();
    await h.api.stop(RECORDING); const events = h.batches.flat();
    expect(events.map((event) => event.atsumiViewerSample ? event.viewerCount : event.msg)).toEqual([321,"ordered chat",null]);
    expect(events[0]).toMatchObject({ viewerSource: "chzzk_live_status_api_v1", viewerChannelId: CHANNEL,
      viewerBroadcastStartedAt: Date.parse("2024-07-03T09:00:00+09:00") });
    expect(request.mock.calls[0]).toEqual([
      `https://api.chzzk.naver.com/polling/v3.1/channels/${CHANNEL}/live-status`,
      expect.objectContaining({ method: "GET", credentials: "omit", cache: "no-store", redirect: "error", referrerPolicy: "no-referrer", signal: expect.any(AbortSignal) }),
    ]);
    expect(request.mock.calls[0]![1]).not.toHaveProperty("headers");
    expect(socket.sent).toEqual([]); expect(readViewerCount).not.toHaveBeenCalled();
    vi.advanceTimersByTime(20000); await flush(); expect(request).toHaveBeenCalledTimes(2);
  });
  it("bounds viewer request timeout, never overlaps, records unknown and retries next interval", async () => {
    const h = fixture(); let resolve!: (response: Response) => void;
    const request = vi.fn<typeof fetch>().mockImplementationOnce(() => new Promise<Response>((done) => { resolve = done; }))
      .mockResolvedValue(viewerResponse(0));
    h.window.fetch = request; h.begin(); await flush();
    vi.advanceTimersByTime(7999); await flush(); expect(request).toHaveBeenCalledTimes(1);
    h.clock.wall += 8000; h.clock.monotonic += 8000; vi.advanceTimersByTime(1); await flush();
    expect(request.mock.calls[0]![1]!.signal!.aborted).toBe(true);
    resolve(viewerResponse(999)); await flush();
    h.clock.wall += 2000; h.clock.monotonic += 2000; vi.advanceTimersByTime(2000); await flush();
    await h.api.stop(RECORDING);
    expect(request).toHaveBeenCalledTimes(2);
    expect(h.batches.flat().map(event => event.viewerCount)).toEqual([null, 0]);
  });
  it.each(["stop", "channel", "source", "seek"])("rejects viewer responses after %s invalidation", async (kind) => {
    const h = fixture(); let resolve!: (response: Response) => void;
    const request = vi.fn<typeof fetch>().mockImplementation(() => new Promise<Response>((done) => { resolve = done; }));
    h.window.fetch = request; h.begin(); await flush();
    if (kind === "stop") await h.api.stop(RECORDING);
    if (kind === "channel") h.window.location = new URL(`https://chzzk.naver.com/live/${"a".repeat(32)}`);
    if (kind === "source") h.video.currentSrc = "blob:next";
    if (kind === "seek") h.video.dispatchEvent(new Event("seeking"));
    resolve(viewerResponse(555)); await flush(); await h.api.stop(RECORDING);
    expect(h.batches.flat().map(event => event.viewerCount)).toEqual(kind === "source" || kind === "seek" ? [null] : []);
    vi.advanceTimersByTime(20000); await flush(); expect(request).toHaveBeenCalledTimes(1);
  });
  it("pins the public broadcast generation and stops polling when another live replaces it", async () => {
    const h = fixture(); const request = vi.fn<typeof fetch>().mockResolvedValueOnce(viewerResponse(15))
      .mockResolvedValue(viewerResponse(777, { openDate: "2024-07-03 10:00:00" }));
    h.window.fetch = request; h.begin(); await flush();
    h.clock.wall += 10000; h.clock.monotonic += 10000; vi.advanceTimersByTime(10000); await flush();
    vi.advanceTimersByTime(30000); await flush(); await h.api.stop(RECORDING);
    expect(request).toHaveBeenCalledTimes(2);
    expect(h.batches.flat().map(event => event.viewerCount)).toEqual([15, null]);
  });
  it.each([
    { cvExposure: false }, { channelId: "a".repeat(32) }, { status: "CLOSE" },
    { concurrentUserCount: -1 }, { concurrentUserCount: 1.5 }, { concurrentUserCount: "500" },
    { concurrentUserCount: 100000001 }, { openDate: null }, { openDate: "invalid" },
  ])("keeps unavailable or unexposed public counts unknown: %j", async (content) => {
    const h = fixture(); h.window.fetch = vi.fn<typeof fetch>().mockResolvedValue(viewerResponse(777, content));
    h.begin(); await flush(); await h.api.stop(RECORDING);
    expect(h.batches.flat().map(event => event.viewerCount)).toEqual([null]);
  });
  it("bounds chunked viewer responses and never exports full response routing or auth fields", async () => {
    const h = fixture(); const response = viewerResponse(14, { liveTokenList: ["PRIVATE_TOKEN"], chatChannelId: "PRIVATE_ROUTE" });
    h.window.fetch = vi.fn<typeof fetch>().mockResolvedValueOnce(response)
      .mockResolvedValue(new Response("x".repeat(32769), { headers: { "content-type": "application/json" } }));
    h.begin(); await flush();
    h.clock.wall += 10000; h.clock.monotonic += 10000; vi.advanceTimersByTime(10000); await flush();
    await h.api.stop(RECORDING);
    expect(h.batches.flat().map(event => event.viewerCount)).toEqual([14, null]);
    expect(JSON.stringify(h.batches)).not.toMatch(/PRIVATE|liveTokenList|chatChannelId|openDate/);
  });
  it.each([
    () => new Response("{}", { status: 403, headers: { "content-type": "application/json" } }),
    () => new Response("{}", { headers: { "content-type": "text/html" } }),
    () => new Response("{bad", { headers: { "content-type": "application/json" } }),
  ])("records unknown for rejected, invalid-type and malformed viewer responses", async (response) => {
    const h = fixture(); h.window.fetch = vi.fn<typeof fetch>().mockResolvedValue(response());
    h.begin(); await flush(); await h.api.stop(RECORDING);
    expect(h.batches.flat().map(event => event.viewerCount)).toEqual([null]);
  });
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
