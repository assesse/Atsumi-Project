import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import receiver from "../../../src-tauri/src/streaming/browser_auto_view.js?raw";
import toast from "../../../src-tauri/src/streaming/browser_recording_notice.js?raw";

const vmName = "node:vm";
const { runInNewContext } = await import(vmName) as { runInNewContext(source: string, context: Record<string, unknown>): unknown };
beforeEach(() => { vi.useFakeTimers(); });
afterEach(() => { vi.useRealTimers(); });
describe("background receiver and recording notices", () => {
  function standardFixture(markup = '<button>설치없이 일반 화질 시청</button>') {
    const page = document.implementation.createHTMLDocument();
    page.body.innerHTML = markup;
    for (const node of page.querySelectorAll('button,a,[role="button"]')) {
      Object.defineProperty(node, "getClientRects", { value: () => node.hasAttribute("hidden") ? [] : [{}] });
    }
    const useStandardQuality = vi.fn();
    const pageWindow = Object.assign(new EventTarget(), {
      getComputedStyle: (node: Element) => ({ display: node.hasAttribute("hidden") ? "none" : "block", visibility: "visible" }),
      __atsumiQuality: { useStandardQuality },
      __atsumiEncodedCapture: { getStatus: () => ({ active: false }) },
      __atsumiAutoReceiver: undefined as undefined | { prepareRecording(): void; configure(value: { revision: number; viewing: boolean }): void; getPreparationStatus(): string },
    });
    runInNewContext(receiver, { document: page, window: pageWindow, location: new URL("https://chzzk.naver.com/live/" + "a".repeat(32)), setInterval, clearInterval });
    const click = vi.fn(); page.addEventListener("click", click);
    return { page, pageWindow, click, useStandardQuality, api: pageWindow.__atsumiAutoReceiver! };
  }
  it("starts install-free preparation without first opening live, once per receiver document", async () => {
    const f = standardFixture();
    await vi.advanceTimersByTimeAsync(3000);
    expect(f.click).not.toHaveBeenCalled(); // Viewing-only creation is not an opt-in.
    f.api.prepareRecording();
    await vi.advanceTimersByTimeAsync(3000);
    expect(f.click).toHaveBeenCalledOnce(); expect(f.useStandardQuality).toHaveBeenCalledOnce();
    expect(f.api.getPreparationStatus()).toBe("standard_quality_selected");
    await vi.advanceTimersByTimeAsync(90_000);
    expect(f.click).toHaveBeenCalledOnce();
  });
  it("leaves login, permissions, installation and ambiguous/invisible choices to the user", async () => {
    for (const markup of [
      '<button>로그인</button><button>설치</button><button>권한 허용</button><button>확인</button>',
      '<button disabled>설치없이 일반 화질 시청</button>',
      '<button aria-disabled="true">설치없이 일반 화질 시청</button>',
      '<button hidden>설치없이 일반 화질 시청</button>',
      '<button>설치없이 일반 화질 시청</button><button>설치없이 일반 화질 시청</button>',
      '<a href="https://example.com/download">설치없이 일반 화질 시청</a>',
    ]) {
      const f = standardFixture(markup); f.api.prepareRecording();
      await vi.advanceTimersByTimeAsync(6000);
      expect(f.click).not.toHaveBeenCalled(); expect(f.useStandardQuality).not.toHaveBeenCalled();
      f.pageWindow.dispatchEvent(new Event("pagehide"));
    }
  });
  it("never downgrades playable video or changes quality during an active recording", async () => {
    const f = standardFixture('<video></video><button>설치없이 일반 화질 시청</button>');
    const video = f.page.querySelector("video")!;
    Object.defineProperties(video, { readyState: { value: 4, configurable: true }, videoWidth: { value: 1920 } });
    vi.spyOn(video, "play").mockResolvedValue();
    f.api.prepareRecording(); await vi.advanceTimersByTimeAsync(3000);
    expect(f.click).not.toHaveBeenCalled(); expect(f.api.getPreparationStatus()).toBe("video_ready");
    Object.defineProperty(video, "readyState", { value: 0 });
    f.pageWindow.__atsumiEncodedCapture.getStatus = () => ({ active: true });
    await vi.advanceTimersByTimeAsync(3000); expect(f.click).not.toHaveBeenCalled();
  });
  it("does not click a quality choice while the receiver is being watched", async () => {
    const f = standardFixture(); f.api.prepareRecording();
    f.api.configure({ revision: 1, viewing: true });
    await vi.advanceTimersByTimeAsync(6000); expect(f.click).not.toHaveBeenCalled();
    f.api.configure({ revision: 2, viewing: false });
    await vi.advanceTimersByTimeAsync(3000); expect(f.click).toHaveBeenCalledOnce();
  });
  it("records a failed choice without retrying it indefinitely", async () => {
    const f = standardFixture(); f.api.prepareRecording();
    const button = f.page.querySelector("button")!;
    const click = vi.spyOn(button, "click").mockImplementation(() => { throw new Error("upstream changed"); });
    await vi.advanceTimersByTimeAsync(9000);
    expect(f.api.getPreparationStatus()).toBe("standard_quality_failed"); expect(click).toHaveBeenCalledOnce();
  });
  it("retains playback and user mute in a hidden tab or PiP while its watch remains attached", async () => {
    const page = document.implementation.createHTMLDocument(); page.body.innerHTML = '<video></video>';
    const video = page.querySelector("video")!;
    const play = vi.spyOn(video, "play").mockResolvedValue();
    Object.defineProperties(page, { hidden: { value: true }, pictureInPictureElement: { value: video } });
    const pageWindow = Object.assign(new EventTarget(), { __atsumiAutoReceiver: undefined as undefined | { configure(value: { revision: number; viewing: boolean }): void } });
    runInNewContext(receiver, { document: page, window: pageWindow, location: new URL("https://chzzk.naver.com/live/" + "a".repeat(32)), setInterval, clearInterval });
    pageWindow.__atsumiAutoReceiver!.configure({ revision: 1, viewing: true });
    video.volume = 0.4; video.muted = false;
    await vi.advanceTimersByTimeAsync(9000);
    expect(video.muted).toBe(false); expect(video.volume).toBe(0.4);
    video.muted = true;
    pageWindow.__atsumiAutoReceiver!.configure({ revision: 2, viewing: true });
    await vi.advanceTimersByTimeAsync(9000);
    expect(video.muted).toBe(true); expect(play).not.toHaveBeenCalled();
    expect(page.pictureInPictureElement).toBe(video);
  });
  it("does not repeatedly remute a borrowed player and rejects late presentation commands", async () => {
    const page = document.implementation.createHTMLDocument(); page.body.innerHTML = '<video></video>';
    const video = page.querySelector("video")!; const play = vi.spyOn(video, "play").mockResolvedValue();
    const configure = vi.fn(), layout = vi.fn();
    const pageWindow = Object.assign(new EventTarget(), { __atsumiPlayerUI: { configure }, __atsumiMultiView: { setVideoOnly: layout }, __atsumiAutoReceiver: undefined as undefined | { configure(value: { revision: number; viewing: boolean }): void } });
    runInNewContext(receiver, { document: page, window: pageWindow, location: new URL("https://chzzk.naver.com/live/" + "a".repeat(32)), setInterval, clearInterval });
    pageWindow.__atsumiAutoReceiver!.configure({ revision: 2, viewing: true });
    video.muted = false; await vi.advanceTimersByTimeAsync(6000); expect(video.muted).toBe(false);
    expect(play).not.toHaveBeenCalled();
    expect(configure).toHaveBeenLastCalledWith({ automaticWatch: false, multiview: false });
    expect(layout).toHaveBeenLastCalledWith(false);
    pageWindow.__atsumiAutoReceiver!.configure({ revision: 1, viewing: false });
    await vi.advanceTimersByTimeAsync(3000); expect(video.muted).toBe(false);
    pageWindow.__atsumiAutoReceiver!.configure({ revision: 3, viewing: false });
    await vi.advanceTimersByTimeAsync(3000); expect(video.muted).toBe(true);
    expect(play).toHaveBeenCalledOnce();
    expect(configure).toHaveBeenLastCalledWith({ automaticWatch: true, multiview: false });
    pageWindow.__atsumiAutoReceiver!.configure({ revision: 4, viewing: true });
    expect(video.muted).toBe(false);
    video.muted = true;
    pageWindow.__atsumiAutoReceiver!.configure({ revision: 5, viewing: true });
    expect(video.muted).toBe(true); // Geometry never overrides the official mute button.
    pageWindow.__atsumiAutoReceiver!.configure({ revision: 6, viewing: false });
    pageWindow.__atsumiAutoReceiver!.configure({ revision: 7, viewing: true });
    expect(video.muted).toBe(true);
    expect(page.querySelector("video")).toBe(video);
  });
  it("only mutes and resumes the existing official video; never clicks access gates", async () => {
    const page = document.implementation.createHTMLDocument();
    page.body.innerHTML = '<video></video><button>로그인</button><button>확장 설치</button>';
    const video = page.querySelector("video")!;
    const play = vi.spyOn(video, "play").mockResolvedValue();
    const click = vi.fn(); page.addEventListener("click", click);
    const pageWindow = Object.assign(new EventTarget(), { __atsumiAutoReceiver: false });
    const context = { document: page, window: pageWindow, location: new URL("https://chzzk.naver.com/live/" + "a".repeat(32)), setInterval, clearInterval };
    runInNewContext(receiver, context); runInNewContext(receiver, context);
    await vi.advanceTimersByTimeAsync(3000);
    expect(play).toHaveBeenCalledOnce(); expect(video.muted).toBe(true); expect(click).not.toHaveBeenCalled();
    pageWindow.dispatchEvent(new Event("pagehide")); await vi.advanceTimersByTimeAsync(3000); expect(play).toHaveBeenCalledOnce();
  });
  it("shows a single non-modal bottom-right toast without altering playback, focus or markup", async () => {
    const page = document.implementation.createHTMLDocument(); page.body.innerHTML = '<video></video><input>';
    const video = page.querySelector("video")!; const pause = vi.spyOn(video, "pause");
    const context = { document: page, location: new URL("https://chzzk.naver.com"), setTimeout };
    runInNewContext(`(${toast})("<b>녹화 시작</b>",18,18)`, context);
    const notice = page.querySelector<HTMLElement>('[role="status"]')!;
    expect(notice.textContent).toBe("<b>녹화 시작</b>"); expect(notice.querySelector("b")).toBeNull();
    expect(notice.style.pointerEvents).toBe("none"); expect(notice.style.right).toBe("18px"); expect(notice.style.bottom).toBe("18px");
    runInNewContext(`(${toast})("녹화 종료",18,18)`, context);
    expect(page.querySelectorAll('[role="status"]')).toHaveLength(1); expect(pause).not.toHaveBeenCalled();
    expect(page.querySelector("video")).toBe(video); expect(page.querySelector('[role="dialog"],[role="alertdialog"]')).toBeNull();
    await vi.advanceTimersByTimeAsync(2601); expect(page.querySelector('[role="status"]')).toBeNull();
  });
});
