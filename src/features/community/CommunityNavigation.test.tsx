import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import App from "../../App";
import { backend } from "../../api/backend";
import type { GalleryDetail } from "../../api/contracts";
import type { WorkKey } from "./api";
import { galleryId } from "../../core/types";
import { browserFixtureThumbnailAdapter, ThumbnailClient, ThumbnailProvider } from "../../thumbnail";

vi.mock("./CommunityWorkspace", () => ({ CommunityWorkspace: ({ onNavigate, initialReview }: { onNavigate: (view: string) => void; initialReview?: WorkKey | null }) => <section aria-label="커뮤니티 화면" data-review-work={initialReview ? `${initialReview.source}:${initialReview.workId}` : "none"}><button onClick={() => onNavigate("explore")}>탐색으로</button><button onClick={() => onNavigate("downloads")}>보관함으로</button></section> }));
afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const settle = () => new Promise((resolve) => setTimeout(resolve, 25));
describe("community app composition", () => {
  it("opens work-specific reviews from both detail headers and retains the search and tabs on return", async () => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => setTimeout(() => callback(Date.now()), 0));
    vi.stubGlobal("cancelAnimationFrame", (timer: ReturnType<typeof setTimeout>) => clearTimeout(timer));
    const previousShowModal = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "showModal");
    Object.defineProperty(HTMLDialogElement.prototype, "showModal", { configurable: true, value: function (this: HTMLDialogElement) { this.setAttribute("open", ""); } });
    const source = localStorage.getItem("atsumi.content-source.v1"); const tutorial = localStorage.getItem("atsumi.tutorial.dismissed.v1");
    localStorage.setItem("atsumi.content-source.v1", "hitomi"); localStorage.setItem("atsumi.tutorial.dismissed.v1", "true");
    const gallery: GalleryDetail = { id: galleryId(9_123_456), title: "보존되는 탐색 결과", artist: "fixture", pages: 1, language: "korean", tags: [], series: [], characters: [], publishedRank: 20260914, popularity: 1, thumbnailWidth: 512, thumbnailHeight: 768, related: [], pageDimensions: [{ sourcePage: 1, width: 512, height: 768 }] };
    const search = vi.spyOn(backend, "searchSubmit").mockResolvedValue({ ok: true, data: { queryId: "community-retain", firstPage: { page: 1, totalPages: 1, items: [gallery] } } });
    const detail = vi.spyOn(backend, "galleryDetailGet").mockResolvedValue({ ok: true, data: gallery });
    const cancel = vi.spyOn(backend, "downloadCancel"); const thumbnails = new ThumbnailClient(browserFixtureThumbnailAdapter);
    const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
    try {
      await act(async () => { root.render(<ThumbnailProvider client={thumbnails}><App /></ThumbnailProvider>); await settle(); });
      await act(async () => { host.querySelector<HTMLButtonElement>('button[type="submit"][aria-label="검색"]')!.click(); await settle(); });
      expect(host.querySelector('[data-gallery-id="9123456"]')).toHaveTextContent("보존되는 탐색 결과");
      const initialSearches = search.mock.calls.length;
      await act(async () => { host.querySelector<HTMLElement>('[data-gallery-id="9123456"]')!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })); await settle(); });
      expect(host.querySelector('.detail-workspace')).toBeInTheDocument();
      await act(async () => { host.querySelector<HTMLButtonElement>('.detail-title-actions [aria-label="후기 남기기"]')!.click(); await settle(); });
      expect(host.querySelector('[aria-label="커뮤니티 화면"]')).toHaveAttribute("data-review-work", "hitomi:9123456");
      expect(host.querySelector('.detail-workspace')).toBeNull();
      await act(async () => { [...host.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent === "탐색으로")!.click(); await settle(); });
      expect(host.querySelector('[role="tab"][aria-selected="true"]')).toHaveTextContent("보존되는 탐색 결과");
      await act(async () => { host.querySelector<HTMLButtonElement>('.preview-thumb')!.click(); await settle(); });
      expect(host.querySelector('.page-preview-dialog[open]')).toBeInTheDocument();
      await act(async () => { host.querySelector<HTMLButtonElement>('.page-preview-header-actions [aria-label="후기 남기기"]')!.click(); await settle(); });
      expect(host.querySelector('[aria-label="커뮤니티 화면"]')).toHaveAttribute("data-review-work", "hitomi:9123456");
      expect(host.querySelector('dialog[open]')).toBeNull();
      await act(async () => { [...host.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent === "탐색으로")!.click(); await settle(); });
      expect(host.querySelector('[role="tab"][aria-selected="true"]')).toHaveTextContent("보존되는 탐색 결과");
      expect(detail).toHaveBeenCalledTimes(1);
      await act(async () => { host.querySelector<HTMLButtonElement>('[aria-label="커뮤니티"]')!.click(); await settle(); });
      expect(host.querySelector('[aria-label="커뮤니티 화면"]')).toHaveAttribute("data-review-work", "none");
      await act(async () => { [...host.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent === "탐색으로")!.click(); await settle(); });
      expect(host.querySelector('[data-gallery-id="9123456"]')).toHaveTextContent("보존되는 탐색 결과");
      expect(search).toHaveBeenCalledTimes(initialSearches); expect(cancel).not.toHaveBeenCalled();
      await act(async () => { host.querySelector<HTMLButtonElement>('[aria-label="커뮤니티"]')!.click(); await settle(); });
      await act(async () => { [...host.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent === "보관함으로")!.click(); await settle(); });
      expect(host.querySelector('[aria-label="Downloads"]')).toHaveAttribute("aria-current", "page");
    } finally {
      await act(async () => root.unmount()); host.remove(); thumbnails.dispose();
      if (source === null) localStorage.removeItem("atsumi.content-source.v1"); else localStorage.setItem("atsumi.content-source.v1", source);
      if (tutorial === null) localStorage.removeItem("atsumi.tutorial.dismissed.v1"); else localStorage.setItem("atsumi.tutorial.dismissed.v1", tutorial);
      if (previousShowModal) Object.defineProperty(HTMLDialogElement.prototype, "showModal", previousShowModal);
      else Reflect.deleteProperty(HTMLDialogElement.prototype, "showModal");
    }
  });
});
