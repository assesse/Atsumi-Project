import { act, Profiler } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ApiResult } from "../../api/contracts";
import type { ReplayApi, ReplayMessage, ReplayPage, ReplaySession } from "../../api/replay";
import { RecordingReplay } from "./RecordingReplay";
import { RecordingReplayChat } from "./RecordingReplayChat";
import { PLAYER_CHANNEL, type OriginalPlayerState } from "./OriginalChzzkPlayer";
import { boundedReplayMessages, replayFocusableControls, replayMessageTime, replayVirtualRange, safeNicknameColor } from "./RecordingReplayModel";

vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => false, convertFileSrc: (file: string, protocol: string) => `http://${protocol}.localhost/${file}` }));

const ok = <T,>(data: T): ApiResult<T> => ({ ok: true, data });
const session: ReplaySession = { token: "a".repeat(64), recordingId: "synthetic-recording", title: "합성 다시보기", recordedAt: 1789272000000, durationSeconds: 120, mimeType: "video/mp4", chatStatus: "partial", indexState: "ready", syncQuality: "receive_time_approximate", manualOffsetSeconds: 0, warnings: [] };
const message = (sequence: number, patch: Partial<ReplayMessage> = {}): ReplayMessage => ({ sequence, sender: `시청자 ${sequence}`, text: `채팅 ${sequence}`, offsetSeconds: sequence, mediaTimeSeconds: sequence, receivedAt: 1_800_000_000_000 + sequence * 1000, serverTime: null, broadcastOffsetSeconds: null, syncQuality: "receive_time_approximate", ...patch });
const page = (generation: number, items: ReplayMessage[] = []): ReplayPage => ({ generation, items, previousCursor: null, nextCursor: null, indexState: "ready", syncQuality: "receive_time_approximate", warnings: [] });
const mockApi = () => ({
  open: vi.fn<ReplayApi["open"]>().mockResolvedValue(ok(session)),
  chatAt: vi.fn<ReplayApi["chatAt"]>().mockImplementation(async (_token, time, generation) => ok(page(generation, [message(Math.floor(time))]))),
  chatPage: vi.fn<ReplayApi["chatPage"]>().mockImplementation(async (_token, _cursor, generation) => ok(page(generation, [message(110)]))),
  chatSearch: vi.fn<NonNullable<ReplayApi["chatSearch"]>>().mockImplementation(async (_token, _query, _field, _cursor, generation) => ok(page(generation, [message(95, { sender: "찾은 닉네임", text: "저장 로그 검색 결과" })]))),
  close: vi.fn<ReplayApi["close"]>().mockResolvedValue(ok(undefined)),
  setOffset: vi.fn<ReplayApi["setOffset"]>().mockImplementation(async (_token, value) => ok(value)),
  timeline: vi.fn<ReplayApi["timeline"]>().mockResolvedValue(ok({ bucketSeconds: 30, indexState: "ready", viewerMetricStatus: "not_recorded", buckets: [{ startSeconds: 30, chatCount: 7, uniqueSenderCount: null, viewerCount: null }] })),
  openProfile: vi.fn<NonNullable<ReplayApi["openProfile"]>>().mockResolvedValue(ok(undefined)),
  mediaUrl: vi.fn<ReplayApi["mediaUrl"]>().mockImplementation((token) => `http://atsumi-replay.localhost/${encodeURIComponent(token)}`),
});
let root: Root, container: HTMLDivElement;
type PlayerEnvelope = { channel: string; nonce: string; type: string; data?: unknown };
let sent: PlayerEnvelope[], readyFrames: WeakSet<HTMLIFrameElement>;
const playerFrame = () => document.querySelector<HTMLIFrameElement>('iframe[title="CHZZK 원본 다시보기 플레이어"]')!;
const messages = (type: string) => sent.filter(message => message.type === type).map(message => message.data);
const chatShadow = () => document.querySelector<HTMLElement>(".recording-replay-chat")!.shadowRoot!;
const chatContent = () => chatShadow().querySelector<HTMLElement>(".original-chat-root")!;
const button = (label: string) => [...document.querySelectorAll<HTMLButtonElement>("button"), ...(document.querySelector<HTMLElement>(".recording-replay-chat")?.shadowRoot?.querySelectorAll<HTMLButtonElement>("button") ?? [])].find((entry) => entry.getAttribute("aria-label") === label || entry.textContent === label)!;
const deferred = <T,>() => { let resolve!: (value: T) => void; const promise = new Promise<T>((complete) => { resolve = complete; }); return { promise, resolve }; };
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  sent = []; readyFrames = new WeakSet();
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.useRealTimers(); });
function dispatchPlayer(type: string, data?: unknown, overrides: Partial<MessageEventInit> = {}) {
  const frame = playerFrame();
  window.dispatchEvent(new MessageEvent("message", { source: frame.contentWindow, origin: "null", data: { channel: PLAYER_CHANNEL, nonce: new URL(frame.src).hash.slice(1), type, data }, ...overrides }));
}
async function readyPlayer() {
  const frame = playerFrame();
  if (!frame || readyFrames.has(frame)) return;
  readyFrames.add(frame);
  vi.spyOn(frame.contentWindow!, "postMessage").mockImplementation((data: unknown) => { sent.push(data as PlayerEnvelope); });
  await act(async () => dispatchPlayer("ready"));
}
async function render(api: ReplayApi, privacyMode = false) { await act(async () => root.render(<RecordingReplay recordingId={session.recordingId} runtime="tauri" api={api} privacyMode={privacyMode} liveRecording onClose={vi.fn()} />)); await readyPlayer(); }
const playerState = (time: number, patch: Partial<OriginalPlayerState> = {}): OriginalPlayerState => ({ time, duration: 120, aspect: 16 / 9, paused: false, seeking: false, ...patch });
async function moveVideo(time: number) { await act(async () => { dispatchPlayer("state", playerState(time, { seeking: true })); dispatchPlayer("state", playerState(time)); }); }
async function openOffset() {
  await act(async () => button("채팅 메뉴").click());
  await act(async () => button("채팅 시간 조절").click());
}

