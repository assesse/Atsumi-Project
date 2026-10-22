import { normalizeImagePreviewWidth } from "../platforms/imagePreviewPreferences";

export const PINTEREST_MASONRY_GAP = 14;
export const PINTEREST_MASONRY_ROW = 2;
export type PinterestImageDimensions = { width: number; height: number };
export type PinterestMasonryItem = { key: string; width?: number; height?: number };
export type PinterestMasonryPlacement = {
  key: string; column: number; row: number; span: number;
  x: number; y: number; width: number; height: number;
};

/** Metadata and already-decoded thumbnail dimensions share the same bound. */
export function pinterestImageDimensions(width: unknown, height: unknown): PinterestImageDimensions | null {
  return typeof width === "number" && typeof height === "number"
    && Number.isInteger(width) && Number.isInteger(height)
    && width > 0 && height > 0 && width <= 100_000 && height <= 100_000
    ? { width, height } : null;
}

/**
 * Each entry occupies one column. Choosing the first shortest column guarantees
 * nondecreasing top coordinates in source order (leftmost wins equal heights).
 * Selection, keyboard navigation and screen readers therefore keep that order.
 */
export function layoutPinterestMasonry(items: readonly PinterestMasonryItem[], containerWidth: number, targetWidth: unknown) {
  if (!Number.isFinite(containerWidth) || containerWidth <= 0 || containerWidth > 65_536) {
    return { columns: 1, columnWidth: 0, placements: [] as PinterestMasonryPlacement[], height: 0 };
  }
  const width = containerWidth;
  const target = normalizeImagePreviewWidth(targetWidth);
  const columns = Math.max(1, Math.floor((width + PINTEREST_MASONRY_GAP) / (target + PINTEREST_MASONRY_GAP)));
  const columnWidth = Math.max(0, (width - (columns - 1) * PINTEREST_MASONRY_GAP) / columns);
  const columnRows = Array.from({ length: columns }, () => 0);
  const gapRows = PINTEREST_MASONRY_GAP / PINTEREST_MASONRY_ROW;
  const placements: PinterestMasonryPlacement[] = items.map(item => {
    const dimensions = pinterestImageDimensions(item.width, item.height);
    const ratio = dimensions ? dimensions.height / dimensions.width : 4 / 3;
    // This is only a bounded browsing thumbnail. Original/detail media remains
    // unchanged, including unusually tall posters or very wide panoramas.
    const height = Math.max(120, Math.min(columnWidth * 3, columnWidth * ratio));
    const span = Math.ceil(height / PINTEREST_MASONRY_ROW);
    const column = columnRows.indexOf(Math.min(...columnRows));
    const row = columnRows[column]!;
    columnRows[column] = row + span + gapRows;
    return { key: item.key, column: column + 1, row: row + 1, span,
      x: column * (columnWidth + PINTEREST_MASONRY_GAP), y: row * PINTEREST_MASONRY_ROW,
      width: columnWidth, height: span * PINTEREST_MASONRY_ROW };
  });
  return { columns, columnWidth, placements, height: placements.length ? (Math.max(...columnRows) - gapRows) * PINTEREST_MASONRY_ROW : 0 };
}
