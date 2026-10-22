import { useLayoutEffect, useRef, type ReactNode } from "react";
import { normalizeImagePreviewWidth } from "../platforms/imagePreviewPreferences";
import { layoutPinterestMasonry } from "./pinterestMasonryLayout";

type PinterestMasonryProps = { previewWidth?: number; contextKey: string; children: ReactNode; tour?: string };

/** Keeps the existing keyed articles in DOM order; only their grid tracks move. */
export function PinterestMasonry({ previewWidth = 180, contextKey, children, tour }: PinterestMasonryProps) {
  const gridRef = useRef<HTMLDivElement>(null);
  const options = useRef({ previewWidth, contextKey });
  options.current = { previewWidth, contextKey };
  const measureRef = useRef<() => void>(() => {});

  useLayoutEffect(() => {
    const grid = gridRef.current;
    if (!grid) return;
    let disposed = false, frame = 0;
    let previous: { signature: string; keys: string; context: string } | null = null;
    let observedWidth = grid.getBoundingClientRect().width;
    const measure = () => {
      if (frame) cancelAnimationFrame(frame);
      frame = 0;
      if (disposed || !grid.isConnected) return;
      const width = grid.getBoundingClientRect().width;
      // Hidden workspaces retain their last useful geometry, never a zero-width
      // intermediate layout that could destroy a restored scroll position.
      if (!Number.isFinite(width) || width <= 0 || width > 65_536) return;
      const cards = [...grid.querySelectorAll<HTMLElement>(":scope > .pinterest-card")];
      if (!cards.length) { previous = null; return; }
      const items = cards.map(card => ({ key: card.dataset.masonryKey ?? "", width: Number(card.dataset.masonryWidth), height: Number(card.dataset.masonryHeight) }));
      const keys = JSON.stringify(items.map(item => item.key));
      const current = options.current;
      const signature = JSON.stringify([width, normalizeImagePreviewWidth(current.previewWidth), items, current.contextKey]);
      if (previous?.signature === signature) return;
      const scrollRoot = grid.closest<HTMLElement>(".pinterest-removed-scroll, .pinterest-workspace");
      let anchor: { card: HTMLElement; offset: number } | null = null;
      if (scrollRoot && previous?.keys === keys && previous.context === current.contextKey) {
        const viewport = scrollRoot.getBoundingClientRect();
        const candidates = cards.map(card => ({ card, rect: card.getBoundingClientRect() }))
          .filter(item => item.rect.bottom > viewport.top && item.rect.top < viewport.bottom);
        candidates.sort((a, b) => Math.abs(a.rect.top - viewport.top) - Math.abs(b.rect.top - viewport.top));
        const nearest = candidates[0];
        if (nearest) anchor = { card: nearest.card, offset: nearest.rect.top - viewport.top };
      }
      const layout = layoutPinterestMasonry(items, width, current.previewWidth);
      grid.style.gridTemplateColumns = `repeat(${layout.columns}, minmax(0, 1fr))`;
      grid.dataset.masonryReady = "true";
      layout.placements.forEach((placement, index) => {
        const card = cards[index]!;
        card.style.gridColumn = String(placement.column);
        card.style.gridRow = `${placement.row} / span ${placement.span}`;
      });
      previous = { signature, keys, context: current.contextKey };
      if (anchor && scrollRoot && anchor.card.isConnected) {
        const delta = anchor.card.getBoundingClientRect().top - scrollRoot.getBoundingClientRect().top - anchor.offset;
        if (Number.isFinite(delta) && Math.abs(delta) > .5) scrollRoot.scrollTop += delta;
      }
    };
    const schedule = () => { if (!disposed && !frame) frame = requestAnimationFrame(measure); };
    measureRef.current = measure;
    const resize = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(() => {
      const width = grid.getBoundingClientRect().width;
      // Our own row writes change height, not width. Ignore those notifications.
      if (width === observedWidth) return;
      observedWidth = width; schedule();
    });
    resize?.observe(grid);
    const mutation = typeof MutationObserver === "undefined" ? null : new MutationObserver(schedule);
    mutation?.observe(grid, { childList: true, subtree: true, attributes: true,
      attributeFilter: ["data-masonry-key", "data-masonry-width", "data-masonry-height"] });
    measure();
    return () => {
      disposed = true; if (frame) cancelAnimationFrame(frame);
      resize?.disconnect(); mutation?.disconnect(); measureRef.current = () => {};
    };
  }, []);

  useLayoutEffect(() => { measureRef.current(); }, [previewWidth, contextKey, children]);
  return <div ref={gridRef} className="pinterest-grid" data-tour={tour}>{children}</div>;
}
