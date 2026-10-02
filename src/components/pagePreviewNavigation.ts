export type PreviewReadingDirection = "ltr" | "rtl";

/** Zero is a leading empty slot, not a real/source page. */
export function pagePreviewSlots(anchor: number | null, count: number, spread: boolean,
  reading: PreviewReadingDirection): (number | null)[] {
  if (anchor === null || count < 1) return [];
  const pages = (spread ? [anchor, anchor + 1] : [anchor])
    .map((page) => page >= 1 && page <= count ? page : null);
  return reading === "rtl" && spread ? pages.reverse() : pages;
}

export function nextPagePreviewAnchor(anchor: number, count: number, spread: boolean,
  direction: -1 | 1): number | null {
  const next = anchor + direction * (spread ? 2 : 1);
  return next >= (spread ? 0 : 1) && next <= count ? next : null;
}
