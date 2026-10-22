import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PinterestWorkspace } from "./PinterestWorkspace";
import { createPinterestApi } from "./api";
import { type PinterestBrowserApi } from "./browserApi";
import type { PinterestAccountApi, PinterestAccountSnapshot } from "./accountApi";
import { loadPinterestFavorites, savePinterestFavorites, PINTEREST_FAVORITES_KEY } from "./storage";
import { normalizePinterestInput, selectPinterestCard, type PinterestApi, type PinterestDownloadOutcome, type PinterestPin, type PinterestView } from "./types";

const pin = (id = "621004236103700020", title = "산 풍경"): PinterestPin => ({ id, title, description: "공개 이미지", sourceUrl: `https://www.pinterest.com/pin/${id}/`, creatorName: "작가", media: [{ url: "https://i.pinimg.com/originals/a.jpg" }], unsupportedVideo: false });
const mockApi = (): PinterestApi => ({ removed: vi.fn().mockResolvedValue([]), discover: vi.fn().mockResolvedValue({ title: "Pinterest", path: "/pinterest/", pins: [pin()], boards: [], limited: false }), libraryAction: vi.fn().mockResolvedValue("621004236103700020-abc"), get: vi.fn().mockImplementation(async (id: string) => pin(id)), preview: vi.fn().mockResolvedValue("data:image/jpeg;base64,YQ=="), download: vi.fn().mockImplementation(async (id: string) => ({ pin: pin(id), files: ["001.jpg"], bytes: 100, savedAt: 1 })), downloadBatch: vi.fn().mockImplementation(async (ids: string[], _requestId: string, onProgress: (outcome: PinterestDownloadOutcome) => void) => ids.map(pinId => { const outcome: PinterestDownloadOutcome = { pinId, status: "completed", record: { pin: pin(pinId), files: ["001.jpg"], bytes: 100, savedAt: 1 } }; onProgress(outcome); return outcome; })), downloads: vi.fn().mockResolvedValue([]), cancel: vi.fn().mockResolvedValue(undefined), openExternal: vi.fn().mockResolvedValue(undefined) });
let root: Root, container: HTMLDivElement;
beforeEach(() => { localStorage.clear(); vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); container = document.createElement("div"); document.body.append(container); root = createRoot(container); });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const button = (label: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find(node => node.getAttribute("aria-label") === label || node.textContent === label)!;
async function render(api: PinterestApi, view: PinterestView = "explore", privacyMode = false, previewWidth?: number) { await act(async () => root.render(<PinterestWorkspace active runtime="browser-mock" privacyMode={privacyMode} view={view} previewWidth={previewWidth} api={api} />)); }
async function loadPin(id: string) {
  await act(async () => { const input = container.querySelector<HTMLInputElement>("input")!; Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, id); input.dispatchEvent(new Event("input", { bubbles: true })); });
  await act(async () => container.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
}

describe("Pinterest contracts", () => {
  it("normalizes long IDs without number precision loss and rejects unsafe URLs", () => {
    expect(normalizePinterestInput("https://se.pinterest.com/pin/mountains--621004236103700020/?x=1")).toBe("621004236103700020");
    expect(normalizePinterestInput("621004236103700020")).toBe("621004236103700020");
    for (const input of ["https://pinterest.com.evil.test/pin/12345", "http://www.pinterest.com/pin/12345", "https://user@www.pinterest.com/pin/12345", "https://127.0.0.1/pin/12345", "https://pin.it/short", "mountains", "https://www.pinterest.com/pin/12345/extra"]) expect(normalizePinterestInput(input)).toBeNull();
  });
  it("implements independent Ctrl/Shift selection", () => {
    const ids = ["a", "b", "c", "d"], modifiers = { ctrlKey: false, metaKey: false, shiftKey: false };
    expect([...selectPinterestCard(ids, new Set(), "b", null, modifiers)]).toEqual(["b"]);
    expect([...selectPinterestCard(ids, new Set(["b"]), "d", "b", { ...modifiers, shiftKey: true })]).toEqual(["b", "c", "d"]);
    expect([...selectPinterestCard(ids, new Set(["b", "c"]), "b", "b", { ...modifiers, ctrlKey: true })]).toEqual(["c"]);
  });
  it("does not fake successful remote features in a browser-only runtime", async () => {
    await expect(createPinterestApi("browser-mock").get("621004236103700020", "test")).rejects.toThrow("데스크톱");
  });
  it("ignores malformed persisted favorite data", () => {
    localStorage.setItem(PINTEREST_FAVORITES_KEY, JSON.stringify([{ pin: { id: 621004236103700020 }, page: null }, { pin: pin(), page: 9 }, { pin: pin(), page: null }]));
    expect(loadPinterestFavorites()).toHaveLength(1);
  });
  it("persists only sanitized allowlisted metadata and never image bytes", () => {
    const value = { ...pin(), thumbnail: "data:image/jpeg;base64,YQ==" };
    expect(savePinterestFavorites([{ pin: value, page: null }])).toBe(true);
    expect(localStorage.getItem(PINTEREST_FAVORITES_KEY)).not.toContain("base64");
    expect(savePinterestFavorites([{ pin: { ...pin(), media: [{ url: "https://evil.test/image.jpg" }] }, page: null }])).toBe(false);
    expect(loadPinterestFavorites()).toHaveLength(1);
  });
});

describe("Pinterest session-native home", () => {
  const proof = (generation = 1, status: PinterestAccountSnapshot["status"] = "signedIn"): PinterestAccountSnapshot => ({ generation, status, open: false, busy: false });
  const account = (): PinterestAccountApi => ({ status: vi.fn().mockResolvedValue(proof()), open: vi.fn().mockResolvedValue(proof()) });
  const result = (pins: PinterestPin[], generation = 1, cursor: string | null = null) => ({ title: "홈 피드", path: "/", pins, boards: [], limited: false, generation, cursor, authStatus: "signedIn" as const });
  const show = async (api: PinterestApi, accountApi: PinterestAccountApi, view: PinterestView = "explore", active = true, privacy = false) => {
    await act(async () => root.render(<PinterestWorkspace active={active} runtime="browser-mock" privacyMode={privacy} view={view} api={api} accountApi={accountApi} />));
  };
  it("loads home automatically once and uses the same card actions for local favorites and original saving", async () => {
    const api = mockApi(), auth = account(); vi.mocked(api.discover).mockResolvedValue(result([pin()]));
    await show(api, auth);
    expect(api.discover).toHaveBeenCalledExactlyOnceWith("", "home", expect.any(String));
    const card = container.querySelector<HTMLElement>(".pinterest-card")!;
    await act(async () => card.querySelector<HTMLButtonElement>(".pinterest-bookmark")!.click());
    expect(loadPinterestFavorites()[0]!.pin.id).toBe(pin().id);
    await act(async () => card.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, clientX: 20, clientY: 20 })));
    await act(async () => button("원본 다운로드").click());
    expect(api.download).toHaveBeenCalledExactlyOnceWith(pin().id, expect.any(String));
    await show(api, auth, "favorites"); await show(api, auth);
    expect(api.discover).toHaveBeenCalledTimes(1);
    expect(button("로그인")).toBeDisabled(); expect(button("로그아웃")).not.toBeDisabled();
  });
  it("waits for initial account generation before starting the home request", async () => {
    const api = mockApi(), auth = account(); let verified!: (value: PinterestAccountSnapshot) => void;
    vi.mocked(auth.status).mockImplementationOnce(() => new Promise(resolve => { verified = resolve; }));
    vi.mocked(api.discover).mockResolvedValue(result([pin()]));
    await show(api, auth); expect(api.discover).not.toHaveBeenCalled();
    await act(async () => verified(proof()));
    expect(api.discover).toHaveBeenCalledExactlyOnceWith("", "home", expect.any(String));
    expect(container.querySelector(".pinterest-card")).not.toBeNull();
  });
  it("appends opaque continuation with deduplication, stable cards and selection", async () => {
    const api = mockApi(), auth = account(), second = pin("621004236103700021", "두 번째");
    vi.mocked(api.discover).mockResolvedValueOnce(result([pin()], 1, "native-cursor")).mockResolvedValueOnce(result([pin(), second]));
    await show(api, auth); const card = container.querySelector<HTMLElement>(".pinterest-card")!;
    await act(async () => card.click()); await act(async () => button("핀 더 불러오기").click());
    expect(api.discover).toHaveBeenLastCalledWith("", "home", expect.any(String), "native-cursor");
    expect(container.querySelectorAll(".pinterest-card")).toHaveLength(2);
    expect(container.querySelector(".pinterest-card")).toBe(card); expect(card).toHaveClass("is-selected");
    expect(button("핀 더 불러오기")).toBeUndefined();
  });
  it("retains local favorites on logout, cancels stale home replies and loads only the new epoch", async () => {
    const api = mockApi(), auth = account(); let complete!: (value: ReturnType<typeof result>) => void;
    savePinterestFavorites([{ pin: pin(), page: null }]);
    vi.mocked(api.discover).mockImplementationOnce(() => new Promise(resolve => { complete = resolve; })).mockResolvedValue(result([], 2));
    vi.mocked(auth.open).mockResolvedValue(proof(2, "signedOut"));
    vi.mocked(auth.status).mockResolvedValue(proof(1));
    await show(api, auth); await act(async () => button("로그아웃").click());
    await act(async () => complete(result([pin()], 1)));
    expect(api.cancel).toHaveBeenCalled(); expect(container.querySelector(".pinterest-card")).toBeNull();
    expect(loadPinterestFavorites()).toHaveLength(1);
    await show(api, auth, "favorites"); expect(container.querySelector(".pinterest-card")).not.toBeNull();
  });
  it("does not silently substitute a company profile or retry forever on a home login wall", async () => {
    const api = mockApi(), auth = account(); vi.mocked(api.discover).mockRejectedValue(new Error("홈 피드를 확인하려면 로그인해 주세요."));
    await show(api, auth); await show(api, auth);
    expect(api.discover).toHaveBeenCalledTimes(1);
    expect(container.querySelector('.pinterest-message[role="alert"]')).toHaveTextContent("로그인해 주세요");
    expect(container).not.toHaveTextContent("결과 없음");
    await act(async () => button("홈 피드 불러오기").click()); expect(api.discover).toHaveBeenCalledTimes(2);
  });
  it("defers home/account network work while hidden or private", async () => {
    const api = mockApi(), auth = account(); vi.mocked(api.discover).mockResolvedValue(result([pin()]));
    await show(api, auth, "explore", false); expect(api.discover).not.toHaveBeenCalled(); expect(auth.status).not.toHaveBeenCalled();
    await show(api, auth, "explore", true, true); expect(api.discover).not.toHaveBeenCalled(); expect(auth.status).not.toHaveBeenCalled();
    await show(api, auth); expect(api.discover).toHaveBeenCalledTimes(1);
  });
  it("routes ordinary keyword search to native session discovery, not an automatic web wrapper", async () => {
    const api = mockApi(), auth = account(); vi.mocked(api.discover).mockResolvedValue(result([pin()]));
    await show(api, auth); await loadPin("landscape");
    expect(api.discover).toHaveBeenLastCalledWith("landscape", "keyword", expect.any(String));
    expect(container.querySelector(".pinterest-official-search")).toBeNull();
  });
});

