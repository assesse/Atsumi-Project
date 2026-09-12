import { act, Profiler } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ApiResult } from "../../api/contracts";
import type { ReplayApi, ReplayMessage, ReplayPage, ReplaySession } from "../../api/replay";
import { RecordingReplay } from "./RecordingReplay";
import { RecordingReplayChat } from "./RecordingReplayChat";
import { boundedReplayMessages, replayMessageTime, replayVirtualRange, safeNicknameColor } from "./RecordingReplayModel";

const ok = <T,>(data: T): ApiResult<T> => ({ ok: true, data });
const session: ReplaySession = { token: "a".repeat(64), recordingId: "synthetic-recording", title: "합성 다시보기", durationSeconds: 120, mimeType: "video/mp4", chatStatus: "partial", indexState: "ready", syncQuality: "receive_time_approximate", manualOffsetSeconds: 0, warnings: [] };
const message = (sequence: number, patch: Partial<ReplayMessage> = {}): ReplayMessage => ({ sequence, sender: `시청자 ${sequence}`, text: `채팅 ${sequence}`, offsetSeconds: sequence, mediaTimeSeconds: sequence, receivedAt: 1_800_000_000_000 + sequence * 1000, serverTime: null, broadcastOffsetSeconds: null, syncQuality: "receive_time_approximate", ...patch });
const page = (generation: number, items: ReplayMessage[] = []): ReplayPage => ({ generation, items, previousCursor: null, nextCursor: null, indexState: "ready", syncQuality: "receive_time_approximate", warnings: [] });
const mockApi = () => ({
  open: vi.fn<ReplayApi["open"]>().mockResolvedValue(ok(session)),
  chatAt: vi.fn<ReplayApi["chatAt"]>().mockImplementation(async (_token, time, generation) => ok(page(generation, [message(Math.floor(time))]))),
  chatPage: vi.fn<ReplayApi["chatPage"]>().mockImplementation(async (_token, _cursor, generation) => ok(page(generation, [message(110)]))),
  close: vi.fn<ReplayApi["close"]>().mockResolvedValue(ok(undefined)),
  setOffset: vi.fn<ReplayApi["setOffset"]>().mockImplementation(async (_token, value) => ok(value)),
  timeline: vi.fn<ReplayApi["timeline"]>().mockResolvedValue(ok({ bucketSeconds: 30, indexState: "ready", viewerMetricStatus: "not_recorded", buckets: [{ startSeconds: 30, chatCount: 7, uniqueSenderCount: null, viewerCount: null }] })),
  mediaUrl: vi.fn<ReplayApi["mediaUrl"]>().mockImplementation((token) => `http://atsumi-replay.localhost/${encodeURIComponent(token)}`),
});
let root: Root, container: HTMLDivElement;
const button = (label: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find((entry) => entry.getAttribute("aria-label") === label || entry.textContent === label)!;
const deferred = <T,>() => { let resolve!: (value: T) => void; const promise = new Promise<T>((complete) => { resolve = complete; }); return { promise, resolve }; };
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.spyOn(HTMLMediaElement.prototype, "play").mockImplementation(async function(this: HTMLMediaElement) { Object.defineProperty(this, "paused", { configurable: true, value: false }); this.dispatchEvent(new Event("play")); });
  vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(function(this: HTMLMediaElement) { Object.defineProperty(this, "paused", { configurable: true, value: true }); this.dispatchEvent(new Event("pause")); });
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.useRealTimers(); });
async function render(api: ReplayApi, privacyMode = false) { await act(async () => root.render(<RecordingReplay recordingId={session.recordingId} runtime="tauri" api={api} privacyMode={privacyMode} liveRecording onClose={vi.fn()} />)); }
async function moveVideo(time: number) { await act(async () => { const video = document.querySelector("video")!; video.currentTime = time; video.dispatchEvent(new Event("seeking")); video.dispatchEvent(new Event("seeked")); video.dispatchEvent(new Event("timeupdate")); }); }

