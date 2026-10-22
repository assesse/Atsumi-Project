import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { PinterestOfficialSearch } from "./PinterestOfficialSearch";
import { createPinterestBrowserApi, pinterestBrowserRect, pinterestBrowserSurfaceRect, type PinterestBrowserApi, type PinterestBrowserSnapshot } from "./browserApi";

const pinId = "1076430748452447388";
const snapshot: PinterestBrowserSnapshot = { epoch: 7, open: true, loading: false, currentPinId: pinId, error: null };
const makeApi = (): PinterestBrowserApi => ({ open: vi.fn().mockResolvedValue(snapshot), snapshot: vi.fn().mockResolvedValue(snapshot), viewport: vi.fn().mockResolvedValue(undefined), select: vi.fn().mockResolvedValue(pinId), action: vi.fn().mockResolvedValue(undefined) });
let root: Root, container: HTMLDivElement;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({ left: 200, top: 180, right: 900, bottom: 600, x: 200, y: 180, width: 700, height: 420, toJSON: () => ({}) });
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const button = (text: string) => [...container.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === text || item.getAttribute("aria-label") === text)!;
const render = async (api: PinterestBrowserApi, opts: { active?: boolean; privacyMode?: boolean; onImport?: (id: string) => Promise<void>; onReturn?: () => void; query?: string; revision?: number } = {}) => {
  await act(async () => root.render(<PinterestOfficialSearch api={api} active={opts.active ?? true} privacyMode={opts.privacyMode ?? false} query={opts.query ?? "landscape"} revision={opts.revision ?? 1} onImport={opts.onImport ?? (async () => {})} onReturn={opts.onReturn ?? (() => {})} />));
};
describe("isolated Pinterest official search", () => {
  it("clips a native rectangle within the main app viewport", () => {
    expect(pinterestBrowserRect({ left: -40, top: 30, right: 1200, bottom: 900 }, 960, 640)).toEqual({ x: 0, y: 30, width: 960, height: 610, visible: true });
    expect(pinterestBrowserRect({ left: NaN, top: 0, right: 100, bottom: 100 }, 960, 640).visible).toBe(false);
  });
  it("does not pretend official WebView exists in browser mock", async () => {
    await expect(createPinterestBrowserApi("browser-mock").open("tree")).rejects.toThrow("데스크톱");
  });
  it("clips a scrolled native surface to CSS overflow ancestors, not over the app toolbar", () => {
    const parent = document.createElement("div"), child = document.createElement("div"); parent.style.overflowX = "hidden"; parent.style.overflowY = "hidden"; parent.append(child); container.append(parent);
    parent.getBoundingClientRect = vi.fn().mockReturnValue({ left: 180, top: 80, right: 940, bottom: 620 } as DOMRect);
    child.getBoundingClientRect = vi.fn().mockReturnValue({ left: 200, top: 20, right: 980, bottom: 740 } as DOMRect);
    expect(getComputedStyle(parent).overflowX).toBe("hidden");
    expect(parent.getBoundingClientRect().top).toBe(80);
    expect(pinterestBrowserSurfaceRect(child, 960, 640)).toEqual({ x: 200, y: 80, width: 740, height: 540, visible: true });
  });
  it("opens once, hides immediately for privacy/inactivity, then resumes the same epoch", async () => {
    const api = makeApi(); await render(api);
    expect(api.open).toHaveBeenCalledExactlyOnceWith("landscape");
    expect(api.viewport).toHaveBeenLastCalledWith(expect.objectContaining({ epoch: 7, visible: true }));
    await render(api, { privacyMode: true });
    expect(api.viewport).toHaveBeenLastCalledWith(expect.objectContaining({ visible: false }));
    await render(api, { active: false });
    await render(api);
    expect(api.open).toHaveBeenCalledTimes(1);
    expect(api.viewport).toHaveBeenLastCalledWith(expect.objectContaining({ epoch: 7, visible: true }));
  });
  it("never imports from page messages; explicit current-pin selection hands off only the native ID", async () => {
    const api = makeApi(), onImport = vi.fn().mockResolvedValue(undefined);
    await render(api, { onImport });
    window.dispatchEvent(new MessageEvent("message", { data: { pinId: "12345", action: "download" }, origin: "https://www.pinterest.com" }));
    expect(api.select).not.toHaveBeenCalled();
    await act(async () => button("핀 가져오기").click());
    expect(api.select).toHaveBeenCalledExactlyOnceWith(7, pinId);
    expect(onImport).toHaveBeenCalledExactlyOnceWith(pinId);
    expect(api.viewport).toHaveBeenCalledWith(expect.objectContaining({ visible: false }));
  });
  it("rejects stale selection without importing and returns to its preserved official view", async () => {
    const api = makeApi(), onImport = vi.fn(); vi.mocked(api.select).mockRejectedValue("이전 핀입니다.");
    await render(api, { onImport });
    await act(async () => button("핀 가져오기").click());
    expect(onImport).not.toHaveBeenCalled();
    expect(container.querySelector('[role="alert"]')).toHaveTextContent("이전 핀");
    expect(api.viewport).toHaveBeenLastCalledWith(expect.objectContaining({ visible: true }));
  });
  it("does not let a late selected ID import after the user returned to the native list", async () => {
    const api = makeApi(), onImport = vi.fn(), onReturn = vi.fn(); let finish!: (id: string) => void;
    vi.mocked(api.select).mockImplementation(() => new Promise(resolve => { finish = resolve; }));
    await render(api, { onImport, onReturn });
    await act(async () => button("핀 가져오기").click());
    await act(async () => button("목록 보기").click());
    await act(async () => finish(pinId));
    expect(onReturn).toHaveBeenCalledOnce(); expect(onImport).not.toHaveBeenCalled();
  });
  it("hides for an app modal and never recreates or navigates the official page", async () => {
    const api = makeApi(); await render(api);
    const dialog = document.createElement("div"); dialog.setAttribute("role", "dialog"); dialog.setAttribute("aria-modal", "true");
    try {
      await act(async () => { document.body.append(dialog); await Promise.resolve(); });
      expect(api.viewport).toHaveBeenLastCalledWith(expect.objectContaining({ visible: false }));
      await act(async () => { dialog.remove(); await Promise.resolve(); });
      expect(api.viewport).toHaveBeenLastCalledWith(expect.objectContaining({ visible: true }));
      expect(api.open).toHaveBeenCalledOnce();
    } finally { dialog.remove(); }
  });
  it("closes a late-created obsolete epoch instead of displaying it over the newer search", async () => {
    const api = makeApi(); let finish!: (value: PinterestBrowserSnapshot) => void;
    vi.mocked(api.open).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; })).mockResolvedValueOnce({ ...snapshot, epoch: 8 });
    await render(api); await render(api, { query: "trees", revision: 2 });
    await act(async () => finish(snapshot));
    expect(api.action).toHaveBeenCalledWith(7, "close");
    expect(api.viewport).toHaveBeenLastCalledWith(expect.objectContaining({ epoch: 8 }));
  });
  it("hides behind shared toast and observes style changes without navigating again", async () => {
    const api = makeApi(); await render(api);
    const toast = document.createElement("div"); toast.className = "toast"; toast.style.display = "none";
    try {
      await act(async () => { document.body.append(toast); });
      expect(api.viewport).toHaveBeenLastCalledWith(expect.objectContaining({ visible: true }));
      await act(async () => { toast.style.display = "block"; });
      expect(api.viewport).toHaveBeenLastCalledWith(expect.objectContaining({ visible: false }));
      await act(async () => { toast.style.display = "none"; });
      expect(api.viewport).toHaveBeenLastCalledWith(expect.objectContaining({ visible: true }));
      expect(api.open).toHaveBeenCalledOnce();
    } finally { toast.remove(); }
  });
  it("does not let an older action settle a newer opening request", async () => {
    const api = makeApi(); let actionDone!: () => void, openDone!: (value: PinterestBrowserSnapshot) => void;
    await render(api);
    vi.mocked(api.action).mockImplementationOnce(() => new Promise(resolve => { actionDone = resolve; }));
    await act(async () => button("공식 검색 새로고침").click());
    vi.mocked(api.open).mockImplementationOnce(() => new Promise(resolve => { openDone = resolve; }));
    await render(api, { query: "trees", revision: 2 });
    await act(async () => actionDone());
    expect(container.querySelector('[aria-label="공식 창 여는 중"]')).not.toBeNull();
    expect(button("공식 검색 새로고침")).toBeDisabled();
    await act(async () => openDone({ ...snapshot, epoch: 8 }));
    expect(button("공식 검색 새로고침")).not.toBeDisabled();
  });
  it("shows actual service failures and offers retry without a never-ending spinner", async () => {
    const api = makeApi(); vi.mocked(api.open).mockRejectedValueOnce("연결하지 못했습니다.");
    await render(api); expect(container.querySelector('[role="alert"]')).toHaveTextContent("연결하지");
    expect(container.querySelector('[aria-label="공식 창 여는 중"]')).toBeNull();
    await act(async () => button("공식 검색 새로고침").click());
    expect(api.open).toHaveBeenCalledTimes(2);
  });
});