describe("Pinterest saved boards and direct preview", () => {
  const proof = (generation = 1, status: PinterestAccountSnapshot["status"] = "signedIn"): PinterestAccountSnapshot => ({ generation, status, open: false, busy: false });
  const account = (): PinterestAccountApi => ({ status: vi.fn().mockResolvedValue(proof()), open: vi.fn().mockResolvedValue(proof(2, "signedOut")) });
  const show = async (api: PinterestApi, auth: PinterestAccountApi, privacyMode = false) => { await act(async () => root.render(<PinterestWorkspace active runtime="browser-mock" privacyMode={privacyMode} view="favorites" api={api} accountApi={auth} />)); };
  const result = (pins: PinterestPin[] = [], cursor: string | null = null) => ({ title: "내 보드", path: "/fixture/_saved/", pins, boards: [{ title: "풍경 보드", path: "/fixture/landscape/" }], limited: false, cursor, generation: 1, authStatus: "signedIn" as const });
  it("opens albums directly in pinned preview, keeps both bookmarks and closes in one Escape", async () => {
    const api = mockApi(); await render(api); await loadPin(pin().id);
    const card = container.querySelector<HTMLElement>(".pinterest-card")!;
    await act(async () => card.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    expect(container.querySelector('[role="dialog"]')).toHaveAttribute("aria-label", "Pinterest 페이지 미리보기");
    expect(button("미리보기 조작부 고정")).toHaveAttribute("aria-pressed", "true");
    await act(async () => button("앨범 즐겨찾기").click()); await act(async () => button("1쪽 즐겨찾기").click());
    expect(loadPinterestFavorites().map(item => item.page).sort()).toEqual([0, null]);
    await act(async () => button("정보").click()); expect(container.querySelector(".pinterest-detail aside")).not.toBeNull();
    await act(async () => button("미리보기").click());
    await act(async () => container.querySelector('[role="dialog"]')!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(container.querySelector('[role="dialog"]')).toBeNull(); expect(container.querySelector(".pinterest-card")).toBe(card);
  });
  it("lists own boards, appends pins without clearing selection, preserves local favorites and board return", async () => {
    const api = mockApi(), auth = account(), local = pin("9000001", "로컬"), one = pin("9000002", "첫 핀"), two = pin("9000003", "다음 핀");
    savePinterestFavorites([{ pin: local, page: null }]);
    vi.mocked(api.discover).mockResolvedValueOnce(result()).mockResolvedValueOnce(result([one], "next")).mockResolvedValueOnce(result([one, two]));
    await show(api, auth); expect(api.discover).not.toHaveBeenCalled();
    await act(async () => button("내 보드").click()); expect(api.discover).toHaveBeenLastCalledWith("", "saved", expect.any(String));
    await act(async () => button("풍경 보드").click()); expect(api.discover).toHaveBeenLastCalledWith("/fixture/landscape/", "profile", expect.any(String));
    const card = container.querySelector<HTMLElement>(".pinterest-card")!; await act(async () => card.click());
    await act(async () => button("핀 더 불러오기").click());
    expect(container.querySelector(".pinterest-card")).toBe(card); expect(card).toHaveClass("is-selected");
    await act(async () => container.querySelectorAll(".pinterest-card")[1]!.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true })));
    await act(async () => button("즐겨찾기 추가").click()); expect(loadPinterestFavorites().map(item => item.pin.id)).toEqual([local.id, one.id, two.id]);
    await act(async () => button("보드 목록").click()); await act(async () => button("풍경 보드").click()); expect(api.discover).toHaveBeenCalledTimes(3);
    await act(async () => button("앨범").click()); expect(container.querySelectorAll(".pinterest-card")).toHaveLength(3);
  });
  it("cancels and discards late board data on logout without deleting local favorites", async () => {
    const api = mockApi(), auth = account(); let finish!: (value: ReturnType<typeof result>) => void;
    savePinterestFavorites([{ pin: pin(), page: null }]); vi.mocked(api.discover).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    await show(api, auth); await act(async () => button("내 보드").click());
    await act(async () => button("로그아웃").click()); await act(async () => finish(result()));
    expect(api.cancel).toHaveBeenCalled(); expect(button("내 보드")).toBeDisabled();
    expect(container.querySelector('[aria-label="내 Pinterest 보드"]')).toBeNull(); expect(loadPinterestFavorites()).toHaveLength(1);
  });
  it("does not start remote boards while private, and does not auto-retry a failed response", async () => {
    const api = mockApi(), auth = account(); vi.mocked(api.discover).mockRejectedValue(new Error("보드 확인 실패"));
    await show(api, auth, true); expect(api.discover).not.toHaveBeenCalled();
    await show(api, auth); await act(async () => button("내 보드").click());
    await show(api, auth); expect(api.discover).toHaveBeenCalledTimes(1); expect(container).toHaveTextContent("보드 확인 실패");
    await act(async () => button("새로고침").click()); expect(api.discover).toHaveBeenCalledTimes(2);
  });
});

