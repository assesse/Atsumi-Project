import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import source from "../../../src-tauri/src/streaming/browser_player_ui.js?raw";

const vmName = "node:vm";
const { runInNewContext } = await import(vmName) as { runInNewContext(source: string, context: Record<string, unknown>): unknown };
const CHANNEL = "b3e262a2795f17734c149afc738ad250";
const REQUEST = "10000000-0000-4000-8000-000000000001";
type Message = { id: string; kind: string; [key: string]: unknown };
const flush = async () => { for (let i = 0; i < 100; i++) await Promise.resolve(); };
function fixture(options: { url?: string; frame?: boolean; strip?: boolean; editable?: boolean; noPlayer?: boolean; notReady?: boolean; inlineLive?: boolean; liveLeft?: number; liveTop?: number; playerLeft?: number; playerWidth?: number; playerHeight?: number; replyData?: Record<string, unknown>; rejectReply?: boolean; manualReply?: boolean } = {}) {
  const page = document.implementation.createHTMLDocument("fixture");
  Object.defineProperty(page, "hidden", { value: false, configurable: true });
  const liveMarkup = options.inlineLive ? '<div id="official-top" style="display:flex;flex-direction:row;column-gap:4px"><button id="official-live" type="button">LIVE</button><button id="official-login">로그인</button></div>' : '<span id="official-live">LIVE</span>';
  page.body.innerHTML = `<div ${options.noPlayer ? "" : 'class="pzp-pc"'}><video></video>${liveMarkup}${options.strip === false ? "" : '<div class="pzp-pc__bottom-buttons-right"><button id="official">Pause</button><button id="official-wide">넓게 보기</button></div>'}</div><aside>${options.editable ? '<div contenteditable="true" role="textbox" data-placeholder="채팅을 입력해주세요"></div>' : '<textarea placeholder="채팅을 입력해주세요"></textarea>'}</aside>`;
  const rect = (left: number, top: number, width: number, height: number) => ({ left, top, width, height, right: left + width, bottom: top + height, x: left, y: top, toJSON() {} });
  const playerLeft = options.playerLeft ?? 0, playerWidth = options.playerWidth ?? 960;
  const player = page.querySelector("video")!.parentElement!;
  player.style.position = "relative";
  vi.spyOn(player, "getBoundingClientRect").mockReturnValue(rect(playerLeft, 0, playerWidth, options.playerHeight ?? 540));
  const live = page.querySelector("#official-live")!;
  vi.spyOn(live, "getBoundingClientRect").mockReturnValue(rect(options.liveLeft ?? playerLeft + playerWidth - 64, options.liveTop ?? 12, 48, 24));
  const video = page.querySelector("video")!;
  Object.defineProperties(video, { readyState: { value: options.notReady ? 0 : 4, configurable: true }, videoWidth: { value: 1920, configurable: true }, videoHeight: { value: 1080 }, currentTime: { value: 100, writable: true },
    seekable: { value: { length: 1, start: () => 90, end: () => 104.25 }, configurable: true }, buffered: { value: { length: 1, start: () => 90, end: () => 106 }, configurable: true },
    paused: { value: false, writable: true }, playbackRate: { value: 1, writable: true }, seeking: { value: false, writable: true } });
  let keyListener: ((event: Record<string, unknown>) => void) | undefined;
  const addListener = page.addEventListener.bind(page);
  vi.spyOn(page, "addEventListener").mockImplementation(((name: string, listener: EventListener, options?: boolean) => {
    if (name === "keydown") keyListener = listener as unknown as typeof keyListener;
    addListener(name, listener, options);
  }) as typeof page.addEventListener);
  const events = new Map<Element, (event: unknown) => void>();
  const makeElement = page.createElement.bind(page);
  let canvasWrites = 0;
  let pngSize = 8;
  const canvas = makeElement("canvas");
  vi.spyOn(page, "createElement").mockImplementation(((tag: string) => {
    if (tag === "canvas") { canvasWrites++; return canvas; }
    const element = makeElement(tag);
    if (tag === "div") vi.spyOn(element, "getBoundingClientRect").mockImplementation(() => rect(options.inlineLive ? live.getBoundingClientRect().left - 46 : 0, 12, 38, 38));
    if (tag === "button") {
      const listen = element.addEventListener.bind(element);
      vi.spyOn(element, "addEventListener").mockImplementation(((type: string, callback: EventListener) => {
        if (type === "click") events.set(element, callback as (event: unknown) => void);
        listen(type, callback);
      }) as typeof element.addEventListener);
    }
    return element;
  }) as typeof page.createElement);
  vi.spyOn(canvas, "getContext").mockReturnValue({ drawImage: vi.fn() } as unknown as CanvasRenderingContext2D);
  vi.spyOn(canvas, "toBlob").mockImplementation((callback) => callback({ size: pngSize, arrayBuffer: async () => new Uint8Array(pngSize).buffer } as Blob));
  const messages: Message[] = [];
  let frameId = 0;
  const frames = new Map<number, FrameRequestCallback>();
  const pageWindow = Object.assign(new EventTarget(), { location: new URL(options.url ?? `https://chzzk.naver.com/live/${CHANNEL}`), top: null as unknown,
    innerWidth: 1280, innerHeight: 800,
    requestAnimationFrame: vi.fn((callback: FrameRequestCallback) => { frames.set(++frameId, callback); return frameId; }),
    cancelAnimationFrame: vi.fn((id: number) => { frames.delete(id); }),
    getComputedStyle: window.getComputedStyle,
    chrome: { webview: { postMessage: (raw: string) => {
      const message = JSON.parse(raw.slice("ATSUMI_BROWSER_CAPTURE:".length)) as Message;
      messages.push(message);
      const data = options.replyData ?? { saved: true };
      if (!options.manualReply) queueMicrotask(() => pageWindow.dispatchEvent(new CustomEvent("atsumi-browser-reply", { detail: { id: message.id, ok: !options.rejectReply, data } })));
    } } }, __atsumiPlayerUI: undefined as undefined | { update(state: Record<string, unknown>): void; configure(state: Record<string, unknown>): void; stopCatchup(): void },
    __atsumiEncodedCapture: undefined as undefined | { canChangePlaybackRate(video: HTMLVideoElement): boolean },
  });
  const windowListener = vi.spyOn(pageWindow, "addEventListener");
  pageWindow.top = options.frame ? {} : pageWindow;
  const context = { window: pageWindow, document: page, crypto, CustomEvent, Date, setTimeout, clearTimeout, setInterval, clearInterval, Uint8Array, btoa };
  runInNewContext(source, context);
  return { page, player, live, video, messages, canvas, window: pageWindow,
    windowListener,
    flushFrame: () => { const pendingFrames = [...frames.values()]; frames.clear(); for (const callback of pendingFrames) callback(0); },
    movePlayer: (top: number) => {
      vi.mocked(player.getBoundingClientRect).mockReturnValue(rect(playerLeft, top, playerWidth, options.playerHeight ?? 540));
      vi.mocked(live.getBoundingClientRect).mockReturnValue(rect(options.liveLeft ?? playerLeft + playerWidth - 64, top + (options.liveTop ?? 12), 48, 24));
    },
    input: page.querySelector<HTMLTextAreaElement>('textarea,[contenteditable]')!,
    writes: () => canvasWrites,
    setPngSize: (value: number) => { pngSize = value; },
    ready: () => pageWindow.__atsumiPlayerUI?.update({ ready: true, recording: false, detail: "ready" }),
    trustedClick: (name: string) => { const button = page.querySelector(`[aria-label="${name}"]`)!; events.get(button)?.({ isTrusted: true, stopPropagation() {} }); },
    key: (fields: Record<string, unknown> = {}) => {
      const event = { isTrusted: true, code: "KeyS", key: "s", target: page.body, preventDefault: vi.fn(), stopPropagation: vi.fn(), ...fields };
      keyListener?.(event); return event;
    },
    command: (fields: Record<string, unknown> = {}) => pageWindow.dispatchEvent(new CustomEvent("atsumi-browser-command", { detail: { kind: "screenshot", channelId: CHANNEL, requestId: REQUEST, ...fields } })),
    again: () => runInNewContext(source, context),
  };
}
beforeEach(() => { vi.useFakeTimers(); });
afterEach(() => { vi.clearAllTimers(); vi.useRealTimers(); vi.restoreAllMocks(); });

