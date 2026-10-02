import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { mockGalleries } from "../data/mockGalleries";
import type { DownloadState, Gallery } from "../core/types";
import { DownloadProgressContext, DownloadProgressStore } from "../state/downloadProgress";
import { downloadStatus } from "../state/downloadStatus";
import { GalleryProcessingBadge, GalleryProcessingSurface } from "./GalleryProcessingBadge";

describe("shared album state presentation", () => {
  it.each(Object.keys(downloadStatus) as DownloadState[])("uses one icon and an accessible description for %s, without repeated text", async (state) => {
    const gallery: Gallery = { ...mockGalleries[0]!, download: { entryId: "badge", state } };
    const container = document.createElement("div"); const root = createRoot(container);
    try {
      await act(async () => root.render(<GalleryProcessingBadge gallery={gallery} />));
      expect(container.querySelectorAll("svg")).toHaveLength(1);
      expect(container.textContent).toBe("");
      expect(container.querySelector('[role="img"]')).toHaveAccessibleName(`${gallery.title}, ${downloadStatus[state].label}`);
      expect(container.querySelector('[role="img"]')).toHaveAttribute("title", `${gallery.title}, ${downloadStatus[state].label}`);
      if (!["completed", "downloading", "review_required"].includes(state)) {
        // The legacy SVG class sets fill/currentColor + stroke/none on the
        // parent. Line icons must override those inherited paints on the path.
        expect(container.querySelector("svg path")).toHaveAttribute("fill", "none");
        expect(container.querySelector("svg path")).toHaveAttribute("stroke", "currentColor");
      }
    } finally { await act(async () => root.unmount()); }
  });

  it("updates a related card's tone and completed mask from live ID events, and rejects older revisions", async () => {
    const gallery: Gallery = { ...mockGalleries[0]!, download: { entryId: "shared", state: "hashing", revision: 1 } };
    const store = new DownloadProgressStore();
    const container = document.createElement("div"); const root = createRoot(container);
    try {
      await act(async () => root.render(<DownloadProgressContext.Provider value={store}>
        <GalleryProcessingSurface className="related-card" gallery={gallery}><GalleryProcessingBadge gallery={gallery} /></GalleryProcessingSurface>
      </DownloadProgressContext.Provider>));
      expect(container.querySelector("article")).toHaveAttribute("data-processing-tone", "processing");
      expect(container.querySelector("article")).not.toHaveAttribute("data-processing-muted");
      await act(async () => { store.apply({ galleryId: gallery.id, entryId: "shared", revision: 2, state: "completed", progress: 100 }, gallery.download); });
      expect(container.querySelector("article")).toHaveAttribute("data-processing-tone", "complete");
      expect(container.querySelector("article")).toHaveAttribute("data-processing-muted", "true");
      expect(container.querySelector('[data-processing-state="completed"]')).not.toBeNull();
      await act(async () => { store.apply({ galleryId: gallery.id, entryId: "shared", revision: 1, state: "downloading" }, gallery.download); });
      expect(container.querySelector("article")).toHaveAttribute("data-processing-tone", "complete");
    } finally { await act(async () => root.unmount()); store.clear(); }
  });

  it("preserves duplicate review actions in the single icon", async () => {
    const gallery = { ...mockGalleries[0]!, download: undefined };
    const onClick = vi.fn();
    const container = document.createElement("div"); const root = createRoot(container);
    try {
      await act(async () => root.render(<GalleryProcessingBadge gallery={gallery} duplicateCount={3} onClick={onClick} />));
      const button = container.querySelector("button")!;
      expect(button).toHaveAccessibleName(`${gallery.title}, 중복 후보 3개`);
      expect(button.querySelector('[data-status-icon="duplicate"]')).not.toBeNull();
      expect(button.querySelector("svg path")).toHaveAttribute("stroke", "currentColor");
      expect(button.textContent).toBe("");
      await act(async () => button.click());
      expect(onClick).toHaveBeenCalledOnce();
    } finally { await act(async () => root.unmount()); }
  });
});
