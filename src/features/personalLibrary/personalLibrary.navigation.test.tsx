import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import App from "../../App";
import { backend } from "../../api/backend";
import type { GalleryDetail } from "../../api/contracts";
import { galleryId } from "../../core/types";
import { ThumbnailClient, ThumbnailProvider } from "../../thumbnail";
import { createBrowserLibraryApi } from "./api";

const settle = () => new Promise((resolve) => setTimeout(resolve, 25));
describe("personal library app navigation", () => {
  it("saves while viewing, returns to the precise page, and preserves Explore without queuing or changing metadata favorites", async () => {
    localStorage.clear(); sessionStorage.clear();
    localStorage.setItem("atsumi.content-source.v1", "hitomi");
    localStorage.setItem("atsumi.tutorial.dismissed.v1", "true");
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => setTimeout(() => callback(Date.now()), 0));
    vi.stubGlobal("cancelAnimationFrame", (timer: ReturnType<typeof setTimeout>) => clearTimeout(timer));
    const descriptors = ["showModal", "close"].map((name) => [name, Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, name)] as const);
    Object.defineProperty(HTMLDialogElement.prototype, "showModal", { configurable: true, value: function(this: HTMLDialogElement) { this.setAttribute("open", ""); } });
    Object.defineProperty(HTMLDialogElement.prototype, "close", { configurable: true, value: function(this: HTMLDialogElement) { this.removeAttribute("open"); this.dispatchEvent(new Event("close")); } });
    const gallery: GalleryDetail = { id: galleryId(9_123_457), title: "다시 찾는 장면", artist: "fixture", pages: 3, language: "korean", tags: [], series: [], characters: [], publishedRank: 20261001, popularity: 1, thumbnailWidth: 512, thumbnailHeight: 768, related: [], pageDimensions: [1, 2, 3].map((sourcePage) => ({ sourcePage, width: 512, height: 768 })) };
    const search = vi.spyOn(backend, "searchSubmit").mockResolvedValue({ ok: true, data: { queryId: "personal-retain", firstPage: { page: 1, totalPages: 1, items: [gallery] } } });
    vi.spyOn(backend, "galleryDetailGet").mockResolvedValue({ ok: true, data: gallery });
    const favorite = vi.spyOn(backend, "favoriteSet"), download = vi.spyOn(backend, "downloadQueueAdd");
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "offline fixture" }) });
    const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
    const click = async (selector: string) => act(async () => { const node = host.querySelector<HTMLButtonElement>(selector); expect(node).not.toBeNull(); node!.click(); await settle(); });
    try {
      await act(async () => { root.render(<ThumbnailProvider client={client}><App /></ThumbnailProvider>); await settle(); });
      await click('button[type="submit"][aria-label="검색"]');
      const initialSearches = search.mock.calls.length;
      await click('[data-gallery-id="9123457"] [aria-label="앨범 즐겨찾기 저장"]');
      expect(host.querySelector(".detail-workspace")).toBeNull();
      await act(async () => { host.querySelector('[data-gallery-id="9123457"]')!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })); await settle(); });
      await click('.preview-thumb[title="2페이지 확대"]');
      await click('[aria-label="2페이지 즐겨찾기 저장"]');
      await click('[aria-label="페이지 미리보기 닫기"]');
      await click('[aria-label="내 즐겨찾기"]');
      expect(host.querySelector('[aria-label="내 즐겨찾기"][aria-current="page"]')).toBeInTheDocument();
      await act(async () => { window.dispatchEvent(new KeyboardEvent("keydown", { key: "f", ctrlKey: true, bubbles: true, cancelable: true })); });
      expect(document.activeElement).toBe(host.querySelector("#personal-library-search input"));
      expect(host.querySelectorAll("#gallery-search-form")).toHaveLength(1);
      await act(async () => { [...host.querySelectorAll<HTMLButtonElement>('.personal-library-controls button')].find((node) => node.textContent === "페이지 1")!.click(); await settle(); });
      expect(host.querySelectorAll(".saved-card")).toHaveLength(1);
      const viewport = host.querySelector<HTMLDivElement>(".personal-library-viewport")!; viewport.scrollTop = 150;
      await click(".saved-card-preview");
      expect(host.querySelector(".page-preview-dialog[open] #page-preview-title")).toHaveTextContent("2페이지");
      await click('[aria-label="페이지 미리보기 닫기"]');
      expect(viewport.scrollTop).toBe(150);
      expect(host.querySelector('.personal-library-controls button[aria-pressed="true"]')).toHaveTextContent("페이지 1");
      await act(async () => { host.querySelector(".saved-card-preview")!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2 })); await settle(); });
      expect(host.querySelector(".detail-workspace")).toBeInTheDocument();
      expect(host.querySelector(".page-preview-dialog[open]")).toBeNull();
      expect(viewport.scrollTop).toBe(150);
      await act(async () => { host.querySelector(".saved-card")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })); await settle(); });
      expect(host.querySelector(".page-preview-dialog[open] #page-preview-title")).toHaveTextContent("2페이지");
      await click('[aria-label="페이지 미리보기 닫기"]');
      await click('[aria-label="Explore"]');
      expect(host.querySelector('[data-gallery-id="9123457"]')).toHaveTextContent("다시 찾는 장면");
      expect(search).toHaveBeenCalledTimes(initialSearches);
      expect(favorite).not.toHaveBeenCalled(); expect(download).not.toHaveBeenCalled();
      expect((await createBrowserLibraryApi(localStorage)({ action: "summary" })).summary.keys.map((key) => key.page).sort()).toEqual([0, 2]);
    } finally {
      await act(async () => root.unmount()); host.remove(); client.dispose();
      for (const [name, descriptor] of descriptors) { if (descriptor) Object.defineProperty(HTMLDialogElement.prototype, name, descriptor); else Reflect.deleteProperty(HTMLDialogElement.prototype, name); }
      vi.restoreAllMocks(); vi.unstubAllGlobals(); localStorage.clear(); sessionStorage.clear();
    }
  });
});
