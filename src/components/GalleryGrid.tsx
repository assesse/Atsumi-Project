import { useLayoutEffect, useRef, type ReactNode } from "react";
import { galleryPreviewPreset, galleryPreviewPresetStyle } from "../layout/galleryPreviewPresets";
import { groupGalleryCardRows } from "./galleryRowLayout";

type GalleryGridProps = {
  columns: number;
  previewWidth: number;
  selectionContext: boolean;
  ariaLabel: string;
  children: ReactNode;
};

export function GalleryGrid({ columns, previewWidth, selectionContext, ariaLabel, children }: GalleryGridProps) {
  const gridRef = useRef<HTMLDivElement>(null);
  const preset = galleryPreviewPreset(previewWidth);

  useLayoutEffect(() => {
    const grid = gridRef.current;
    if (!grid) return undefined;
    let frame = 0;
    let disposed = false;

    const measure = () => {
      frame = 0;
      if (disposed) return;
      const cards = [...grid.querySelectorAll<HTMLElement>(":scope > .gallery-card")];
      const metrics = cards.map((card, index) => {
        const cover = card.querySelector<HTMLElement>(":scope > .cover");
        const width = Number(cover?.dataset.thumbnailIntrinsicWidth ?? 1);
        const height = Number(cover?.dataset.thumbnailIntrinsicHeight ?? 1);
        const coverWidth = cover?.getBoundingClientRect().width ?? 0;
        return {
          index,
          top: card.offsetTop,
          intrinsicThumbnailHeight: width > 0 && height > 0 ? coverWidth * height / width : coverWidth,
        };
      });
      const rows = groupGalleryCardRows(metrics);
      const nextHeights = new Map<number, number>();
      for (const row of rows) for (const index of row.indices) nextHeights.set(index, row.height);
      cards.forEach((card, index) => {
        const height = nextHeights.get(index);
        if (!height) return;
        const value = `${height}px`;
        if (card.style.getPropertyValue("--gallery-card-height") !== value) {
          card.style.setProperty("--gallery-card-height", value);
        }
        card.dataset.rowHeight = value;
      });
    };
    const schedule = () => {
      if (!frame) frame = window.requestAnimationFrame(measure);
    };
    schedule();
    const resize = typeof ResizeObserver === "function" ? new ResizeObserver(schedule) : null;
    resize?.observe(grid);
    const mutation = typeof MutationObserver === "function" ? new MutationObserver(schedule) : null;
    mutation?.observe(grid, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["data-thumbnail-intrinsic-width", "data-thumbnail-intrinsic-height"],
    });
    document.fonts?.ready.then(schedule).catch(() => undefined);
    return () => {
      disposed = true;
      if (frame) window.cancelAnimationFrame(frame);
      resize?.disconnect();
      mutation?.disconnect();
    };
  }, [columns, preset.key]);

  return (
    <div
      ref={gridRef}
      className={`gallery-grid${selectionContext ? " is-selection-context" : ""}`}
      data-preview-size={preset.key}
      style={{
        gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`,
        ...galleryPreviewPresetStyle(preset),
      }}
      role="list"
      aria-label={ariaLabel}
    >
      {children}
    </div>
  );
}
