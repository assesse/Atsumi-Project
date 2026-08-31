const GALLERY_GAP = 12;
const MINIMUM_TEXT_SPACE = 260;
const ABSOLUTE_MINIMUM_CARD_WIDTH = 460;
const COMPACT_GALLERY_GAP = 12;
const COMPACT_LAYOUT_SAFETY_LIMIT = 64;

export function resolveGalleryColumns(
  availableWidth: number,
  maximumColumns: number,
  previewWidth: number,
): number {
  const safeMaximum = Math.min(4, Math.max(1, Math.trunc(maximumColumns)));
  const minimumCardWidth = Math.max(
    ABSOLUTE_MINIMUM_CARD_WIDTH,
    Math.trunc(previewWidth) + MINIMUM_TEXT_SPACE,
  );
  const fittingColumns = Math.floor(
    (Math.max(0, availableWidth) + GALLERY_GAP) / (minimumCardWidth + GALLERY_GAP),
  );
  return Math.min(safeMaximum, Math.max(1, fittingColumns));
}

export function resolveCompactGalleryColumns(availableWidth: number, previewWidth: number): number {
  const safePreviewWidth = Math.max(1, Math.trunc(previewWidth));
  const fittingColumns = Math.floor(
    (Math.max(0, availableWidth) + COMPACT_GALLERY_GAP)
      / (safePreviewWidth + COMPACT_GALLERY_GAP),
  );
  return Math.min(COMPACT_LAYOUT_SAFETY_LIMIT, Math.max(1, fittingColumns));
}
