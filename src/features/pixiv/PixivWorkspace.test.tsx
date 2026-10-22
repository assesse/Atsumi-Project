import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { PixivWorkspace } from "./PixivWorkspace";
import { savePixivSafeOnly } from "./displayPreferences";
import type { PixivAccountSnapshot, PixivApi, PixivDiscovery, PixivSearch, PixivWork } from "./model";
const work: PixivWork = { id: "123", title: "Landscape fixture", artist: "Artist", artistId: "9", pageCount: 2, width: 800, height: 1000, thumbnailUrl: "https://i.pximg.net/c/250x250/img-master/img/test.jpg", bookmarkCount: 3, tags: ["landscape"], createdAt: "", type: "manga" };
let host: HTMLDivElement, root: Root;
const apiFactory = (): PixivApi => ({
  search: vi.fn().mockResolvedValue({ items: [work, { ...work, id: "456", title: "Second fixture" }], page: 1, hasMore: false }),
  detail: vi.fn().mockResolvedValue({ work, pages: [0, 1].map(index => ({ index, width: 800, height: 1000, previewUrl: `https://i.pximg.net/img-master/img/test_p${index}.jpg`, originalUrl: `https://i.pximg.net/img-original/img/test_p${index}.jpg` })) }),
  localDetail: vi.fn().mockResolvedValue({ work, pages: [0, 1].map(index => ({ index, width: 800, height: 1000, previewUrl: `atsumi-pixiv:123:${index}`, originalUrl: `atsumi-pixiv:123:${index}` })) }),
  localMedia: vi.fn().mockResolvedValue("data:image/jpeg;base64,bG9jYWw="),
  pageMedia: vi.fn().mockResolvedValue("data:image/jpeg;base64,b3JpZ2luYWw="),
  downloadBatch: vi.fn().mockResolvedValue([]),
  libraryAction: vi.fn().mockResolvedValue({}),
  media: vi.fn().mockResolvedValue("data:image/jpeg;base64,ZmFrZQ=="), download: vi.fn(), downloads: vi.fn().mockResolvedValue([]), progress: vi.fn().mockResolvedValue([]), cancel: vi.fn().mockResolvedValue(undefined), open: vi.fn().mockResolvedValue(undefined),
});
async function render(api: PixivApi, privacy = false, view: "explore" | "favorites" | "downloads" = "explore", previewWidth?: number) { await act(async () => { root.render(<PixivWorkspace active runtime="browser-mock" privacyMode={privacy} view={view} api={api} previewWidth={previewWidth} />); }); }
async function click(node: Element, options?: MouseEventInit) { await act(async () => { node.dispatchEvent(new MouseEvent("click", { bubbles: true, ...options })); }); }
async function settleClose() { await act(async () => { await new Promise(resolve => setTimeout(resolve, 160)); }); }
async function type(input: HTMLInputElement, value: string) { await act(async () => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value); input.dispatchEvent(new Event("input", { bubbles: true })); }); }
const homeResult = (tag = "sky"): PixivSearch => ({ items: [work], page: 1, hasMore: false, homeSuggestions: { recommendedTags: [tag], popularTags: ["landscape"] } });
const feedResult = (items: PixivWork[] = [work], page = 1, homeCursor: string | null = "pool:next"): PixivSearch => ({ ...homeResult(), items, page, hasMore: Boolean(homeCursor), homeCursor: homeCursor ?? undefined });
const favoriteHomeResult = (favoriteTags?: string[]): PixivSearch => ({ ...feedResult(), homeSuggestions: { recommendedTags: ["sky"], popularTags: ["landscape"], favoriteTags } });
async function submitSearch() { await act(async () => { host.querySelector("form.pixiv-search")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); }); }
describe("Pixiv workspace interaction", () => {
  beforeEach(() => { Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true }); localStorage.clear(); host = document.createElement("div"); document.body.append(host); root = createRoot(host); });
  afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.useRealTimers(); vi.restoreAllMocks(); });
  const discovery = (prefix = "Home"): PixivDiscovery => ({
    sections: [
      { id: "followed", title: "팔로우 신작", kind: "followed", items: [work] },
      { id: "recommended", title: "당신을 위한 추천 작품", kind: "recommended", items: [{ ...work, id: "456", title: `${prefix} second` }] },
      { id: "tag-0", title: "#sky", kind: "tag", tag: "sky", items: [{ ...work, id: "789", title: `${prefix} tagged` }] },
    ], creators: [{ id: "9", name: "Creator fixture", imageUrl: "https://i.pximg.net/user-profile/fixture.jpg", works: [] }],
    suggestions: { favoriteTags: ["sky"], favoriteTagTranslations: [{ tag: "sky", translation: "하늘", language: "ko" }], popularTags: ["landscape"], recommendedTags: [], popularNovelTags: ["nature"] }, cursor: "fixture:60", page: 1, issues: [],
  });
  it("renders separate account home sections and retains category tabs without changing card actions", async () => {
    const api = apiFactory(); api.discover = vi.fn().mockImplementation(async request => discovery(request.surface));
    await render(api);
    expect(api.discover).toHaveBeenCalledWith({ surface: "home" }); expect(api.search).not.toHaveBeenCalled();
    expect(host.querySelectorAll(".pixiv-discovery-section")).toHaveLength(4);
    const cards = host.querySelectorAll(".pixiv-card");
    await click(cards[0]!); expect(api.detail).not.toHaveBeenCalled();
    await click(cards[1]!, { ctrlKey: true }); expect(host.querySelector(".pixiv-selection-bar")?.textContent).toContain("2개 선택");
    await click(cards[0]!.querySelector(".pixiv-bookmark")!);
    expect(host.querySelectorAll(".pixiv-card.is-focused")).toHaveLength(2);
    const shell = host.querySelector<HTMLElement>(".pixiv-workspace")!; shell.scrollTop = 430;
    await click(host.querySelectorAll(".pixiv-home-categories button")[1]!);
    expect(api.discover).toHaveBeenLastCalledWith({ surface: "illustration" });
    await click(host.querySelectorAll(".pixiv-home-categories button")[0]!);
    await act(async () => { await new Promise(resolve => requestAnimationFrame(resolve)); });
    expect(api.discover).toHaveBeenCalledTimes(2); expect(shell.scrollTop).toBe(430);
    expect(host.querySelector(".pixiv-bookmark.is-saved")).not.toBeNull();
  });
  it("appends recommendations in-place without dropping other sections or selected cards", async () => {
    const api = apiFactory(); api.discover = vi.fn().mockResolvedValueOnce(discovery()).mockResolvedValueOnce({ sections: [{ id: "recommended", title: "추천", kind: "recommended", items: [work, { ...work, id: "999", title: "New appended" }] }], creators: [], cursor: undefined, suggestions: undefined, page: 2, issues: [] });
    await render(api); await click(host.querySelector(".pixiv-card")!);
    await click(host.querySelector('[aria-label="추천 작품 더 불러오기"]')!);
    expect(api.discover).toHaveBeenLastCalledWith({ surface: "home", cursor: "fixture:60", page: 2 });
    expect(host.querySelector('[aria-label="#sky"]')).not.toBeNull();
    expect(host.querySelector('[aria-label="New appended"]')).not.toBeNull();
    expect(host.querySelector(".pixiv-card.is-focused")).not.toBeNull();
    expect(host.querySelector('[aria-label="추천 작품 더 불러오기"]')).toBeNull();
  });
  it("overlays favorite translations and illustration/novel search tags even after leaving home", async () => {
    const api = apiFactory(); api.account = vi.fn().mockResolvedValue({ authenticated: true, open: false, busy: false, revision: 1 }); api.discover = vi.fn().mockResolvedValue(discovery());
    await render(api); const input = host.querySelector<HTMLInputElement>(".pixiv-search input")!;
    await type(input, "landscape"); await submitSearch();
    await act(async () => input.dispatchEvent(new FocusEvent("focusin", { bubbles: true })));
    const panel = host.querySelector(".pixiv-search-panel")!;
    expect(panel.textContent).toContain("인기 소설 태그"); expect(panel.textContent).toContain("인기 일러스트 태그"); expect(panel.textContent).toContain("하늘");
    await click([...panel.querySelectorAll("button")].find(button => button.textContent?.includes("하늘"))!);
    expect(api.search).toHaveBeenLastCalledWith({ query: "sky", mode: "latest", page: 1 });
    expect(host.querySelector(".pixiv-search-panel")).toBeNull();
  });
  it("keeps original preview usable when related discovery fails and makes a separate retry", async () => {
    const api = apiFactory(); api.discover = vi.fn().mockImplementation(async request => { if (request.surface === "related") throw new Error("fixture"); return discovery(); });
    await render(api);
    await act(async () => host.querySelector(".pixiv-card")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
    expect(document.querySelector(".pixiv-reader .pixiv-original-pages")).not.toBeNull();
    expect(document.querySelector(".pixiv-reader-related")?.textContent).toContain("관련 작품을 불러오지 못했습니다");
    expect(api.pageMedia).toHaveBeenCalled();
    expect(document.querySelector(".pixiv-detail-backdrop")).toBeNull();
    await click(document.querySelector(".pixiv-reader-related button:not(.pixiv-reader-related-nav button)")!);
    expect(api.discover).toHaveBeenCalledTimes(3);
  });
  it("ignores late related results when another work opens and keeps local readers offline", async () => {
    const api = apiFactory(); let complete!: (value: PixivDiscovery) => void;
    api.discover = vi.fn().mockImplementation(async request => request.surface === "related" ? new Promise(resolve => { complete = resolve; }) : discovery());
    await render(api); await act(async () => host.querySelector(".pixiv-card")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
    await render(api, true); await act(async () => complete(discovery("Late")));
    expect(document.querySelector(".pixiv-reader-related")).toBeNull();
    expect(document.body.textContent).not.toContain("Late second");
    await act(async () => root.render(null));
    api.downloads = vi.fn().mockResolvedValue([{ work, savedAt: "2026", pageCount: 2, bytes: 10, folder: "fixture" }]);
    vi.mocked(api.discover).mockClear();
    await render(api, false, "downloads"); await act(async () => host.querySelector(".pixiv-card")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
    expect(api.discover).not.toHaveBeenCalled(); expect(api.localMedia).toHaveBeenCalled();
  });
  it("applies both display choices to home and invalidates retained keyword results without submitting drafts", async () => {
    const api = apiFactory();
    await render(api);
    expect(api.search).toHaveBeenLastCalledWith({ query: "", mode: "home", page: 1 });
    await act(async () => { savePixivSafeOnly(true); });
    expect(api.search).toHaveBeenLastCalledWith({ query: "", mode: "home", page: 1, safeOnly: true });
    const input = host.querySelector<HTMLInputElement>(".pixiv-search input")!;
    await type(input, "landscape"); await submitSearch();
    expect(api.search).toHaveBeenLastCalledWith({ query: "landscape", mode: "latest", page: 1, safeOnly: true });
    const calls = vi.mocked(api.search).mock.calls.length;
    await act(async () => { savePixivSafeOnly(false); });
    expect(api.search).toHaveBeenCalledTimes(calls);
    expect(host.querySelectorAll(".pixiv-card")).toHaveLength(0);
    expect(host.textContent).toContain("표시 설정이 변경되었습니다");
    expect(input.value).toBe("landscape");
    await submitSearch();
    expect(api.search).toHaveBeenLastCalledWith({ query: "landscape", mode: "latest", page: 1 });
    expect(host.querySelectorAll(".pixiv-card")).toHaveLength(2);
  });
  it("ignores an old in-flight result after the display preference changes", async () => {
    const api = apiFactory(); let resolve!: (value: PixivSearch) => void;
    await render(api);
    vi.mocked(api.search).mockImplementationOnce(() => new Promise(done => { resolve = done; }));
    await type(host.querySelector<HTMLInputElement>(".pixiv-search input")!, "landscape");
    await submitSearch();
    await act(async () => { savePixivSafeOnly(true); });
    await act(async () => resolve({ items: [work], page: 1, hasMore: true }));
    expect(host.querySelectorAll(".pixiv-card")).toHaveLength(0);
    expect(host.querySelector(".pixiv-pagination")).toBeNull();
    expect(host.textContent).toContain("표시 설정이 변경되었습니다");
  });
  it("keeps local favorites and saved readers when the discovery preference changes", async () => {
    const api = apiFactory();
    localStorage.setItem("atsumi.pixiv.bookmarks.v1", JSON.stringify([{ work, savedAt: 1 }]));
    api.downloads = vi.fn().mockResolvedValue([{ work, savedAt: "2026", pageCount: 2, bytes: 10, folder: "fixture" }]);
    await render(api, false, "downloads");
    await act(async () => host.querySelector(".pixiv-card")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
    expect(document.querySelector('[aria-label="Pixiv 페이지 미리보기"]')).not.toBeNull();
    const bookmarkData = localStorage.getItem("atsumi.pixiv.bookmarks.v1");
    await act(async () => { savePixivSafeOnly(true); });
    expect(document.querySelector('[aria-label="Pixiv 페이지 미리보기"]')).not.toBeNull();
    expect(localStorage.getItem("atsumi.pixiv.bookmarks.v1")).toBe(bookmarkData);
    expect(api.search).not.toHaveBeenCalled();
  });
  it("leaves the next button absent on an empty result page", async () => {
    const api = apiFactory(); await render(api);
    vi.mocked(api.search).mockResolvedValue({ items: [], page: 2, hasMore: true });
    await type(host.querySelector<HTMLInputElement>(".pixiv-search input")!, "landscape"); await submitSearch();
    expect(host.querySelector(".pixiv-pagination")).toBeNull();
  });
  it("does not append a response from an earlier display preference into the new home pool", async () => {
    const api = apiFactory(); let resolve!: (value: PixivSearch) => void;
    vi.mocked(api.search).mockImplementation(async request => request.homeCursor
      ? new Promise(done => { resolve = done; })
      : feedResult([request.safeOnly ? { ...work, id: "789", title: "New preference fixture" } : work], 1, request.safeOnly ? "new:pool" : "old:pool"));
    await render(api); await click(host.querySelector('[aria-label="추천 작품 더 불러오기"]')!);
    await act(async () => { savePixivSafeOnly(true); });
    await act(async () => resolve(feedResult([{ ...work, id: "456", title: "Stale preference fixture" }], 2, "old:next")));
    expect(host.textContent).toContain("New preference fixture");
    expect(host.textContent).not.toContain("Stale preference fixture");
    expect(api.search).toHaveBeenLastCalledWith({ query: "", mode: "home", page: 1, safeOnly: true });
  });
  it.each(["explore", "favorites", "downloads"] as const)("puts %s search first, before account controls and their errors", async view => {
    const api = apiFactory();
    api.account = vi.fn().mockResolvedValue({ authenticated: null, open: false, busy: false, revision: 1, error: "로그인 확인 오류" });
    await render(api, false, view);
    const workspace = host.querySelector(".pixiv-workspace")!;
    const toolbar = workspace.firstElementChild!;
    const form = host.querySelector("form.pixiv-search")!;
    const account = host.querySelector(".pixiv-account-controls")!;
    expect(toolbar.className).toBe("pixiv-toolbar");
    expect(toolbar.firstElementChild).toBe(form);
    expect(form.compareDocumentPosition(account) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(toolbar.nextElementSibling).toBe(host.querySelector(".pixiv-account-error"));
    expect(host.querySelector(".pixiv-heading")).toBeNull();
    expect(toolbar.querySelector("input,select,button")).toBe(form.querySelector("input"));
  });
  it("keeps anonymous home and keyword results through unchanged signed-out polls", async () => {
    vi.useFakeTimers(); const api = apiFactory();
    api.account = vi.fn().mockResolvedValue({ authenticated: false, open: false, busy: false, revision: 1 });
    await render(api);
    expect(api.search).toHaveBeenCalledWith({ query: "", mode: "home", page: 1 });
    const input = host.querySelector<HTMLInputElement>(".pixiv-search input")!;
    await type(input, "landscape");
    await act(async () => { host.querySelector("form.pixiv-search")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); });
    expect(api.search).toHaveBeenLastCalledWith({ query: "landscape", mode: "latest", page: 1 });
    const cards = [...host.querySelectorAll(".pixiv-card")];
    expect(cards).toHaveLength(2);
    await act(async () => { await vi.advanceTimersByTimeAsync(15000); });
    expect([...host.querySelectorAll(".pixiv-card")]).toEqual(cards);
    expect(api.search).toHaveBeenCalledTimes(2);
    expect(input.value).toBe("landscape");
    expect(host.querySelector(".pixiv-account-controls")?.textContent).toContain("로그아웃됨");
    expect(host.querySelector(".pixiv-account-error")).toBeNull();
  });
  it.each([false, true])("waits for initial account proof (%s) and preserves an unsubmitted draft while loading home", async authenticated => {
    const api = apiFactory(); let resolveAccount!: (value: PixivAccountSnapshot) => void;
    api.account = vi.fn().mockImplementation(() => new Promise(resolve => { resolveAccount = resolve; }));
    vi.mocked(api.search).mockResolvedValue(homeResult());
    await render(api);
    expect(api.search).not.toHaveBeenCalled();
    expect(host.querySelector<HTMLButtonElement>('[aria-label="추천 작품 보기"]')!.disabled).toBe(true);
    const input = host.querySelector<HTMLInputElement>(".pixiv-search input")!;
    await type(input, "not submitted");
    await act(async () => resolveAccount({ authenticated, open: false, busy: false, revision: 7 }));
    expect(api.search).toHaveBeenCalledExactlyOnceWith({ query: "", page: 1, mode: "home" });
    expect(input.value).toBe("not submitted");
    expect(host.querySelectorAll(".pixiv-card")).toHaveLength(1);
    expect(host.querySelector('[aria-label="Pixiv 추천 검색어"]')?.textContent).toContain("sky");
    expect(host.querySelector(".pixiv-pagination")).toBeNull();
  });
  it("uses official tags for retained searches and returns to cached home without another request", async () => {
    const api = apiFactory();
    vi.mocked(api.search).mockImplementation(async request => request.mode === "home" ? homeResult() : { items: [{ ...work, id: "456", title: "Search fixture" }], page: 1, hasMore: false });
    await render(api);
    const tag = [...host.querySelectorAll(".pixiv-discovery-tags button")].find(button => button.textContent === "sky")!;
    await click(tag);
    expect(api.search).toHaveBeenLastCalledWith({ query: "sky", page: 1, mode: "latest" });
    expect(host.querySelector<HTMLInputElement>(".pixiv-search input")!.value).toBe("sky");
    expect(host.querySelector('[aria-label="Pixiv 추천 검색어"]')).toBeNull();
    expect(host.querySelectorAll(".pixiv-search-tabs>div")).toHaveLength(2);
    await click([...host.querySelectorAll(".pixiv-search-tabs button")].find(button => button.textContent === "추천")!);
    expect(api.search).toHaveBeenCalledTimes(2);
    expect(host.querySelector(".pixiv-card-info strong")?.textContent).toBe(work.title);
    expect(host.querySelector('[aria-label="Pixiv 추천 검색어"]')?.textContent).toContain("sky");
    const input = host.querySelector<HTMLInputElement>(".pixiv-search input")!;
    await type(input, "another draft"); await type(input, ""); await submitSearch();
    expect(api.search).toHaveBeenCalledTimes(2);
    expect(host.querySelector<HTMLSelectElement>(".pixiv-search select")!.value).toBe("home");
  });
  it("shows official account favorite tags before recommendations using the same tag buttons", async () => {
    const api = apiFactory(); api.account = vi.fn().mockResolvedValue({ authenticated: true, open: false, busy: false, revision: 1 });
    vi.mocked(api.search).mockResolvedValue(favoriteHomeResult(["favorite-sky", "favorite-landscape"]));
    await render(api);
    const groups = host.querySelectorAll(".pixiv-discovery-tags>div");
    expect([...groups].map(group => group.querySelector("span")?.textContent)).toEqual(["즐겨찾기 태그", "추천 검색어", "인기 태그"]);
    expect([...groups[0]!.querySelectorAll("button")].map(button => button.textContent)).toEqual(["favorite-sky", "favorite-landscape"]);
    expect(host.querySelector(".pixiv-favorite-tags-error")).toBeNull();
    expect(host.querySelector(".pixiv-workspace")?.firstElementChild).toBe(host.querySelector(".pixiv-toolbar"));
    expect(api.search).toHaveBeenCalledExactlyOnceWith({ query: "", page: 1, mode: "home" });
    expect(api.download).not.toHaveBeenCalled(); expect(api.open).not.toHaveBeenCalled();
  });
  it("favorite tag click uses retained keyword search and restores appended home with its tags and scroll", async () => {
    vi.useFakeTimers(); const api = apiFactory(); api.account = vi.fn().mockResolvedValue({ authenticated: true, open: false, busy: false, revision: 1 });
    vi.mocked(api.search).mockResolvedValueOnce(favoriteHomeResult(["my-landscape"])).mockResolvedValueOnce(feedResult([{ ...work, id: "456", title: "Appended favorite fixture" }], 2)).mockResolvedValueOnce({ items: [{ ...work, id: "789", title: "Favorite tag search fixture" }], page: 1, hasMore: false });
    localStorage.setItem("atsumi.pixiv.bookmarks.v1", JSON.stringify([{ work, savedAt: 1 }]));
    await render(api); const bookmarks = localStorage.getItem("atsumi.pixiv.bookmarks.v1");
    await click(host.querySelector('[aria-label="추천 작품 더 불러오기"]')!);
    const workspace = host.querySelector<HTMLElement>(".pixiv-workspace")!; workspace.scrollTop = 320;
    await click([...host.querySelectorAll(".pixiv-discovery-tags button")].find(button => button.textContent === "my-landscape")!);
    expect(api.search).toHaveBeenLastCalledWith({ query: "my-landscape", page: 1, mode: "latest" });
    await act(async () => { await vi.advanceTimersByTimeAsync(20); }); expect(workspace.scrollTop).toBe(0);
    expect(host.querySelector<HTMLInputElement>(".pixiv-search input")!.value).toBe("my-landscape");
    await click([...host.querySelectorAll(".pixiv-search-tabs button")].find(button => button.textContent === "추천")!);
    await act(async () => { await vi.advanceTimersByTimeAsync(20); }); expect(workspace.scrollTop).toBe(320);
    expect(api.search).toHaveBeenCalledTimes(3); expect(host.querySelectorAll(".pixiv-card")).toHaveLength(2);
    expect(host.querySelector(".pixiv-discovery-tags")?.textContent).toContain("my-landscape");
    expect(localStorage.getItem("atsumi.pixiv.bookmarks.v1")).toBe(bookmarks); expect(api.download).not.toHaveBeenCalled(); expect(api.libraryAction).not.toHaveBeenCalled();
  });
  it("a verified empty favorite list is not reported as a failure", async () => {
    const api = apiFactory(); api.account = vi.fn().mockResolvedValue({ authenticated: true, open: false, busy: false, revision: 1 });
    vi.mocked(api.search).mockResolvedValue(favoriteHomeResult([]));
    await render(api);
    expect(host.querySelector(".pixiv-discovery-tags")?.textContent).not.toContain("즐겨찾기 태그");
    expect(host.querySelector(".pixiv-favorite-tags-error")).toBeNull(); expect(host.querySelectorAll(".pixiv-card")).toHaveLength(1);
  });
  it.each([undefined, null, "not an array", ["valid", 42]])("unavailable or malformed favorite tags show retry guidance without losing recommendations (%j)", async favoriteTags => {
    const api = apiFactory(); api.account = vi.fn().mockResolvedValue({ authenticated: true, open: false, busy: false, revision: 1 });
    vi.mocked(api.search).mockResolvedValueOnce(favoriteHomeResult(favoriteTags as unknown as string[])).mockResolvedValueOnce(favoriteHomeResult(["recovered-favorite"]));
    await render(api);
    expect(host.querySelector(".pixiv-favorite-tags-error")?.textContent).toContain("추천 작품을 새로고침");
    expect(host.querySelectorAll(".pixiv-card")).toHaveLength(1); expect(host.querySelector(".pixiv-discovery-tags")?.textContent).toContain("sky");
    await click(host.querySelector('[aria-label="추천 작품 새로고침"]')!);
    expect(host.querySelector(".pixiv-favorite-tags-error")).toBeNull(); expect(host.querySelector(".pixiv-discovery-tags")?.textContent).toContain("recovered-favorite");
  });
  it("initial home loading never reports favorite tags as failed before the response", async () => {
    const api = apiFactory(); let finish!: (value: PixivSearch) => void;
    api.account = vi.fn().mockResolvedValue({ authenticated: true, open: false, busy: false, revision: 1 });
    vi.mocked(api.search).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    await render(api); expect(host.querySelector(".pixiv-favorite-tags-error")).toBeNull();
    await act(async () => finish(favoriteHomeResult([]))); expect(host.querySelector(".pixiv-favorite-tags-error")).toBeNull();
  });
  it("guest sessions never display favorite tags or unavailable guidance even from an unexpected fixture", async () => {
    const api = apiFactory(); api.account = vi.fn().mockResolvedValue({ authenticated: false, open: false, busy: false, revision: 1 });
    vi.mocked(api.search).mockResolvedValueOnce(favoriteHomeResult(["private-account-tag"])).mockResolvedValueOnce(favoriteHomeResult());
    await render(api); expect(host.textContent).not.toContain("private-account-tag"); expect(host.querySelector(".pixiv-favorite-tags-error")).toBeNull();
    await click(host.querySelector('[aria-label="추천 작품 새로고침"]')!); expect(host.querySelector(".pixiv-favorite-tags-error")).toBeNull();
    expect(host.querySelector(".pixiv-discovery-tags")?.textContent).toContain("sky");
  });
  it("hides account favorite tags during privacy, inactive and unverified-busy states", async () => {
    vi.useFakeTimers(); const api = apiFactory(); api.account = vi.fn().mockResolvedValue({ authenticated: true, open: false, busy: false, revision: 1 });
    vi.mocked(api.search).mockResolvedValue(favoriteHomeResult(["private-account-tag"]));
    await render(api); expect(host.textContent).toContain("private-account-tag");
    await render(api, true); expect(host.textContent).not.toContain("private-account-tag"); expect(host.querySelector(".pixiv-favorite-tags-error")).toBeNull();
    await act(async () => root.render(<PixivWorkspace active={false} runtime="browser-mock" privacyMode={false} view="explore" api={api} />));
    expect(host.textContent).not.toContain("private-account-tag"); expect(api.search).toHaveBeenCalledTimes(1);
    await render(api); expect(host.textContent).toContain("private-account-tag");
    vi.mocked(api.account).mockResolvedValue({ authenticated: true, open: false, busy: true, revision: 1 });
    await act(async () => { await vi.advanceTimersByTimeAsync(2500); });
    expect(host.textContent).not.toContain("private-account-tag"); expect(host.querySelector(".pixiv-favorite-tags-error")).toBeNull();
  });
  it("logout and reauthentication cannot reuse favorite tags even when the account revision is unchanged", async () => {
    vi.useFakeTimers(); const api = apiFactory(); api.account = vi.fn().mockResolvedValue({ authenticated: true, open: false, busy: false, revision: 1 });
    api.logout = vi.fn().mockResolvedValue({ authenticated: false, open: false, busy: false, revision: 1 });
    vi.mocked(api.search).mockResolvedValueOnce(favoriteHomeResult(["old-account-favorite"])).mockResolvedValueOnce(favoriteHomeResult(["new-account-favorite"]));
    localStorage.setItem("atsumi.pixiv.bookmarks.v1", JSON.stringify([{ work, savedAt: 1 }]));
    await render(api); const bookmarks = localStorage.getItem("atsumi.pixiv.bookmarks.v1");
    const input = host.querySelector<HTMLInputElement>(".pixiv-search input")!; await type(input, "unsubmitted draft");
    await click([...host.querySelectorAll(".pixiv-account-controls button")].find(button => button.textContent === "로그아웃")!);
    expect(host.textContent).not.toContain("old-account-favorite"); expect(host.querySelector(".pixiv-favorite-tags-error")).toBeNull(); expect(input.value).toBe("unsubmitted draft");
    await act(async () => { await vi.advanceTimersByTimeAsync(2500); });
    expect(host.textContent).not.toContain("old-account-favorite"); expect(api.search).toHaveBeenCalledTimes(1);
    await click(host.querySelector('[aria-label="추천 작품 새로고침"]')!);
    expect(host.textContent).toContain("new-account-favorite"); expect(host.textContent).not.toContain("old-account-favorite");
    expect(input.value).toBe("unsubmitted draft"); expect(localStorage.getItem("atsumi.pixiv.bookmarks.v1")).toBe(bookmarks);
  });
  it("unknown proof fences late account favorite tags until a fresh authenticated home response", async () => {
    vi.useFakeTimers(); const api = apiFactory(); let finish!: (value: PixivSearch) => void;
    api.account = vi.fn().mockResolvedValue({ authenticated: true, open: false, busy: false, revision: 1 });
    vi.mocked(api.search).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; })).mockResolvedValueOnce(favoriteHomeResult(["current-account-favorite"]));
    await render(api);
    vi.mocked(api.account).mockResolvedValue({ authenticated: null, open: false, busy: false, revision: 1 });
    await act(async () => { await vi.advanceTimersByTimeAsync(2500); finish(favoriteHomeResult(["stale-account-favorite"])); });
    expect(host.textContent).not.toContain("stale-account-favorite"); expect(host.querySelector(".pixiv-favorite-tags-error")).toBeNull();
    vi.mocked(api.account).mockResolvedValue({ authenticated: true, open: false, busy: false, revision: 1 });
    await act(async () => { await vi.advanceTimersByTimeAsync(2500); });
    expect(host.textContent).toContain("current-account-favorite"); expect(host.textContent).not.toContain("stale-account-favorite");
  });
  it("refreshes a changed account home once while removing old tags before its reply", async () => {
    vi.useFakeTimers(); const api = apiFactory(); let finish!: (value: PixivSearch) => void;
    api.account = vi.fn().mockResolvedValue({ authenticated: false, open: false, busy: false, revision: 1 });
    vi.mocked(api.search).mockResolvedValueOnce(homeResult("old-account-tag")).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    await render(api);
    vi.mocked(api.account).mockResolvedValue({ authenticated: true, open: false, busy: false, revision: 2 });
    await act(async () => { await vi.advanceTimersByTimeAsync(2500); });
    expect(host.textContent).not.toContain("old-account-tag");
    expect(host.querySelectorAll(".pixiv-card")).toHaveLength(0);
    expect(api.search).toHaveBeenCalledTimes(2);
    await act(async () => finish(homeResult("new-account-tag")));
    expect(host.textContent).toContain("new-account-tag");
    await act(async () => { await vi.advanceTimersByTimeAsync(15000); });
    expect(api.search).toHaveBeenCalledTimes(2);
  });
  it("clears old home suggestions on account change without erasing a draft or selecting home over a keyword tab", async () => {
    vi.useFakeTimers(); const api = apiFactory();
    api.account = vi.fn().mockResolvedValue({ authenticated: true, open: false, busy: false, revision: 1 });
    vi.mocked(api.search).mockResolvedValue(homeResult("old-private-tag"));
    await render(api);
    const input = host.querySelector<HTMLInputElement>(".pixiv-search input")!;
    await type(input, "draft survives");
    vi.mocked(api.account).mockResolvedValue({ authenticated: false, open: false, busy: false, revision: 2 });
    await act(async () => { await vi.advanceTimersByTimeAsync(2500); });
    expect(host.textContent).not.toContain("old-private-tag");
    expect(input.value).toBe("draft survives"); expect(api.search).toHaveBeenCalledTimes(1);
    vi.mocked(api.search).mockResolvedValue({ items: [work], page: 1, hasMore: false });
    await submitSearch();
    const activeTab = host.querySelector(".pixiv-search-tabs>.is-active");
    vi.mocked(api.account).mockResolvedValue({ authenticated: true, open: false, busy: false, revision: 3 });
    await act(async () => { await vi.advanceTimersByTimeAsync(2500); });
    expect(host.querySelector(".pixiv-search-tabs>.is-active")).toBe(activeTab);
    expect(input.value).toBe("draft survives"); expect(api.search).toHaveBeenCalledTimes(2);
    expect(host.textContent).not.toContain("old-private-tag");
  });
  it("unknown account proof does not automatically fetch home and disables home refresh", async () => {
    vi.useFakeTimers(); const api = apiFactory();
    api.account = vi.fn().mockResolvedValue({ authenticated: null, open: false, busy: false, revision: 4, error: "확인 필요" });
    await render(api); await act(async () => { await vi.advanceTimersByTimeAsync(5000); });
    expect(api.search).not.toHaveBeenCalled();
    expect(host.querySelector<HTMLButtonElement>('[aria-label="추천 작품 보기"]')!.disabled).toBe(true);
    vi.mocked(api.account).mockResolvedValue({ authenticated: false, open: false, busy: false, revision: 4 });
    await act(async () => { await vi.advanceTimersByTimeAsync(2500); });
    expect(api.search).toHaveBeenCalledExactlyOnceWith({ query: "", mode: "home", page: 1 });
  });
  it("cancelled home responses cannot publish suggestions or cards and refresh can recover", async () => {
    const api = apiFactory(); let finish!: (value: PixivSearch) => void;
    vi.mocked(api.search).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; })).mockResolvedValue(homeResult("fresh-tag"));
    await render(api);
    await click([...host.querySelectorAll("form.pixiv-search button")].find(button => button.textContent === "취소")!);
    await act(async () => finish(homeResult("cancelled-tag")));
    expect(host.querySelectorAll(".pixiv-card")).toHaveLength(0);
    expect(host.textContent).not.toContain("cancelled-tag");
    await click(host.querySelector('[aria-label="추천 작품 새로고침"]')!);
    expect(host.textContent).toContain("fresh-tag"); expect(api.search).toHaveBeenCalledTimes(2);
  });
  it("failed home refresh has an explicit retry and never polls search repeatedly", async () => {
    vi.useFakeTimers(); const api = apiFactory();
    api.account = vi.fn().mockResolvedValue({ authenticated: false, open: false, busy: false, revision: 1 });
    vi.mocked(api.search).mockResolvedValueOnce(homeResult()).mockRejectedValueOnce(new Error("일시적인 추천 오류")).mockResolvedValueOnce(homeResult("recovered-tag"));
    await render(api); await click(host.querySelector('[aria-label="추천 작품 새로고침"]')!);
    expect(host.querySelector('[role="alert"]')?.textContent).toContain("일시적인 추천 오류");
    expect(host.querySelector<HTMLButtonElement>('[aria-label="추천 작품 새로고침"]')!.disabled).toBe(false);
    await act(async () => { await vi.advanceTimersByTimeAsync(10000); });
    expect(api.search).toHaveBeenCalledTimes(2);
    await click(host.querySelector('[aria-label="추천 작품 새로고침"]')!);
    expect(host.querySelector('[role="alert"]')).toBeNull();
    expect(host.textContent).toContain("recovered-tag"); expect(api.search).toHaveBeenCalledTimes(3);
  });
  it("hides retained suggestions in privacy and inactive views without fetching", async () => {
    const api = apiFactory(); vi.mocked(api.search).mockResolvedValue(homeResult());
    await render(api, true); expect(api.search).not.toHaveBeenCalled();
    await render(api); expect(api.search).toHaveBeenCalledTimes(1);
    expect(host.querySelector('[aria-label="Pixiv 추천 검색어"]')).not.toBeNull();
    await render(api, true);
    expect(host.querySelector('[aria-label="Pixiv 추천 검색어"]')).toBeNull();
    expect(host.querySelector("img")).toBeNull(); expect(api.search).toHaveBeenCalledTimes(1);
    await act(async () => root.render(<PixivWorkspace active={false} runtime="browser-mock" privacyMode={false} view="explore" api={api} />));
    expect(host.children).toHaveLength(0); expect(api.search).toHaveBeenCalledTimes(1);
    await render(api); expect(host.querySelector('[aria-label="Pixiv 추천 검색어"]')).not.toBeNull();
    expect(api.search).toHaveBeenCalledTimes(1);
  });
  it("closing the last home tab offers a one-shot return instead of reopening on every poll", async () => {
    vi.useFakeTimers(); const api = apiFactory();
    api.account = vi.fn().mockResolvedValue({ authenticated: false, open: false, busy: false, revision: 1 });
    vi.mocked(api.search).mockResolvedValue(homeResult());
    await render(api); await click(host.querySelector('[aria-label="추천 닫기"]')!);
    await act(async () => { await vi.advanceTimersByTimeAsync(15000); });
    expect(host.querySelector(".pixiv-search-tabs")).toBeNull();
    expect(host.querySelector('[aria-label="Pixiv 추천 검색어"]')).toBeNull();
    expect(api.search).toHaveBeenCalledTimes(1);
    await click(host.querySelector('[aria-label="추천 작품 보기"]')!);
    expect(api.search).toHaveBeenCalledTimes(2);
    expect(host.querySelectorAll(".pixiv-search-tabs>div")).toHaveLength(1);
  });
  it("appends home without replacing card nodes, selected IDs, Shift anchor, draft or scroll", async () => {
    const api = apiFactory(), second = { ...work, id: "456", title: "Second fixture" }, third = { ...work, id: "789", title: "Third fixture" };
    vi.mocked(api.search).mockResolvedValueOnce(feedResult([work, second])).mockResolvedValueOnce({ ...feedResult([{ ...work, title: "Duplicate replacement" }, third, third], 2), homeSuggestions: { recommendedTags: ["must-not-replace-tags"], popularTags: [] } });
    await render(api);
    const workspace = host.querySelector<HTMLElement>(".pixiv-workspace")!, cards = [...host.querySelectorAll(".pixiv-card")];
    await click(cards[0]!); await click(cards[1]!, { ctrlKey: true });
    const input = host.querySelector<HTMLInputElement>(".pixiv-search input")!; await type(input, "unsubmitted draft");
    workspace.scrollTop = 320;
    const mediaCalls = vi.mocked(api.media).mock.calls.length;
    await click(host.querySelector('[aria-label="추천 작품 더 불러오기"]')!);
    expect(api.search).toHaveBeenLastCalledWith({ query: "", mode: "home", page: 2, homeCursor: "pool:next" });
    const appended = [...host.querySelectorAll(".pixiv-card")];
    expect(appended).toHaveLength(3); expect(appended[0]).toBe(cards[0]); expect(appended[1]).toBe(cards[1]);
    expect(host.textContent).not.toContain("Duplicate replacement"); expect(workspace.scrollTop).toBe(320);
    expect(input.value).toBe("unsubmitted draft"); expect(host.querySelectorAll('[role="checkbox"][aria-checked="true"]')).toHaveLength(2);
    expect(host.textContent).not.toContain("must-not-replace-tags"); expect(api.media).toHaveBeenCalledTimes(mediaCalls);
    await click(appended[2]!, { shiftKey: true });
    expect(appended[0]!.classList.contains("is-focused")).toBe(false);
    expect(appended[1]!.classList.contains("is-focused")).toBe(true); expect(appended[2]!.classList.contains("is-focused")).toBe(true);
  });
  it("keeps failed append cards and retries exactly the same batch and cursor", async () => {
    const api = apiFactory();
    vi.mocked(api.search).mockResolvedValueOnce(feedResult()).mockRejectedValueOnce(new Error("추천 보관 시간이 만료되었습니다. 새로고침해 주세요.")).mockResolvedValueOnce(feedResult([{ ...work, id: "456", title: "Added fixture" }], 2, null));
    await render(api); const card = host.querySelector(".pixiv-card")!;
    const workspace = host.querySelector<HTMLElement>(".pixiv-workspace")!; workspace.scrollTop = 320; await click(card);
    await click(host.querySelector('[aria-label="추천 작품 더 불러오기"]')!);
    expect(host.querySelector(".pixiv-card")).toBe(card); expect(card.classList.contains("is-focused")).toBe(true); expect(workspace.scrollTop).toBe(320);
    expect(host.querySelector('.pixiv-home-more [role="alert"]')?.textContent).toContain("새로고침");
    expect(host.querySelector<HTMLButtonElement>('[aria-label="추천 작품 새로고침"]')!.disabled).toBe(false);
    await click(host.querySelector('[aria-label="추천 작품 추가 불러오기 다시 시도"]')!);
    expect(vi.mocked(api.search).mock.calls[2]).toEqual(vi.mocked(api.search).mock.calls[1]);
    expect(host.querySelectorAll(".pixiv-card")).toHaveLength(2); expect(host.querySelector(".pixiv-home-more")).toBeNull();
  });
  it("does not duplicate an in-flight append, supports cancellation and ignores its late reply", async () => {
    const api = apiFactory(); let finish!: (value: PixivSearch) => void;
    vi.mocked(api.search).mockResolvedValueOnce(feedResult()).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; })).mockResolvedValueOnce(feedResult([{ ...work, id: "789", title: "Retried fixture" }], 2, null));
    await render(api); const more = host.querySelector('[aria-label="추천 작품 더 불러오기"]')!;
    await act(async () => { more.dispatchEvent(new MouseEvent("click", { bubbles: true })); more.dispatchEvent(new MouseEvent("click", { bubbles: true })); });
    expect(api.search).toHaveBeenCalledTimes(2);
    await click(host.querySelector('[aria-label="추천 작품 추가 불러오기 취소"]')!);
    await act(async () => finish(feedResult([{ ...work, id: "456", title: "Cancelled fixture" }], 2)));
    expect(host.textContent).not.toContain("Cancelled fixture"); expect(host.querySelectorAll(".pixiv-card")).toHaveLength(1);
    await click(host.querySelector('[aria-label="추천 작품 더 불러오기"]')!);
    expect(vi.mocked(api.search).mock.calls[2]).toEqual(vi.mocked(api.search).mock.calls[1]);
    expect(host.textContent).toContain("Retried fixture");
  });
  it("refresh invalidates pending append and starts a new pool without old cursor", async () => {
    const api = apiFactory(); let finish!: (value: PixivSearch) => void;
    vi.mocked(api.search).mockResolvedValueOnce(feedResult()).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; })).mockResolvedValueOnce(feedResult([{ ...work, id: "789", title: "Refreshed fixture" }], 1, "fresh:pool")).mockResolvedValueOnce(feedResult([], 2, null));
    await render(api); await click(host.querySelector('[aria-label="추천 작품 더 불러오기"]')!);
    await click(host.querySelector('[aria-label="추천 작품 새로고침"]')!);
    expect(api.search).toHaveBeenLastCalledWith({ query: "", mode: "home", page: 1 });
    await act(async () => finish(feedResult([{ ...work, id: "456", title: "Stale fixture" }], 2)));
    expect(host.textContent).not.toContain("Stale fixture"); expect(host.textContent).toContain("Refreshed fixture");
    await click(host.querySelector('[aria-label="추천 작품 더 불러오기"]')!);
    expect(api.search).toHaveBeenLastCalledWith({ query: "", mode: "home", page: 2, homeCursor: "fresh:pool" });
  });
  it("closing and reopening home cannot receive an old pool reply with the same tab ID", async () => {
    const api = apiFactory(); let finish!: (value: PixivSearch) => void;
    vi.mocked(api.search).mockResolvedValueOnce(feedResult()).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; })).mockResolvedValueOnce(feedResult([{ ...work, id: "789", title: "New home fixture" }], 1, "new:pool"));
    await render(api); await click(host.querySelector('[aria-label="추천 작품 더 불러오기"]')!);
    await click(host.querySelector('[aria-label="추천 닫기"]')!); await click(host.querySelector('[aria-label="추천 작품 보기"]')!);
    await act(async () => finish(feedResult([{ ...work, id: "456", title: "Closed tab fixture" }], 2)));
    expect(host.textContent).not.toContain("Closed tab fixture"); expect(host.textContent).toContain("New home fixture");
    expect(host.querySelectorAll(".pixiv-card")).toHaveLength(1);
  });
  it("retains appended home independent of batch number after tag search and empty submit", async () => {
    vi.useFakeTimers(); const api = apiFactory();
    vi.mocked(api.search).mockResolvedValueOnce(feedResult()).mockResolvedValueOnce(feedResult([{ ...work, id: "456", title: "More fixture" }], 2)).mockResolvedValueOnce({ items: [{ ...work, id: "789", title: "Keyword fixture" }], page: 1, hasMore: false });
    await render(api); await click(host.querySelector('[aria-label="추천 작품 더 불러오기"]')!);
    const workspace = host.querySelector<HTMLElement>(".pixiv-workspace")!; workspace.scrollTop = 320;
    await click([...host.querySelectorAll(".pixiv-discovery-tags button")].find(button => button.textContent === "sky")!);
    await act(async () => { await vi.advanceTimersByTimeAsync(20); }); expect(workspace.scrollTop).toBe(0);
    await type(host.querySelector<HTMLInputElement>(".pixiv-search input")!, ""); await submitSearch();
    await act(async () => { await vi.advanceTimersByTimeAsync(20); }); expect(workspace.scrollTop).toBe(320);
    expect(api.search).toHaveBeenCalledTimes(3); expect(host.querySelectorAll(".pixiv-card")).toHaveLength(2);
    expect(host.textContent).toContain("More fixture"); expect(host.textContent).not.toContain("Keyword fixture");
  });
  it("background append updates only its retained home, not a selected keyword tab or draft", async () => {
    const api = apiFactory(); let finish!: (value: PixivSearch) => void;
    vi.mocked(api.search).mockResolvedValueOnce(feedResult()).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; })).mockResolvedValueOnce({ items: [{ ...work, id: "789", title: "Keyword fixture" }], page: 1, hasMore: false });
    await render(api); await click(host.querySelector('[aria-label="추천 작품 더 불러오기"]')!);
    const input = host.querySelector<HTMLInputElement>(".pixiv-search input")!; await type(input, "sky"); await submitSearch(); await type(input, "next draft");
    const keyword = host.querySelector(".pixiv-card");
    await act(async () => finish(feedResult([{ ...work, id: "456", title: "Background fixture" }], 2)));
    expect(host.querySelector(".pixiv-card")).toBe(keyword); expect(input.value).toBe("next draft"); expect(host.querySelector(".pixiv-home-more")).toBeNull();
    await click([...host.querySelectorAll(".pixiv-search-tabs button")].find(button => button.textContent === "추천")!);
    expect(host.querySelectorAll(".pixiv-card")).toHaveLength(2); expect(host.textContent).toContain("Background fixture"); expect(api.search).toHaveBeenCalledTimes(3);
  });
  it("account invalidation discards a pending home append and its continuation", async () => {
    vi.useFakeTimers(); const api = apiFactory(); let finish!: (value: PixivSearch) => void;
    api.account = vi.fn().mockResolvedValue({ authenticated: true, open: false, busy: false, revision: 1 });
    vi.mocked(api.search).mockResolvedValueOnce(feedResult()).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; })).mockResolvedValueOnce(feedResult([{ ...work, id: "789", title: "New account fixture" }], 1, "new-account:pool"));
    await render(api); await click(host.querySelector('[aria-label="추천 작품 더 불러오기"]')!);
    vi.mocked(api.account).mockResolvedValue({ authenticated: null, open: false, busy: false, revision: 2 });
    await act(async () => { await vi.advanceTimersByTimeAsync(2500); finish(feedResult([{ ...work, id: "456", title: "Old account fixture" }], 2)); });
    expect(host.querySelectorAll(".pixiv-card")).toHaveLength(0); expect(host.querySelector(".pixiv-home-more")).toBeNull();
    expect(host.querySelector<HTMLButtonElement>('[aria-label="추천 작품 새로고침"]')!.disabled).toBe(true);
    vi.mocked(api.account).mockResolvedValue({ authenticated: false, open: false, busy: false, revision: 2 });
    await act(async () => { await vi.advanceTimersByTimeAsync(2500); });
    expect(api.search).toHaveBeenLastCalledWith({ query: "", mode: "home", page: 1 });
    expect(host.textContent).toContain("New account fixture"); expect(host.textContent).not.toContain("Old account fixture");
  });
  it.each(["privacy", "inactive", "downloads"] as const)("cancels append when %s hides discovery, without starting a new request", async hidden => {
    const api = apiFactory(); let finish!: (value: PixivSearch) => void;
    vi.mocked(api.search).mockResolvedValueOnce(feedResult()).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    await render(api); await click(host.querySelector('[aria-label="추천 작품 더 불러오기"]')!);
    if (hidden === "inactive") await act(async () => root.render(<PixivWorkspace active={false} runtime="browser-mock" privacyMode={false} view="explore" api={api} />));
    else await render(api, hidden === "privacy", hidden === "downloads" ? "downloads" : "explore");
    expect(host.querySelector(".pixiv-home-more")).toBeNull(); expect(host.querySelector('[aria-label="Pixiv 추천 검색어"]')).toBeNull();
    await act(async () => finish(feedResult([{ ...work, id: "456", title: "Hidden reply fixture" }], 2)));
    await render(api); expect(api.search).toHaveBeenCalledTimes(2); expect(host.querySelectorAll(".pixiv-card")).toHaveLength(1);
    expect(host.textContent).not.toContain("Hidden reply fixture"); expect(host.querySelector('[aria-label="추천 작품 더 불러오기"]')).not.toBeNull();
  });
  it("allows explicit continuation after empty eligible pages but never automatically fills or repeats", async () => {
    vi.useFakeTimers(); const api = apiFactory();
    vi.mocked(api.search).mockResolvedValueOnce(feedResult([], 1, "pool:1")).mockResolvedValueOnce(feedResult([work], 2, "pool:2")).mockResolvedValueOnce(feedResult([work], 3, "pool:3")).mockResolvedValueOnce(feedResult([], 4, null));
    await render(api); expect(host.querySelectorAll(".pixiv-card")).toHaveLength(0);
    await act(async () => { await vi.advanceTimersByTimeAsync(10000); }); expect(api.search).toHaveBeenCalledTimes(1);
    for (let page = 2; page <= 4; page++) {
      await click(host.querySelector('[aria-label="추천 작품 더 불러오기"]')!);
      expect(api.search).toHaveBeenLastCalledWith({ query: "", mode: "home", page, homeCursor: `pool:${page - 1}` });
      await act(async () => { await vi.advanceTimersByTimeAsync(10000); }); expect(api.search).toHaveBeenCalledTimes(page);
    }
    expect(host.querySelectorAll(".pixiv-card")).toHaveLength(1); expect(host.querySelector(".pixiv-home-more")).toBeNull();
  });
  it.each([undefined, "", "x".repeat(97)])("does not offer continuation with invalid opaque cursor %s", async cursor => {
    const api = apiFactory(); vi.mocked(api.search).mockResolvedValue({ ...homeResult(), hasMore: true, homeCursor: cursor });
    await render(api); expect(host.querySelectorAll(".pixiv-card")).toHaveLength(1); expect(host.querySelector(".pixiv-home-more")).toBeNull(); expect(api.search).toHaveBeenCalledOnce();
  });
  it("leaves keyword pagination as replacement pages and never sends a home continuation", async () => {
    const api = apiFactory();
    vi.mocked(api.search).mockResolvedValueOnce(feedResult()).mockResolvedValueOnce({ items: [work], page: 1, hasMore: true }).mockResolvedValueOnce({ items: [{ ...work, id: "456", title: "Next keyword page" }], page: 2, hasMore: false });
    await render(api); await type(host.querySelector<HTMLInputElement>(".pixiv-search input")!, "sky"); await submitSearch();
    await click([...host.querySelectorAll(".pixiv-pagination button")].find(button => button.textContent === "다음")!);
    expect(api.search).toHaveBeenLastCalledWith({ query: "sky", mode: "latest", page: 2 }); expect(host.querySelectorAll(".pixiv-card")).toHaveLength(1);
    expect(host.textContent).toContain("Next keyword page"); expect(host.querySelector(".pixiv-home-more")).toBeNull();
  });
  it("disables the opposite account action only after verified status and preserves button nodes", async () => {
    vi.useFakeTimers(); const api = apiFactory();
    api.account = vi.fn().mockResolvedValue({ authenticated: false, open: false, busy: false, revision: 1 });
    api.login = vi.fn().mockResolvedValue({ authenticated: false, open: true, busy: false, revision: 1 });
    api.logout = vi.fn().mockResolvedValue({ authenticated: null, open: true, busy: true, revision: 3 });
    await render(api);
    const login = host.querySelector<HTMLButtonElement>(".pixiv-account-controls button")!;
    const logout = host.querySelectorAll<HTMLButtonElement>(".pixiv-account-controls button")[1]!;
    expect(login.disabled).toBe(false); expect(logout.disabled).toBe(true);
    await click(login); expect(api.login).toHaveBeenCalledOnce(); expect(login.disabled).toBe(true);
    vi.mocked(api.account).mockResolvedValue({ authenticated: true, open: false, busy: false, revision: 2 });
    await act(async () => { await vi.advanceTimersByTimeAsync(2500); });
    expect(host.querySelector(".pixiv-account-controls button")).toBe(login);
    expect(login.disabled).toBe(true); expect(logout.disabled).toBe(false);
    expect(host.querySelector(".pixiv-account-controls")?.textContent).toContain("로그인됨");
    await click(logout); expect(api.logout).toHaveBeenCalledOnce(); expect(logout.disabled).toBe(true);
    expect(host.querySelector(".pixiv-account-controls")?.textContent).toContain("확인 중");
  });
  it("rejects old remote results on account change while keeping tabs, draft and local favorites", async () => {
    vi.useFakeTimers(); const api = apiFactory();
    localStorage.setItem("atsumi.pixiv.bookmarks.v1", JSON.stringify([{ work, savedAt: 1 }]));
    api.account = vi.fn().mockResolvedValue({ authenticated: true, open: false, busy: false, revision: 1 });
    let finish!: (value: { items: PixivWork[]; page: number; hasMore: boolean }) => void;
    vi.mocked(api.search).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    await render(api); await type(host.querySelector<HTMLInputElement>(".pixiv-search input")!, "unsubmitted");
    const tab = host.querySelector(".pixiv-search-tabs>div");
    vi.mocked(api.account).mockResolvedValue({ authenticated: false, open: false, busy: false, revision: 2 });
    await act(async () => { await vi.advanceTimersByTimeAsync(2500); finish({ items: [work], page: 1, hasMore: false }); });
    expect(host.querySelectorAll(".pixiv-card")).toHaveLength(0);
    expect(host.querySelector(".pixiv-search-tabs>div")).toBe(tab);
    expect(host.querySelector<HTMLInputElement>(".pixiv-search input")?.value).toBe("unsubmitted");
    expect(api.search).toHaveBeenCalledOnce();
    await render(api, false, "favorites"); expect(host.querySelectorAll(".pixiv-card")).toHaveLength(1);
    expect(JSON.parse(localStorage.getItem("atsumi.pixiv.bookmarks.v1")!)[0].work.id).toBe("123");
    expect(api.libraryAction).not.toHaveBeenCalled();
  });
  it("renders unknown status on failed verification, never retained logged-in success", async () => {
    vi.useFakeTimers(); const api = apiFactory();
    api.account = vi.fn().mockResolvedValue({ authenticated: true, open: false, busy: false, revision: 1 });
    await render(api);
    vi.mocked(api.account).mockRejectedValue(new Error("untrusted remote detail"));
    await act(async () => { await vi.advanceTimersByTimeAsync(2500); });
    expect(host.querySelector(".pixiv-account-controls")?.textContent).toContain("로그인 상태 확인 필요");
    expect(host.querySelector(".pixiv-account-controls")?.textContent).not.toContain("untrusted");
    expect(host.querySelectorAll<HTMLButtonElement>(".pixiv-account-controls button")[1]!.disabled).toBe(true);
    expect(host.querySelectorAll(".pixiv-card")).toHaveLength(0);
  });
  it("does not initialize an account profile while private or inactive", async () => {
    const api = apiFactory(); api.account = vi.fn().mockResolvedValue({ authenticated: false, open: false, busy: false, revision: 1 }); api.login = vi.fn();
    await render(api, true); expect(api.account).not.toHaveBeenCalled(); expect(api.search).not.toHaveBeenCalled();
    expect(host.querySelector<HTMLButtonElement>(".pixiv-account-controls button")!.disabled).toBe(true);
    await act(async () => { host.querySelector("form.pixiv-search")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); });
    expect(api.search).not.toHaveBeenCalled();
    await act(async () => root.render(<PixivWorkspace active={false} runtime="browser-mock" privacyMode={false} view="explore" api={api} />));
    expect(api.account).not.toHaveBeenCalled(); expect(api.login).not.toHaveBeenCalled(); expect(api.search).not.toHaveBeenCalled();
    await render(api, false); expect(api.account).toHaveBeenCalledOnce(); expect(api.search).toHaveBeenCalledOnce();
    expect(host.querySelectorAll(".pixiv-card")).toHaveLength(2);
  });
  it("ignores a delayed older account revision instead of restoring stale authentication", async () => {
    vi.useFakeTimers(); const api = apiFactory();
    api.account = vi.fn().mockResolvedValue({ authenticated: false, open: false, busy: false, revision: 4 });
    await render(api); const cards = [...host.querySelectorAll(".pixiv-card")];
    vi.mocked(api.account).mockResolvedValue({ authenticated: true, open: false, busy: false, revision: 3 });
    await act(async () => { await vi.advanceTimersByTimeAsync(2500); });
    expect(host.querySelector(".pixiv-account-controls")?.textContent).toContain("로그아웃됨");
    expect(host.querySelectorAll<HTMLButtonElement>(".pixiv-account-controls button")[1]!.disabled).toBe(true);
    expect([...host.querySelectorAll(".pixiv-card")]).toEqual(cards);
  });
  it("uses only bounded card widths and keeps the default independent of the page reader", async () => {
    const api = apiFactory();
    await render(api);
    const workspace = host.querySelector<HTMLElement>(".pixiv-workspace")!;
    expect(workspace.style.getPropertyValue("--pixiv-preview-width")).toBe("180px");
    for (const width of [140, 180, 220, 280, 340]) {
      await render(api, false, "explore", width);
      expect(workspace.style.getPropertyValue("--pixiv-preview-width")).toBe(`${width}px`);
    }
    for (const width of [NaN, Infinity, -1, 181, 9999]) {
      await render(api, false, "explore", width);
      expect(workspace.style.getPropertyValue("--pixiv-preview-width")).toBe("180px");
    }
    expect(api.search).toHaveBeenCalledTimes(1);
    expect(document.querySelector(".pixiv-reader")).toBeNull();
  });
  it("resizes retained cards without refetching, remounting or clearing the browsing context", async () => {
    const api = apiFactory();
    await render(api, false, "explore", 180);
    const workspace = host.querySelector<HTMLElement>(".pixiv-workspace")!;
    const cards = [...host.querySelectorAll(".pixiv-card")];
    const tabs = [...host.querySelectorAll(".pixiv-search-tabs>div")];
    await click(cards[0]!); await click(cards[1]!, { ctrlKey: true });
    await type(host.querySelector<HTMLInputElement>(".pixiv-search input")!, "unsubmitted query");
    workspace.scrollTop = 320;
    const searchCalls = vi.mocked(api.search).mock.calls.length;
    const imageCalls = vi.mocked(api.media).mock.calls.length;
    const savedBookmarks = localStorage.getItem("atsumi.pixiv.bookmarks.v1");
    await render(api, false, "explore", 340);
    expect(host.querySelector(".pixiv-workspace")).toBe(workspace);
    expect(workspace.style.getPropertyValue("--pixiv-preview-width")).toBe("340px");
    expect(workspace.scrollTop).toBe(320);
    [...host.querySelectorAll(".pixiv-card")].forEach((card, index) => expect(card).toBe(cards[index]));
    [...host.querySelectorAll(".pixiv-search-tabs>div")].forEach((tab, index) => expect(tab).toBe(tabs[index]));
    expect(host.querySelector<HTMLInputElement>(".pixiv-search input")?.value).toBe("unsubmitted query");
    expect(host.querySelectorAll('[role="checkbox"][aria-checked="true"]')).toHaveLength(2);
    expect(api.search).toHaveBeenCalledTimes(searchCalls);
    expect(api.media).toHaveBeenCalledTimes(imageCalls);
    expect(api.downloadBatch).not.toHaveBeenCalled();
    expect(localStorage.getItem("atsumi.pixiv.bookmarks.v1")).toBe(savedBookmarks);
  });
  it.each(["favorites", "downloads"] as const)("applies card size to %s without reloading its entries", async view => {
    const api = apiFactory();
    localStorage.setItem("atsumi.pixiv.bookmarks.v1", JSON.stringify([{ work, savedAt: 1 }]));
    vi.mocked(api.downloads).mockResolvedValue([{ work, savedAt: "1", pageCount: 2, bytes: 200, folder: "fixture" }]);
    await render(api, false, view, 140);
    const card = host.querySelector(".pixiv-card");
    expect(card).not.toBeNull();
    const downloadsCalls = vi.mocked(api.downloads).mock.calls.length;
    expect(host.querySelector<HTMLElement>(".pixiv-workspace")?.style.getPropertyValue("--pixiv-preview-width")).toBe("140px");
    await render(api, false, view, 280);
    expect(host.querySelector(".pixiv-card")).toBe(card);
    expect(host.querySelector<HTMLElement>(".pixiv-workspace")?.style.getPropertyValue("--pixiv-preview-width")).toBe("280px");
    expect(api.downloads).toHaveBeenCalledTimes(downloadsCalls);
    expect(api.search).not.toHaveBeenCalled();
  });
  it("does not request or render images in privacy mode", async () => { const api = apiFactory(); await render(api, true); expect(api.media).not.toHaveBeenCalled(); expect(host.querySelector("img")).toBeNull(); expect(host.textContent).toContain("프라이버시 모드"); });
  it("uses the common Korean-title preference only on cards", async () => {
    const api = apiFactory(); vi.mocked(api.search).mockResolvedValue({ items: [{ ...work, title: "Landscape | 풍경" }], page: 1, hasMore: false }); await render(api);
    const title = host.querySelector(".pixiv-card-info strong")!;
    expect(title.textContent).toBe("풍경"); expect(title.getAttribute("title")).toBeNull();
    vi.useFakeTimers(); await act(async () => title.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
    await act(async () => vi.advanceTimersByTime(300));
    expect(document.querySelector('[role="tooltip"] .pixiv-metadata-original')?.textContent).toBe("Landscape | 풍경");
  });
  it("single click has no checkbox; ctrl second click enables multi-selection", async () => { await render(apiFactory()); const cards = host.querySelectorAll(".pixiv-card"); await click(cards[0]!); expect(host.querySelectorAll('[role="checkbox"]')).toHaveLength(0); await click(cards[1]!, { ctrlKey: true }); expect(host.querySelectorAll('[role="checkbox"]')).toHaveLength(2); expect(host.textContent).toContain("2개 선택"); });
  it("double click opens the original reader, D changes page, Escape closes", async () => {
    const api = apiFactory(); await render(api); await act(async () => { host.querySelector(".pixiv-card")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })); });
    expect(document.querySelector('[role="dialog"]')).not.toBeNull(); expect(document.querySelector('[aria-label="현재 쪽"]')?.textContent).toBe("1 / 2");
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "d", bubbles: true }))); expect(document.querySelector('[aria-label="현재 쪽"]')?.textContent).toBe("2 / 2");
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(document.querySelector(".pixiv-reader-backdrop")?.classList.contains("is-closing")).toBe(true);
    await settleClose(); expect(document.querySelector('[role="dialog"]')).toBeNull();
  });
  it("local bookmarks do not mutate Pixiv accounts", async () => { const api = apiFactory(); await render(api); await click(host.querySelector('.pixiv-bookmark')!); await render(api, false, "favorites"); expect(host.querySelectorAll(".pixiv-card")).toHaveLength(1); expect(api.open).not.toHaveBeenCalled(); expect(api.download).not.toHaveBeenCalled(); expect(localStorage.getItem("atsumi.pixiv.bookmarks.v1")).not.toContain("base64"); });
  it("opens directly into pinned original reading with work actions and returns focus to the card", async () => {
    localStorage.setItem("atsumi.pixiv.reader.v1", JSON.stringify({ reading: "rtl", pinned: false }));
    const api = apiFactory(); await render(api);
    const opener = host.querySelector<HTMLElement>(".pixiv-card")!; opener.focus();
    await act(async () => { opener.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })); });
    const reader = document.querySelector(".pixiv-reader")!;
    expect(reader.getAttribute("data-controls-visible")).toBe("true");
    expect(reader.getAttribute("data-reading")).toBe("rtl");
    expect(document.querySelector(".pixiv-detail-backdrop")).toBeNull();
    expect(document.querySelector(".pixiv-preview-open")).toBeNull();
    expect([...reader.querySelectorAll(".pixiv-detail-actions button")].map(node => node.textContent)).toEqual(["앨범", "다운로드", "웹에서 열기"]);
    expect(reader.querySelector('[aria-label="1쪽 즐겨찾기"]')).not.toBeNull();
    await click([...reader.querySelectorAll(".pixiv-detail-actions button")].find(node => node.textContent === "앨범")!);
    expect(JSON.parse(localStorage.getItem("atsumi.pixiv.bookmarks.v1")!)[0].page).toBeUndefined();
    await click([...reader.querySelectorAll(".pixiv-detail-actions button")].find(node => node.textContent === "웹에서 열기")!);
    expect(api.open).toHaveBeenCalledExactlyOnceWith("123");
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "a", bubbles: true })));
    expect(reader.querySelector('[aria-label="현재 쪽"]')?.textContent).toBe("2 / 2");
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }))); await settleClose();
    expect(document.querySelector(".pixiv-reader")).toBeNull();
    expect(document.querySelector(".pixiv-detail-backdrop")).toBeNull();
    expect(document.activeElement).toBe(opener); expect(api.search).toHaveBeenCalledTimes(1); expect(api.detail).toHaveBeenCalledTimes(1);
  });
  it("page favorites use the selected page preview, not the first page", async () => {
    await render(apiFactory()); await act(async () => { host.querySelector(".pixiv-card")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })); });
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "d", bubbles: true })));
    const button = document.querySelector('[aria-label="2쪽 즐겨찾기"]')!; await click(button);
    const records = JSON.parse(localStorage.getItem("atsumi.pixiv.bookmarks.v1")!); expect(records[0].page).toBe(1); expect(records[0].work.thumbnailUrl).toContain("test_p1.jpg");
  });
  it("right-click offers existing card actions without opening the work", async () => { const api = apiFactory(); await render(api); await act(async () => { host.querySelector(".pixiv-card")!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, clientX: 20, clientY: 30 })); }); expect(document.querySelector('[role="menu"]')?.textContent).toContain("원본 다운로드"); expect(api.detail).not.toHaveBeenCalled(); });
  it("keeps the context menu across rerenders and dismisses it when a local filter changes", async () => {
    const api = apiFactory(); vi.mocked(api.downloads).mockResolvedValue([{ work, savedAt: "1", pageCount: 2, bytes: 200, folder: "fixture" }]);
    await render(api, false, "downloads");
    await act(async () => host.querySelector(".pixiv-card")!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, clientX: 20, clientY: 30 })));
    expect(document.querySelector('[role="menu"]')).not.toBeNull();
    await render(api, false, "downloads"); expect(document.querySelector('[role="menu"]')).not.toBeNull();
    await type(host.querySelector<HTMLInputElement>(".pixiv-search input")!, "unmatched");
    expect(document.querySelector('[role="menu"]')).toBeNull();
    expect(host.querySelectorAll(".pixiv-card")).toHaveLength(0);
  });
  it("cancelled requests cannot replace newer results", async () => {
    const api = apiFactory(); let complete!: (value: { items: PixivWork[]; page: number; hasMore: boolean }) => void;
    vi.mocked(api.search).mockImplementationOnce(() => new Promise(resolve => { complete = resolve; })); await render(api); const cancel = [...host.querySelectorAll("button")].find(button => button.textContent === "취소")!; await click(cancel);
    await act(async () => { complete({ items: [work], page: 1, hasMore: false }); }); expect(host.querySelectorAll(".pixiv-card")).toHaveLength(0);
  });
  it("opens downloaded pages and favorites offline without remote metadata or media", async () => {
    const api = apiFactory(); vi.mocked(api.downloads).mockResolvedValue([{ work, savedAt: "1", pageCount: 2, bytes: 200, folder: "fixture" }]);
    vi.mocked(api.detail).mockRejectedValue(new Error("offline")); vi.mocked(api.media).mockRejectedValue(new Error("offline"));
    await render(api, false, "downloads");
    await act(async () => { host.querySelector(".pixiv-card")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })); });
    expect(api.localDetail).toHaveBeenCalledWith("123"); expect(api.localMedia).toHaveBeenCalledWith("123", 0, true);
    expect(api.detail).not.toHaveBeenCalled(); expect(api.media).not.toHaveBeenCalled();
    expect(document.querySelector(".pixiv-save-button")?.textContent).toBe("저장 완료");
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "d", bubbles: true })));
    expect(document.querySelector('[aria-label="현재 쪽"]')?.textContent).toBe("2 / 2");
    await click(document.querySelector('[aria-label="2쪽 즐겨찾기"]')!);
    expect(localStorage.getItem("atsumi.pixiv.bookmarks.v1")).not.toContain("atsumi-pixiv:");
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    await render(api, false, "favorites"); await click([...host.querySelectorAll(".pixiv-segments button")].find(node => node.textContent === "페이지")!);
    expect(api.localMedia).toHaveBeenCalledWith("123", 1, false); expect(api.media).not.toHaveBeenCalled();
  });
  it("keeps local reader blobs, current page and focus across remote account invalidation", async () => {
    vi.useFakeTimers(); const api = apiFactory();
    api.account = vi.fn().mockResolvedValue({ authenticated: true, open: false, busy: false, revision: 1 });
    vi.mocked(api.downloads).mockResolvedValue([{ work, savedAt: "1", pageCount: 2, bytes: 200, folder: "fixture" }]);
    vi.mocked(api.localMedia).mockImplementation(async (_id, page, detail) => ({ url: `blob:local-${page}-${detail}`, bytes: 100 }));
    const previousRevoke = URL.revokeObjectURL, revoke = vi.fn(); URL.revokeObjectURL = revoke;
    try {
      await render(api, false, "downloads");
      const card = host.querySelector(".pixiv-card")!, thumbnail = card.querySelector("img")!;
      await act(async () => { card.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })); });
      await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "d", bubbles: true })));
      const reader = document.querySelector(".pixiv-reader")!, original = reader.querySelector("img")!;
      const focused = reader.querySelector<HTMLButtonElement>('[aria-label="2쪽 즐겨찾기"]')!;
      await act(async () => focused.focus());
      expect(reader.querySelector('[aria-label="현재 쪽"]')?.textContent).toBe("2 / 2");
      expect(original.getAttribute("src")).toBe("blob:local-1-true");
      const calls = vi.mocked(api.localMedia).mock.calls.length; revoke.mockClear();
      for (const snapshot of [
        { authenticated: null, open: false, busy: false, revision: 2, error: "확인 실패" },
        { authenticated: false, open: false, busy: false, revision: 3 },
      ]) {
        vi.mocked(api.account).mockResolvedValue(snapshot);
        await act(async () => { await vi.advanceTimersByTimeAsync(2500); });
        expect(document.querySelector(".pixiv-reader")).toBe(reader);
        expect(reader.querySelector("img")).toBe(original);
        expect(card.querySelector("img")).toBe(thumbnail);
        expect(reader.querySelector('[aria-label="현재 쪽"]')?.textContent).toBe("2 / 2");
        expect(document.activeElement).toBe(focused);
        expect(api.localMedia).toHaveBeenCalledTimes(calls);
        expect(revoke).not.toHaveBeenCalled();
      }
      expect(api.pageMedia).not.toHaveBeenCalled(); expect(api.detail).not.toHaveBeenCalled(); expect(api.media).not.toHaveBeenCalled();
      // Unmount while the revoke spy is installed so local cleanup remains observable and safe.
      await act(async () => root.render(null));
      expect(revoke).toHaveBeenCalledWith("blob:local-1-true");
    } finally { if (previousRevoke) URL.revokeObjectURL = previousRevoke; else Reflect.deleteProperty(URL, "revokeObjectURL"); }
  });
  it("does not silently replace a broken local original with a remote image", async () => {
    const api = apiFactory(); vi.mocked(api.downloads).mockResolvedValue([{ work, savedAt: "1", pageCount: 2, bytes: 200, folder: "fixture" }]);
    vi.mocked(api.localDetail).mockRejectedValue(new Error("저장된 페이지가 없습니다.")); await render(api, false, "downloads");
    await act(async () => { host.querySelector(".pixiv-card")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })); });
    expect(document.querySelector('[role="dialog"]')?.textContent).toContain("저장된 페이지가 없습니다."); expect(api.detail).not.toHaveBeenCalled();
  });
  it("creator navigation opens in the background with Ctrl and returns to retained detail", async () => {
    const api = apiFactory(); await render(api);
    await act(async () => host.querySelector(".pixiv-card")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "d", bubbles: true })));
    await click(document.querySelector(".pixiv-reader .pixiv-artist")!, { ctrlKey: true });
    expect(api.search).toHaveBeenLastCalledWith({ query: "9", mode: "artist", page: 1 });
    expect(document.querySelector('[aria-label="현재 쪽"]')?.textContent).toBe("2 / 2");
    await click(document.querySelector(".pixiv-reader .pixiv-artist")!);
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    await click(host.querySelector('[aria-label="Artist 닫기"]')!);
    expect(document.querySelector('[aria-label="현재 쪽"]')?.textContent).toBe("2 / 2");
    expect(api.search).toHaveBeenCalledTimes(2);
  });
  it("late discovery errors do not contaminate the downloads view", async () => {
    const api = apiFactory(); let reject!: (error: Error) => void;
    vi.mocked(api.search).mockImplementationOnce(() => new Promise((_, fail) => { reject = fail; }));
    await render(api); await render(api, false, "downloads");
    await act(async () => reject(new Error("Search-only failure")));
    expect(host.textContent).not.toContain("Search-only failure");
    await render(api); expect(host.textContent).toContain("Search-only failure");
  });
  it("Ctrl Enter registers the whole selection in one native batch without a full list reload", async () => {
    const api = apiFactory(); await render(api); const initialLists = vi.mocked(api.downloads).mock.calls.length;
    const cards = host.querySelectorAll(".pixiv-card"); await click(cards[0]!); await click(cards[1]!, { shiftKey: true });
    vi.mocked(api.downloadBatch).mockResolvedValue([123, 456].map(id => ({ id: String(id), saved: { work: { ...work, id: String(id) }, savedAt: "1", pageCount: 2, bytes: 200, folder: "fixture" } })));
    await act(async () => cards[1]!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true })));
    expect(api.downloadBatch).toHaveBeenCalledWith(["123", "456"]); expect(api.download).not.toHaveBeenCalled();
    expect(api.downloads).toHaveBeenCalledTimes(initialLists); expect(host.querySelectorAll(".pixiv-saved-marker")).toHaveLength(2);
  });
  it("Enter opens selection tabs, Q/E switch works and closing all asks once", async () => {
    const api = apiFactory(); vi.mocked(api.detail).mockImplementation(async input => ({ work: { ...work, id: input, title: input === "456" ? "Second fixture" : work.title }, pages: [0, 1].map(index => ({ index, width: 800, height: 1000, previewUrl: `https://i.pximg.net/p${index}.jpg`, originalUrl: `https://i.pximg.net/p${index}.jpg` })) }));
    await render(api); const cards = host.querySelectorAll(".pixiv-card"); await click(cards[0]!); await click(cards[1]!, { ctrlKey: true });
    await act(async () => cards[0]!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    expect(document.querySelectorAll(".pixiv-detail-tabs > div")).toHaveLength(2);
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "e", bubbles: true })));
    expect(api.detail).toHaveBeenLastCalledWith("456");
    await click(document.querySelector('[aria-label="미리보기 닫기"]')!);
    expect(document.querySelector('[role="alertdialog"]')?.textContent).toContain("작품 2개를 닫을까요?");
    expect(document.querySelector(".pixiv-reader-backdrop")?.classList.contains("is-closing")).toBe(false);
    await act(async () => document.activeElement?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(document.querySelector('[role="alertdialog"]')).toBeNull();
    expect(document.querySelector(".pixiv-reader")).not.toBeNull();
    expect(document.activeElement).toBe(document.querySelector('[aria-label="미리보기 닫기"]'));
    await click(document.querySelector('[aria-label="미리보기 닫기"]')!);
    await click([...document.querySelectorAll('[role="alertdialog"] button')].find(node => node.textContent === "취소")!);
    expect(document.activeElement).toBe(document.querySelector('[aria-label="미리보기 닫기"]'));
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "d", bubbles: true })));
    expect(document.querySelector('[aria-label="현재 쪽"]')?.textContent).toBe("2 / 2");
    await click(document.querySelector('[aria-label="미리보기 닫기"]')!);
    await click([...document.querySelectorAll('[role="alertdialog"] button')].find(node => node.textContent === "닫기")!);
    await settleClose();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
  });
  it("downloads directly from the pinned reader and changes the same action to saved state", async () => {
    const api = apiFactory(); vi.mocked(api.download).mockResolvedValue({ work, savedAt: "1", pageCount: 2, bytes: 200, folder: "fixture" });
    await render(api); await act(async () => host.querySelector(".pixiv-card")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
    await click(document.querySelector(".pixiv-reader .pixiv-save-button")!);
    expect(api.download).toHaveBeenCalledExactlyOnceWith("123");
    const saved = document.querySelector<HTMLButtonElement>(".pixiv-reader .pixiv-save-button")!;
    expect(saved.textContent).toBe("저장 완료"); expect(saved.disabled).toBe(true);
    expect(document.querySelector(".pixiv-reader")?.getAttribute("data-controls-visible")).toBe("true");
  });
  it("retains each work page when switching preview tabs and closing only one", async () => {
    const api = apiFactory(); vi.mocked(api.detail).mockImplementation(async input => ({ work: { ...work, id: input, title: input === "456" ? "Second fixture" : work.title }, pages: [0, 1].map(index => ({ index, width: 800, height: 1000, previewUrl: `https://i.pximg.net/p${index}.jpg`, originalUrl: `https://i.pximg.net/p${index}.jpg` })) }));
    await render(api); const cards = host.querySelectorAll(".pixiv-card"); await click(cards[0]!); await click(cards[1]!, { ctrlKey: true });
    await act(async () => cards[0]!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "d", bubbles: true })));
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "e", bubbles: true })));
    expect(document.querySelector('[aria-label="현재 쪽"]')?.textContent).toBe("1 / 2");
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "q", bubbles: true })));
    expect(document.querySelector('[aria-label="현재 쪽"]')?.textContent).toBe("2 / 2");
    await click(document.querySelector(`[aria-label="${work.title} 닫기"]`)!);
    expect(document.querySelector(".pixiv-reader-header strong")?.textContent).toBe("Second fixture");
    expect(document.querySelector('[role="alertdialog"]')).toBeNull();
    expect(document.querySelector('[aria-label="현재 쪽"]')?.textContent).toBe("1 / 2");
  });
  it("defers shortcuts to tutorials and topmost dialogs", async () => {
    await render(apiFactory()); await act(async () => host.querySelector(".pixiv-card")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
    const modal = document.createElement("dialog"); modal.setAttribute("open", ""); document.body.append(modal);
    try {
      await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "d", bubbles: true })));
      expect(document.querySelector('[aria-label="현재 쪽"]')?.textContent).toBe("1 / 2");
      modal.remove(); document.documentElement.dataset.tutorialOpen = "true";
      await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
      expect(document.querySelector(".pixiv-reader-backdrop")?.classList.contains("is-closing")).toBe(false);
      delete document.documentElement.dataset.tutorialOpen;
      await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "d", bubbles: true })));
      expect(document.querySelector('[aria-label="현재 쪽"]')?.textContent).toBe("2 / 2");
    } finally { modal.remove(); delete document.documentElement.dataset.tutorialOpen; }
  });
  it("an old close timer cannot remove newly opened detail", async () => {
    const api = apiFactory(); await render(api);
    await act(async () => host.querySelector(".pixiv-card")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
    await click(document.querySelector('[aria-label="미리보기 닫기"]')!);
    await act(async () => host.querySelectorAll(".pixiv-card")[1]!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
    await settleClose(); expect(document.querySelector('[role="dialog"]')).not.toBeNull(); expect(api.detail).toHaveBeenLastCalledWith("456");
  });
  it("searches saved works and favorites locally without remote requests", async () => {
    const api = apiFactory(); vi.mocked(api.downloads).mockResolvedValue([work, { ...work, id: "456", title: "Second fixture", artist: "Other" }].map(work => ({ work, savedAt: "1", pageCount: 2, bytes: 200, folder: "fixture" })));
    await render(api, false, "downloads");
    const cards = host.querySelectorAll(".pixiv-card"); await click(cards[0]!); await click(cards[1]!, { ctrlKey: true });
    expect(host.textContent).toContain("2개 선택");
    await type(host.querySelector<HTMLInputElement>(".pixiv-search input")!, "Other");
    expect(host.querySelectorAll(".pixiv-card")).toHaveLength(1); expect(host.querySelector(".pixiv-card-info strong")?.textContent).toBe("Second fixture");
    expect(host.textContent).not.toContain("2개 선택");
    await click(host.querySelector(".pixiv-bookmark")!); await render(api, false, "favorites");
    await type(host.querySelector<HTMLInputElement>(".pixiv-search input")!, "unmatched"); expect(host.querySelectorAll(".pixiv-card")).toHaveLength(0);
    await type(host.querySelector<HTMLInputElement>(".pixiv-search input")!, "456"); expect(host.querySelectorAll(".pixiv-card")).toHaveLength(1);
    expect(api.search).not.toHaveBeenCalled();
  });
  it("library removal confirms recoverable behavior, retains favorites, and supports undo", async () => {
    const api = apiFactory(); const record = { work, savedAt: "1", pageCount: 2, bytes: 200, folder: "fixture" };
    vi.mocked(api.downloads).mockResolvedValue([record]);
    vi.mocked(api.libraryAction).mockImplementation(async (_, action) => action === "remove" ? { token: "123-fixture" } : { record });
    await render(api, false, "downloads"); await click(host.querySelector(".pixiv-bookmark")!);
    await act(async () => host.querySelector(".pixiv-card")!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, clientX: 20, clientY: 30 })));
    await click([...document.querySelectorAll('[role="menuitem"]')].find(node => node.textContent === "보관함에서 제거")!);
    expect(document.querySelector('[role="alertdialog"]')?.textContent).toContain("파일은 복원용 폴더에 남고");
    expect(api.libraryAction).not.toHaveBeenCalled();
    await click([...document.querySelectorAll('[role="alertdialog"] button')].find(node => node.textContent === "제거")!);
    expect(host.querySelectorAll(".pixiv-card")).toHaveLength(0); expect(localStorage.getItem("atsumi.pixiv.bookmarks.v1")).toContain("123");
    await click([...host.querySelectorAll("button")].find(node => node.textContent === "되돌리기")!);
    expect(api.libraryAction).toHaveBeenLastCalledWith("123", "restore", "123-fixture"); expect(host.querySelectorAll(".pixiv-card")).toHaveLength(1);
  });
  it("search history overlays the list and supports removing one or all entries", async () => {
    localStorage.setItem("atsumi.pixiv.search-history.v1", JSON.stringify(["landscape", "river"]));
    await render(apiFactory()); const input = host.querySelector<HTMLInputElement>(".pixiv-search input")!;
    await act(async () => input.focus()); expect(host.querySelector('[aria-label="최근 검색 기록"]')).not.toBeNull();
    await click(host.querySelector('[aria-label="landscape 기록 삭제"]')!); expect(host.querySelector('[aria-label="landscape 기록 삭제"]')).toBeNull();
    await click([...host.querySelectorAll("button")].find(node => node.textContent === "기록 모두 지우기")!);
    expect(host.querySelector('[aria-label="최근 검색 기록"]')).toBeNull(); expect(JSON.parse(localStorage.getItem("atsumi.pixiv.search-history.v1")!)).toEqual([]);
  });
  it("rediscovers recoverable removal after a fresh mount without browser token storage", async () => {
    const api = apiFactory(); const record = { work, savedAt: "1", pageCount: 2, bytes: 200, folder: "fixture" };
    vi.mocked(api.downloads).mockResolvedValue([{ ...record, removedToken: "123-persisted-token" }]);
    vi.mocked(api.libraryAction).mockResolvedValue({ record });
    await render(api, false, "downloads");
    expect(host.querySelectorAll(".pixiv-card")).toHaveLength(0);
    expect(host.textContent).toContain("복원할 수 있는 작품 1개");
    expect(localStorage.getItem("atsumi.pixiv.bookmarks.v1")).not.toContain("persisted-token");
    await click([...host.querySelectorAll("button")].find(node => node.textContent === "되돌리기")!);
    expect(api.libraryAction).toHaveBeenCalledWith("123", "restore", "123-persisted-token");
    expect(host.querySelectorAll(".pixiv-card")).toHaveLength(1);
    expect(host.textContent).not.toContain("복원할 수 있는 작품");
  });
  it("keeps incomplete records manageable without marking them complete or falling back online", async () => {
    const api = apiFactory(); vi.mocked(api.downloads).mockResolvedValue([{ work, savedAt: "1", pageCount: 2, bytes: 200, folder: "fixture", issue: "파일 확인 필요" }]);
    vi.mocked(api.localDetail).mockRejectedValue(new Error("저장된 페이지가 없습니다."));
    await render(api, false, "downloads");
    expect(host.querySelector('[aria-label="저장 완료"]')).toBeNull();
    expect(host.querySelector('[aria-label="파일 확인 필요"]')).not.toBeNull();
    await act(async () => host.querySelector(".pixiv-card")!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, clientX: 20, clientY: 30 })));
    expect(document.querySelector('[role="menu"]')?.textContent).toContain("보관함에서 제거");
    await click([...document.querySelectorAll('[role="menuitem"]')].find(node => node.textContent?.startsWith("상세 보기"))!);
    expect(api.localDetail).toHaveBeenCalledWith("123"); expect(api.detail).not.toHaveBeenCalled();
    expect(document.querySelector('[role="dialog"]')?.textContent).toContain("저장된 페이지가 없습니다.");
  });
});
