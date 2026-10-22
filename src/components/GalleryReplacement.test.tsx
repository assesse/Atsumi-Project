import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { galleryId, type Gallery } from "../core/types";
import { ActivityDrawer } from "./ActivityDrawer";
import { applyDownloadChanged } from "../state/downloadProjection";
import { mergeDownloadEntries, mergeDownloadLibraryPage } from "../state/galleryProjection";
import { linkedGalleryId } from "../state/downloadStatus";
import { DownloadProgressStore } from "../state/downloadProgress";

const gallery: Gallery = {
  id: galleryId(10), title: "Replacement fixture", subtitle: "", artist: "fixture",
  pages: 2, score: 0, publishedAt: "", coverIndex: 0, language: "korean", tags: [], series: [], characters: [],
  download: { entryId: "old-entry", revision: 2, state: "failed", errorCode: "SOURCE_INVALID_DATA" },
};
const linked = {
  galleryId: 10, entryId: "old-entry", revision: 3, state: "cancelled" as const,
  progress: 0, errorCode: "SOURCE_GALLERY_LINKED", errorMessage: "새 번호 #20의 기존 작업에 연결했습니다.",
  linkedGalleryId: 20,
};

describe("verified gallery replacement presentation", () => {
  it("retains link identity through events and lists without accepting an older failure", () => {
    const result=applyDownloadChanged(new Map([[gallery.id,gallery]]),linked);
    expect(result.applied).toBe(true);
    expect(linkedGalleryId(result.galleries.get(gallery.id)?.download)).toBe(20);
    const stale={...linked,revision:2,state:"failed" as const,linkedGalleryId:undefined};
    expect(applyDownloadChanged(result.galleries,stale).applied).toBe(false);
    const entries=mergeDownloadEntries(new Map([[gallery.id,gallery]]),[{...linked,galleryId:gallery.id}]);
    expect(linkedGalleryId(entries.get(gallery.id)?.download)).toBe(20);
    const page=mergeDownloadLibraryPage(new Map([[gallery.id,gallery]]),{
      page:1,totalItems:1,items:[{gallery:{id:gallery.id,title:gallery.title},download:{...linked,galleryId:gallery.id}}],
    });
    expect(linkedGalleryId(page.galleries.get(gallery.id)?.download)).toBe(20);
    const store=new DownloadProgressStore();
    expect(store.apply(linked,gallery.download)).toEqual({applied:true,structural:true});
    expect(linkedGalleryId(store.read(10,gallery.download))).toBe(20);
    store.clear();
  });

  it("shows a link, not success/user cancellation/retry, and opens only its verified target", async () => {
    const host=document.createElement("div"),root=createRoot(host),open=vi.fn(),retry=vi.fn();
    const updated={...gallery,download:{...linked}};
    try {
      await act(async()=>root.render(<ActivityDrawer open galleries={[updated]}
        sessionDownloads={[{galleryId:gallery.id,occurredAt:1}]}
        onClose={vi.fn()} onReview={vi.fn()} onRetry={retry} onCancel={vi.fn()} onOpenLinkedGallery={open}/>));
      expect(host.querySelector(".activity-outcome")).toHaveTextContent("새 번호에 연결됨");
      expect(host.querySelector('[data-status-icon="linked"]')).not.toBeNull();
      expect(host.querySelector('[data-status-icon="completed"]')).toBeNull();
      expect(host.querySelector(".activity-command-buttons")).not.toHaveTextContent("재시도");
      expect(host.querySelector('[role="progressbar"]')).toBeNull();
      const button=host.querySelector<HTMLButtonElement>(".activity-command-buttons button")!;
      await act(async()=>button.click());
      expect(open).toHaveBeenCalledExactlyOnceWith(20);
      expect(retry).not.toHaveBeenCalled();
    } finally {await act(async()=>root.unmount());}
  });

  it("does not treat an arbitrary error message or unsafe ID as a link", () => {
    expect(linkedGalleryId({...linked,linkedGalleryId:-1})).toBeUndefined();
    expect(linkedGalleryId({...linked,linkedGalleryId:Number.MAX_SAFE_INTEGER+1})).toBeUndefined();
    expect(linkedGalleryId({...linked,errorCode:"SOURCE_INVALID_DATA"})).toBeUndefined();
    expect(linkedGalleryId({...linked,state:"failed"})).toBeUndefined();
  });
});
