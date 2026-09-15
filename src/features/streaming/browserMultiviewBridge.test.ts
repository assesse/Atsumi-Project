import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import source from "../../../src-tauri/src/streaming/browser_multiview.js?raw";

const vmName = "node:vm";
const { runInNewContext } = await import(vmName) as {
  runInNewContext(source: string, context: Record<string, unknown>): unknown;
};
const ID = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
type State = { kind: string; active: boolean; reason: string; audioEnabled: boolean };
type PageWindow = EventTarget & {
  location: URL; top: unknown; innerWidth: number; innerHeight: number;
  getComputedStyle: typeof window.getComputedStyle;
  __atsumiMultiView?: { getState(): State; configureChat(value: { channelId: string; number: number; channelName: string }): void };
};
const observers: MutationObserver[] = [];
const rect = (node: Element, width = 900, height = 600) => vi.spyOn(node, "getBoundingClientRect").mockReturnValue({
  x: 0, y: 0, left: 0, top: 0, width, height, right: width, bottom: height, toJSON: () => ({}),
});
function fixture(options: { chat?: boolean; url?: string; nested?: boolean; width?: number; height?: number } = {}) {
  const page = document.implementation.createHTMLDocument("isolated multiview fixture");
  page.body.innerHTML = `<header>Original</header><main><div class="pzp-pc"><div class="translated"><div class="aspect"><video></video></div></div><button id="official">Control</button></div></main><aside><textarea placeholder="채팅 입력"></textarea></aside>`;
  const pageWindow = Object.assign(new EventTarget(), {
    location: new URL(options.url ?? `https://chzzk.naver.com/live/${ID}${options.chat ? "/chat" : ""}`), top: null as unknown,
    innerWidth: options.width ?? 640, innerHeight: options.height ?? 360,
    getComputedStyle: window.getComputedStyle.bind(window),
  }) as PageWindow;
  pageWindow.top = options.nested ? {} : pageWindow;
  const video = page.querySelector("video")!;
  Object.defineProperties(video, { readyState: { value: 4 }, videoWidth: { value: 1920 }, videoHeight: { value: 1080 }, paused: { value: false } });
  const pause = vi.spyOn(video, "pause").mockImplementation(() => undefined);
  rect(video); rect(page.querySelector(".pzp-pc")!);
  class Observer extends MutationObserver {
    constructor(callback: MutationCallback) { super(callback); observers.push(this); }
  }
  const context = { window: pageWindow, document: page, MutationObserver: Observer, CustomEvent, setTimeout, clearTimeout };
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
  it("restores the original page when a dialog becomes visible", () => {
    const f = fixture(); f.advance();
    const dialog = f.page.createElement("div"); dialog.setAttribute("role", "dialog"); rect(dialog, 400, 200); f.page.body.appendChild(dialog);
    f.pageWindow.dispatchEvent(new Event("resize")); f.advance();
    expect(f.state()).toMatchObject({ active: false, reason: "dialog_visible" });
    expect(f.page.querySelectorAll("[data-atsumi-mado-media]")).toHaveLength(0);
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
    (f.page.querySelector('#atsumi-mado-toggle') as HTMLButtonElement).click();
    expect(originalWideAction).toHaveBeenCalledTimes(2); expect(wide.getAttribute('aria-label')).toBe('넓은 화면');
    expect(f.state()?.active).toBe(false); expect(f.video.muted).toBe(false);
    (f.page.querySelector('#atsumi-mado-toggle') as HTMLButtonElement).click();
    expect(originalWideAction).toHaveBeenCalledTimes(3); expect(f.state()?.active).toBe(true);
  });
  it("preserves an already-wide official player when returning to the original page", () => {
    const f = fixture();
    const wide = f.page.createElement('button'); wide.className = 'pzp-pc__viewmode-button'; wide.setAttribute('aria-label', '좁은 화면');
    f.page.querySelector('.pzp-pc')!.appendChild(wide); const click = vi.fn(); wide.addEventListener('click', click);
    f.advance(); (f.page.querySelector('#atsumi-mado-toggle') as HTMLButtonElement).click();
    expect(click).not.toHaveBeenCalled(); expect(wide.getAttribute('aria-label')).toBe('좁은 화면');
  });
  it("initializes official metadata when the native wide control arrives after the video", async () => {
    const f = fixture(); f.advance(); await Promise.resolve();
    const wide = f.page.createElement('button'); wide.className = 'pzp-pc__viewmode-button'; wide.setAttribute('aria-label', '넓은 화면');
    const click = vi.fn(); wide.addEventListener('click', click); f.page.querySelector('.pzp-pc')!.appendChild(wide);
    await Promise.resolve(); f.advance(); expect(click).toHaveBeenCalledOnce();
    for (let index = 0; index < 3; index++) { f.pageWindow.dispatchEvent(new Event('resize')); f.advance(); }
    expect(click).toHaveBeenCalledOnce();
  });
  it("does not force a video layout into an undersized viewport", () => {
    const f = fixture({ width: 159, height: 89 }); f.advance();
    expect(f.state()).toMatchObject({ active: false, reason: "viewport_too_small" }); expect(f.video.muted).toBe(true);
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
  const chatHeader = (page: Document) => {
    const header = page.createElement("div"); header.className = "_container_1e2su_2";
    header.innerHTML = '<h2 class="_title_1e2su_12" style="font-family:Sandoll Nemony2;font-size:15px;font-weight:400">채팅</h2><div class="_wrapper_1e2su_28 _scale_1e2su_39"><button>−</button><span>100%</span><button>+</button></div><div class="_wrapper_1e2su_28 _menu_1e2su_36"><button>메뉴</button></div>';
    rect(header, 480, 44); page.body.appendChild(header); return header;
  };
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
  it("is idempotent and allows a reversible original-page toggle", () => {
    const f = fixture(); f.advance(); f.again(); f.advance();
    expect(f.page.querySelectorAll("#atsumi-mado-toggle")).toHaveLength(1);
    (f.page.querySelector("#atsumi-mado-toggle") as HTMLButtonElement).click();
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
  it("still restores for an existing dialog revealed by an ancestor style", async () => {
    const f = fixture();
    const wrapper = f.page.createElement("section");
    const dialog = f.page.createElement("div"); dialog.setAttribute("role", "dialog");
    const size = rect(dialog, 0, 0); wrapper.appendChild(dialog); f.page.body.appendChild(wrapper);
    f.advance(); await Promise.resolve(); expect(f.state()?.active).toBe(true);
    size.mockReturnValue({ x: 0, y: 0, left: 0, top: 0, width: 400, height: 200, right: 400, bottom: 200, toJSON: () => ({}) });
    wrapper.style.visibility = "visible";
    await Promise.resolve(); f.advance();
    expect(f.state()).toMatchObject({ active: false, reason: "dialog_visible" });
  });
  it("contains no capture, token, websocket, IPC or fetch paths", () => {
    for (const forbidden of ["captureStream", "MediaRecorder", "postMessage", "document.cookie", "localStorage", "WebSocket", "fetch(", "window.open", "currentTime ="]) expect(source).not.toContain(forbidden);
  });
});
