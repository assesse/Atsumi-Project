/**
 * Detail source pages are intentionally bounded independently of DOM geometry.
 * Related cards and decoded image dimensions must never change subscription
 * volume after the Detail is visible.
 */
export const DETAIL_PREVIEW_WINDOW_SIZE_BY_COLUMNS = {
  2: 8,
  3: 9,
} as const;

export function detailPreviewWindowSize(pageCount: number, columns: 2 | 3): number {
  const total = Math.max(0, Math.floor(pageCount));
  return Math.min(total, DETAIL_PREVIEW_WINDOW_SIZE_BY_COLUMNS[columns]);
}

export function detailPreviewWindowStart(page: number, pageCount: number, windowSize: number): number {
  const total = Math.max(0, Math.floor(pageCount));
  const size = Math.max(1, Math.floor(windowSize));
  if (!total) return 1;
  const clamped = Math.min(total, Math.max(1, Math.floor(page)));
  return Math.floor((clamped - 1) / size) * size + 1;
}

/** Clamps a preserved tab position without snapping it to a new window boundary. */
export function detailPreviewWindowClampStart(start: number, pageCount: number, windowSize: number): number {
  const total = Math.max(0, Math.floor(pageCount));
  const size = Math.max(1, Math.floor(windowSize));
  if (!total) return 1;
  // Preserve a true final partial bundle instead of shifting it backwards to
  // fill every slot with older pages.
  const lastStart = Math.floor((total - 1) / size) * size + 1;
  return Math.min(lastStart, Math.max(1, Math.floor(start)));
}

export function detailPreviewWindowRange(start: number, pageCount: number, windowSize: number): readonly number[] {
  const total = Math.max(0, Math.floor(pageCount));
  const size = Math.max(0, Math.floor(windowSize));
  if (!total || !size) return [];
  const first = Math.min(total, Math.max(1, Math.floor(start)));
  return Array.from({ length: Math.min(size, total - first + 1) }, (_, index) => first + index);
}