describe("offline recording replay", () => {
  const rangeSession = (token: string, count = 2): ReplaySession => ({ ...session, token, durationSeconds: count * 60, recordingActive: true,
    parts: Array.from({ length: count }, (_, index) => ({ index, startSeconds: index * 60, durationSeconds: 60 })) });
  const mockRangeVideo = () => {
    vi.spyOn(HTMLMediaElement.prototype, "play").mockResolvedValue(undefined);
    vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => {});
  };
  it("keeps the current prefix open when there is no new range and releases only the extra lease", async () => {
    mockRangeVideo(); const api = mockApi();
    api.open.mockResolvedValueOnce(ok(rangeSession("first"))).mockResolvedValueOnce(ok(rangeSession("unchanged")));
    await render(api);
    await act(async () => button("새 저장 구간 불러오기").click());
    expect(api.close).toHaveBeenCalledExactlyOnceWith("unchanged");
    expect(document.querySelector("video")).toBeNull();
    expect(messages("init").at(-1)).toMatchObject({ url: "http://atsumi-replay.localhost/first", parts: rangeSession("first").parts });
    expect(document.body).toHaveTextContent("아직 새로 확정된 구간이 없습니다");
  });
  it("preserves the latest global position across a new saved-range snapshot and releases each lease once", async () => {
    mockRangeVideo(); const api = mockApi(); const next = deferred<ApiResult<ReplaySession>>();
    api.open.mockResolvedValueOnce(ok(rangeSession("first"))).mockReturnValueOnce(next.promise);
    await render(api);
    const frame = playerFrame();
    await moveVideo(35);
    await act(async () => button("새 저장 구간 불러오기").click());
    await moveVideo(40);
    await act(async () => next.resolve(ok(rangeSession("extended", 3))));
    expect(api.close).not.toHaveBeenCalled();
    await act(async () => dispatchPlayer("source", "http://atsumi-replay.localhost/extended"));
    expect(playerFrame()).toBe(frame);
    expect(messages("init").at(-1)).toMatchObject({ duration: 180, parts: rangeSession("extended", 3).parts });
    expect(api.close).toHaveBeenCalledExactlyOnceWith("first");
    await act(async () => root.render(<div />));
    expect(api.close.mock.calls.map(([token]) => token)).toEqual(["first", "extended"]);
  });
  it("closes a refresh result arriving after the player unmounts without closing it twice", async () => {
    mockRangeVideo(); const api = mockApi(); const next = deferred<ApiResult<ReplaySession>>();
    api.open.mockResolvedValueOnce(ok(rangeSession("first"))).mockReturnValueOnce(next.promise);
    await render(api); await act(async () => button("새 저장 구간 불러오기").click());
    await act(async () => root.render(<div />)); await act(async () => next.resolve(ok(rangeSession("late", 3))));
    expect(api.close.mock.calls.map(([token]) => token)).toEqual(["first", "late"]);
  });
  it("checks for another saved range when playback reaches the current tail", async () => {
    mockRangeVideo(); const api = mockApi();
    api.open.mockResolvedValueOnce(ok(rangeSession("first", 1))).mockResolvedValueOnce(ok(rangeSession("extended", 2)));
    await render(api); await act(async () => dispatchPlayer("tail"));
    await act(async () => dispatchPlayer("source", "http://atsumi-replay.localhost/extended"));
    expect(api.open).toHaveBeenCalledTimes(2); expect(api.close).toHaveBeenCalledExactlyOnceWith("first");
  });
  it("checks a still-recording tail periodically but never polls while watching older saved footage or in privacy mode", async () => {
    vi.useFakeTimers(); const api = mockApi(); api.open.mockResolvedValue(ok(rangeSession("first")));
    await render(api); await moveVideo(20);
    await act(async () => vi.advanceTimersByTimeAsync(15000)); expect(api.open).toHaveBeenCalledTimes(1);
    await moveVideo(120);
    await act(async () => vi.advanceTimersByTimeAsync(15000)); expect(api.open).toHaveBeenCalledTimes(2);
    await render(api, true);
    await act(async () => vi.advanceTimersByTimeAsync(30000)); expect(api.open).toHaveBeenCalledTimes(2);
  });
  it("forwards privacy to the opaque player without reloading its media or inspecting a foreign video", async () => {
    const api = mockApi(); await render(api);
    const frame = playerFrame();
    expect(messages("init")).toEqual([{ url: `http://atsumi-replay.localhost/${session.token}`, duration: 120, privacy: false, recording: { title: session.title, channelName: undefined, recordedAt: session.recordedAt } }]);
    expect(messages("metadata").at(-1)).toMatchObject({ title: session.title, recordedAt: session.recordedAt });
    expect(document.querySelector("h2.recording-replay-sr-only")).not.toBeNull();
    expect(document.querySelector(".recording-replay-heading")).toBeNull();
    expect(button("다시보기 닫기").closest(".recording-replay-visual")).not.toBeNull();
    await render(api, true);
    expect(messages("privacy").at(-1)).toBe(true);
    expect(document.querySelector(".recording-replay-body")).toHaveAttribute("hidden");
    await render(api, false);
    expect(messages("privacy").at(-1)).toBe(false);
    expect(messages("init")).toHaveLength(1);
    expect(playerFrame()).toBe(frame); expect(document.querySelector("video")).toBeNull();
    expect(api.open).toHaveBeenCalledTimes(1);
  });
  it("preserves the original player frame through chat settings and its wide-screen message", async () => {
    const api = mockApi(); await render(api);
    const frame = playerFrame();
    await act(async () => dispatchPlayer("state", playerState(4)));
    await openOffset();
    await act(async () => dispatchPlayer("wide"));
    expect(playerFrame()).toBe(frame);
    expect(messages("init")).toHaveLength(1);
    expect(messages("privacy")).not.toContain(true);
    expect(document.querySelector(".recording-replay")).toHaveClass("is-wide");
    expect(document.querySelector(".recording-replay-chat")).not.toHaveAttribute("hidden");
    expect(api.open).toHaveBeenCalledTimes(1);
  });

  it("passes a stored channel name and local raster into the original broadcast overlay", async () => {
    const api = mockApi();
    const image = "data:image/png;base64,iVBORw0KGgo=";
    api.open.mockResolvedValue(ok({ ...session, channelName: "저장 채널", channelProfileImage: image }));
    await render(api);
    expect(messages("metadata").at(-1)).toMatchObject({ title: session.title, channelName: "저장 채널", profileImage: image });
    expect(messages("init")).toHaveLength(1);
  });

  it("refits to actual picture dimensions without reloading or pausing the frame", async () => {
    const api = mockApi(); await render(api);
    const frame = playerFrame(), dialog = document.querySelector<HTMLElement>('.recording-replay')!;
    const originalWidth = dialog.style.width;
    await act(async () => dispatchPlayer("state", playerState(4, { aspect: 9 / 16 })));
    expect(dialog.style.width).not.toBe(originalWidth);
    const size = { width: dialog.style.width, height: dialog.style.height };
    await openOffset();
    expect({ width: dialog.style.width, height: dialog.style.height }).toEqual(size);
    expect(document.querySelector('.recording-replay-notices')?.contains(document.querySelector('.recording-replay-warning'))).toBe(true);
    expect(playerFrame()).toBe(frame);
    expect(messages("init")).toHaveLength(1);
    expect(messages("privacy")).not.toContain(true);
  });

  it("searches saved text and nicknames without seeking, pausing or reloading the player, and clears with Escape", async () => {
    vi.useFakeTimers();
    const api = mockApi(); await render(api);
    const frame = playerFrame();
    const input = chatShadow().querySelector<HTMLInputElement>('input[aria-label="검색"]')!;
    const enter = async (value: string) => { await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    }); await act(async () => vi.advanceTimersByTime(210)); };
    await enter("검색");
    expect(api.chatSearch).toHaveBeenLastCalledWith(session.token, "검색", "all", null, expect.any(Number));
    expect(chatContent().textContent).toContain("저장 로그 검색 결과");
    const calls = api.chatAt.mock.calls.length;
    await act(async () => dispatchPlayer("state", playerState(40)));
    expect(api.chatAt).toHaveBeenCalledTimes(calls);
    expect(chatShadow().querySelector('.replay-search__meta')).toBeNull();
    expect(chatShadow().querySelector('[aria-label="검색 범위"]')).toBeNull();
    for (const label of ["전체", "본문", "닉네임", "재생 위치"]) expect(button(label)).toBeUndefined();
    await enter("찾은 닉네임");
    expect(api.chatSearch).toHaveBeenLastCalledWith(session.token, "찾은 닉네임", "all", null, expect.any(Number));
    expect(messages("seek")).toHaveLength(0);
    expect(messages("privacy")).not.toContain(true);
    expect(messages("init")).toHaveLength(1);
    expect(playerFrame()).toBe(frame);
    await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, composed: true })));
    expect(input.value).toBe("");
    expect(document.querySelector('[role="dialog"]')).not.toBeNull();
    expect(api.chatAt.mock.calls.length).toBeGreaterThan(calls);
  });

  it("does not replace a corrected timeline with an older in-flight response", async () => {
    const api = mockApi();
    const old = deferred<Awaited<ReturnType<ReplayApi["timeline"]>>>();
    api.timeline.mockReturnValueOnce(old.promise);
    await render(api);
    await openOffset();
    await act(async () => chatShadow().querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    const metrics = messages("metrics").at(-1);
    expect(api.timeline).toHaveBeenCalledTimes(2);
    await act(async () => old.resolve(ok({ bucketSeconds: 30, indexState: "ready", viewerMetricStatus: "not_recorded", buckets: [{ startSeconds: 0, chatCount: 77, uniqueSenderCount: 77, viewerCount: null }] })));
    expect(messages("metrics").at(-1)).toEqual(metrics);
    expect(metrics).toMatchObject({ participantCounts: false, buckets: [{ chatCount: 7 }] });
  });

  it("preserves safe body and nickname colors and opens only the saved profile after a click", async () => {
    const api = mockApi();
    api.chatAt.mockImplementation(async (_token, _time, generation) => ok(page(generation, [message(1, { rich: { nicknameColor: "#11ee77", textColor: "#f0c080", profileUrl: `https://chzzk.naver.com/${"b".repeat(32)}`, badges: [], emojis: [] } })])));
    await render(api);
    const name = button("시청자 1 프로필 열기");
    expect([...name.querySelectorAll<HTMLElement>("[style]")].some(item => item.style.color === "rgb(17, 238, 119)")).toBe(true);
    expect(chatShadow().querySelector<HTMLElement>('[data-sequence="1"]')!.style.getPropertyValue("--replay-text-color")).toBe("#f0c080");
    expect(api.openProfile).not.toHaveBeenCalled();
    await act(async () => name.click());
    expect(api.openProfile).toHaveBeenCalledExactlyOnceWith(session.token, 1);
    api.chatAt.mockImplementation(async (_token, _time, generation) => ok(page(generation, [message(2, { rich: { textColor: "url(https://bad.test)", profileUrl: `https://chzzk.naver.com.evil.test/${"b".repeat(32)}`, badges: [], emojis: [] } })])));
    await moveVideo(2);
    expect(button("시청자 2 프로필 열기")).toBeUndefined();
    expect(chatShadow().querySelector<HTMLElement>('[data-sequence="2"]')!.style.getPropertyValue("--replay-text-color")).toBe("");
    expect(api.openProfile).toHaveBeenCalledTimes(1);
  });
  it("opens the actual isolated player with a token-local source and right chat, preserving live activity", async () => {
    const api = mockApi(); await render(api);
    expect(api.open).toHaveBeenCalledExactlyOnceWith(session.recordingId);
    const frame = playerFrame();
    expect(frame).toHaveAttribute("sandbox", "allow-scripts");
    expect(frame).toHaveAttribute("referrerpolicy", "no-referrer");
    expect(frame.src).toMatch(/^http:\/\/atsumi-player\.localhost\/frame\.html#[a-f0-9]{32}$/);
    expect(frame.src).not.toContain(session.token);
    expect(messages("init")).toEqual([{ url: `http://atsumi-replay.localhost/${session.token}`, duration: 120, privacy: false, recording: { title: session.title, channelName: undefined, recordedAt: session.recordedAt } }]);
    expect(document.querySelector("video")).toBeNull();
    expect(document.querySelector('[role="dialog"]')).toHaveAttribute("aria-modal", "true");
    expect(document.querySelector('[data-native-overlay="true"]')).not.toBeNull();
    expect(document.body).toHaveTextContent("다른 녹화가 진행 중입니다");
    for (const text of ["로컬 저장본", "읽기 전용", "채팅 다시보기", "동기화 근사"]) expect(document.body).not.toHaveTextContent(text);
    expect(document.querySelector('[aria-label="채팅 글자 크기"]')).toBeNull();
    expect(document.querySelector(".recording-replay-controls")).toBeNull();
    expect(frame.closest(".recording-replay-stage")).not.toBeNull();
    expect(chatContent()).toHaveTextContent("채팅 일부 누락 가능");
    await moveVideo(30);
    expect(chatContent()).toHaveTextContent("채팅 30");
    await moveVideo(4);
    expect(chatContent()).toHaveTextContent("채팅 4");
    expect(chatContent()).not.toHaveTextContent("채팅 30");
    const calls = api.chatAt.mock.calls.length;
    await act(async () => dispatchPlayer("state", playerState(4, { paused: true })));
    expect(api.chatAt).toHaveBeenCalledTimes(calls);
    await act(async () => root.unmount());
    expect(api.close).toHaveBeenCalledExactlyOnceWith(session.token);
    expect(messages("dispose")).toHaveLength(1);
    root = createRoot(container);
  });

  it("invalidates an older chat response when the video seeks backwards", async () => {
    const api = mockApi(); await render(api);
    const old = deferred<ApiResult<ReplayPage>>();
    let oldGeneration = 0;
    api.chatAt.mockImplementationOnce((_token, _time, generation) => { oldGeneration = generation; return old.promise; });
    await moveVideo(90);
    await moveVideo(8);
    expect(chatContent()).toHaveTextContent("채팅 8");
    await act(async () => old.resolve(ok(page(oldGeneration, [message(90)]))));
    expect(chatContent()).not.toHaveTextContent("채팅 90");
    expect(chatContent()).toHaveTextContent("채팅 8");
  });

  it("ignores forged player messages before they can move chat or change the dialog", async () => {
    const api = mockApi(); await render(api);
    const frame = playerFrame(), dialog = document.querySelector<HTMLDivElement>('[role="dialog"]')!;
    dialog.requestFullscreen = vi.fn().mockResolvedValue(undefined);
    const envelope = { channel: PLAYER_CHANNEL, nonce: new URL(frame.src).hash.slice(1), type: "state", data: playerState(80) };
    const calls = api.chatAt.mock.calls.length;
    for (const overrides of [
      { source: window }, { origin: location.origin },
      { data: { ...envelope, nonce: "forged" } }, { data: { ...envelope, channel: "foreign-player" } },
      { data: { ...envelope, data: playerState(Number.NaN) } },
      { data: { ...envelope, data: playerState(604801) } },
      { data: { ...envelope, data: { ...playerState(80), paused: "false" } } },
    ]) await act(async () => dispatchPlayer("state", playerState(80), overrides));
    await act(async () => { dispatchPlayer("fullscreen", undefined, { source: window }); dispatchPlayer("wide", undefined, { origin: location.origin }); });
    expect(api.chatAt).toHaveBeenCalledTimes(calls);
    expect(chatContent()).not.toHaveTextContent("채팅 80");
    expect(dialog.requestFullscreen).not.toHaveBeenCalled();
    expect(dialog).not.toHaveClass("is-wide");
    await moveVideo(8);
    expect(chatContent()).toHaveTextContent("채팅 8");
  });

  it("routes keyboard actions into the scoped player and clamps seeks to the saved duration", async () => {
    const api = mockApi(); await render(api);
    const dialog = document.querySelector<HTMLDivElement>('[role="dialog"]')!;
    const key = async (value: string) => act(async () => dialog.dispatchEvent(new KeyboardEvent("keydown", { key: value, bubbles: true })));
    await key("ArrowLeft"); expect(messages("seek").at(-1)).toBe(0);
    await moveVideo(119); await key("ArrowRight"); expect(messages("seek").at(-1)).toBe(120);
    await key("k"); await key("m");
    expect(messages("toggle")).toHaveLength(1); expect(messages("mute")).toHaveLength(1);
    await render(api, true); await key("k");
    expect(messages("toggle")).toHaveLength(1);
  });

  it("rejects a previous frame and pending offset after switching recording sessions", async () => {
    const api = mockApi(); await render(api);
    const oldFrame = playerFrame(), oldWindow = oldFrame.contentWindow, oldNonce = new URL(oldFrame.src).hash.slice(1);
    const saving = deferred<Awaited<ReturnType<ReplayApi["setOffset"]>>>(); api.setOffset.mockReturnValueOnce(saving.promise);
    await openOffset();
    await act(async () => chatShadow().querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    const nextSession = { ...session, token: "b".repeat(64), recordingId: "second-recording", manualOffsetSeconds: 7 };
    api.open.mockResolvedValueOnce(ok(nextSession));
    await act(async () => root.render(<RecordingReplay recordingId={nextSession.recordingId} runtime="tauri" api={api} privacyMode={false} liveRecording={false} onClose={vi.fn()} />));
    await readyPlayer();
    expect(playerFrame()).not.toBe(oldFrame);
    expect(api.close).toHaveBeenCalledExactlyOnceWith(session.token);
    const calls = api.chatAt.mock.calls.length;
    await act(async () => window.dispatchEvent(new MessageEvent("message", { source: oldWindow, origin: "null", data: { channel: PLAYER_CHANNEL, nonce: oldNonce, type: "state", data: playerState(99) } })));
    await act(async () => saving.resolve(ok(99)));
    expect(api.chatAt).toHaveBeenCalledTimes(calls);
    expect(api.chatAt.mock.calls.at(-1)?.slice(0, 2)).toEqual([nextSession.token, 0]);
    await openOffset();
    expect(chatShadow().querySelector<HTMLInputElement>(".recording-replay-sync input")!.value).toBe("7");
    expect(messages("init").at(-1)).toMatchObject({ url: `http://atsumi-replay.localhost/${nextSession.token}` });
  });

  it("avoids repeated UI commits for frame updates within the same tenth of a second", async () => {
    const api = mockApi(), commit = vi.fn();
    await act(async () => root.render(<Profiler id="replay" onRender={commit}><RecordingReplay recordingId={session.recordingId} runtime="tauri" api={api} privacyMode={false} liveRecording={false} onClose={vi.fn()} /></Profiler>));
    await readyPlayer();
    await act(async () => dispatchPlayer("state", playerState(.11)));
    commit.mockClear();
    for (const time of [.12, .14, .17, .19]) {
      await act(async () => dispatchPlayer("state", playerState(time)));
    }
    expect(commit).not.toHaveBeenCalled();
    expect(api.chatAt).toHaveBeenCalledTimes(1);
  });

  it("polls index readiness while paused without advancing chat time and updates final sync quality", async () => {
    vi.useFakeTimers();
    const api = mockApi();
    api.open.mockResolvedValue(ok({ ...session, indexState: "building" }));
    api.chatAt.mockImplementationOnce(async (_token, _time, generation) => ok({ ...page(generation), indexState: "building" }));
    api.chatAt.mockImplementation(async (_token, _time, generation) => ok({ ...page(generation, [message(0)]), syncQuality: "observed_media" }));
    await render(api);
    expect(chatContent()).toHaveTextContent("채팅 기록 준비 중");
    await act(async () => vi.advanceTimersByTimeAsync(1000));
    expect(chatContent()).not.toHaveTextContent("채팅 기록 준비 중");
    expect(chatContent()).not.toHaveTextContent("영상 시각 기준");
    expect(api.chatAt.mock.calls.map((call) => call[1])).toEqual([0, 0]);
    await act(async () => vi.advanceTimersByTimeAsync(60_000));
    expect(api.chatAt).toHaveBeenCalledTimes(2);
    expect(messages("seek")).toEqual([]);
    expect(messages("init")).toHaveLength(1);
  });

  it("stops following on manual scroll, pages the entire log and resumes at video time", async () => {
    const api = mockApi(); await render(api); await moveVideo(20);
    const scroll = chatShadow().querySelector<HTMLElement>(".recording-replay-chat-scroll")!;
    Object.defineProperties(scroll, { scrollHeight: { configurable: true, value: 2000 }, clientHeight: { configurable: true, value: 400 } });
    await act(async () => { scroll.scrollTop = -50; scroll.dispatchEvent(new Event("scroll", { bubbles: true })); });
    expect(button("↓ 현재 재생 위치로")).toBeDefined();
    await act(async () => button("채팅 메뉴").click());
    await act(async () => button("전체 로그").click());
    expect(api.chatPage).toHaveBeenCalled();
    expect(chatContent()).toHaveTextContent("채팅 110");
    expect(chatShadow().querySelector('[aria-label="전체 채팅 로그 페이지"]')).not.toBeNull();
    await act(async () => button("↓ 현재 재생 위치로").click());
    expect(chatContent()).toHaveTextContent("채팅 20");
    expect(chatContent()).not.toHaveTextContent("채팅 110");
  });

  it("saves recording-specific offset, guards typing shortcuts and keeps the session through privacy", async () => {
    const api = mockApi(); await render(api);
    await openOffset();
    const offset = chatShadow().querySelector<HTMLInputElement>('.recording-replay-sync input')!;
    await act(async () => { const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!; setter.call(offset, "2.5"); offset.dispatchEvent(new Event("input", { bubbles: true })); });
    await act(async () => offset.dispatchEvent(new KeyboardEvent("keydown", { key: " ", bubbles: true })));
    expect(messages("toggle")).toEqual([]);
    await act(async () => chatShadow().querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(api.setOffset).toHaveBeenCalledExactlyOnceWith(session.token, 2.5);
    expect(offset.value).toBe("2.5");
    expect(chatShadow().querySelector(".recording-replay-sync")).toBeNull();
    expect(button("채팅 메뉴")).toHaveAttribute("aria-expanded", "false");
    expect(chatShadow().activeElement).toBe(button("채팅 메뉴"));
    await openOffset();
    expect(chatShadow().querySelector<HTMLInputElement>('.recording-replay-sync input')?.value).toBe("2.5");
    expect(document.body).not.toHaveTextContent("양수는 채팅을 더 늦게 표시");
    const frame = playerFrame();
    await render(api, true);
    expect(document.querySelector(".recording-replay-body")).toHaveAttribute("hidden");
    expect(messages("privacy").at(-1)).toBe(true);
    await render(api, false);
    expect(playerFrame()).toBe(frame);
    expect(messages("init")).toHaveLength(1);
    expect(api.open).toHaveBeenCalledTimes(1);
  });

  it("keeps chat time correction open when saving fails so the user can retry", async () => {
    const api = mockApi(); api.setOffset.mockResolvedValueOnce({ ok: false, error: { code: "SAVE_FAILED", message: "저장 실패", retryable: true } });
    await render(api);
    await openOffset();
    await act(async () => chatShadow().querySelector(".recording-replay-sync")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(chatShadow().querySelector(".recording-replay-sync")).not.toBeNull();
    expect(chatContent()).toHaveTextContent("저장 실패");
    expect(button("적용")).toBeEnabled();
    await act(async () => button("적용").click());
    expect(chatShadow().querySelector(".recording-replay-sync")).toBeNull();
  });

  it("does not route keys from the original chat's shadow controls to the video", async () => {
    const api = mockApi(); await render(api);
    await act(async () => button("채팅 메뉴").click());
    const select = chatShadow().querySelector<HTMLButtonElement>('[role="radio"][aria-checked="true"]')!;
    await act(async () => select.focus());
    const prior = sent.length;
    for (const key of [" ", "k", "m", "ArrowLeft", "ArrowRight", "f", "t"]) {
      await act(async () => select.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, composed: true, cancelable: true })));
    }
    expect(sent).toHaveLength(prior);
    expect(document.querySelector(".recording-replay")).not.toHaveClass("is-wide");
    await act(async () => select.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, composed: true, cancelable: true })));
    expect(button("채팅 메뉴")).toHaveAttribute("aria-expanded", "false");
    expect(chatShadow().activeElement).toBe(button("채팅 메뉴"));
    expect(messages("privacy")).not.toContain(true);
  });

  it("keeps Tab traversal inside the dialog across the shadow boundary and skips private chat", async () => {
    const api = mockApi(); await render(api);
    const dialog = document.querySelector<HTMLDivElement>('[role="dialog"]')!;
    const controls = replayFocusableControls(dialog), first = controls[0]!, last = controls.at(-1)!;
    expect(controls).toContain(button("채팅 메뉴"));
    expect(last.getRootNode()).toBe(chatShadow());
    await act(async () => last.focus());
    const forward = new KeyboardEvent("keydown", { key: "Tab", bubbles: true, composed: true, cancelable: true });
    await act(async () => last.dispatchEvent(forward));
    expect(forward.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(first);
    const backward = new KeyboardEvent("keydown", { key: "Tab", shiftKey: true, bubbles: true, composed: true, cancelable: true });
    await act(async () => first.dispatchEvent(backward));
    expect(backward.defaultPrevented).toBe(true);
    expect(chatShadow().activeElement).toBe(last);
    await render(api, true);
    expect(replayFocusableControls(dialog).some(control => control.getRootNode() === chatShadow())).toBe(false);
    expect(document.querySelector(".recording-replay-body")).toHaveAttribute("hidden");
  });

  it("closes a session that finishes opening after the dialog was removed", async () => {
    const api = mockApi(), open = deferred<ApiResult<ReplaySession>>(); api.open.mockReturnValue(open.promise);
    await render(api); await act(async () => root.unmount()); root = createRoot(container);
    await act(async () => open.resolve(ok(session)));
    expect(api.close).toHaveBeenCalledExactlyOnceWith(session.token);
  });

  it("the original player's fullscreen request targets the container that owns both frame and chat", async () => {
    const api = mockApi(); await render(api);
    const dialog = document.querySelector<HTMLDivElement>('[role="dialog"]')!;
    dialog.requestFullscreen = vi.fn().mockResolvedValue(undefined);
    await act(async () => dispatchPlayer("fullscreen"));
    expect(dialog.requestFullscreen).toHaveBeenCalledOnce();
    expect(dialog.querySelector("iframe")).toBe(playerFrame());
    expect(dialog.querySelector("aside")).not.toBeNull();
  });

  it("renders malicious saved text and offline decorations safely without remote image requests", async () => {
    const api = mockApi();
    const rich = { nicknameColor: "url(https://bad.test/track)", badges: [{ kind: "subscription", title: "구독 12개월", imageUrl: "https://bad.test/badge" }], emojis: [{ id: "hello", imageUrl: "https://bad.test/emoji" }] };
    api.chatAt.mockImplementation(async (_token, _time, generation) => ok(page(generation, [message(1, { sender: "<script>alert(1)</script>", text: '<img src="https://bad.test/leak"> {:hello:}', rich })])));
    await render(api);
    expect(chatContent()).toHaveTextContent('<img src="https://bad.test/leak"> {:hello:}');
    expect(chatContent()).toHaveTextContent("구독 12개월");
    expect(chatShadow().querySelector("img,script,iframe")).toBeNull();
    expect(document.querySelectorAll("iframe")).toHaveLength(1);
    expect([...chatShadow().querySelectorAll<HTMLElement>('[data-sequence="1"] [style]')].every(item => !item.getAttribute("style")?.includes("bad.test"))).toBe(true);
    const localAsset = "b".repeat(64);
    api.chatAt.mockImplementation(async (_token, _time, generation) => ok(page(generation, [message(2, { rich, assetIds: { "https://bad.test/badge": localAsset } })])));
    await moveVideo(2);
    const image = chatShadow().querySelector("img")!;
    expect(image.src).toBe(`http://atsumi-replay.localhost/${session.token}/asset/${localAsset}`);
    await act(async () => image.dispatchEvent(new Event("error")));
    expect(chatShadow().querySelector("img")).toBeNull();
    expect(chatContent()).toHaveTextContent("구독 12개월");
  });

  it("uses bounded variable-height virtualization while preserving unknown uptime", async () => {
    const api = mockApi(); api.chatAt.mockImplementation(async (_token, _time, generation) => ok(page(generation, Array.from({ length: 5000 }, (_, index) => message(index)))));
    await act(async () => root.render(<RecordingReplayChat api={api} session={session} time={6000} seekVersion={0} onSeek={vi.fn()} onIndexReady={vi.fn()} />));
    expect(chatShadow().querySelectorAll(".recording-replay-message").length).toBeLessThanOrEqual(80);
    expect(chatContent()).toHaveTextContent("채팅 4999");
    expect(chatContent()).not.toHaveTextContent("채팅 4799");
    await act(async () => button("채팅 메뉴").click());
    await act(async () => button("방송 업타임").click());
    expect(chatShadow().querySelector(".recording-replay-chat-time")).toHaveTextContent("—");
    expect(api.chatAt).toHaveBeenCalledTimes(1);
    expect(replayMessageTime(message(1), "hidden")).toBeNull();
    expect(safeNicknameColor("#67EfB3")).toBe("#67EfB3");
    expect(boundedReplayMessages([message(1), message(1), message(2, { mediaTimeSeconds: NaN })])).toHaveLength(1);
    const range = replayVirtualRange([20, 300, 80, ...Array<number>(197).fill(44)], 2800, 400, false);
    expect(range.start).toBeGreaterThan(0); expect(range.end - range.start).toBeLessThanOrEqual(80);
    expect(range.before + range.after).toBeGreaterThan(5000);
  });
});