describe("Pinterest workspace", () => {
  it("keeps source-order Ctrl/Shift selection and existing hitboxes on mixed-aspect Masonry cards", async () => {
    const originalBounds = HTMLElement.prototype.getBoundingClientRect;
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function(this: HTMLElement) {
      return this.matches(".pinterest-grid") ? new DOMRect(0, 0, 600, 1000) : originalBounds.call(this);
    });
    const api = mockApi();
    const pins = [1200, 500, 800, 1600].map((height, index) => ({ ...pin(String(900001 + index), `핀 ${index}`), media: [{ ...pin().media[0]!, width: 800, height }] }));
    vi.mocked(api.discover).mockResolvedValue({ title: "Mixed", path: "/pinterest/", pins, boards: [], limited: false });
    await render(api); await loadPin("@pinterest");
    const cards = [...container.querySelectorAll<HTMLElement>(".pinterest-card")];
    expect(container.querySelector(".pinterest-grid")).toHaveAttribute("data-masonry-ready", "true");
    expect(cards.map(card => card.dataset.masonryKey)).toEqual(pins.map(item => `${item.id}:album`));
    expect(new Set(cards.map(card => card.style.gridRow)).size).toBeGreaterThan(1);
    await act(async () => cards[0]!.click());
    await act(async () => cards[3]!.dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true })));
    expect(container.querySelectorAll('.pinterest-check[aria-checked="true"]')).toHaveLength(4);
    await act(async () => cards[1]!.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true })));
    expect(container.querySelectorAll('.pinterest-check[aria-checked="true"]')).toHaveLength(3);
    const previewCalls = vi.mocked(api.preview).mock.calls.length;
    await render(api, "explore", false, 340);
    expect([...container.querySelectorAll(".pinterest-card")]).toEqual(cards);
    expect(container.querySelectorAll('.pinterest-check[aria-checked="true"]')).toHaveLength(3);
    await act(async () => cards[1]!.querySelector<HTMLButtonElement>(".pinterest-bookmark")!.click());
    expect(loadPinterestFavorites().map(item => item.pin.id)).toEqual([pins[1]!.id]);
    expect(container.querySelectorAll('.pinterest-check[aria-checked="true"]')).toHaveLength(3);
    await act(async () => cards[2]!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, clientX: 60, clientY: 60 })));
    expect(document.querySelector('[role="menu"]')).not.toBeNull();
    expect(api.preview).toHaveBeenCalledTimes(previewCalls); expect(api.discover).toHaveBeenCalledTimes(1);
    expect(api.downloadBatch).not.toHaveBeenCalled();
  });
  it("uses the selected favorite page dimensions, not the album cover dimensions", async () => {
    const value = { ...pin(), media: [{ ...pin().media[0]!, width: 900, height: 300 }, { url: "https://i.pinimg.com/originals/b.jpg", width: 400, height: 900 }] };
    savePinterestFavorites([{ pin: value, page: 0 }, { pin: value, page: 1 }]);
    const api = mockApi(); await render(api, "favorites"); await act(async () => button("페이지").click());
    const cards = [...container.querySelectorAll<HTMLElement>(".pinterest-card")];
    expect(cards.map(card => [card.dataset.masonryWidth, card.dataset.masonryHeight])).toEqual([["900", "300"], ["400", "900"]]);
    expect(cards.map(card => card.dataset.masonryKey)).toEqual([`${value.id}:0`, `${value.id}:1`]);
    expect(api.get).not.toHaveBeenCalled();
  });
  it("fills missing dimensions from the existing decoded thumbnail without a new request or persisted metadata rewrite", async () => {
    const api = mockApi(); await render(api); await loadPin(pin().id);
    const card = container.querySelector<HTMLElement>(".pinterest-card")!, image = card.querySelector("img")!;
    const previewCalls = vi.mocked(api.preview).mock.calls.length;
    Object.defineProperties(image, { naturalWidth: { value: 320 }, naturalHeight: { value: 640 } });
    await act(async () => image.dispatchEvent(new Event("load", { bubbles: false })));
    expect(container.querySelector(".pinterest-card")).toBe(card);
    expect(card.dataset.masonryWidth).toBe("320"); expect(card.dataset.masonryHeight).toBe("640");
    expect(api.preview).toHaveBeenCalledTimes(previewCalls); expect(api.get).toHaveBeenCalledTimes(1);
    expect(loadPinterestFavorites()).toEqual([]);
  });
  it("keeps trusted page metadata over decoded thumbnail size and archive identities independent", async () => {
    const api = mockApi();
    const first = { ...pin(), media: [{ ...pin().media[0]!, width: 1600, height: 900 }] };
    const second = { ...first, media: [{ ...first.media[0]!, width: 900, height: 1600 }] };
    vi.mocked(api.removed).mockResolvedValue([first, second].map((value, index) => ({ token: `owned-${index}`, record: { pin: value, files: ["001.jpg"], bytes: 10, savedAt: 1 } })));
    await render(api, "downloads"); await act(async () => button("제외된 핀").click());
    const cards = [...container.querySelectorAll<HTMLElement>(".pinterest-removed .pinterest-card")];
    expect(cards.map(card => card.dataset.masonryKey)).toEqual(["owned-0", "owned-1"]);
    const image = cards[0]!.querySelector("img")!;
    Object.defineProperties(image, { naturalWidth: { value: 400 }, naturalHeight: { value: 400 } });
    await act(async () => image.dispatchEvent(new Event("load", { bubbles: false })));
    expect(cards.map(card => [card.dataset.masonryWidth, card.dataset.masonryHeight])).toEqual([["1600", "900"], ["900", "1600"]]);
    expect(api.get).not.toHaveBeenCalled(); expect(api.libraryAction).not.toHaveBeenCalled();
  });
  it("resizes existing explore cards without refetching, remounting or clearing the current selection", async () => {
    const api = mockApi();
    vi.mocked(api.discover).mockResolvedValue({ title: "Pinterest", path: "/pinterest/", pins: [pin(), pin("621004236103700021", "하늘 풍경")], boards: [], limited: false });
    await render(api);
    await loadPin("@pinterest");
    // Finish the pre-existing view-entry restoration before simulating a user
    // scroll; otherwise its queued animation frame can race this assertion.
    await act(async () => { await new Promise<void>(resolve => requestAnimationFrame(() => resolve())); });
    const workspace = container.querySelector<HTMLElement>(".pinterest-workspace")!;
    const cards = [...container.querySelectorAll<HTMLElement>(".pinterest-card")];
    expect(workspace.style.getPropertyValue("--pinterest-preview-width")).toBe("180px");
    await act(async () => cards[0]!.click());
    await act(async () => cards[1]!.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true })));
    workspace.scrollTop = 231;
    const previewCalls = vi.mocked(api.preview).mock.calls.length;
    const cancelCalls = vi.mocked(api.cancel).mock.calls.length;
    await render(api, "explore", false, 340);
    expect(container.querySelector(".pinterest-workspace")).toBe(workspace);
    expect([...container.querySelectorAll(".pinterest-card")]).toEqual(cards);
    expect(workspace.style.getPropertyValue("--pinterest-preview-width")).toBe("340px");
    expect(workspace.scrollTop).toBe(231);
    expect(container.querySelector('[aria-label="선택한 핀 작업"]')).toHaveTextContent("2개 선택");
    expect(api.discover).toHaveBeenCalledTimes(1);
    expect(api.get).not.toHaveBeenCalled();
    expect(api.preview).toHaveBeenCalledTimes(previewCalls);
    expect(api.cancel).toHaveBeenCalledTimes(cancelCalls);
    expect(loadPinterestFavorites()).toEqual([]);
    expect(api.download).not.toHaveBeenCalled();
    expect(api.downloadBatch).not.toHaveBeenCalled();
  });
  it.each(["favorites", "downloads"] as const)("applies preview width to %s cards without changing saved data", async view => {
    const api = mockApi();
    savePinterestFavorites([{ pin: pin(), page: null }]);
    const persisted = localStorage.getItem(PINTEREST_FAVORITES_KEY);
    vi.mocked(api.downloads).mockResolvedValue([{ pin: pin(), files: ["001.jpg"], bytes: 100, savedAt: 1 }]);
    await render(api, view, false, 140);
    const card = container.querySelector(".pinterest-card");
    expect(card).not.toBeNull();
    const previewCalls = vi.mocked(api.preview).mock.calls.length;
    const downloadListCalls = vi.mocked(api.downloads).mock.calls.length;
    await render(api, view, false, 280);
    expect(container.querySelector(".pinterest-card")).toBe(card);
    expect(container.querySelector<HTMLElement>(".pinterest-workspace")!.style.getPropertyValue("--pinterest-preview-width")).toBe("280px");
    expect(api.preview).toHaveBeenCalledTimes(previewCalls);
    expect(api.downloads).toHaveBeenCalledTimes(downloadListCalls);
    expect(localStorage.getItem(PINTEREST_FAVORITES_KEY)).toBe(persisted);
    expect(api.libraryAction).not.toHaveBeenCalled();
  });
  it("keeps excluded pins and their selection mounted inside the resized workspace", async () => {
    const api = mockApi();
    const items = [pin(), pin("621004236103700021", "하늘 풍경")].map((value, index) => ({ token: `${value.id}-${index}`, record: { pin: value, files: ["001.jpg"], bytes: 100, savedAt: 1 } }));
    vi.mocked(api.removed).mockResolvedValue(items);
    await render(api, "downloads", false, 220);
    await act(async () => button("제외된 핀").click());
    const dialog = container.querySelector(".pinterest-removed");
    const cards = [...container.querySelectorAll<HTMLElement>(".pinterest-removed .pinterest-card")];
    await act(async () => cards[0]!.click());
    await act(async () => cards[1]!.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true })));
    const previewCalls = vi.mocked(api.preview).mock.calls.length;
    await render(api, "downloads", false, 340);
    expect(container.querySelector(".pinterest-removed")).toBe(dialog);
    expect([...container.querySelectorAll(".pinterest-removed .pinterest-card")]).toEqual(cards);
    expect(dialog!.closest<HTMLElement>(".pinterest-workspace")!.style.getPropertyValue("--pinterest-preview-width")).toBe("340px");
    expect(container.querySelectorAll('.pinterest-removed [aria-checked="true"]')).toHaveLength(2);
    expect(api.removed).toHaveBeenCalledTimes(1);
    expect(api.downloads).toHaveBeenCalledTimes(1);
    expect(api.preview).toHaveBeenCalledTimes(previewCalls);
    expect(api.libraryAction).not.toHaveBeenCalled();
  });
  it("normalizes invalid preview widths before assigning the source-local CSS property", async () => {
    await render(mockApi(), "explore", false, Number.NaN);
    expect(container.querySelector<HTMLElement>(".pinterest-workspace")!.style.getPropertyValue("--pinterest-preview-width")).toBe("180px");
  });
  it("preserves official page and native results across inactive/library views, then imports into native favorites and saving", async () => {
    const api = mockApi(), officialPin = pin("1076430748452447388", "나무와 하늘");
    const snapshot = { epoch: 9, open: true, loading: false, currentPinId: officialPin.id, error: null };
    const browserApi: PinterestBrowserApi = { open: vi.fn().mockResolvedValue(snapshot), snapshot: vi.fn().mockResolvedValue(snapshot), viewport: vi.fn().mockResolvedValue(undefined), select: vi.fn().mockResolvedValue(officialPin.id), action: vi.fn().mockResolvedValue(undefined) };
    vi.mocked(api.get).mockImplementation(async id => id === officialPin.id ? officialPin : pin(id));
    const show = async (active: boolean, view: PinterestView = "explore") => { await act(async () => root.render(<PinterestWorkspace active={active} runtime="browser-mock" privacyMode={false} view={view} api={api} browserApi={browserApi} />)); };
    await show(true); await loadPin(pin().id); await loadPin("landscape");
    expect(browserApi.open).not.toHaveBeenCalled();
    await act(async () => button("공식 웹에서 검색").click());
    expect(browserApi.open).toHaveBeenCalledExactlyOnceWith("landscape");
    await show(false);
    expect(container.querySelector('.pinterest-workspace')).toHaveAttribute("hidden");
    expect(browserApi.viewport).toHaveBeenLastCalledWith(expect.objectContaining({ visible: false }));
    await show(true, "favorites"); await show(true);
    expect(browserApi.open).toHaveBeenCalledTimes(1);
    expect(browserApi.action).not.toHaveBeenCalledWith(expect.anything(), "close");
    await act(async () => button("목록 보기").click());
    expect(container.querySelector('.pinterest-card')).toHaveAttribute("aria-label", "산 풍경");
    expect(container.querySelector('.pinterest-card')!.closest('[hidden]')).toBeNull();
    await act(async () => button("공식 웹에서 검색").click());
    await act(async () => button("핀 가져오기").click());
    expect(browserApi.select).toHaveBeenCalledExactlyOnceWith(9, officialPin.id);
    expect(api.get).toHaveBeenCalledWith(officialPin.id, expect.any(String));
    expect(container.querySelector('[aria-label="Pinterest 페이지 미리보기"]')).toHaveTextContent("나무와 하늘");
    await act(async () => button("앨범 즐겨찾기").click());
    expect(loadPinterestFavorites().map(item => item.pin.id)).toContain(officialPin.id);
    await act(async () => button("원본 저장").click());
    expect(api.download).toHaveBeenCalledExactlyOnceWith(officialPin.id, expect.any(String));
    await act(async () => button("미리보기 닫기").click());
    await act(async () => button("공식 웹에서 검색").click());
    expect(browserApi.open).toHaveBeenCalledTimes(1);
  });
  it("does not open a late official pin after returning to the native list", async () => {
    const api = mockApi(); let finish!: (value: PinterestPin) => void;
    const snapshot = { epoch: 9, open: true, loading: false, currentPinId: pin().id, error: null };
    const browserApi: PinterestBrowserApi = { open: vi.fn().mockResolvedValue(snapshot), snapshot: vi.fn().mockResolvedValue(snapshot), viewport: vi.fn().mockResolvedValue(undefined), select: vi.fn().mockResolvedValue(pin().id), action: vi.fn().mockResolvedValue(undefined) };
    await act(async () => root.render(<PinterestWorkspace active runtime="browser-mock" privacyMode={false} view="explore" api={api} browserApi={browserApi} />));
    await loadPin("landscape");
    await act(async () => button("공식 웹에서 검색").click());
    vi.mocked(api.get).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    await act(async () => button("핀 가져오기").click());
    await act(async () => button("목록 보기").click());
    expect(api.cancel).toHaveBeenCalledWith(expect.any(String));
    await act(async () => finish(pin()));
    expect(container.querySelector('[aria-label="Pinterest 페이지 미리보기"]')).toBeNull();
    expect(container.querySelector('.pinterest-official-search')).toHaveAttribute("hidden");
  });
  it("reports successful saving once through the shared toast without inserting a list banner", async () => {
    const api = mockApi(), notice = vi.fn();
    await act(async () => root.render(<PinterestWorkspace active runtime="browser-mock" privacyMode={false} view="explore" api={api} onNotice={notice} />));
    await loadPin(pin().id);
    const card = container.querySelector('.pinterest-card')!;
    await act(async () => card.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
    await act(async () => button("원본 저장").click());
    expect(notice).toHaveBeenCalledExactlyOnceWith("원본 저장 완료");
    expect(container.querySelector('.pinterest-workspace > .pinterest-message')).toBeNull();
    expect(container.querySelector('.pinterest-grid')?.previousElementSibling?.classList.contains('pinterest-message')).toBe(false);
    expect(button("저장 완료")).toBeDisabled();
  });
  it("rediscovers excluded pins after remount and previews their owned local originals", async () => {
    const api = mockApi(), item = { token: `${pin().id}-7a69ec70-40a1-4c63-b75d-ddd61522c712`, record: { pin: pin(), files: ["001.jpg"], bytes: 100, savedAt: 1 } };
    vi.mocked(api.removed).mockResolvedValue([item]);
    await render(api, "downloads"); await act(async () => button("제외된 핀").click());
    expect(api.removed).toHaveBeenCalledTimes(1);
    expect(api.preview).toHaveBeenCalledWith(item.record.pin.id, 0, expect.any(String), true, item.token);
    await act(async () => container.querySelector('.pinterest-removed .pinterest-card')!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
    expect(api.preview).toHaveBeenCalledWith(item.record.pin.id, 0, expect.any(String), false, item.token);
    expect(api.get).not.toHaveBeenCalled();
    await act(async () => button("미리보기 닫기").click());
    await act(async () => button("제외된 핀 닫기").click());
    await act(async () => root.unmount()); root = createRoot(container);
    await render(api, "downloads"); await act(async () => button("제외된 핀").click());
    expect(api.removed).toHaveBeenCalledTimes(2);
    expect(container.querySelectorAll('.pinterest-removed .pinterest-card')).toHaveLength(1);
    expect(localStorage.length).toBe(0);
  });
  it("keeps excluded cards unchecked on single click and restores Ctrl-selected records", async () => {
    const api = mockApi();
    const items = [pin(), pin("621004236103700021", "하늘 풍경")].map((value, index) => ({ token: `${value.id}-${index}`, record: { pin: value, files: ["001.jpg"], bytes: 100, savedAt: 1 } }));
    vi.mocked(api.removed).mockResolvedValue(items);
    await render(api, "downloads"); await act(async () => button("제외된 핀").click());
    const cards = [...container.querySelectorAll<HTMLElement>('.pinterest-removed .pinterest-card')];
    await act(async () => cards[0]!.click());
    expect(container.querySelector('.pinterest-removed [role="checkbox"]')).toBeNull();
    await act(async () => cards[1]!.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true })));
    expect(container.querySelectorAll('.pinterest-removed [aria-checked="true"]')).toHaveLength(2);
    await act(async () => button("선택 복원 (2)").click());
    expect(api.libraryAction).toHaveBeenCalledWith("restore", items[0]!.token, expect.any(String));
    expect(api.libraryAction).toHaveBeenCalledWith("restore", items[1]!.token, expect.any(String));
    expect(container.querySelectorAll('.pinterest-removed .pinterest-card')).toHaveLength(0);
    expect(container.querySelector('.pinterest-removed [role="status"]')).toHaveTextContent("복원 종료 · 2/2");
  });
  it("preserves excluded records on restore conflict and closes by Escape with focus restored", async () => {
    const api = mockApi();
    vi.mocked(api.removed).mockResolvedValue([{ token: "owned-token", record: { pin: pin(), files: ["001.jpg"], bytes: 100, savedAt: 1, missingFiles: true } }]);
    vi.mocked(api.libraryAction).mockRejectedValue("같은 핀이 이미 있어 덮어쓰지 않았습니다.");
    await render(api, "downloads"); const trigger = button("제외된 핀"); trigger.focus();
    await act(async () => trigger.click());
    expect(container.querySelector('.pinterest-removed')).toHaveTextContent("원본 파일 없음");
    await act(async () => button("산 풍경 복원").click());
    expect(container.querySelector('.pinterest-removed [role="alert"]')).toHaveTextContent("덮어쓰지");
    expect(container.querySelectorAll('.pinterest-removed .pinterest-card')).toHaveLength(1);
    await act(async () => container.querySelector('.pinterest-removed')!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(container.querySelector('.pinterest-removed')).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });
  it("cancels multi-restore without starting remaining records or reporting them restored", async () => {
    const api = mockApi(); let rejectRestore!: (reason: string) => void;
    const items = [pin(), pin("621004236103700021", "하늘 풍경")].map((value, index) => ({ token: `${value.id}-${index}`, record: { pin: value, files: ["001.jpg"], bytes: 100, savedAt: 1 } }));
    vi.mocked(api.removed).mockResolvedValue(items);
    vi.mocked(api.libraryAction).mockImplementation(() => new Promise((_, reject) => { rejectRestore = reject; }));
    vi.mocked(api.cancel).mockImplementation(async () => { rejectRestore?.("Pinterest 요청을 취소했습니다."); });
    await render(api, "downloads"); await act(async () => button("제외된 핀").click());
    const cards = [...container.querySelectorAll<HTMLElement>('.pinterest-removed .pinterest-card')];
    await act(async () => cards[0]!.click());
    await act(async () => cards[1]!.dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true })));
    await act(async () => button("선택 복원 (2)").click());
    await act(async () => container.querySelector<HTMLButtonElement>('.pinterest-removed > footer > button')!.click());
    expect(api.libraryAction).toHaveBeenCalledTimes(1);
    expect(container.querySelectorAll('.pinterest-removed .pinterest-card')).toHaveLength(2);
    expect(container.querySelector('.pinterest-removed [role="status"]')).toHaveTextContent("복원 종료 · 0/2");
  });
  it("retains same-query results and restores the prior search tab position", async () => {
    const api = mockApi(); vi.mocked(api.discover).mockImplementation(async input => ({ title: input, path: `/${input}/`, pins: [pin()], boards: [], limited: false }));
    await render(api); await loadPin("@creator");
    const pane = container.querySelector<HTMLElement>(".pinterest-workspace")!;
    pane.scrollTop = 410;
    await loadPin("@second");
    await loadPin("@creator");
    await act(async () => new Promise<void>(resolve => requestAnimationFrame(() => resolve())));
    expect(api.discover).toHaveBeenCalledTimes(2);
    expect(pane.scrollTop).toBe(410);
    expect(container.querySelectorAll(".pinterest-card")).toHaveLength(1);
  });
  it("restores each search tab's list page and controls on revisit and adjacent close", async () => {
    const api = mockApi();
    vi.mocked(api.discover).mockImplementation(async input => ({ title: input, path: `/${input}/`, pins: Array.from({ length: 60 }, (_, index) => pin(String(800000 + index))), boards: [], limited: true }));
    await render(api); await loadPin("@creator");
    await act(async () => button("다음 목록 페이지").click());
    const pane = container.querySelector<HTMLElement>(".pinterest-workspace")!; pane.scrollTop = 270;
    await loadPin("@creator");
    expect(container.querySelector('.pinterest-pagination')).toHaveTextContent("2 / 2");
    expect(container.querySelectorAll('.pinterest-card')).toHaveLength(12);
    await loadPin("@second");
    await act(async () => button("@second 탐색 닫기").click());
    await act(async () => new Promise<void>(resolve => requestAnimationFrame(() => resolve())));
    expect(container.querySelector('.pinterest-pagination')).toHaveTextContent("2 / 2");
    expect(container.querySelectorAll('.pinterest-card')).toHaveLength(12);
    expect(pane.scrollTop).toBe(270);
    expect(container.querySelector('input')).toHaveValue("@creator");
    expect(container.querySelector('select')).toHaveValue("profile");
    expect(api.discover).toHaveBeenCalledTimes(2);
  });
  it("opens every selected pin in independent detail tabs with Enter", async () => {
    const api = mockApi(); await render(api); await loadPin("621004236103700020"); await loadPin("621004236103700021");
    const cards = [...container.querySelectorAll<HTMLElement>(".pinterest-card")];
    await act(async () => cards[0]!.click());
    await act(async () => cards[1]!.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true })));
    await act(async () => cards[1]!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    expect(container.querySelectorAll(".pinterest-detail-tabs > span")).toHaveLength(2);
    expect(container.querySelector('[role="toolbar"]')).toBeNull();
  });
  it("does not replace the focused search tab when another request completes", async () => {
    const api = mockApi(); let complete!: (value: Awaited<ReturnType<PinterestApi["discover"]>>) => void;
    vi.mocked(api.discover).mockImplementation(async input => ({ title: input, path: `/${input}/`, pins: [pin()], boards: [], limited: false }));
    await render(api); await loadPin("@first"); await loadPin("@second");
    vi.mocked(api.discover).mockImplementationOnce(() => new Promise(resolve => { complete = resolve; }));
    await loadPin("@third");
    await act(async () => button("@first").click());
    await act(async () => complete({ title: "@third", path: "/third/", pins: [pin("621004236103700022", "세 번째")], boards: [], limited: false }));
    expect(container.querySelector('.pinterest-search-tabs > .is-active')).toHaveTextContent("@first");
    expect(container.querySelector('.pinterest-card')).toHaveAttribute("aria-label", "산 풍경");
    await act(async () => button("@third").click());
    expect(container.querySelector('.pinterest-card')).toHaveAttribute("aria-label", "세 번째");
    expect(api.discover).toHaveBeenCalledTimes(3);
  });
  it("confirms whole-detail multi-tab closure but not individual tab close", async () => {
    const api = mockApi(); await render(api); await loadPin("621004236103700020"); await loadPin("621004236103700021");
    const cards = [...container.querySelectorAll<HTMLElement>(".pinterest-card")];
    await act(async () => cards[0]!.click());
    await act(async () => cards[1]!.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true })));
    await act(async () => cards[1]!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    await act(async () => button("미리보기 닫기").click());
    expect(container.querySelector('[role="alertdialog"]')).toHaveTextContent("열린 핀 2개를 닫을까요?");
    await act(async () => button("취소").click());
    expect(container.querySelectorAll('.pinterest-detail-tabs > span')).toHaveLength(2);
    await act(async () => container.querySelector<HTMLButtonElement>('.pinterest-detail-tabs > span:last-child > button:last-child')!.click());
    expect(container.querySelector('[role="alertdialog"]')).toBeNull();
    expect(container.querySelector('.pinterest-detail')).not.toBeNull();
    await act(async () => button("미리보기 닫기").click());
    expect(container.querySelector('.pinterest-detail')).toBeNull();
  });
  it("opens selected excluded versions independently and only previews owned local tokens", async () => {
    const api = mockApi();
    const items = ["archive-one", "archive-two"].map(token => ({ token, record: { pin: pin(), files: ["001.jpg"], bytes: 100, savedAt: 1 } }));
    vi.mocked(api.removed).mockResolvedValue(items);
    await render(api, "downloads"); await act(async () => button("제외된 핀").click());
    const cards = [...container.querySelectorAll<HTMLElement>('.pinterest-removed .pinterest-card')];
    await act(async () => cards[0]!.click());
    await act(async () => cards[1]!.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true })));
    await act(async () => cards[1]!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    expect(container.querySelectorAll('.pinterest-detail-tabs > span')).toHaveLength(2);
    expect(api.preview).toHaveBeenCalledWith(pin().id, 0, expect.any(String), false, "archive-one");
    await act(async () => container.querySelector<HTMLButtonElement>('.pinterest-detail-tabs > span:last-child > button')!.click());
    expect(api.preview).toHaveBeenCalledWith(pin().id, 0, expect.any(String), false, "archive-two");
    expect(container.querySelectorAll('.pinterest-detail-tabs .is-active')).toHaveLength(1);
    expect(api.get).not.toHaveBeenCalled(); expect(api.libraryAction).not.toHaveBeenCalled();
  });
  it("stops detail loading on failure and offers an explicit retry", async () => {
    const api = mockApi(); vi.mocked(api.discover).mockResolvedValue({ title: "작가", path: "/creator/", pins: [{ ...pin(), summaryOnly: true }], boards: [], limited: false });
    vi.mocked(api.get).mockRejectedValueOnce("핀 정보를 불러오지 못했습니다.");
    await render(api); await loadPin("@creator");
    await act(async () => container.querySelector(".pinterest-card")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
    expect(container.querySelector('.pinterest-detail [aria-label="상세 불러오는 중"]')).toBeNull();
    expect(container.querySelector('.pinterest-detail [role="alert"]')).toHaveTextContent("핀 정보를");
    await act(async () => button("다시 시도").click());
    expect(api.get).toHaveBeenCalledTimes(2);
  });
  it("shows ten removable session history entries without persistent query storage", async () => {
    const api = mockApi(); await render(api);
    for (let i = 0; i < 11; i++) await loadPin(String(900000 + i));
    await act(async () => container.querySelector<HTMLInputElement>("input")!.focus());
    expect(container.querySelectorAll(".pinterest-history > div")).toHaveLength(10);
    await act(async () => button("900010 기록 삭제").click());
    expect(container.querySelectorAll(".pinterest-history > div")).toHaveLength(9);
    await act(async () => button("기록 모두 지우기").click());
    expect(container.querySelector(".pinterest-history")).toBeNull();
    expect(localStorage.length).toBe(0);
  });
  it("discovers public profiles and opens returned boards without an external browser", async () => {
    const api = mockApi();
    vi.mocked(api.discover).mockResolvedValueOnce({ title: "작가", path: "/creator/", pins: [pin()], boards: [{ title: "그림", path: "/creator/art/" }], limited: true });
    await render(api); await loadPin("@creator");
    expect(api.discover).toHaveBeenCalledWith("@creator", "profile", expect.any(String));
    expect(container.querySelector(".pinterest-card")).not.toBeNull();
    expect(container).toHaveTextContent("불러온 핀 1개");
    await act(async () => button("그림").click());
    expect(api.discover).toHaveBeenLastCalledWith("/creator/art/", "profile", expect.any(String));
    expect(api.openExternal).not.toHaveBeenCalled();
    expect(container.querySelectorAll(".pinterest-search-tabs > span")).toHaveLength(2);
  });
  it("shows authentication restrictions without reporting successful zero results", async () => {
    const api = mockApi(); vi.mocked(api.discover).mockRejectedValue("Pinterest 키워드 검색은 현재 로그인이 필요합니다.");
    await render(api); await loadPin("landscape");
    expect(api.discover).toHaveBeenCalledWith("landscape", "keyword", expect.any(String));
    expect(container.querySelector('[role="alert"]')).toHaveTextContent("로그인이 필요");
    expect(container.querySelector(".pinterest-search-tabs")).toBeNull();
  });
  it("downloads multiple selected originals and reports settled results", async () => {
    const api = mockApi(); await render(api); await loadPin("621004236103700020"); await loadPin("621004236103700021");
    const cards = [...container.querySelectorAll<HTMLElement>(".pinterest-card")];
    await act(async () => cards[0]!.click());
    await act(async () => cards[1]!.dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true })));
    await act(async () => button("원본 다운로드").click());
    expect(api.downloadBatch).toHaveBeenCalledExactlyOnceWith(expect.arrayContaining(["621004236103700020", "621004236103700021"]), expect.any(String), expect.any(Function));
    expect(api.download).not.toHaveBeenCalled();
    expect(container.querySelector(".pinterest-batch")).toHaveTextContent("처리 종료 · 2/2");
  });
  it("settles cancelled queue entries without reporting them as failures", async () => {
    const api = mockApi(); let settleBatch!: () => void;
    vi.mocked(api.downloadBatch).mockImplementation((ids, _request, progress) => new Promise(resolve => { settleBatch = () => { const outcomes: PinterestDownloadOutcome[] = ids.map(pinId => ({ pinId, status: "cancelled" })); outcomes.forEach(progress); resolve(outcomes); }; }));
    vi.mocked(api.cancel).mockImplementation(async () => { settleBatch?.(); });
    await render(api); await loadPin("621004236103700020"); await loadPin("621004236103700021");
    const cards = [...container.querySelectorAll<HTMLElement>(".pinterest-card")];
    await act(async () => cards[0]!.click());
    await act(async () => cards[1]!.dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true })));
    await act(async () => button("원본 다운로드").click());
    await act(async () => container.querySelector<HTMLButtonElement>(".pinterest-batch button")!.click());
    expect(container.querySelector(".pinterest-batch")).toHaveTextContent("처리 종료 · 2/2 · 취소 2");
    expect(container.querySelector(".pinterest-batch")).not.toHaveTextContent("실패");
    expect(api.downloadBatch).toHaveBeenCalledTimes(1);
    expect(api.cancel).toHaveBeenCalledWith(vi.mocked(api.downloadBatch).mock.calls[0]![1]);
    expect(api.download).not.toHaveBeenCalled();
  });
  it("keeps a native batch active between per-item outcomes and reconciles final replies once", async () => {
    const api = mockApi(); let progress!: (outcome: PinterestDownloadOutcome) => void, finish!: (outcomes: PinterestDownloadOutcome[]) => void;
    vi.mocked(api.downloadBatch).mockImplementation((_ids, _request, callback) => { progress = callback; return new Promise(resolve => { finish = resolve; }); });
    await render(api); await loadPin("621004236103700020"); await loadPin("621004236103700021");
    const cards = [...container.querySelectorAll<HTMLElement>(".pinterest-card")];
    await act(async () => cards[0]!.click());
    await act(async () => cards[1]!.dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true })));
    await act(async () => button("원본 다운로드").click());
    const ids = vi.mocked(api.downloadBatch).mock.calls[0]![0];
    const done: PinterestDownloadOutcome = { pinId: ids[0]!, status: "completed", record: { pin: pin(ids[0]), files: ["001.jpg"], bytes: 100, savedAt: 1 } };
    await act(async () => progress(done));
    expect(container.querySelector(".pinterest-batch")).toHaveTextContent("다운로드 중 · 1/2");
    expect(button("원본 다운로드")).toBeDisabled();
    // Lost last progress messages are recovered from the final native response,
    // while the first item emitted on both transports must not be counted twice.
    await act(async () => finish([done, { pinId: ids[1]!, status: "failed", error: "fixture network failure" }]));
    expect(container.querySelector(".pinterest-batch")).toHaveTextContent("처리 종료 · 2/2 · 실패 1");
    expect(container.querySelector('[role="alert"]')).toHaveTextContent("fixture network failure");
    expect(api.download).not.toHaveBeenCalled();
  });
  it("cancels a whole registered batch once on unmount and ignores late progress", async () => {
    const api = mockApi(); let progress!: (outcome: PinterestDownloadOutcome) => void;
    vi.mocked(api.downloadBatch).mockImplementation((_ids, _request, callback) => { progress = callback; return new Promise(() => {}); });
    await render(api); await loadPin("621004236103700020"); await loadPin("621004236103700021");
    const cards = [...container.querySelectorAll<HTMLElement>(".pinterest-card")];
    await act(async () => cards[0]!.click());
    await act(async () => cards[1]!.dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true })));
    await act(async () => button("원본 다운로드").click());
    const requestId = vi.mocked(api.downloadBatch).mock.calls[0]![1];
    await act(async () => root.unmount()); root = createRoot(container);
    expect(vi.mocked(api.cancel).mock.calls.filter(([id]) => id === requestId)).toHaveLength(1);
    await act(async () => progress({ pinId: "621004236103700020", status: "cancelled" }));
    expect(container).toBeEmptyDOMElement();
  });
  it("re-click clears selection and focus without sticky checkbox controls", async () => {
    const api = mockApi(); await render(api); await loadPin("621004236103700020");
    const card = container.querySelector<HTMLElement>(".pinterest-card")!;
    await act(async () => { card.focus(); card.click(); });
    await act(async () => card.click());
    expect(card).not.toHaveClass("is-selected"); expect(document.activeElement).not.toBe(card);
  });
  it("hydrates summaries only when detail opens and preserves local favorites without image bytes", async () => {
    const api = mockApi(); vi.mocked(api.discover).mockResolvedValue({ title: "작가", path: "/creator/", pins: [{ ...pin(), summaryOnly: true }], boards: [], limited: false });
    await render(api); await loadPin("@creator");
    expect(api.get).not.toHaveBeenCalled();
    await act(async () => container.querySelector(".pinterest-card")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
    expect(api.get).toHaveBeenCalledTimes(1);
    await act(async () => button("앨범 즐겨찾기").click());
    expect(loadPinterestFavorites()[0]!.pin.summaryOnly).not.toBe(true);
    expect(localStorage.getItem(PINTEREST_FAVORITES_KEY)).not.toContain("base64");
  });
  it("keeps missing-file records manageable and archive confirmation is non-destructive", async () => {
    const api = mockApi(); vi.mocked(api.downloads).mockResolvedValue([{ pin: pin(), files: ["001.jpg"], bytes: 100, savedAt: 1, missingFiles: true }]);
    await render(api, "downloads"); expect(container).toHaveTextContent("원본 파일 없음");
    await act(async () => container.querySelector(".pinterest-card")!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 20, clientY: 20 })));
    await act(async () => button("보관함에서 제외").click());
    expect(api.libraryAction).not.toHaveBeenCalled();
    expect(container.querySelector('[role="alertdialog"]')).toHaveTextContent("원본과 즐겨찾기는 유지");
    expect(container.querySelector('[role="alertdialog"]')).toHaveTextContent("저장 공간은 줄어들지 않습니다");
    await act(async () => button("제외").click());
    expect(api.libraryAction).toHaveBeenCalledWith("archive", pin().id, expect.any(String));
    expect(container.querySelector(".pinterest-card")).toBeNull();
    expect(button("제외된 핀")).toBeDefined();
  });
  it("prioritizes the Korean listing title while preserving the original detail title", async () => {
    const api = mockApi(); vi.mocked(api.get).mockResolvedValue(pin("621004236103700020", "Mountain | 산 풍경"));
    await render(api); await loadPin("621004236103700020");
    expect(container.querySelector(".pinterest-card-copy strong")).toHaveTextContent(/^산 풍경$/);
    await act(async () => container.querySelector(".pinterest-card")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
    expect(container.querySelector(".pinterest-detail h2")).toHaveTextContent("Mountain | 산 풍경");
  });
  it("loads cards, leaves a single selection unchecked, and opens details on double click", async () => {
    const api = mockApi(); await render(api); await loadPin("621004236103700020");
    expect(api.get).toHaveBeenCalledTimes(1);
    const card = container.querySelector<HTMLElement>(".pinterest-card")!;
    await act(async () => card.click());
    expect(card.classList.contains("is-selected")).toBe(true);
    expect(container.querySelector('[role="checkbox"]')).toBeNull();
    await act(async () => card.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
    expect(container.querySelector('[role="dialog"]')).not.toBeNull();
    expect(container.querySelector('[role="toolbar"]')).toBeNull();
    await act(async () => button("미리보기 닫기").click());
    expect(container.querySelector('[role="dialog"]')).toBeNull();
  });
  it("reveals the multi-select bar only from the second card and supports clearing", async () => {
    const api = mockApi(); await render(api); await loadPin("621004236103700020"); await loadPin("621004236103700021");
    const cards = [...container.querySelectorAll<HTMLElement>(".pinterest-card")];
    await act(async () => cards[0]!.click());
    await act(async () => cards[1]!.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true })));
    expect(container.querySelector('[role="toolbar"]')).toHaveTextContent("2개 선택");
    expect(container.querySelectorAll('[role="checkbox"]')).toHaveLength(2);
    await act(async () => button("선택 해제").click());
    expect(container.querySelector('[role="toolbar"]')).toBeNull();
  });
  it("provides context menu details, favorite and download controls", async () => {
    const api = mockApi(); await render(api); await loadPin("621004236103700020");
    await act(async () => container.querySelector(".pinterest-card")!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 20, clientY: 20 })));
    expect(document.querySelector('[role="menu"]')).toHaveTextContent("상세 열기");
    expect(document.querySelector('[role="menu"]')).toHaveTextContent("원본 다운로드");
    await act(async () => button("즐겨찾기 추가").click());
    expect(loadPinterestFavorites()).toHaveLength(1);
    expect(localStorage.getItem(PINTEREST_FAVORITES_KEY)).not.toContain("data:image");
  });
  it("keeps only album and page favorite categories and previews page favorites", async () => {
    localStorage.setItem(PINTEREST_FAVORITES_KEY, JSON.stringify([{ pin: { ...pin(), media: [pin().media[0], pin().media[0]] }, page: 1 }]));
    const api = mockApi(); await render(api, "favorites");
    expect(container.querySelector(".pinterest-card")).toBeNull();
    await act(async () => button("페이지").click());
    expect(container.querySelectorAll(".pinterest-card")).toHaveLength(1);
    await act(async () => container.querySelector(".pinterest-card")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
    expect(container.querySelector('[role="dialog"]')).toHaveAttribute("aria-label", "Pinterest 페이지 미리보기");
    expect(container.querySelector(".pinterest-page-count")).toHaveTextContent("2 / 2");
    await act(async () => container.querySelector('[role="dialog"]')!.dispatchEvent(new KeyboardEvent("keydown", { key: "a", bubbles: true })));
    expect(container.querySelector(".pinterest-page-count")).toHaveTextContent("1 / 2");
  });
  it("keeps different favorite pages of one pin independent in detail tabs", async () => {
    const value = { ...pin(), media: [pin().media[0]!, pin().media[0]!] };
    savePinterestFavorites([{ pin: value, page: 0 }, { pin: value, page: 1 }]);
    const api = mockApi(); await render(api, "favorites"); await act(async () => button("페이지").click());
    const cards = [...container.querySelectorAll<HTMLElement>('.pinterest-card')];
    await act(async () => cards[0]!.click());
    await act(async () => cards[1]!.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true })));
    await act(async () => cards[1]!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    expect(container.querySelector('.pinterest-page-count')).toHaveTextContent("1 / 2");
    expect(container.querySelectorAll('.pinterest-detail-tabs .is-active')).toHaveLength(1);
    await act(async () => button("산 풍경 · 2쪽").click());
    expect(container.querySelector('.pinterest-detail')).toHaveAttribute("aria-label", "Pinterest 페이지 미리보기");
    expect(container.querySelector('.pinterest-page-count')).toHaveTextContent("2 / 2");
    await act(async () => button("산 풍경 1쪽 상세 탭 닫기").click());
    expect(container.querySelector('[role="alertdialog"]')).toBeNull();
    expect(container.querySelector('.pinterest-page-count')).toHaveTextContent("2 / 2");
    expect(loadPinterestFavorites().map(item => item.page)).toEqual([0, 1]);
  });
  it("cancels pending loads and ignores late results", async () => {
    const api = mockApi(); let resolve!: (pin: PinterestPin) => void;
    vi.mocked(api.get).mockImplementation(() => new Promise(complete => { resolve = complete; }));
    await render(api); await loadPin("621004236103700020");
    await act(async () => button(" 취소").click());
    expect(api.cancel).toHaveBeenCalledTimes(1);
    await act(async () => resolve(pin()));
    expect(container.querySelector(".pinterest-card")).toBeNull();
  });
  it("shows invalid input and remote errors without empty-result success", async () => {
    const api = mockApi(); await render(api); await loadPin("https://localhost/pin/12345");
    expect(api.get).not.toHaveBeenCalled(); expect(container.querySelector('[role="alert"]')).toHaveTextContent("전체 링크");
    vi.mocked(api.get).mockRejectedValue("로그인이 필요한 핀입니다.");
    await loadPin("621004236103700020");
    expect(container.querySelector('[role="alert"]')).toHaveTextContent("로그인이 필요한");
  });
  it("disables video downloads and hides private images consistently", async () => {
    const api = mockApi(); vi.mocked(api.get).mockResolvedValue({ ...pin(), unsupportedVideo: true });
    await render(api, "explore", true); await loadPin("621004236103700020");
    expect(container.querySelector(".pinterest-workspace")).toHaveClass("is-private");
    await act(async () => container.querySelector(".pinterest-card")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
    expect(button("원본 저장")).toBeDisabled(); expect(container.querySelector(".pinterest-detail")).toHaveClass("is-private");
  });
  it("loads real download records and retries list failure", async () => {
    const api = mockApi(); vi.mocked(api.downloads).mockRejectedValueOnce("저장 목록 오류");
    await render(api, "downloads"); expect(container.querySelector('[role="alert"]')).toHaveTextContent("저장 목록 오류");
    vi.mocked(api.downloads).mockResolvedValue([{ pin: pin(), files: ["001.jpg"], bytes: 100, savedAt: 1 }]);
    await act(async () => button("새로고침").click());
    expect(container.querySelector(".pinterest-card")).toHaveTextContent("저장됨");
  });
  it("bounds rendered media to 48 cards when a library grows", async () => {
    const api = mockApi();
    localStorage.setItem(PINTEREST_FAVORITES_KEY, JSON.stringify(Array.from({ length: 100 }, (_, index) => ({ pin: pin(String(900000 + index)), page: null }))));
    await render(api, "favorites");
    expect(container.querySelectorAll(".pinterest-card")).toHaveLength(48);
    expect(container.querySelector(".pinterest-pagination")).toHaveTextContent("1 / 3");
    await act(async () => button("다음 목록 페이지").click());
    await act(async () => button("다음 목록 페이지").click());
    expect(container.querySelectorAll(".pinterest-card")).toHaveLength(4);
  });
  it("retains a library page across view changes even without a scroll event", async () => {
    const api = mockApi();
    savePinterestFavorites(Array.from({ length: 55 }, (_, index) => ({ pin: pin(String(700000 + index)), page: null })));
    await render(api, "favorites");
    await act(async () => button("다음 목록 페이지").click());
    await render(api, "downloads"); await render(api, "favorites");
    expect(container.querySelector('.pinterest-pagination')).toHaveTextContent("2 / 2");
    expect(container.querySelectorAll('.pinterest-card')).toHaveLength(7);
  });
});

