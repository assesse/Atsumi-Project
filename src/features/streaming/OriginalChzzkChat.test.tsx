import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ApiResult } from "../../api/contracts";
import type { ReplayApi, ReplayMessage, ReplayPage, ReplaySession } from "../../api/replay";
import { RecordingReplayChat } from "./RecordingReplayChat";
import presentationSource from "./generated/originalChatPresentation.js?raw";

const ok = <T,>(data: T): ApiResult<T> => ({ ok: true, data });
const token = "a".repeat(32), asset = "b".repeat(64);
const session: ReplaySession = {
  token, recordingId: "synthetic-official-chat", title: "합성 채팅", durationSeconds: 600,
  mimeType: "video/mp4", chatStatus: "recorded", indexState: "ready",
  syncQuality: "observed_media", manualOffsetSeconds: 0, warnings: [],
};
const message = (sequence: number, patch: Partial<ReplayMessage> = {}): ReplayMessage => ({
  sequence, sender: `합성 이용자 ${sequence}`, text: `저장 메시지 ${sequence}`,
  serverTime: null, receivedAt: 1800000000000 + sequence * 1000,
  offsetSeconds: sequence, broadcastOffsetSeconds: null, mediaTimeSeconds: sequence,
  syncQuality: "observed_media", ...patch,
});
const page = (generation: number, items: ReplayMessage[]): ReplayPage => ({
  generation, items, indexState: "ready", syncQuality: "observed_media", warnings: [],
});
const mockApi = (items: ReplayMessage[]) => ({
  open: vi.fn<ReplayApi["open"]>().mockResolvedValue(ok(session)),
  close: vi.fn<ReplayApi["close"]>().mockResolvedValue(ok(undefined)),
  chatAt: vi.fn<ReplayApi["chatAt"]>().mockImplementation(async (_token, _time, generation) => ok(page(generation, items))),
  chatPage: vi.fn<ReplayApi["chatPage"]>().mockImplementation(async (_token, _cursor, generation) => ok(page(generation, items))),
  timeline: vi.fn<ReplayApi["timeline"]>().mockResolvedValue(ok({ bucketSeconds: 30, buckets: [], indexState: "ready", viewerMetricStatus: "not_recorded" })),
  setOffset: vi.fn<ReplayApi["setOffset"]>().mockImplementation(async (_token, value) => ok(value)),
  openProfile: vi.fn<NonNullable<ReplayApi["openProfile"]>>().mockResolvedValue(ok(undefined)),
  mediaUrl: vi.fn<ReplayApi["mediaUrl"]>().mockImplementation(value => `http://atsumi-replay.localhost/${value}`),
});
const deferred = <T,>() => { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { promise, resolve }; };
let root: Root, container: HTMLDivElement;
const onSeek = vi.fn(), onIndexReady = vi.fn();
const shadow = () => container.querySelector<HTMLElement>(".recording-replay-chat")!.shadowRoot!;
const row = (sequence: number) => shadow().querySelector<HTMLElement>(`[data-sequence="${sequence}"]`)!;
const button = (label: string) => [...shadow().querySelectorAll<HTMLButtonElement>("button")].find(item => item.getAttribute("aria-label") === label || item.title === label || item.textContent === label)!;
const render = async (api: ReplayApi, time = 10, seekVersion = 0) => {
  await act(async () => root.render(<RecordingReplayChat api={api} session={session} time={time} seekVersion={seekVersion} onSeek={onSeek} onIndexReady={onIndexReady} />));
};
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
  onSeek.mockClear(); onIndexReady.mockClear();
});
afterEach(async () => {
  await act(async () => root.unmount()); container.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals();
});