describe("offline recording replay", () => {
  it("opens a token-local video with custom controls and right chat, preserving live activity", async () => {
    const api = mockApi(); await render(api);
    expect(api.open).toHaveBeenCalledExactlyOnceWith(session.recordingId);
    const video = document.querySelector("video")!;
    expect(video).not.toHaveAttribute("controls");
    expect(video).toHaveAttribute("src", `http://atsumi-replay.localhost/${session.token}`);
    expect(document.querySelector("iframe")).toBeNull();
    expect(document.querySelector('[role="dialog"]')).toHaveAttribute("aria-modal", "true");
    expect(document.querySelector('[data-native-overlay="true"]')).not.toBeNull();
    expect(document.body).toHaveTextContent("다른 녹화가 진행 중입니다");
    expect(document.body).toHaveTextContent("동기화 근사");
    expect(document.querySelector(".recording-replay-quality")).toHaveAttribute("title", expect.stringContaining("수신 시각 기준·동기화 근사"));
    expect(document.querySelector(".recording-replay-heading .recording-replay-offline")).toHaveAttribute("title", expect.stringContaining("시청자 수: 기록 없음"));
    expect(document.body).toHaveTextContent("채팅 일부 누락 가능");
    await moveVideo(30);
    expect(document.body).toHaveTextContent("채팅 30");
    await act(async () => { const speed = document.querySelector<HTMLSelectElement>('[aria-label="재생 속도"]')!; speed.value = "2"; speed.dispatchEvent(new Event("change", { bubbles: true })); });
    expect(video.playbackRate).toBe(2);
    await moveVideo(4);
    expect(document.body).toHaveTextContent("채팅 4");
    expect(document.body).not.toHaveTextContent("채팅 30");
    const calls = api.chatAt.mock.calls.length;
    await act(async () => { video.dispatchEvent(new Event("pause")); await new Promise((resolve) => setTimeout(resolve, 20)); });
    expect(api.chatAt).toHaveBeenCalledTimes(calls);
    await act(async () => root.unmount());
    expect(api.close).toHaveBeenCalledExactlyOnceWith(session.token);
    root = createRoot(container);
  });

  it("invalidates an older chat response when the video seeks backwards", async () => {
    const api = mockApi(); await render(api);
    const old = deferred<ApiResult<ReplayPage>>();
    let oldGeneration = 0;
    api.chatAt.mockImplementationOnce((_token, _time, generation) => { oldGeneration = generation; return old.promise; });
    await moveVideo(90);
    await moveVideo(8);
    expect(document.body).toHaveTextContent("채팅 8");
    await act(async () => old.resolve(ok(page(oldGeneration, [message(90)]))));
    expect(document.body).not.toHaveTextContent("채팅 90");
    expect(document.body).toHaveTextContent("채팅 8");
  });

  it("avoids repeated UI commits for frame updates within the same tenth of a second", async () => {
    const api = mockApi(), commit = vi.fn();
    await act(async () => root.render(<Profiler id="replay" onRender={commit}><RecordingReplay recordingId={session.recordingId} runtime="tauri" api={api} privacyMode={false} liveRecording={false} onClose={vi.fn()} /></Profiler>));
    const video = document.querySelector("video")!;
    await act(async () => { video.currentTime = .11; video.dispatchEvent(new Event("timeupdate")); });
    commit.mockClear();
    for (const time of [.12, .14, .17, .19]) {
      await act(async () => { video.currentTime = time; video.dispatchEvent(new Event("timeupdate")); });
    }
    expect(commit).not.toHaveBeenCalled();
    expect(video.currentTime).toBe(.19);
    expect(api.chatAt).toHaveBeenCalledTimes(1);
  });

  it("polls index readiness while paused without advancing chat time and updates final sync quality", async () => {
    vi.useFakeTimers();
    const api = mockApi();
    api.open.mockResolvedValue(ok({ ...session, indexState: "building" }));
    api.chatAt.mockImplementationOnce(async (_token, _time, generation) => ok({ ...page(generation), indexState: "building" }));
    api.chatAt.mockImplementation(async (_token, _time, generation) => ok({ ...page(generation, [message(0)]), syncQuality: "observed_media" }));
    await render(api);
    expect(document.body).toHaveTextContent("채팅 기록 준비 중");
    await act(async () => vi.advanceTimersByTimeAsync(1000));
    expect(document.body).not.toHaveTextContent("채팅 기록 준비 중");
    expect(document.body).toHaveTextContent("영상 시각 기준");
    expect(api.chatAt.mock.calls.map((call) => call[1])).toEqual([0, 0]);
    await act(async () => vi.advanceTimersByTimeAsync(60_000));
    expect(api.chatAt).toHaveBeenCalledTimes(2);
    expect(document.querySelector("video")!.currentTime).toBe(0);
  });

  it("stops following on manual scroll, pages the entire log and resumes at video time", async () => {
    const api = mockApi(); await render(api); await moveVideo(20);
    const scroll = document.querySelector<HTMLElement>(".recording-replay-chat-scroll")!;
    Object.defineProperties(scroll, { scrollHeight: { configurable: true, value: 2000 }, clientHeight: { configurable: true, value: 400 } });
    await act(async () => { scroll.scrollTop = 50; scroll.dispatchEvent(new Event("scroll", { bubbles: true })); });
    expect(button("↓ 현재 재생 위치로")).toBeDefined();
    await act(async () => button("전체 로그").click());
    expect(api.chatPage).toHaveBeenCalled();
    expect(document.body).toHaveTextContent("채팅 110");
    expect(document.body).toHaveTextContent("한 번에 최대 200개");
    await act(async () => button("↓ 현재 재생 위치로").click());
    expect(document.body).toHaveTextContent("채팅 20");
    expect(document.body).not.toHaveTextContent("채팅 110");
  });

  it("saves recording-specific offset, guards typing shortcuts and keeps the session through privacy", async () => {
    const api = mockApi(); await render(api);
    await act(async () => button("채팅 동기화 설정").click());
    const offset = document.querySelector<HTMLInputElement>('.recording-replay-sync input')!;
    await act(async () => { const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!; setter.call(offset, "2.5"); offset.dispatchEvent(new Event("input", { bubbles: true })); });
    await act(async () => offset.dispatchEvent(new KeyboardEvent("keydown", { key: " ", bubbles: true })));
    expect(HTMLMediaElement.prototype.play).not.toHaveBeenCalled();
    await act(async () => document.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(api.setOffset).toHaveBeenCalledExactlyOnceWith(session.token, 2.5);
    expect(document.querySelector(".recording-replay-quality")).toHaveAttribute("title", expect.stringContaining("채팅 보정 +2.5초"));
    expect(document.body).toHaveTextContent("양수는 채팅을 더 늦게 표시");
    const video = document.querySelector("video");
    await render(api, true);
    expect(document.querySelector(".recording-replay-body")).toHaveAttribute("hidden");
    expect(HTMLMediaElement.prototype.pause).toHaveBeenCalled();
    await render(api, false);
    expect(document.querySelector("video")).toBe(video);
    expect(api.open).toHaveBeenCalledTimes(1);
  });

  it("closes a session that finishes opening after the dialog was removed", async () => {
    const api = mockApi(), open = deferred<ApiResult<ReplaySession>>(); api.open.mockReturnValue(open.promise);
    await render(api); await act(async () => root.unmount()); root = createRoot(container);
    await act(async () => open.resolve(ok(session)));
    expect(api.close).toHaveBeenCalledExactlyOnceWith(session.token);
  });

  it("fullscreen targets the container that owns both video and chat", async () => {
    const api = mockApi(); await render(api);
    const dialog = document.querySelector<HTMLDivElement>('[role="dialog"]')!;
    dialog.requestFullscreen = vi.fn().mockResolvedValue(undefined);
    await act(async () => button("영상과 채팅 전체화면").click());
    expect(dialog.requestFullscreen).toHaveBeenCalledOnce();
    expect(dialog.querySelector("video")).not.toBeNull();
    expect(dialog.querySelector("aside")).not.toBeNull();
  });

  it("renders malicious saved text and offline decorations safely without remote image requests", async () => {
    const api = mockApi();
    const rich = { nicknameColor: "url(https://bad.test/track)", badges: [{ kind: "subscription", title: "구독 12개월", imageUrl: "https://bad.test/badge" }], emojis: [{ id: "hello", imageUrl: "https://bad.test/emoji" }] };
    api.chatAt.mockImplementation(async (_token, _time, generation) => ok(page(generation, [message(1, { sender: "<script>alert(1)</script>", text: '<img src="https://bad.test/leak"> {:hello:}', rich })])));
    await render(api);
    expect(document.body).toHaveTextContent('<img src="https://bad.test/leak"> {:hello:}');
    expect(document.body).toHaveTextContent("구독 12개월");
    expect(document.querySelector("img,script,iframe")).toBeNull();
    expect(document.querySelector('.recording-replay-message > strong')).not.toHaveAttribute("style");
    const localAsset = "b".repeat(64);
    api.chatAt.mockImplementation(async (_token, _time, generation) => ok(page(generation, [message(2, { rich, assetIds: { "https://bad.test/badge": localAsset } })])));
    await moveVideo(2);
    const image = document.querySelector("img")!;
    expect(image.src).toBe(`http://atsumi-replay.localhost/${session.token}/asset/${localAsset}`);
    await act(async () => image.dispatchEvent(new Event("error")));
    expect(document.querySelector("img")).toBeNull();
    expect(document.body).toHaveTextContent("구독 12개월");
  });

  it("uses bounded variable-height virtualization while preserving unknown uptime", async () => {
    const api = mockApi(); api.chatAt.mockImplementation(async (_token, _time, generation) => ok(page(generation, Array.from({ length: 5000 }, (_, index) => message(index)))));
    await act(async () => root.render(<RecordingReplayChat api={api} session={session} time={6000} seekVersion={0} onSeek={vi.fn()} onIndexReady={vi.fn()} />));
    expect(container.querySelectorAll(".recording-replay-message").length).toBeLessThanOrEqual(80);
    expect(container).toHaveTextContent("채팅 4999");
    expect(container).not.toHaveTextContent("채팅 4799");
    await act(async () => { const mode = container.querySelector<HTMLSelectElement>('[aria-label="채팅 시각 표시"]')!; mode.value = "broadcast"; mode.dispatchEvent(new Event("change", { bubbles: true })); });
    expect(container.querySelector(".recording-replay-chat-time")).toHaveTextContent("—");
    expect(api.chatAt).toHaveBeenCalledTimes(1);
    expect(replayMessageTime(message(1), "hidden")).toBeNull();
    expect(safeNicknameColor("#67EfB3")).toBe("#67EfB3");
    expect(boundedReplayMessages([message(1), message(1), message(2, { mediaTimeSeconds: NaN })])).toHaveLength(1);
    const range = replayVirtualRange([20, 300, 80, ...Array<number>(197).fill(44)], 2800, 400, false);
    expect(range.start).toBeGreaterThan(0); expect(range.end - range.start).toBeLessThanOrEqual(80);
    expect(range.before + range.after).toBeGreaterThan(5000);
  });
});
