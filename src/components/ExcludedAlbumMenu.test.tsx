import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../api/backend";
import type { ExplorationExclusionContext } from "../api/contracts";
import { galleryId } from "../core/types";
import { mockGalleries } from "../data/mockGalleries";
import { browserFixtureThumbnailAdapter, ThumbnailClient } from "../thumbnail";
import { GalleryCard } from "./GalleryCard";
import { ExcludedAlbumDialog } from "./ExcludedAlbumDialog";

const gallery = { ...mockGalleries[0]!, download: undefined };
const context: ExplorationExclusionContext = {
  galleryId: gallery.id, reasons: [{ kind: "duplicate_hidden", detail: "중복 판정에서 제외", excludedAt: "2026-10-06T00:00:00Z" }],
  quarantined: false, quarantineEntryId: null, reviewId: "selected-review", reviewGalleryId: gallery.id, legacyCandidateId: null,
  retainedGallery: { galleryId: galleryId(3295979), title: "Kept album" },
};

describe("excluded album management", () => {
  const testEnvironment = globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean };
  const previousEnvironment = testEnvironment.IS_REACT_ACT_ENVIRONMENT;
  const showModalDescriptor = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "showModal");
  beforeAll(() => {
    testEnvironment.IS_REACT_ACT_ENVIRONMENT = true;
    Object.defineProperty(HTMLDialogElement.prototype, "showModal", { configurable: true, value: function (this: HTMLDialogElement) { this.setAttribute("open", ""); } });
  });
  afterAll(() => {
    testEnvironment.IS_REACT_ACT_ENVIRONMENT = previousEnvironment;
    if (showModalDescriptor) Object.defineProperty(HTMLDialogElement.prototype, "showModal", showModalDescriptor);
    else Reflect.deleteProperty(HTMLDialogElement.prototype, "showModal");
  });
  let host: HTMLDivElement, root: Root;
  const inspect = vi.fn(), restore = vi.fn(), openDetail = vi.fn(), select = vi.fn();
  const client = new ThumbnailClient(browserFixtureThumbnailAdapter);
  const menu = () => document.querySelector<HTMLElement>('[role="menu"]')!;
  const option = (text: string) => [...menu().querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent?.includes(text))!;
  const rightClick = () => act(async () => host.querySelector("article")!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 100, clientY: 100 })));
  const render = () => act(async () => root.render(<GalleryCard gallery={gallery} view="explore" selected={false} selectionContext={false}
    explorationExcluded favoriteMetadata={new Set()} onSelect={select} onOpenDetail={openDetail} onOpenArtifact={vi.fn()}
    onOpenReview={vi.fn()} onStatusDetail={vi.fn()} onMetadataSearch={vi.fn()} onMetadataFavorite={vi.fn()}
    onInspectExclusion={inspect} onRestoreExclusion={restore} thumbnailClient={client} />));
  beforeEach(() => { vi.clearAllMocks(); host = document.createElement("div"); document.body.append(host); root = createRoot(host); });
  afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); });

  it("queries only the opened card and offers evidence, explicit restore and the exact keeper", async () => {
    const read = vi.spyOn(backend, "explorationExclusionContext").mockResolvedValue({ ok: true, data: context });
    await render();
    expect(read).not.toHaveBeenCalled();
    const article = host.querySelector<HTMLElement>("article")!;
    expect(article.inert).not.toBe(true);
    expect((article.firstElementChild as HTMLElement).inert).toBe(true);
    await act(async () => article.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
    expect(openDetail).not.toHaveBeenCalled();
    await rightClick();
    expect(read).toHaveBeenCalledExactlyOnceWith(gallery.id);
    expect(menu().querySelectorAll("button")).toHaveLength(3);
    expect(option("근거")).toHaveFocus();
    await act(async () => option("근거").click());
    expect(inspect).toHaveBeenCalledExactlyOnceWith(gallery.id, context);
    await rightClick();
    await act(async () => option("복원").click());
    expect(restore).toHaveBeenCalledExactlyOnceWith(gallery.id);
    await rightClick();
    await act(async () => option("보존된").click());
    expect(openDetail).toHaveBeenCalledExactlyOnceWith(context.retainedGallery!.galleryId);
    expect(select).not.toHaveBeenCalled();
  });

  it("allows an in-place retry and never invents a missing keeper", async () => {
    const read = vi.spyOn(backend, "explorationExclusionContext").mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValue({ ok: true, data: { ...context, retainedGallery: null } });
    await render(); await rightClick();
    expect(option("복원")).toBeDisabled();
    await act(async () => option("다시 시도").click());
    expect(read).toHaveBeenCalledTimes(2);
    expect(option("근거")).not.toBeDisabled();
    expect(option("보존된 판본 연결 없음")).toBeDisabled();
  });

  it("keeps evidence open on restoration failure and exposes comparison separately", async () => {
    const onRestore = vi.fn().mockResolvedValueOnce(false).mockResolvedValueOnce(true), onClose = vi.fn(), review = vi.fn();
    await act(async () => root.render(<ExcludedAlbumDialog context={context} title="Excluded album" onClose={onClose} onRestore={onRestore} onReview={review} onOpenRetained={openDetail} />));
    const button = [...host.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === "제외 해제·복원")!;
    await act(async () => button.click());
    expect(onClose).not.toHaveBeenCalled();
    await act(async () => button.click());
    expect(onClose).toHaveBeenCalledOnce();
    expect(host).toHaveTextContent("다운로드를 새로 시작하지는 않습니다");
  });
});