describe("official player controls", () => {
  it("does not install on foreign origins, non-live paths or child frames", () => {
    for (const options of [{ url: "https://evil.test/live/" + CHANNEL }, { url: "https://chzzk.naver.com:444/live/" + CHANNEL }, { url: "https://chzzk.naver.com/live/" + CHANNEL + "/chat" }, { frame: true }]) {
      const h = fixture(options); expect(h.window.__atsumiPlayerUI).toBeUndefined(); expect(h.messages).toEqual([]);
    }
  });
  it("adds controls inside the official strip without replacing video or its controls", () => {
    const h = fixture(); h.ready(); h.again();
    expect(h.page.querySelectorAll("#atsumi-player-controls")).toHaveLength(1);
    expect(h.page.querySelector(".pzp-pc__bottom-buttons-right #atsumi-player-controls")).not.toBeNull();
    expect(h.page.querySelector("video")).toBe(h.video);
    expect(h.page.querySelector("#official")?.textContent).toBe("Pause");
    expect(h.writes()).toBe(0); expect(h.messages).toEqual([]);
  });
  it("uses a hover-only player-local fallback without adding a layout row", () => {
    const h = fixture({ strip: false });
    expect(h.page.querySelector(".pzp-pc > #atsumi-player-controls[data-floating]")).not.toBeNull();
    expect(h.page.querySelector("#atsumi-player-ui-style")?.textContent).toContain("position:absolute");
    expect(h.page.querySelector("#atsumi-player-ui-style")?.textContent).toContain("bottom:12px;top:auto");
  });
  it("places the upper view and settings toolbar before LIVE with a clear gap", () => {
    const h = fixture({ playerLeft: 120, playerWidth: 900 });
    const toolbar = h.page.querySelector<HTMLElement>("#atsumi-player-top-controls")!;
    expect(toolbar.dataset.placement).toBe("before-live");
    expect(toolbar.parentElement).toBe(h.page.body);
    expect(Number.parseFloat(toolbar.style.left) + 38 + 8).toBeLessThanOrEqual(h.live.getBoundingClientRect().left);
    expect(h.live.textContent).toBe("LIVE"); expect(h.live.getAttribute("style")).toBeNull();
    expect(h.page.querySelectorAll('[data-settings]')).toHaveLength(1);
    expect(h.page.querySelector('[data-presentation],#atsumi-presentation-toggle')).toBeNull();
  });
  it("uses a separate safe row when there is no horizontal space before LIVE", () => {
    const h = fixture({ playerWidth: 360, liveLeft: 12 });
    const toolbar = h.page.querySelector<HTMLElement>("#atsumi-player-top-controls")!;
    expect(toolbar.dataset.placement).toBe("below-live");
    expect(Number.parseFloat(toolbar.style.top)).toBeGreaterThanOrEqual(h.live.getBoundingClientRect().bottom + 8);
    expect(Number.parseFloat(toolbar.style.left)).toBe(8);
  });
  it("coalesces capturing scroll and resize updates into one animation frame", () => {
    const h = fixture({});
    const toolbar = h.page.querySelector<HTMLElement>("#atsumi-player-top-controls")!;
    const initialTop = Number.parseFloat(toolbar.style.top);
    expect(h.windowListener).toHaveBeenCalledWith("scroll", expect.any(Function), true);
    h.movePlayer(75);
    for (let index = 0; index < 20; index++) h.window.dispatchEvent(new Event("scroll"));
    h.window.dispatchEvent(new Event("resize"));
    expect(h.window.requestAnimationFrame).toHaveBeenCalledOnce();
    expect(Number.parseFloat(toolbar.style.top)).toBe(initialTop);
    h.flushFrame();
    expect(Number.parseFloat(toolbar.style.top)).toBe(initialTop + 75);
    h.window.dispatchEvent(new Event("scroll"));
    expect(h.window.requestAnimationFrame).toHaveBeenCalledTimes(2);
  });
  it("cancels a pending frame on pagehide and cannot recreate controls until pageshow", async () => {
    const h = fixture({});
    h.window.dispatchEvent(new Event("scroll"));
    h.window.dispatchEvent(new Event("pagehide"));
    expect(h.window.cancelAnimationFrame).toHaveBeenCalledOnce();
    h.flushFrame(); h.ready();
    h.window.dispatchEvent(new Event("scroll"));
    await vi.advanceTimersByTimeAsync(3000);
    expect(h.page.querySelector("#atsumi-player-top-controls")).toBeNull();
    expect(h.page.querySelector("#atsumi-player-controls")).toBeNull();
    expect(h.window.requestAnimationFrame).toHaveBeenCalledOnce();
    h.window.dispatchEvent(new Event("pageshow")); h.flushFrame();
    expect(h.page.querySelectorAll("#atsumi-player-top-controls")).toHaveLength(1);
    expect(h.page.querySelectorAll("#atsumi-player-controls")).toHaveLength(1);
    expect(h.messages).toEqual([]);
  });
  it("does not follow the bottom-strip LIVE label and keeps upper controls inside the player", () => {
    const h = fixture({ liveLeft: 12, liveTop: 500 });
    const toolbar = h.page.querySelector<HTMLElement>("#atsumi-player-top-controls")!;
    expect(toolbar.dataset.placement).toBe("player-left");
    expect(Number.parseFloat(toolbar.style.top) + 38).toBeLessThanOrEqual(540 - 8);
  });
  it("falls back beside LIVE when the lower row would leave a short player", () => {
    const h = fixture({ liveLeft: 12, playerHeight: 64, playerWidth: 360 });
    const toolbar = h.page.querySelector<HTMLElement>("#atsumi-player-top-controls")!;
    expect(toolbar.hidden).toBe(false);
    expect(toolbar.dataset.placement).toBe("player-right");
    expect(Number.parseFloat(toolbar.style.top) + 38).toBeLessThanOrEqual(64 - 8);
    expect(Number.parseFloat(toolbar.style.left)).toBeGreaterThanOrEqual(h.live.getBoundingClientRect().right + 8);
  });
  it("hides an overlay that cannot fit rather than covering LIVE or escaping tiny bounds", () => {
    const h = fixture({ liveLeft: 12, playerWidth: 80, playerHeight: 48 });
    const toolbar = h.page.querySelector<HTMLElement>("#atsumi-player-top-controls")!;
    expect(toolbar.hidden).toBe(true);
    expect(h.page.body.hasAttribute("data-atsumi-player-top-controls")).toBe(false);
    expect(h.live.textContent).toBe("LIVE");
  });
  it("keeps settings available before a video or known player becomes ready", async () => {
    const h = fixture({ noPlayer: true, notReady: true });
    expect(h.page.querySelector("#atsumi-player-controls")).toBeNull();
    expect(h.page.querySelector("body > #atsumi-player-top-controls[data-bootstrap]")).not.toBeNull();
    expect(h.page.body.hasAttribute("data-atsumi-player-top-controls")).toBe(false);
    h.trustedClick("시청 설정"); await flush();
    expect(h.messages).toEqual([expect.objectContaining({ kind: "view_intent", action: "open_settings", channelId: CHANNEL })]);
    expect(h.writes()).toBe(0);
  });
  it("stably inserts only our gear beside the official LIVE control without replacing it", async () => {
    const h = fixture({ inlineLive: true });
    const host = h.page.querySelector("#official-top")!;
    const toolbar = h.page.querySelector<HTMLElement>("#atsumi-player-top-controls")!;
    expect(toolbar.parentElement).toBe(host);
    expect(toolbar.nextSibling).toBe(h.live);
    expect(toolbar.dataset.placement).toBe("inline-live");
    expect(h.live.contains(toolbar)).toBe(false);
    expect(h.live.textContent).toBe("LIVE");
    expect(host.getAttribute("style")).toBe("display:flex;flex-direction:row;column-gap:4px");
    expect(toolbar.style.marginInlineEnd).toBe("4px");
    const insert = vi.spyOn(host, "insertBefore");
    h.ready(); await vi.advanceTimersByTimeAsync(3000);
    expect(insert).not.toHaveBeenCalled();
    expect(h.page.querySelectorAll("#atsumi-player-top-controls")).toHaveLength(1);
    const officialClick = vi.fn(); h.live.addEventListener("click", officialClick);
    h.page.querySelector<HTMLButtonElement>("[data-settings]")!.click();
    expect(officialClick).not.toHaveBeenCalled();
    h.live.dispatchEvent(new Event("click"));
    expect(officialClick).toHaveBeenCalledOnce();
  });
  it("leaves official wide/narrow controls and single-view Escape handling to the site", () => {
    const h = fixture();
    const official = h.page.querySelector<HTMLButtonElement>("#official-wide")!;
    const toggle = vi.fn(() => { official.textContent = official.textContent === "넓게 보기" ? "좁게 보기" : "넓게 보기"; });
    official.addEventListener("click", toggle);
    official.click(); expect(official.textContent).toBe("좁게 보기");
    official.click(); expect(official.textContent).toBe("넓게 보기");
    expect(h.page.querySelector("[data-presentation],#atsumi-presentation-toggle")).toBeNull();
    expect(h.page.querySelector<HTMLButtonElement>('[aria-label="기본 보기"]')!.hidden).toBe(true);
    expect(h.key({ key: "Escape", code: "Escape" }).preventDefault).not.toHaveBeenCalled();
    expect(h.messages).toEqual([]);
  });
  it("only requests a native confirmation, never starts recording or captures pixels on a click", async () => {
    const h = fixture(); h.ready();
    h.page.querySelector<HTMLButtonElement>('[aria-label="녹화"]')!.click();
    expect(h.messages).toEqual([]);
    h.trustedClick("녹화"); await flush();
    expect(h.messages.map((m) => m.kind)).toEqual(["control_intent"]);
    expect(h.messages[0]).toMatchObject({ action: "record_start", channelId: CHANNEL });
    expect(h.writes()).toBe(0);
  });
  it("indicates ready and recording states and keeps stop available", () => {
    const h = fixture(); h.ready();
    expect(h.page.querySelector<HTMLButtonElement>('[aria-label="녹화"]')!.disabled).toBe(false);
    h.window.__atsumiPlayerUI?.update({ ready: false, recording: true, detail: "recording" });
    const stop = h.page.querySelector<HTMLButtonElement>('[aria-label="녹화 중지"]')!;
    expect(stop.disabled).toBe(false); expect(stop.getAttribute("data-recording")).toBe("true");
    expect(stop.querySelector('circle[r="8"]')?.getAttribute("fill")).toBeNull();
    expect(stop.querySelector(".atsumi-record-idle")).not.toBeNull();
    expect(stop.querySelector(".atsumi-record-stop")).not.toBeNull();
    expect(stop.title).toContain("1920×1080");
  });
  it("shows seekable distance in our speed tooltip without touching chat placeholders", async () => {
    const h = fixture();
    expect(h.input.placeholder).toBe("채팅을 입력해주세요");
    const speed = h.page.querySelector<HTMLButtonElement>('[aria-label="따라잡기"]')!;
    expect(speed.title).toContain("4.3초");
    Object.defineProperty(h.video, "seekable", { value: { length: 0 } });
    await vi.advanceTimersByTimeAsync(1000);
    expect(speed.title).toContain("6.0초");
    Object.defineProperty(h.video, "buffered", { value: { length: 0 } });
    await vi.advanceTimersByTimeAsync(1000);
    expect(speed.title).toContain("확인 전");
    expect(h.input.placeholder).toBe("채팅을 입력해주세요");
  });
  it("never rewrites a textarea during focus, blur, draft or composition transitions", async () => {
    const h = fixture();
    h.input.rows = 1; h.input.style.height = "22px";
    const before = h.input.outerHTML;
    const mutate = vi.spyOn(h.input, "setAttribute");
    for (const name of ["focus", "focusin", "compositionstart", "compositionend", "blur", "focusout"]) {
      h.input.dispatchEvent(new Event(name)); h.ready();
      await vi.advanceTimersByTimeAsync(1000);
    }
    expect(h.input.outerHTML).toBe(before);
    expect(mutate).not.toHaveBeenCalled();
    h.input.value = "작성 중인 채팅"; h.ready();
    expect(h.input.value).toBe("작성 중인 채팅");
    expect(h.input.style.height).toBe("22px");
    h.input.placeholder = "채팅에 참여하려면 로그인 해주세요";
    await vi.advanceTimersByTimeAsync(1000);
    expect(h.input.placeholder).toBe("채팅에 참여하려면 로그인 해주세요");
  });
  it("does not add placeholders or change contenteditable text/height", async () => {
    const h = fixture({ editable: true });
    const before = h.input.outerHTML;
    h.input.dispatchEvent(new Event("focus")); h.ready();
    h.input.dispatchEvent(new Event("blur"));
    await vi.advanceTimersByTimeAsync(2000);
    h.window.dispatchEvent(new Event("pagehide"));
    expect(h.input.outerHTML).toBe(before);
    expect(h.input.getAttribute("placeholder")).toBeNull();
  });
  it("requires a matching screenshot command and sends bounded ACK-ordered PNG data", async () => {
    const h = fixture(); h.command({ requestId: "bad" }); h.command({ channelId: "f".repeat(32) }); await flush();
    expect(h.writes()).toBe(0);
    h.command(); await flush();
    expect(h.messages.map((m) => m.kind)).toEqual(["screenshot_begin", "screenshot_chunk", "screenshot_finish"]);
    expect(h.messages[0]).toMatchObject({ mimeType: "image/png", width: 1920, height: 1080, size: 8 });
    expect(h.canvas.width).toBe(0); expect(h.canvas.height).toBe(0);
  });
  it("rejects oversized screenshot data without uploading it and releases the arm", async () => {
    const h = fixture(); h.setPngSize(16 * 1024 * 1024 + 1); h.command(); await flush();
    expect(h.messages.map((m) => m.kind)).toEqual(["screenshot_abort"]);
  });
  it("maps S to the same screenshot confirmation without directly capturing a frame", async () => {
    const h = fixture(); const event = h.key(); await flush();
    expect(event.preventDefault).toHaveBeenCalledOnce();
    expect(h.messages).toEqual([expect.objectContaining({ kind: "control_intent", action: "screenshot", channelId: CHANNEL })]);
    expect(h.writes()).toBe(0);
  });
  it("does not steal typing, IME, modified, repeated or synthetic keys", () => {
    const h = fixture();
    for (const fields of [{ target: h.input }, { isComposing: true }, { keyCode: 229 }, { ctrlKey: true }, { altKey: true }, { metaKey: true }, { shiftKey: true }, { repeat: true }, { isTrusted: false }, { defaultPrevented: true }]) {
      expect(h.key(fields).preventDefault).not.toHaveBeenCalled();
    }
    expect(h.messages).toEqual([]);
  });
  it("preserves only multiview exit and independent audio controls", async () => {
    const h = fixture();
    expect(h.page.querySelector<HTMLButtonElement>('[aria-label="소리 켜기"]')!.hidden).toBe(true);
    h.window.__atsumiPlayerUI!.configure({ multiview: true, audioEnabled: true });
    expect(h.page.querySelector("#atsumi-player-top-controls")).toBeNull();
    h.trustedClick("기본 보기"); await flush();
    expect(h.messages[0]).toMatchObject({ kind: "view_intent", action: "exit_focus" });
    expect(h.page.querySelector<HTMLButtonElement>('[aria-label="소리 끄기"]')!.hidden).toBe(false);
    h.trustedClick("소리 끄기"); await flush();
    expect(h.messages[1]).toMatchObject({ kind: "view_intent", action: "audio_toggle" });
  });
  it("catches up at 1.2x and returns to 1x near the live edge without seeking or network requests", async () => {
    const h = fixture(); h.ready(); h.trustedClick("따라잡기");
    expect(h.video.playbackRate).toBe(1.2); expect(h.video.currentTime).toBe(100);
    h.video.currentTime = 103.5; await vi.advanceTimersByTimeAsync(1000);
    expect(h.video.playbackRate).toBe(1); expect(h.messages).toEqual([]);
  });
  it("keeps legacy recording at 1x and allows only an approved matching encoded session", () => {
    const h = fixture(); h.window.__atsumiPlayerUI!.update({ ready: true, recording: true, detail: "recording" });
    h.trustedClick("따라잡기"); expect(h.video.playbackRate).toBe(1);
    h.window.__atsumiEncodedCapture = { canChangePlaybackRate: (video) => video === h.video };
    h.trustedClick("따라잡기"); expect(h.video.playbackRate).toBe(1.2);
    h.window.__atsumiPlayerUI!.update({ recording: true, detail: "saving" });
    expect(h.video.playbackRate).toBe(1);
  });
  it.each([.5, .75, 1, 1.25, 1.5, 2])("offers %sx from buffered media while approved encoded recording continues", (rate) => {
    const h = fixture(); h.window.__atsumiEncodedCapture = { canChangePlaybackRate: (video) => video === h.video };
    h.window.__atsumiPlayerUI!.update({ ready: true, recording: true, detail: "recording" });
    h.trustedClick("재생 배속"); expect(h.page.querySelector<HTMLElement>('[role="menu"]')!.hidden).toBe(false);
    h.trustedClick(`${rate}배속`); expect(h.video.playbackRate).toBe(rate); expect(h.video.preservesPitch).toBe(true);
    expect(h.page.querySelector<HTMLElement>('[role="menu"]')!.hidden).toBe(true);
    expect(h.video.currentTime).toBe(100); expect(h.messages).toEqual([]);
  });
  it("prevents speed choices in fallback recording and cancels an owned speed when recording becomes unsafe", () => {
    const h = fixture(); h.ready(); h.trustedClick("2배속"); expect(h.video.playbackRate).toBe(2);
    h.window.__atsumiPlayerUI!.update({ ready: true, recording: true, detail: "recording" });
    expect(h.video.playbackRate).toBe(1); h.trustedClick("0.5배속"); expect(h.video.playbackRate).toBe(1);
    expect(h.page.querySelector<HTMLButtonElement>('[aria-label="재생 배속"]')!.disabled).toBe(true);
    expect(h.messages).toEqual([]);
  });
  it("returns to 1x near the edge, rejects buffer gaps, and invalidates seeking or replaced sources", async () => {
    const h = fixture(); h.ready(); h.trustedClick("2배속"); h.video.currentTime = 103.1;
    await vi.advanceTimersByTimeAsync(250); expect(h.video.playbackRate).toBe(1);
    h.video.currentTime = 100; h.trustedClick("1.5배속"); h.video.dispatchEvent(new Event("seeking"));
    expect(h.video.playbackRate).toBe(1);
    h.trustedClick("0.5배속"); h.video.src = "blob:replacement";
    await vi.advanceTimersByTimeAsync(250); expect(h.video.playbackRate).toBe(1);
    Object.defineProperty(h.video, "buffered", { value: { length: 2, start: (i: number) => i ? 103 : 90, end: (i: number) => i ? 120 : 99 } });
    h.trustedClick("2배속"); expect(h.video.playbackRate).toBe(1);
    expect(h.messages).toEqual([]);
  });
  it("preserves a user's later rate choice and stops when the page leaves", async () => {
    const h = fixture(); h.ready(); h.trustedClick("따라잡기"); h.video.playbackRate = 1.5;
    await vi.advanceTimersByTimeAsync(1000); expect(h.video.playbackRate).toBe(1.5);
    h.video.playbackRate = 1; h.trustedClick("따라잡기");
    h.window.dispatchEvent(new Event("pagehide")); expect(h.video.playbackRate).toBe(1);
  });
});
