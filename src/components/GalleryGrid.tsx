import { useLayoutEffect, useRef, type ReactNode } from "react";
import type { GalleryDisplayMode } from "../core/types";
import { galleryPreviewPreset, galleryPreviewPresetStyle } from "../layout/galleryPreviewPresets";
import { groupGalleryCardRows } from "./galleryRowLayout";

type GalleryGridProps = {
  columns: number;
  previewWidth: number;
  selectionContext: boolean;
  ariaLabel: string;
  displayMode?: GalleryDisplayMode;
  children: ReactNode;
};

export function GalleryGrid({ columns, previewWidth, selectionContext, ariaLabel, displayMode = "detail", children }: GalleryGridProps) {
  const gridRef = useRef<HTMLDivElement>(null);
  const preset = galleryPreviewPreset(previewWidth);

  useLayoutEffect(() => {
    const grid = gridRef.current;
    if (!grid) return undefined;
    if (displayMode === "compact") {
      for (const card of grid.querySelectorAll<HTMLElement>(":scope > .gallery-card, :scope > .progressive-gallery-slot > .gallery-card")) {
        card.style.removeProperty("--gallery-card-height");
        delete card.dataset.rowHeight;
      }
      for (const slot of grid.querySelectorAll<HTMLElement>(":scope > .progressive-gallery-slot")) {
        slot.style.removeProperty("--gallery-card-height");
      }
      return undefined;
    }
    let frame = 0;
    let disposed = false;
    let lastWidth = grid.clientWidth;
    let resizeTimer: number | undefined;

    const measure = () => {
      frame = 0;
      if (disposed) return;
      const hosts = [...grid.children].filter((child): child is HTMLElement => child instanceof HTMLElement);
      const cards = hosts.map((host) => host.matches(".gallery-card")
        ? host
        : host.querySelector<HTMLElement>(":scope > .gallery-card"));
      const metrics = cards.map((card, index) => {
        const host = hosts[index];
        const retainedPlaceholderHeight = host?.classList.contains("is-placeholder")
          ? Number.parseFloat(host.style.getPropertyValue("--gallery-card-height"))
          : Number.NaN;
        const cover = card?.querySelector<HTMLElement>(":scope > .cover");
        const width = Number(cover?.dataset.thumbnailIntrinsicWidth ?? 1);
        const height = Number(cover?.dataset.thumbnailIntrinsicHeight ?? 1);
        const coverWidth = cover?.getBoundingClientRect().width ?? 0;
        return {
          index,
          top: host?.offsetTop ?? 0,
          intrinsicThumbnailHeight: Number.isFinite(retainedPlaceholderHeight) && retainedPlaceholderHeight > 0
            ? retainedPlaceholderHeight
            : width > 0 && height > 0 ? coverWidth * height / width : coverWidth,
        };
      });
      const rows = groupGalleryCardRows(metrics);
      const nextHeights = new Map<number, number>();
      for (const row of rows) for (const index of row.indices) nextHeights.set(index, row.height);
      cards.forEach((card, index) => {
        if (!card) return;
        const height = nextHeights.get(index);
        if (!height) return;
        const value = `${height}px`;
        if (card.style.getPropertyValue("--gallery-card-height") !== value) {
          card.style.setProperty("--gallery-card-height", value);
        }
        if (card.dataset.rowHeight !== value) card.dataset.rowHeight = value;
        const host = hosts[index];
        if (host && host !== card && host.style.getPropertyValue("--gallery-card-height") !== value) {
          host.style.setProperty("--gallery-card-height", value);
        }
      });
    };
    const schedule = () => {
      if (!disposed && !frame) frame = window.requestAnimationFrame(measure);
    };
    schedule();
    const resize = typeof ResizeObserver === "function" ? new ResizeObserver(() => {
      const nextWidth = grid.clientWidth;
      // Our row-height writes also resize the grid; they must not schedule a
      // second full-gallery readback. Column/preset changes measure immediately
      // via this effect, but pixel-only resizing can use the settled geometry.
      if (nextWidth === lastWidth) return;
      lastWidth = nextWidth;
      window.clearTimeout(resizeTimer);
      resizeTimer = window.setTimeout(schedule, 100);
    }) : null;
    resize?.observe(grid);
    const mutation = typeof MutationObserver === "function" ? new MutationObserver((records) => {
      if (records.some((record) => record.type === "attributes"
        || record.target === grid
        || (record.target instanceof HTMLElement
          && record.target.parentElement === grid
          && record.target.classList.contains("progressive-gallery-slot")))) schedule();
    }) : null;
    mutation?.observe(grid, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["data-thumbnail-intrinsic-width", "data-thumbnail-intrinsic-height"],
    });
    document.fonts?.ready.then(schedule).catch(() => undefined);
    return () => {
      disposed = true;
      window.clearTimeout(resizeTimer);
      if (frame) window.cancelAnimationFrame(frame);
      resize?.disconnect();
      mutation?.disconnect();
    };
  }, [columns, displayMode, preset.key]);

  return (
    <div
      ref={gridRef}
      className={`gallery-grid${displayMode === "compact" ? " is-compact" : ""}${selectionContext ? " is-selection-context" : ""}`}
      data-preview-size={preset.key}
      data-display-mode={displayMode}
      style={{
        gridTemplateColumns: displayMode === "compact"
          ? `repeat(${columns}, minmax(0, ${previewWidth}px))`
          : `repeat(${columns}, minmax(0, 1fr))`,
        ...galleryPreviewPresetStyle(preset),
      }}
      role="list"
      aria-label={ariaLabel}
    >
      {children}
    </div>
  );
}
