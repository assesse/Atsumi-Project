import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import source from "../../../src-tauri/src/streaming/browser_multiview.js?raw";
import receiver from "../../../src-tauri/src/streaming/browser_auto_view.js?raw";

const vmName = "node:vm";
const { runInNewContext } = await import(vmName) as {
  runInNewContext(source: string, context: Record<string, unknown>): unknown;
};
const ID = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
type State = { kind: string; active: boolean; reason: string; audioEnabled: boolean };
type PageWindow = EventTarget & {
  location: URL; top: unknown; innerWidth: number; innerHeight: number;
  getComputedStyle: typeof window.getComputedStyle;
  __atsumiMultiView?: { getState(): State; setVideoOnly(enabled: boolean): void; configureChat(value: { channelId: string; number: number; channelName: string }): void };
  __atsumiAutoReceiver?: { configure(value: { revision: number; viewing: boolean; multiview: boolean; audioEnabled: boolean }): void };
};
const observers: MutationObserver[] = [];
const rect = (node: Element, width = 900, height = 600) => vi.spyOn(node, "getBoundingClientRect").mockReturnValue({
  x: 0, y: 0, left: 0, top: 0, width, height, right: width, bottom: height, toJSON: () => ({}),
});
function fixture(options: { chat?: boolean; url?: string; nested?: boolean; width?: number; height?: number; receiver?: boolean } = {}) {
  const page = document.implementation.createHTMLDocument("isolated multiview fixture");
  page.body.innerHTML = `<header>Original</header><main><div class="pzp-pc"><div class="translated"><div class="aspect"><video></video></div></div><button id="official">Control</button></div></main><aside><textarea placeholder="채팅 입력"></textarea></aside>`;
  const pageWindow = Object.assign(new EventTarget(), {
    location: new URL(options.url ?? `https://chzzk.naver.com/live/${ID}${options.chat ? "/chat" : ""}`), top: null as unknown,
    innerWidth: options.width ?? 640, innerHeight: options.height ?? 360,
    getComputedStyle: window.getComputedStyle.bind(window),
  }) as PageWindow;
  pageWindow.top = options.nested ? {} : pageWindow;
  const video = page.querySelector("video")!;
  Object.defineProperties(video, { readyState: { value: 4, configurable: true }, videoWidth: { value: 1920, configurable: true }, videoHeight: { value: 1080 }, paused: { value: false } });
  const pause = vi.spyOn(video, "pause").mockImplementation(() => undefined);
  rect(video); rect(page.querySelector(".pzp-pc")!);
  class Observer extends MutationObserver {
    constructor(callback: MutationCallback) { super(callback); observers.push(this); }
  }
  const context = { window: pageWindow, document: page, location: pageWindow.location, MutationObserver: Observer, CustomEvent, Event, Date, setTimeout, clearTimeout, setInterval, clearInterval };
  if (options.receiver) runInNewContext(receiver, context);
  runInNewContext(source, context);
  const advance = () => vi.advanceTimersByTime(160);
  const audio = (enabled: boolean, applyToMedia = true) => pageWindow.dispatchEvent(new CustomEvent("atsumi-multiview-audio", { detail: { enabled, applyToMedia } }));
  return { page, pageWindow, video, pause, advance, audio,
    state: () => pageWindow.__atsumiMultiView?.getState(), again: () => runInNewContext(source, context) };
}
beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  for (const observer of observers.splice(0)) observer.disconnect();
  vi.clearAllTimers(); vi.useRealTimers(); vi.restoreAllMocks();
});