describe("isolated original CHZZK archived chat", () => {
  it("mounts original chat structure and styles only inside one shadow root without online application code", async () => {
    const request = vi.fn(), socket = vi.fn();
    vi.stubGlobal("fetch", request); vi.stubGlobal("WebSocket", socket);
    const before = document.head.innerHTML;
    const api = mockApi([message(1)]); await render(api);
    expect(shadow().querySelector(".original-chat-root")).not.toBeNull();
    expect(shadow().querySelector("style")).not.toBeNull();
    expect(row(1)).toHaveTextContent("저장 메시지 1");
    expect(container.querySelector("[data-sequence]")).toBeNull();
    expect(document.head.innerHTML).toBe(before);
    expect(shadow().querySelector("script,iframe,object,embed,link[rel=preload],link[rel=stylesheet]")).toBeNull();
    const styles = [...shadow().querySelectorAll("style")].map(style => style.textContent).join("\n");
    expect(styles).not.toContain(":root{");
    expect(styles).toContain(":host{");
    expect(shadow().querySelector("[data-replay-header]")).not.toBeNull();
    expect(styles).not.toMatch(/@import\s|url\(\s*["']?(?:https?:|\/\/|file:)/i);
    expect(request).not.toHaveBeenCalled(); expect(socket).not.toHaveBeenCalled();
    expect(api.openProfile).not.toHaveBeenCalled();
    expect(presentationSource).not.toMatch(/\b(?:fetch|WebSocket|XMLHttpRequest|sendBeacon|invoke|eval)\s*\(|\bimport\s*(?:\(|["'])|dangerouslySetInnerHTML|new\s+Function\s*\(/);
  });

  it("keeps archived markup literal and rejects style or remote-image injection", async () => {
    const text = '<img src="https://invalid.example/log" onerror="alert(1)"> {:unknown:}';
    const api = mockApi([message(1, {
      sender: '<script>alert("nickname")</script>', text,
      rich: { nicknameColor: "url(https://invalid.example/color)", textColor: "red; background:url(https://invalid.example/secret)",
        badges: [{ kind: "subscription", title: "<svg onload=alert(1)>", imageUrl: "https://invalid.example/badge" }],
        emojis: [{ id: "unknown", imageUrl: "https://invalid.example/emoji" }] },
    })]);
    await render(api);
    expect(row(1)).toHaveTextContent(text);
    expect(row(1)).toHaveTextContent('<script>alert("nickname")</script>');
    expect(row(1)).toHaveTextContent("<svg onload=alert(1)>");
    expect(row(1).querySelector("img,script,iframe,object,embed,[onerror],[onload]")).toBeNull();
    for (const styled of row(1).querySelectorAll<HTMLElement>("[style]")) expect(styled.getAttribute("style")).not.toMatch(/invalid\.example|url\(/);
    expect(api.openProfile).not.toHaveBeenCalled();
  });

  it("preserves validated nickname and body colors and opens a saved profile only by session and row identity", async () => {
    const api = mockApi([message(7, { rich: { nicknameColor: "#11ee77", textColor: "#f0c080", profileUrl: `https://chzzk.naver.com/${"c".repeat(32)}`, badges: [], emojis: [] } })]);
    await render(api);
    const name = button("합성 이용자 7 프로필 열기");
    expect(name).toBeDefined();
    expect([...row(7).querySelectorAll<HTMLElement>("[style]")].some(item => item.style.color === "rgb(17, 238, 119)")).toBe(true);
    // The original nickname component prioritizes profile.title.color, so the
    // separately saved body color is supplied through a validated scoped token.
    expect(row(7).style.getPropertyValue("--replay-text-color")).toBe("#f0c080");
    expect(api.openProfile).not.toHaveBeenCalled();
    await act(async () => name.click());
    expect(api.openProfile).toHaveBeenCalledExactlyOnceWith(token, 7);
    expect(row(7).querySelector('a[href^="https://"]')).toBeNull();
  });

  it.each(["javascript:alert(1)", `https://chzzk.naver.com.evil.example/${"c".repeat(32)}`, `https://chzzk.naver.com/${"c".repeat(32)}?auth=private`])("does not expose an active profile destination for %s", async profileUrl => {
    const api = mockApi([message(1, { rich: { profileUrl, badges: [], emojis: [] } })]); await render(api);
    const name = button("합성 이용자 1 프로필 열기");
    if (name) await act(async () => name.click());
    expect(api.openProfile).not.toHaveBeenCalled();
    expect(row(1).querySelector("a[href]")).toBeNull();
  });

  it("uses only archived asset tokens and keeps readable badge/emoji fallback after image failure", async () => {
    const badgeUrl = "https://ssl.pstatic.net/synthetic-badge.png", emojiUrl = "https://ssl.pstatic.net/synthetic-emoji.png";
    const api = mockApi([message(1, { text: "그대로 {:wave:} 끝", assetIds: { [badgeUrl]: asset, [emojiUrl]: "c".repeat(64) },
      rich: { badges: [{ kind: "subscription", title: "구독 12개월", imageUrl: badgeUrl }], emojis: [{ id: "wave", imageUrl: emojiUrl }] },
    })]); await render(api);
    const images = [...row(1).querySelectorAll("img")];
    expect(images).toHaveLength(2);
    for (const image of images) {
      expect(image.src).toMatch(new RegExp(`^http://atsumi-replay\\.localhost/${token}/asset/[a-f0-9]{64}$`));
      expect(image).toHaveAttribute("referrerpolicy", "no-referrer");
    }
    await act(async () => { for (const image of images) image.dispatchEvent(new Event("error")); });
    expect(row(1).querySelector("img")).toBeNull();
    expect(row(1)).toHaveTextContent("구독 12개월");
    expect(row(1)).toHaveTextContent("{:wave:}");
  });

  it("defaults to hidden timestamps and exposes optional recording/uptime labels in the chat menu", async () => {
    const api = mockApi([message(7)]); await render(api);
    expect(shadow().querySelector(".recording-replay-chat-time")).toBeNull();
    await act(async () => button("채팅 메뉴").click());
    expect(shadow().querySelector("select")).toBeNull();
    expect(shadow().querySelector('[role="radiogroup"][aria-label="채팅 시각 표시"]')).not.toBeNull();
    await act(async () => button("녹화 경과").click());
    expect(row(7).querySelector(".recording-replay-chat-time")).toHaveTextContent("00:00:07");
    await act(async () => button("방송 업타임").click());
    expect(row(7).querySelector(".recording-replay-chat-time")).toHaveTextContent("—");
    expect(api.chatAt).toHaveBeenCalledTimes(1);
  });

  it("bounds retained records and shadow DOM while keeping the latest message visible", async () => {
    const api = mockApi(Array.from({ length: 5000 }, (_, index) => message(index, { text: `메시지 ${index}\n${"줄바꿈 ".repeat(index % 10)}` })));
    await render(api, 6000);
    expect(shadow().querySelectorAll("[data-sequence]").length).toBeLessThanOrEqual(80);
    expect(row(4999)).not.toBeNull();
    expect(shadow().querySelector('[data-sequence="4799"]')).toBeNull();
    expect(container.querySelector("[data-sequence]")).toBeNull();
  });

  it("observes the real shadow scroller and remeasures variable-height rows after it mounts", async () => {
    const observers: { callback: ResizeObserverCallback; targets: Set<Element> }[] = [];
    class MockResizeObserver {
      targets = new Set<Element>();
      constructor(callback: ResizeObserverCallback) { observers.push({ callback, targets: this.targets }); }
      observe(target: Element) { this.targets.add(target); }
      unobserve(target: Element) { this.targets.delete(target); }
      disconnect() { this.targets.clear(); }
    }
    vi.stubGlobal("ResizeObserver", MockResizeObserver);
    let narrow = false;
    const original = HTMLElement.prototype.getBoundingClientRect;
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      if (!this.hasAttribute("data-sequence")) return original.call(this);
      const height = (narrow ? 100 : 35) + Number(this.dataset.sequence) % 3 * 15;
      return { x: 0, y: 0, left: 0, top: 0, right: 300, bottom: height, width: 300, height, toJSON: () => ({}) };
    });
    const api = mockApi(Array.from({ length: 200 }, (_, index) => message(index)));
    await render(api, 220);
    const scroll = shadow().querySelector<HTMLElement>(".recording-replay-chat-scroll")!;
    expect(observers.some(observer => observer.targets.has(scroll))).toBe(true);
    expect(observers.some(observer => [...observer.targets].some(target => target.hasAttribute("data-sequence")))).toBe(true);
    narrow = true;
    Object.defineProperty(scroll, "clientHeight", { configurable: true, value: 240 });
    await act(async () => {
      for (const observer of [...observers]) {
        if (observer.targets.size) observer.callback([...observer.targets].map(target => ({ target }) as ResizeObserverEntry), {} as ResizeObserver);
      }
    });
    expect(shadow().querySelectorAll("[data-sequence]").length).toBeLessThanOrEqual(80);
    expect(row(199)).not.toBeNull();
    expect(api.chatAt).toHaveBeenCalledTimes(1);
  });

  it("ignores delayed pre-seek records instead of overwriting the current replay position", async () => {
    const api = mockApi([message(10)]); await render(api);
    const pending = deferred<ApiResult<ReplayPage>>(); let previousGeneration = 0;
    api.chatAt.mockImplementationOnce((_token, _time, generation) => { previousGeneration = generation; return pending.promise; });
    await render(api, 90, 1);
    api.chatAt.mockImplementation(async (_token, time, generation) => ok(page(generation, [message(time)])));
    await render(api, 4, 2);
    expect(row(4)).not.toBeNull();
    await act(async () => pending.resolve(ok(page(previousGeneration, [message(90)]))));
    expect(row(4)).not.toBeNull();
    expect(shadow().querySelector('[data-sequence="90"]')).toBeNull();
  });
});
