import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { auditLayouts, layoutBrowser } from "../../test/offlineLayoutBrowser";
import { PixivWorkspace } from "./PixivWorkspace";
import type { PixivApi, PixivDiscovery, PixivWork } from "./model";
import appCss from "../../styles.css?raw";
import workspaceCss from "./PixivWorkspace.css?raw";
import discoveryCss from "./PixivDiscoverySections.css?raw";
import headerCss from "./PixivHomeHeader.css?raw";
import readerCss from "./PixivPageReader.css?raw";

// Production React components, disposable data-only art, no native IPC or
// external network. These geometry tests do not claim authenticated parity.
const image = `data:image/svg+xml,${encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="600" height="900"><rect width="600" height="900" fill="#283e56"/><circle cx="430" cy="200" r="95" fill="#c7ad86"/><path d="M0 620 250 350 600 740V900H0Z" fill="#5e8182"/></svg>')}`;
const work = (id: number, title: string): PixivWork => ({ id: String(id), title, artist: "샘플 작가", artistId: "99", type: "illustration", pageCount: 2, width: 600, height: 900, thumbnailUrl: "https://i.pximg.net/img-master/fixture.jpg", bookmarkCount: 0, tags: ["landscape"], createdAt: "" });
const artworks = [work(101, "바람이 머무는 곳"), work(102, "오후의 산책"), work(103, "푸른 하늘"), work(104, "겨울 풍경")];
const feed: PixivDiscovery = { sections: [
  { id: "followed", title: "팔로우 신작", kind: "followed", items: artworks.slice(0, 2) },
  { id: "recommended", title: "당신을 위한 추천 작품", kind: "recommended", items: artworks },
  { id: "tag-0", title: "#landscape", kind: "tag", tag: "landscape", items: artworks.slice(1) },
], creators: [{ id: "99", name: "샘플 작가", imageUrl: "https://i.pximg.net/user-profile/fixture.jpg", works: artworks.slice(0, 3) }], suggestions: { favoriteTags: ["landscape"], favoriteTagTranslations: [{ tag: "landscape", translation: "풍경", language: "ko" }], recommendedTags: ["sky", "watercolor"], popularTags: ["illustration", "nature"], popularNovelTags: ["fantasy"] }, cursor: "fixture:60", page: 1, issues: [] };

describe.skipIf(!layoutBrowser)("Pixiv production discovery geometry offline", () => {
  it.each([320, 720, 1280])("keeps home, search overlay and reader content bounded at %ipx", async width => {
    Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true }); localStorage.clear();
    const oldWidth = window.innerWidth, oldHeight = window.innerHeight;
    Object.defineProperty(window, "innerWidth", { configurable: true, value: width });
    Object.defineProperty(window, "innerHeight", { configurable: true, value: 800 });
    const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
    const api: PixivApi = {
      account: vi.fn().mockResolvedValue({ authenticated: true, open: false, busy: false, revision: 1 }),
      discover: vi.fn().mockImplementation(async request => request.surface === "related" ? { ...feed, suggestions: undefined, sections: [{ id: "related", title: "관련 작품", kind: "related", items: artworks }] } : feed),
      search: vi.fn().mockResolvedValue({ items: artworks, page: 1, hasMore: false }),
      detail: vi.fn().mockResolvedValue({ work: artworks[0], pages: [0, 1].map(index => ({ index, width: 600, height: 900, previewUrl: "https://i.pximg.net/img-master/fixture.jpg", originalUrl: "https://i.pximg.net/img-original/fixture.jpg" })) }),
      localDetail: vi.fn(), localMedia: vi.fn(), pageMedia: vi.fn().mockResolvedValue({ url: image, bytes: 500 }), media: vi.fn().mockResolvedValue(image), download: vi.fn(), downloadBatch: vi.fn(), downloads: vi.fn().mockResolvedValue([]), progress: vi.fn().mockResolvedValue([]), cancel: vi.fn(), open: vi.fn(), libraryAction: vi.fn(),
    };
    const snapshots: { name: string; html: string }[] = [];
    try {
      await act(async () => root.render(<PixivWorkspace active runtime="browser-mock" privacyMode={false} view="explore" api={api} previewWidth={180} />));
      snapshots.push({ name: "home", html: host.innerHTML.replace('class="pixiv-workspace"', 'class="workspace pixiv-workspace"') });
      await act(async () => host.querySelector("input")!.dispatchEvent(new FocusEvent("focusin", { bubbles: true })));
      snapshots.push({ name: "search-panel", html: host.innerHTML.replace('class="pixiv-workspace"', 'class="workspace pixiv-workspace"') });
      await act(async () => host.querySelector(".pixiv-card")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
      snapshots.push({ name: "reader", html: document.querySelector(".pixiv-reader-backdrop")!.outerHTML });
      const related = document.querySelector(".pixiv-reader-related")!.innerHTML;
      snapshots.push({ name: "reader-related", html: `<div class="pixiv-reader-backdrop"><section class="pixiv-reader" style="width:${width - 24}px;height:760px"><div class="pixiv-reader-scroll"><div class="pixiv-reader-related">${related}</div></div></section></div>` });
      const results = await auditLayouts(snapshots, `${appCss}\n${workspaceCss}\n${headerCss}\n${readerCss}\n${discoveryCss}\nbody{margin:0}.pixiv-workspace{display:block}`, width, 800, "pixiv-discovery");
      expect(results).toHaveLength(4); expect(results.flatMap(result => result.issues.map(issue => `${result.name}: ${issue}`))).toEqual([]);
    } finally {
      await act(async () => root.unmount()); host.remove();
      Object.defineProperty(window, "innerWidth", { configurable: true, value: oldWidth });
      Object.defineProperty(window, "innerHeight", { configurable: true, value: oldHeight });
    }
  }, 60000);
});