describe("watch-only official multiview bridge", () => {
  it("preserves the official pre-player slot before PZP exists and transitions without touching media", () => {
    const f = fixture(); const main = f.page.querySelector('main')!;
    main.innerHTML = '<div class="_player_jwedy_23"><div class="_container_10lql_1"><div id="preAdPlayerWrapper"><video id="preAdContentPlayer"></video><button>닫기</button></div></div></div><aside>Related broadcasts</aside>';
    const slot = main.firstElementChild!, preparation = f.page.querySelector('#preAdPlayerWrapper')!;
    const media = preparation.querySelector('video')!, parent = media.parentElement;
    f.advance();
    expect(f.state()).toMatchObject({ active: true, reason: 'player_preparing' });
    expect(slot.hasAttribute('data-atsumi-mado-player')).toBe(true);
    expect(main.hasAttribute('data-atsumi-mado-path')).toBe(true);
    expect(preparation.hasAttribute('data-atsumi-mado-path')).toBe(false);
    expect(media.parentElement).toBe(parent);
    expect(media.hasAttribute('data-atsumi-mado-video')).toBe(false);
    expect(f.page.querySelector('[data-atsumi-mado-placeholder]')).toBeNull();
    slot.appendChild(f.video); slot.className = 'chzzk_player';
    f.pageWindow.dispatchEvent(new Event('resize')); f.advance();
    expect(f.state()).toMatchObject({ active: true, reason: 'video_only' });
    expect(f.pause).not.toHaveBeenCalled();
    expect(f.video.hasAttribute('data-atsumi-mado-video')).toBe(true);
  });
  it("uses the exact pre-roll wrapper as a fallback without exposing surrounding navigation", () => {
    const f = fixture(); const main = f.page.querySelector('main')!;
    main.innerHTML = '<div class="future-player-slot"><div id="preAdPlayerWrapper"><video></video></div><aside>Other content</aside></div>';
    f.advance();
    expect(f.page.querySelector('#preAdPlayerWrapper')!.hasAttribute('data-atsumi-mado-player')).toBe(true);
    expect(f.page.querySelector('main aside')!.hasAttribute('data-atsumi-mado-path')).toBe(false);
    expect(f.state()?.reason).toBe('player_preparing');
  });
  it("keeps the outer player's controls and offline notice even before a video decodes", () => {
    const f = fixture();
    const wrapper = f.page.querySelector('main')!; wrapper.className = 'chzzk_player type_live';
    const notice = f.page.createElement('div'); notice.textContent = '방송이 종료되었습니다'; wrapper.appendChild(notice);
    Object.defineProperties(f.video, { readyState: { value: 0 }, videoWidth: { value: 0 } });
    f.advance();
    expect(wrapper.hasAttribute('data-atsumi-mado-player')).toBe(true);
    expect(f.page.body.hasAttribute('data-atsumi-mado')).toBe(true);
    expect(f.page.querySelector('#official')?.closest('[data-atsumi-mado-player]')).toBe(wrapper);
    f.video.remove(); f.pageWindow.dispatchEvent(new Event('resize')); f.advance();
    expect(f.state()).toMatchObject({ active: true, reason: 'player_notice' });
    expect(wrapper.hasAttribute('data-atsumi-mado-player')).toBe(true);
    expect(notice.parentElement).toBe(wrapper);
  });
  it("does not reveal the whole website while the player is absent and recovers when it arrives", () => {
    const f = fixture(); const player = f.page.querySelector('.pzp-pc')!; player.remove(); f.advance();
    expect(f.state()?.reason).toBe('player_unavailable');
    expect(f.page.body.hasAttribute('data-atsumi-mado')).toBe(true);
    expect(f.page.querySelector('[data-atsumi-mado-placeholder]')?.getAttribute('role')).toBe('status');
    f.page.querySelector('main')!.appendChild(player);
    f.pageWindow.dispatchEvent(new Event('resize')); f.advance();
    expect(f.state()?.active).toBe(true);
    expect(f.page.querySelector('[data-atsumi-mado-placeholder]')).toBeNull();
  });
  it("never tears down presentation on repeated host settings or viewport refreshes", () => {
    const f = fixture(); f.advance();
    const remove = vi.spyOn(f.page.body, 'removeAttribute');
    const set = vi.spyOn(f.page.body, 'setAttribute');
    for (let index = 0; index < 20; index++) {
      f.pageWindow.__atsumiMultiView!.setVideoOnly(true);
      f.pageWindow.dispatchEvent(new Event('resize')); f.advance();
    }
    expect(remove).not.toHaveBeenCalled(); expect(set).not.toHaveBeenCalled();
    expect(f.pause).not.toHaveBeenCalled();
  });
  it("does not hide native controls when an internal dialog is present but closed", () => {
    const f = fixture(); const player = f.page.querySelector('.pzp-pc')!;
    const dialog = f.page.createElement('div'); dialog.setAttribute('role', 'dialog'); dialog.style.display = 'none'; player.appendChild(dialog);
    f.advance();
    expect(player.hasAttribute('data-atsumi-mado-path')).toBe(false);
    expect(player.hasAttribute('data-atsumi-mado-player')).toBe(true);
  });
  it("restores the original live chat and controls without recreating video or chat input", () => {
    const f = fixture(); f.advance();
    const input = f.page.querySelector("textarea")!, video = f.video, parent = video.parentElement;
    input.value = "작성 중인 채팅";
    f.pageWindow.__atsumiMultiView!.setVideoOnly(false);
    expect(f.state()).toMatchObject({ active: false, reason: "original" });
    expect(f.page.body.hasAttribute("data-atsumi-mado")).toBe(false);
    expect(f.page.querySelectorAll("[data-atsumi-mado-path],[data-atsumi-mado-media]")).toHaveLength(0);
    f.pageWindow.dispatchEvent(new Event("resize")); f.advance();
    expect(f.state()?.active).toBe(false);
    f.pageWindow.__atsumiMultiView!.setVideoOnly(true);
    expect(f.state()?.active).toBe(true);
    expect(f.page.querySelector("video")).toBe(video); expect(video.parentElement).toBe(parent);
    expect(f.page.querySelector("textarea")).toBe(input); expect(input.value).toBe("작성 중인 채팅");
  });
  it.each([
    "https://evil.example/live/" + ID,
    `https://chzzk.naver.com/live/${ID}?anything=1`,
    `https://chzzk.naver.com/live/${ID}/chat#fragment`,
    `https://chzzk.naver.com/live/${ID}/other`,
  ])("does not inject on unapproved page %s", (url) => {
    const f = fixture({ url }); f.advance(); expect(f.state()).toBeUndefined();
    expect(f.video.muted).toBe(false);
  });
  it("does not inject inside an iframe", () => {
    const f = fixture({ nested: true }); f.advance(); expect(f.state()).toBeUndefined();
  });
  it("normalizes only the original selected video ancestor chain", () => {
    const f = fixture(); const parent = f.video.parentElement; f.advance();
    expect(f.state()).toMatchObject({ active: true, reason: "video_only", audioEnabled: false });
    expect(f.video.muted).toBe(true); expect(f.video.parentElement).toBe(parent);
    expect(f.page.querySelectorAll("[data-atsumi-mado-media]")).toHaveLength(2);
    expect(f.video.hasAttribute("data-atsumi-mado-video")).toBe(true);
    expect(f.page.querySelector("#official")!.hasAttribute("data-atsumi-mado-media")).toBe(false);
    expect(f.page.querySelector("style")!.textContent).toContain("object-fit:contain!important");
    expect(f.page.querySelector("style")!.textContent).toContain("transform:none!important");
  });
  it("keeps an official dialog operable without exposing the rest of the website", () => {
    const f = fixture(); f.advance();
    const dialog = f.page.createElement("div"); dialog.setAttribute("role", "dialog"); rect(dialog, 400, 200); f.page.body.appendChild(dialog);
    f.pageWindow.dispatchEvent(new Event("resize")); f.advance();
    expect(f.state()).toMatchObject({ active: true, reason: "video_only" });
    expect(f.page.querySelectorAll("[data-atsumi-mado-media]")).toHaveLength(2);
    expect(dialog.hasAttribute('data-atsumi-mado-dialog')).toBe(true);
    expect(dialog.parentElement).toBe(f.page.body);
  });
  it("uses the original wide action once to populate live metadata and preserves its updates and audio", async () => {
    const f = fixture({ width: 400, height: 300 });
    const player = f.page.querySelector('.pzp-pc')!;
    const header = f.page.createElement('div'); header.className = 'player_header';
    const information = f.page.createElement('div'); information.className = 'header_info'; header.appendChild(information); player.appendChild(header);
    const unrelated = f.page.createElement('button'); unrelated.className = 'pzp-pc__viewmode-button'; unrelated.setAttribute('aria-label', '오디오 압축'); player.appendChild(unrelated);
    const unrelatedClick = vi.fn(); unrelated.addEventListener('click', unrelatedClick);
    const wide = f.page.createElement('button'); wide.className = 'pzp-pc__viewmode-button'; wide.setAttribute('aria-label', '넓은 화면'); player.appendChild(wide);
    const originalWideAction = vi.fn(() => {
      const expanded = wide.getAttribute('aria-label') === '넓은 화면';
      wide.setAttribute('aria-label', expanded ? '좁은 화면' : '넓은 화면');
      information.innerHTML = expanded ? '<img alt="" src="avatar.png"><p>원본 방송 제목</p><p>원본 채널</p><strong>현재 42</strong><span>01:23:45 스트리밍 중</span>' : '<span>LIVE</span>';
    });
    wide.addEventListener('click', originalWideAction);
    f.advance(); f.audio(true); await Promise.resolve();
    expect(originalWideAction).toHaveBeenCalledOnce(); expect(unrelatedClick).not.toHaveBeenCalled();
    expect(f.page.querySelector('.player_header')).toBe(header); expect(information.parentElement).toBe(header);
    expect(information.querySelector('img')).not.toBeNull(); expect(information.textContent).toContain('원본 방송 제목원본 채널현재 4201:23:45');
    const count = information.querySelector('strong')!; count.textContent = '현재 43';
    f.pageWindow.dispatchEvent(new Event('resize')); f.advance(); await Promise.resolve();
    expect(information.querySelector('strong')).toBe(count); expect(count.textContent).toBe('현재 43');
    expect(originalWideAction).toHaveBeenCalledOnce(); expect(f.video.muted).toBe(false);
    f.pageWindow.__atsumiMultiView!.setVideoOnly(false);
    expect(originalWideAction).toHaveBeenCalledTimes(2); expect(wide.getAttribute('aria-label')).toBe('넓은 화면');
    expect(f.state()?.active).toBe(false); expect(f.video.muted).toBe(false);
    f.pageWindow.__atsumiMultiView!.setVideoOnly(true);
    expect(originalWideAction).toHaveBeenCalledTimes(3); expect(f.state()?.active).toBe(true);
  });
  it("preserves an already-wide official player when returning to the original page", () => {
    const f = fixture();
    const wide = f.page.createElement('button'); wide.className = 'pzp-pc__viewmode-button'; wide.setAttribute('aria-label', '좁은 화면');
    f.page.querySelector('.pzp-pc')!.appendChild(wide); const click = vi.fn(); wide.addEventListener('click', click);
    f.advance(); f.pageWindow.__atsumiMultiView!.setVideoOnly(false);
    expect(click).not.toHaveBeenCalled(); expect(wide.getAttribute('aria-label')).toBe('좁은 화면');
  });
  it("retries a wide-mode click made before the site is ready, then stops after confirmation", async () => {
    const f = fixture(); const player = f.page.querySelector('.pzp-pc')!;
    const wide = f.page.createElement('button'); wide.className = 'pzp-pc__viewmode-button'; wide.setAttribute('aria-label', '넓은 화면'); player.appendChild(wide);
    f.advance(); // Official listener has not attached yet.
    const click = vi.fn(() => wide.setAttribute('aria-label', '좁은 화면'));
    wide.addEventListener('click', click);
    await vi.advanceTimersByTimeAsync(1400);
    expect(click).toHaveBeenCalledOnce();
    await vi.advanceTimersByTimeAsync(10000);
    expect(click).toHaveBeenCalledOnce();
    expect(f.pause).not.toHaveBeenCalled();
    // A site layout reset on the same player must also be recoverable.
    wide.setAttribute('aria-label', '넓은 화면');
    await vi.advanceTimersByTimeAsync(160);
    expect(click).toHaveBeenCalledTimes(2);
  });
  it("bounds failed wide-mode retries and cancels them when leaving player-only mode", async () => {
    const f = fixture(); const wide = f.page.createElement('button'); wide.className = 'pzp-pc__viewmode-button'; wide.setAttribute('aria-label', '넓은 화면'); f.page.querySelector('.pzp-pc')!.appendChild(wide);
    const click = vi.fn(); wide.addEventListener('click', click);
    await vi.advanceTimersByTimeAsync(20000);
    expect(click).toHaveBeenCalledTimes(4);
    f.pageWindow.__atsumiMultiView!.setVideoOnly(false);
    await vi.advanceTimersByTimeAsync(10000);
    expect(click).toHaveBeenCalledTimes(4);
  });
  it("does not exhaust attempts during pre-roll or when React replaces a failed control", async () => {
    const f = fixture(); const player = f.page.querySelector('.pzp-pc')!;
    const wide = f.page.createElement('button'); wide.className = 'pzp-pc__viewmode-button'; wide.setAttribute('aria-label', '넓은 화면'); wide.disabled = true; player.appendChild(wide);
    const click = vi.fn(); wide.addEventListener('click', click);
    await vi.advanceTimersByTimeAsync(10000); expect(click).not.toHaveBeenCalled();
    wide.disabled = false;
    await vi.advanceTimersByTimeAsync(8000); expect(click).toHaveBeenCalledTimes(4);
    const replacement = wide.cloneNode(true) as HTMLButtonElement;
    const ready = vi.fn(() => replacement.setAttribute('aria-label', '좁은 화면'));
    replacement.addEventListener('click', ready); wide.replaceWith(replacement);
    await vi.advanceTimersByTimeAsync(160); expect(ready).toHaveBeenCalledOnce();
    await vi.advanceTimersByTimeAsync(5000); expect(ready).toHaveBeenCalledOnce();
  });
  it("initializes official metadata when the native wide control arrives after the video", async () => {
    const f = fixture(); f.advance(); await Promise.resolve();
    const wide = f.page.createElement('button'); wide.className = 'pzp-pc__viewmode-button'; wide.setAttribute('aria-label', '넓은 화면');
    const click = vi.fn(); wide.addEventListener('click', click); f.page.querySelector('.pzp-pc')!.appendChild(wide);
    await Promise.resolve(); f.advance(); expect(click).toHaveBeenCalledOnce();
    for (let index = 0; index < 3; index++) { f.pageWindow.dispatchEvent(new Event('resize')); f.advance(); }
    expect(click).toHaveBeenCalledOnce();
  });
  it("does not expose site navigation while a small viewport is being resized", () => {
    const f = fixture({ width: 159, height: 89 }); f.advance();
    expect(f.state()).toMatchObject({ active: true, reason: "video_only" }); expect(f.video.muted).toBe(true);
  });
  it("preserves chat input and remains muted even after a host audio request", () => {
    const f = fixture({ chat: true, width: 180, height: 150 });
    const input = f.page.querySelector("textarea")!; const parent = input.parentElement;
    const key = vi.fn(); input.addEventListener("keydown", key);
    f.advance(); f.audio(true); input.value = "synthetic text";
    const event = new KeyboardEvent("keydown", { key: "Enter", cancelable: true }); input.dispatchEvent(event);
    expect(f.state()).toEqual({ kind: "chat", active: false, reason: "chat_only", audioEnabled: false });
    expect(f.video.muted).toBe(true); expect(f.pause).toHaveBeenCalled();
    expect(input.parentElement).toBe(parent); expect(input.value).toBe("synthetic text");
    expect(key).toHaveBeenCalledOnce(); expect(event.defaultPrevented).toBe(false);
    expect(f.page.querySelector("style")).toBeNull(); expect(f.page.querySelector("#atsumi-mado-toggle")).toBeNull();
    expect(f.pageWindow.location.pathname).toBe(`/live/${ID}/chat`);
  });
  it("applies owner changes to newly inserted videos without any playback seek", () => {
    const f = fixture(); f.advance(); f.audio(true); expect(f.video.muted).toBe(false);
    f.audio(false); expect(f.video.muted).toBe(true);
    const replacement = f.page.createElement("video"); f.page.body.appendChild(replacement);
    replacement.dispatchEvent(new Event("play", { bubbles: true }));
    expect(replacement.muted).toBe(true); expect(f.video.currentTime).toBe(0);
  });
  it("lets the original mute and volume controls stay authoritative after the host permits audio", async () => {
    const f = fixture(); f.advance(); expect(f.video.muted).toBe(true);
    f.audio(true, false);
    expect(f.video.muted).toBe(true); // Readiness must not suddenly unmute it.
    const button = f.page.querySelector<HTMLButtonElement>("#official")!;
    button.addEventListener("click", () => { f.video.muted = !f.video.muted; f.video.dispatchEvent(new Event("volumechange", { bubbles: true })); });
    button.click(); expect(f.video.muted).toBe(false);
    f.video.volume = .35; f.video.dispatchEvent(new Event("volumechange", { bubbles: true }));
    button.click(); expect(f.video.muted).toBe(true);
    for (const name of ["play", "playing", "loadedmetadata", "volumechange"]) f.video.dispatchEvent(new Event(name, { bubbles: true }));
    f.pageWindow.dispatchEvent(new Event("resize"));
    button.className = "original-volume-updated"; await Promise.resolve(); f.advance();
    expect(f.video.muted).toBe(true); expect(f.video.volume).toBe(.35);
    f.audio(true, false); expect(f.video.muted).toBe(true);
    button.click(); f.advance(); expect(f.video.muted).toBe(false);
    expect(f.video.currentTime).toBe(0); expect(f.pause).not.toHaveBeenCalled();
  });
  it("keeps independent original-player audio choices and only enforces explicit host denial", () => {
    const first = fixture(), second = fixture(); first.advance(); second.advance();
    first.audio(true, false); second.audio(true, false);
    first.video.muted = false; second.video.muted = false;
    first.video.dispatchEvent(new Event("volumechange", { bubbles: true })); second.video.dispatchEvent(new Event("volumechange", { bubbles: true }));
    first.video.muted = true; first.video.dispatchEvent(new Event("volumechange", { bubbles: true }));
    expect(first.video.muted).toBe(true); expect(second.video.muted).toBe(false);
    second.audio(false, false); second.video.muted = false; second.video.dispatchEvent(new Event("volumechange", { bubbles: true }));
    expect(second.video.muted).toBe(true);
    const chat = fixture({ chat: true }); chat.audio(true, false); chat.video.muted = false; chat.video.dispatchEvent(new Event("volumechange", { bubbles: true }));
    expect(chat.video.muted).toBe(true);
  });
  it("restores the audio gate of borrowed receivers before accepting their independent original volume controls", () => {
    const panes = Array.from({ length: 4 }, () => fixture({ receiver: true }));
    for (const [index, pane] of panes.entries()) {
      pane.advance();
      expect(pane.state()?.audioEnabled).toBe(false);
      expect(pane.video.muted).toBe(true);
      pane.pageWindow.__atsumiAutoReceiver!.configure({ revision: 2, viewing: true, multiview: true, audioEnabled: true });
      // Exercise the same media properties/events used by the original controls.
      pane.video.volume = (index + 1) / 10;
      pane.video.muted = false;
      pane.video.dispatchEvent(new Event("volumechange", { bubbles: true }));
      expect(pane.state()?.audioEnabled).toBe(true);
      expect(pane.video.muted).toBe(false);
    }
    const first = panes[0]!;
    first.video.muted = true;
    first.video.dispatchEvent(new Event("volumechange", { bubbles: true }));
    for (const [index, pane] of panes.entries()) {
      pane.pageWindow.__atsumiAutoReceiver!.configure({ revision: 3, viewing: true, multiview: true, audioEnabled: true });
      pane.advance();
      expect(pane.video.volume).toBe((index + 1) / 10);
      expect(pane.video.muted).toBe(index === 0);
      expect(pane.video.currentTime).toBe(0);
      expect(pane.pause).not.toHaveBeenCalled();
    }
  });
  it("preserves the viewer's mute and volume across parking, reattachment and late viewport requests", () => {
    for (const muted of [false, true]) {
      const pane = fixture({ receiver: true }); pane.advance();
      const configure = pane.pageWindow.__atsumiAutoReceiver!.configure;
      configure({ revision: 1, viewing: true, multiview: true, audioEnabled: true });
      pane.video.volume = .35; pane.video.muted = muted;
      pane.video.dispatchEvent(new Event("volumechange", { bubbles: true }));
      configure({ revision: 2, viewing: false, multiview: true, audioEnabled: false });
      expect(pane.video.muted).toBe(true);
      expect(pane.state()?.audioEnabled).toBe(false);
      configure({ revision: 1, viewing: true, multiview: true, audioEnabled: true });
      expect(pane.state()?.audioEnabled).toBe(false);
      configure({ revision: 3, viewing: true, multiview: true, audioEnabled: true });
      pane.video.dispatchEvent(new Event("volumechange", { bubbles: true }));
      expect(pane.video.muted).toBe(muted);
      expect(pane.video.volume).toBe(.35);
      configure({ revision: 2, viewing: false, multiview: true, audioEnabled: false });
      pane.advance();
      expect(pane.state()?.audioEnabled).toBe(true);
      expect(pane.video.muted).toBe(muted);
      expect(pane.page.querySelector("video")).toBe(pane.video);
      expect(pane.pause).not.toHaveBeenCalled();
    }
  });
  it("never grants audio to a chat-only pane through a receiver presentation request", () => {
    const pane = fixture({ chat: true, receiver: true }); pane.advance();
    pane.pageWindow.__atsumiAutoReceiver!.configure({ revision: 1, viewing: true, multiview: true, audioEnabled: true });
    pane.video.muted = false; pane.video.dispatchEvent(new Event("volumechange", { bubbles: true }));
    expect(pane.state()?.audioEnabled).toBe(false); expect(pane.video.muted).toBe(true);
  });
  const chatHeader = (page: Document) => {
    const header = page.createElement("div"); header.className = "_container_1e2su_2";
    header.innerHTML = '<h2 class="_title_1e2su_12" style="font-family:Sandoll Nemony2;font-size:15px;font-weight:400">채팅</h2><div class="_wrapper_1e2su_28 _scale_1e2su_39"><button>−</button><span>100%</span><button>+</button></div><div class="_wrapper_1e2su_28 _menu_1e2su_36"><button>메뉴</button></div>';
    rect(header, 480, 44); page.body.appendChild(header); return header;
  };
  it("fits the official popup and extends its scale down to 50% without replacing chat or menu", async () => {
    const f = fixture({ chat: true }), header = chatHeader(f.page);
    const wrapper = f.page.createElement('main'); wrapper.className = '_container_1gtzb_1';
    const root = f.page.createElement('aside'); root.id = 'aside-chatting'; root.className = '_container_b8csn_2 _is_popup_chat_b8csn_13';
    const input = f.page.querySelector('textarea')!; input.value = '작성 중';
    root.append(header, input); wrapper.append(root); f.page.body.append(wrapper);
    const original = header.querySelector('._scale_1e2su_39')!, originalButtons = original.querySelectorAll('button');
    const nativeValue = original.querySelector('span')!; nativeValue.className = '_scale_text_1e2su_46';
    const nativePlus = vi.fn(() => { nativeValue.textContent = '125%'; });
    originalButtons[1]!.addEventListener('click', nativePlus);
    const menu = header.querySelector('._menu_1e2su_36')!, menuAction = vi.fn(); menu.querySelector('button')!.addEventListener('click', menuAction);
    f.pageWindow.__atsumiMultiView!.configureChat({ channelId: ID, number: 1, channelName: '테스트' }); f.advance();
    const minus = header.querySelector<HTMLButtonElement>('.atsumi-chat-scale button')!;
    const plus = header.querySelector<HTMLButtonElement>('.atsumi-chat-scale button:last-child')!;
    for (let i = 0; i < 10; i++) minus.click();
    expect(f.page.body.style.getPropertyValue('--atsumi-chat-zoom')).toBe('0.5');
    expect(header.querySelector('.atsumi-chat-scale output')?.textContent).toBe('50%'); expect(minus.disabled).toBe(true);
    expect(wrapper.hasAttribute('data-atsumi-chat-layout')).toBe(true);
    expect(original.hasAttribute('data-atsumi-chat-scale-original')).toBe(true);
    expect(input.value).toBe('작성 중'); expect(input.parentElement).toBe(root); expect(menu.parentElement).toBe(header);
    for (let i = 0; i < 5; i++) plus.click();
    expect(f.page.body.style.getPropertyValue('--atsumi-chat-zoom')).toBe('1'); expect(nativePlus).not.toHaveBeenCalled();
    plus.click(); await Promise.resolve(); f.advance();
    expect(nativePlus).toHaveBeenCalledOnce(); expect(header.querySelector('.atsumi-chat-scale output')?.textContent).toBe('125%');
    menu.querySelector('button')!.click(); expect(menuAction).toHaveBeenCalledOnce();
    f.again(); f.advance(); expect(header.querySelectorAll('.atsumi-chat-scale')).toHaveLength(1);
    f.pageWindow.location = new URL('https://chzzk.naver.com/'); f.pageWindow.dispatchEvent(new Event('resize')); f.advance();
    expect(f.page.body.hasAttribute('data-atsumi-chat-fit')).toBe(false);
    expect(original.hasAttribute('data-atsumi-chat-scale-original')).toBe(false);
    expect(header.querySelector('.atsumi-chat-scale')).toBeNull();
  });
  it("places a plain-text channel label inside the original chat heading, preserving its controls and typography", () => {
    const f = fixture({ chat: true }); const header = chatHeader(f.page), heading = header.querySelector('h2')!;
    const controls = [...header.querySelectorAll('button')], input = f.page.querySelector('textarea');
    f.pageWindow.__atsumiMultiView!.configureChat({ channelId: ID, number: 2, channelName: '로션욤' }); f.advance();
    const label = header.querySelector<HTMLElement>('.atsumi-mado-chat-channel')!;
    expect(label.parentElement).toBe(heading); expect(label.textContent).toBe('2.로션욤'); expect(label.title).toBe('2.로션욤');
    expect(heading.firstChild?.textContent).toBe('채팅'); expect(heading.style.fontSize).toBe('15px');
    expect(f.page.querySelector('style')?.textContent).toContain('font:inherit');
    expect([...header.querySelectorAll('button')]).toEqual(controls); expect(f.page.querySelector('textarea')).toBe(input);
    f.pageWindow.__atsumiMultiView!.configureChat({ channelId: ID, number: 2, channelName: '<img src=x onerror=alert(1)>' });
    expect(label.textContent).toBe('2.<img src=x onerror=alert(1)>'); expect(header.querySelector('img')).toBeNull();
    expect(f.video.muted).toBe(true); expect(f.state()?.kind).toBe('chat');
  });
  it("reinstates the label when the official header mounts late or gets replaced without duplicating it", async () => {
    const f = fixture({ chat: true });
    f.pageWindow.__atsumiMultiView!.configureChat({ channelId: ID, number: 3, channelName: '채널' });
    const header = chatHeader(f.page); await Promise.resolve(); f.advance(); await Promise.resolve();
    expect(header.querySelector('.atsumi-mado-chat-channel')?.textContent).toBe('3.채널');
    const replacement = header.querySelector('h2')!.cloneNode(false) as HTMLElement; replacement.textContent = '채팅';
    header.querySelector('h2')!.replaceWith(replacement); await Promise.resolve(); f.advance(); await Promise.resolve();
    f.again(); f.pageWindow.dispatchEvent(new Event('resize')); f.advance();
    expect(header.querySelectorAll('.atsumi-mado-chat-channel')).toHaveLength(1);
    expect(replacement.textContent).toBe('채팅3.채널');
  });
  it("reserves the original scale and menu space in a narrow chat instead of shrinking fonts", () => {
    const f = fixture({ chat: true }); const header = chatHeader(f.page); rect(header, 220, 44);
    vi.spyOn(header.querySelector('._scale_1e2su_39')!, 'getBoundingClientRect').mockReturnValue(new DOMRect(96, 10, 84, 25));
    vi.spyOn(header.querySelector('._menu_1e2su_36')!, 'getBoundingClientRect').mockReturnValue(new DOMRect(180, 0, 40, 44));
    f.pageWindow.__atsumiMultiView!.configureChat({ channelId: ID, number: 4, channelName: '아주 긴 채널 이름' });
    const heading = header.querySelector('h2')!;
    expect(heading.style.getPropertyValue('--atsumi-chat-title-inset')).toBe('74px');
    expect(heading.style.getPropertyValue('--atsumi-chat-label-width')).toBe('36px');
    expect(heading.style.fontSize).toBe('15px');
  });
  it("rejects mismatched or invalid chat metadata and never adds labels to a video page", () => {
    const f = fixture({ chat: true }); chatHeader(f.page);
    for (const value of [{ channelId: 'b'.repeat(32), number: 2, channelName: '다른 채널' },
      { channelId: ID, number: 5, channelName: '범위 밖' }, { channelId: ID, number: 1, channelName: 'x'.repeat(161) },
      { channelId: ID, number: 1, channelName: '잘못된\n이름' }]) f.pageWindow.__atsumiMultiView!.configureChat(value);
    expect(f.page.querySelector('.atsumi-mado-chat-channel')).toBeNull();
    const video = fixture(); chatHeader(video.page); video.pageWindow.__atsumiMultiView!.configureChat({ channelId: ID, number: 1, channelName: '방송' });
    expect(video.page.querySelector('.atsumi-mado-chat-channel')).toBeNull();
  });
  it("is idempotent without a redundant presentation toggle and preserves host cleanup", () => {
    const f = fixture(); f.advance(); f.again(); f.advance();
    expect(f.page.querySelectorAll("#atsumi-mado-toggle")).toHaveLength(0);
    expect(f.page.querySelector('style')?.textContent).toContain('button.pzp-pc__viewmode-button[aria-label="좁은 화면"] {display:none!important;}');
    f.pageWindow.__atsumiMultiView!.setVideoOnly(false);
    expect(f.state()).toMatchObject({ active: false, reason: "original" });
    expect(f.page.body.hasAttribute("data-atsumi-mado")).toBe(false);
  });
  it("does not restore the layout for frequent progress, volume or chat mutations", async () => {
    const f = fixture(); f.advance(); await Promise.resolve();
    const restore = vi.spyOn(f.page.body, "removeAttribute");
    const control = f.page.querySelector<HTMLElement>("#official")!;
    const chat = f.page.querySelector("aside")!;
    for (let i = 0; i < 30; i++) {
      control.style.width = `${i}px`; control.className = `volume-${i}`;
      chat.className = `messages-${i}`;
      const message = f.page.createElement("div"); message.textContent = "synthetic"; chat.appendChild(message);
    }
    await Promise.resolve(); f.advance();
    expect(restore).not.toHaveBeenCalled(); expect(f.state()?.active).toBe(true);
  });
  it("preserves a dialog revealed by its ancestor without restoring the full site", async () => {
    const f = fixture();
    const wrapper = f.page.createElement("section");
    const dialog = f.page.createElement("div"); dialog.setAttribute("role", "dialog");
    const size = rect(dialog, 0, 0); wrapper.appendChild(dialog); f.page.body.appendChild(wrapper);
    f.advance(); await Promise.resolve(); expect(f.state()?.active).toBe(true);
    size.mockReturnValue({ x: 0, y: 0, left: 0, top: 0, width: 400, height: 200, right: 400, bottom: 200, toJSON: () => ({}) });
    wrapper.style.visibility = "visible";
    await Promise.resolve(); f.advance();
    expect(f.state()).toMatchObject({ active: true, reason: "video_only" });
    expect(wrapper.hasAttribute('data-atsumi-mado-path')).toBe(true);
    expect(wrapper.hasAttribute('data-atsumi-mado-dialog-path')).toBe(true);
  });
  it("contains no capture, token, websocket, IPC or fetch paths", () => {
    for (const forbidden of ["captureStream", "MediaRecorder", "postMessage", "document.cookie", "localStorage", "WebSocket", "fetch(", "window.open", "currentTime ="]) expect(source).not.toContain(forbidden);
  });
});
