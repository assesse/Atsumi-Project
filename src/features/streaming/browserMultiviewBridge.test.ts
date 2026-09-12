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
  __atsumiMultiView?: { getState(): State };
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
  const audio = (enabled: boolean) => pageWindow.dispatchEvent(new CustomEvent("atsumi-multiview-audio", { detail: { enabled } }));
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
