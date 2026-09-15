import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import receiver from "../../../src-tauri/src/streaming/browser_auto_view.js?raw";
import toast from "../../../src-tauri/src/streaming/browser_recording_notice.js?raw";

const vmName = "node:vm";
const { runInNewContext } = await import(vmName) as { runInNewContext(source: string, context: Record<string, unknown>): unknown };
beforeEach(() => { vi.useFakeTimers(); });
afterEach(() => { vi.useRealTimers(); });
describe("background receiver and recording notices", () => {
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
