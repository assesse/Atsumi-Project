import { act, createRef, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { convertFileSrc } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReplayTimeline } from "../../api/replay";
import { OriginalChzzkPlayer, PLAYER_CHANNEL, validPlayerState, type OriginalPlayerHandle, type OriginalPlayerState } from "./OriginalChzzkPlayer";

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: vi.fn((path: string, protocol: string) => `http://${protocol}.localhost/${path}`),
  invoke: vi.fn(),
}));

type Props = ComponentProps<typeof OriginalChzzkPlayer>;
type Envelope = { channel: string; nonce: string; type: string; data?: unknown };
const state = (patch: Partial<OriginalPlayerState> = {}): OriginalPlayerState => ({ time: 12, duration: 120, paused: false, seeking: false, aspect: 16 / 9, ...patch });
const callbacks = { onState: vi.fn(), onError: vi.fn(), onFullscreen: vi.fn(), onWide: vi.fn(), onClose: vi.fn() };
let root: Root;
let container: HTMLDivElement;
let props: Props;
let handle: ReturnType<typeof createRef<OriginalPlayerHandle>>;
let mounted: boolean;
const frame = () => container.querySelector<HTMLIFrameElement>("iframe")!;
const nonce = () => new URL(frame().src).hash.slice(1);
const render = async (patch: Partial<Props> = {}) => {
  props = { ...props, ...patch };
  await act(async () => root.render(<OriginalChzzkPlayer {...props} ref={handle} />));
};
const receive = async (type: string, data?: unknown, envelope: Partial<Envelope> = {}, event: Partial<MessageEventInit> = {}) => {
  await act(async () => window.dispatchEvent(new MessageEvent("message", {
    source: frame().contentWindow,
    origin: "null",
    data: { channel: PLAYER_CHANNEL, nonce: nonce(), type, data, ...envelope },
    ...event,
  })));
};
const outgoing = () => vi.spyOn(frame().contentWindow!, "postMessage").mockImplementation(() => {});

beforeEach(() => {
  vi.clearAllMocks();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  mounted = true;
  handle = createRef<OriginalPlayerHandle>();
  props = { runtime: "browser-mock", mediaUrl: "http://atsumi-replay.localhost/synthetic-token", duration: 120, privacy: false, timeline: null, ...callbacks };
});

afterEach(async () => {
  if (mounted) await act(async () => root.unmount());
  container.remove();
  vi.restoreAllMocks();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("original CHZZK opaque frame bridge", () => {
  it("isolates the SDK and keeps the recording URL out of the frame location", async () => {
    await render();
    expect(frame()).toHaveAttribute("sandbox", "allow-scripts");
    expect(frame()).toHaveAttribute("referrerpolicy", "no-referrer");
    expect(frame()).toHaveAttribute("allow", "autoplay; fullscreen; picture-in-picture");
    expect(frame()).toHaveAttribute("allowfullscreen");
    expect(new URL(frame().src).pathname).toBe("/original-player/frame.html");
    expect(nonce()).toMatch(/^[a-f0-9]{32}$/);
    expect(frame().src).not.toContain("synthetic-token");
    expect(container.querySelector("video")).toBeNull();
    expect(convertFileSrc).not.toHaveBeenCalled();
  });

  it("uses only the native-owned runtime protocol in desktop mode", async () => {
    await render({ runtime: "tauri" });
    expect(convertFileSrc).toHaveBeenCalledExactlyOnceWith("frame.html", "atsumi-player");
    expect(frame().src).toBe(`http://atsumi-player.localhost/frame.html#${nonce()}`);
    expect(frame().getAttribute("sandbox")).not.toContain("allow-same-origin");
  });

  it.each(["source", "missing source", "origin", "channel", "nonce"])("rejects forged %s before granting the ready handshake", async (forgery) => {
    await render();
    const post = outgoing();
    const envelope = forgery === "channel" ? { channel: "other-channel" } : forgery === "nonce" ? { nonce: "0".repeat(32) } : {};
    const event = forgery === "source" ? { source: window } : forgery === "missing source" ? { source: null } : forgery === "origin" ? { origin: location.origin } : {};
    await receive("ready", undefined, envelope, event);
    await receive("state", state(), envelope, event);
    await receive("error", "untrusted error", envelope, event);
    await receive("fullscreen", undefined, envelope, event);
    await receive("wide", undefined, envelope, event);
    await receive("close", undefined, envelope, event);
    expect(post).not.toHaveBeenCalled();
    Object.values(callbacks).forEach(callback => expect(callback).not.toHaveBeenCalled());
  });

  it("initializes only the authenticated frame and does not reload on repeated ready", async () => {
    await render();
    const post = outgoing();
    expect(post).not.toHaveBeenCalled();
    await receive("ready");
    const init = { channel: PLAYER_CHANNEL, nonce: nonce(), type: "init", data: { url: props.mediaUrl, duration: 120, privacy: false } };
    expect(post).toHaveBeenCalledWith(init, "*");
    expect(post).toHaveBeenCalledWith({ channel: PLAYER_CHANNEL, nonce: nonce(), type: "privacy", data: false }, "*");
    expect(post).toHaveBeenCalledWith({ channel: PLAYER_CHANNEL, nonce: nonce(), type: "metrics", data: null }, "*");
    post.mockClear();
    await receive("ready");
    expect(post).not.toHaveBeenCalled();
  });

  it("updates the saved channel profile without reloading or pausing the original player", async () => {
    await render({ recording: { title: "저장 방송", channelName: "저장 채널" } });
    const owned = frame(), post = outgoing(); await receive("ready"); post.mockClear();
    const recording = { title: "저장 방송", channelName: "저장 채널", profileImage: "data:image/png;base64,iVBORw0KGgo=" };
    await render({ recording });
    expect(frame()).toBe(owned);
    expect(post).toHaveBeenCalledExactlyOnceWith({ channel: PLAYER_CHANNEL, nonce: nonce(), type: "metadata", data: recording }, "*");
  });

  it("delivers valid state and events to current callbacks without exposing frame error details", async () => {
    await render();
    outgoing();
    await receive("ready");
    const onState = vi.fn();
    await render({ onState });
    await receive("state", state());
    expect(onState).toHaveBeenCalledExactlyOnceWith(state());
    expect(callbacks.onState).not.toHaveBeenCalled();
    await receive("fullscreen");
    await receive("wide");
    await receive("close");
    expect(callbacks.onFullscreen).toHaveBeenCalledTimes(1);
    expect(callbacks.onWide).toHaveBeenCalledTimes(1);
    expect(callbacks.onClose).toHaveBeenCalledTimes(1);
    await receive("error", { message: "C:\\private\\recording.mp4?token=secret" });
    expect(callbacks.onError).toHaveBeenCalledTimes(1);
    expect(callbacks.onError.mock.calls[0]![0]).not.toMatch(/private|recording\.mp4|secret/);
  });

  it("ignores malformed envelopes, unknown events and out-of-bounds state", async () => {
    await render();
    outgoing();
    await receive("ready");
    for (const data of [null, undefined, true, 1, "ready", [], {}]) {
      await receive("state", undefined, {}, { data });
    }
    await receive("unknown", state());
    for (const data of [null, {}, { ...state(), time: "12" }, state({ time: -1 }), state({ time: NaN }), state({ time: Infinity }), state({ time: 604801 }), state({ duration: -1 }), state({ duration: NaN }), state({ duration: Infinity }), state({ duration: 604801 }), state({ aspect: NaN }), state({ aspect: Infinity }), state({ aspect: .049 }), state({ aspect: 20.001 }), { ...state(), paused: 0 }, { ...state(), seeking: "false" }]) {
      await receive("state", data);
    }
    Object.values(callbacks).forEach(callback => expect(callback).not.toHaveBeenCalled());
  });

  it("changes privacy without reinitializing media, and initializes a replacement source once", async () => {
    await render();
    const frameNonce = nonce();
    const post = outgoing();
    await receive("ready");
    post.mockClear();
    await render({ privacy: true });
    expect(post).toHaveBeenCalledExactlyOnceWith({ channel: PLAYER_CHANNEL, nonce: frameNonce, type: "privacy", data: true }, "*");
    post.mockClear();
    await render({ mediaUrl: "http://atsumi-replay.localhost/replacement-token", duration: 90 });
    expect(post.mock.calls.filter(([message]) => message.type === "init")).toEqual([
      [{ channel: PLAYER_CHANNEL, nonce: frameNonce, type: "init", data: { url: props.mediaUrl, duration: 90, privacy: true } }, "*"],
    ]);
    expect(nonce()).toBe(frameNonce);
  });

  it("scopes imperative commands to the same frame and nonce", async () => {
    await render();
    const post = outgoing();
    await receive("ready");
    post.mockClear();
    await act(async () => {
      handle.current!.seek(42.5);
      handle.current!.toggle();
      handle.current!.mute();
      handle.current!.pause();
    });
    expect(post.mock.calls).toEqual([
      [{ channel: PLAYER_CHANNEL, nonce: nonce(), type: "seek", data: 42.5 }, "*"],
      [{ channel: PLAYER_CHANNEL, nonce: nonce(), type: "toggle", data: undefined }, "*"],
      [{ channel: PLAYER_CHANNEL, nonce: nonce(), type: "mute", data: undefined }, "*"],
      [{ channel: PLAYER_CHANNEL, nonce: nonce(), type: "privacy", data: true }, "*"],
    ]);
  });

  it("caps metrics at 2000 buckets and preserves unknown viewer observations", async () => {
    const timeline: ReplayTimeline = {
      bucketSeconds: 2, indexState: "ready", viewerMetricStatus: "partial",
      buckets: Array.from({ length: 2001 }, (_, index) => ({ startSeconds: index * 2, chatCount: 3, uniqueSenderCount: 2, viewerCount: index === 1 ? null : 50, viewerSampleCount: index === 1 ? 0 : 1, viewerCoverageSeconds: index === 1 ? 0 : 2 })),
    };
    await render({ duration: 4002, timeline });
    const post = outgoing();
    await receive("ready");
    const payload = post.mock.calls.find(([message]) => message.type === "metrics")![0].data;
    expect(payload.buckets).toHaveLength(2000);
    expect(timeline.buckets).toHaveLength(2001);
    expect(payload.buckets[1].viewerCount).toBeNull();
    expect(payload.participantCounts).toBe(true);
    expect(payload.chatPaths.length).toBeGreaterThan(0);
    expect(payload.viewerPaths).toHaveLength(2);
    post.mockClear();
    await render({ timeline: null });
    expect(post).toHaveBeenCalledExactlyOnceWith({ channel: PLAYER_CHANNEL, nonce: nonce(), type: "metrics", data: null }, "*");
  });

  it("uses the chat-count fallback only when participant observations are absent", async () => {
    await render({ timeline: { bucketSeconds: 2, indexState: "ready", viewerMetricStatus: "not_recorded", buckets: [{ startSeconds: 0, chatCount: 3, uniqueSenderCount: null, viewerCount: null }] } });
    const post = outgoing();
    await receive("ready");
    const payload = post.mock.calls.find(([message]) => message.type === "metrics")![0].data;
    expect(payload.participantCounts).toBe(false);
    expect(payload.chatPaths.length).toBeGreaterThan(0);
    expect(payload.viewerPaths).toEqual([]);
  });

  it("times out once at 20 seconds and does not let a forged ready cancel the deadline", async () => {
    vi.useFakeTimers();
    await render();
    outgoing();
    await receive("ready", undefined, { nonce: "not-the-frame-nonce" });
    await act(async () => vi.advanceTimersByTimeAsync(19999));
    expect(callbacks.onError).not.toHaveBeenCalled();
    const onError = vi.fn();
    await render({ onError });
    await act(async () => vi.advanceTimersByTimeAsync(1));
    expect(onError).toHaveBeenCalledTimes(1);
    expect(onError.mock.calls[0]![0]).toContain("원본 플레이어를 불러오지 못했습니다");
    expect(callbacks.onError).not.toHaveBeenCalled();
    await act(async () => vi.advanceTimersByTimeAsync(60000));
    expect(onError).toHaveBeenCalledTimes(1);
  });

  it("cancels the ready timeout only after the authenticated handshake", async () => {
    vi.useFakeTimers();
    await render();
    const post = outgoing();
    await act(async () => vi.advanceTimersByTimeAsync(19999));
    await receive("ready");
    expect(post.mock.calls.filter(([message]) => message.type === "init")).toHaveLength(1);
    await act(async () => vi.advanceTimersByTimeAsync(60000));
    expect(callbacks.onError).not.toHaveBeenCalled();
  });

  it("cancels a pending handshake timeout when the frame unmounts", async () => {
    vi.useFakeTimers();
    await render();
    const originalWindow = frame().contentWindow;
    const originalNonce = nonce();
    const post = outgoing();
    await act(async () => vi.advanceTimersByTimeAsync(19999));
    await act(async () => root.unmount());
    mounted = false;
    expect(post).toHaveBeenCalledExactlyOnceWith({ channel: PLAYER_CHANNEL, nonce: originalNonce, type: "dispose" }, "*");
    await act(async () => {
      window.dispatchEvent(new MessageEvent("message", { source: originalWindow, origin: "null", data: { channel: PLAYER_CHANNEL, nonce: originalNonce, type: "error", data: "late frame error" } }));
      await vi.advanceTimersByTimeAsync(60000);
    });
    expect(callbacks.onError).not.toHaveBeenCalled();
  });

  it("disposes the frame and removes its receiver on unmount", async () => {
    await render();
    const originalWindow = frame().contentWindow;
    const originalNonce = nonce();
    const post = outgoing();
    await receive("ready");
    post.mockClear();
    await act(async () => root.unmount());
    mounted = false;
    expect(post).toHaveBeenCalledExactlyOnceWith({ channel: PLAYER_CHANNEL, nonce: originalNonce, type: "dispose" }, "*");
    expect(handle.current).toBeNull();
    expect(container.querySelector("iframe")).toBeNull();
    await act(async () => window.dispatchEvent(new MessageEvent("message", { source: originalWindow, origin: "null", data: { channel: PLAYER_CHANNEL, nonce: originalNonce, type: "state", data: state() } })));
    expect(callbacks.onState).not.toHaveBeenCalled();
  });
});

describe("original player state bounds", () => {
  it("accepts finite endpoint values including zero duration and paused seeking", () => {
    expect(validPlayerState(state({ time: 0, duration: 0, aspect: .05, paused: true, seeking: true }))).toBe(true);
    expect(validPlayerState(state({ time: 604800, duration: 604800, aspect: 20 }))).toBe(true);
  });
});
