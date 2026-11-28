import type { Gallery, GalleryId } from "../core/types";
import type { GalleryGroup } from "./galleryGrouping";

export type DownloadPreviewItem = Readonly<{
  galleryId: GalleryId;
  folder?: { index: number; total: number; label: string };
}>;
export type DownloadPreviewSession = Readonly<{
  items: readonly DownloadPreviewItem[];
  index: number;
  request: { galleryId: GalleryId; page: number; sequence: number };
}>;

const readable = (gallery: Gallery | undefined): gallery is Gallery =>
  gallery?.download?.state === "completed" && Number.isFinite(gallery.pages) && gallery.pages > 0;

/** Use the displayed order, including collapsed folders and every flat list page.
 * Only IDs and folder positions are retained; no originals or metadata are loaded. */
export function downloadPreviewItems(visible: readonly Gallery[], groups?: readonly GalleryGroup[]): DownloadPreviewItem[] {
  const seen = new Set<GalleryId>();
  const uniqueReadable = (items: readonly Gallery[]) => items.filter(gallery => {
    if (!readable(gallery) || seen.has(gallery.id)) return false;
    seen.add(gallery.id);
    return true;
  });
  if (!groups) return uniqueReadable(visible).map(gallery => ({ galleryId: gallery.id }));
  return groups.flatMap(group => {
    const items = uniqueReadable(group.items);
    return items.map((gallery, index) => ({ galleryId: gallery.id,
      folder: { index: index + 1, total: items.length, label: group.label } }));
  });
}

/** Stop at the outer library boundaries. An excluded or removed item cannot
 * become readable just because it was part of an earlier reading snapshot. */
export function nextDownloadPreviewIndex(items: readonly DownloadPreviewItem[], current: number,
  direction: -1 | 1, galleries: ReadonlyMap<GalleryId, Gallery>): number | null {
  if (current < 0 || current >= items.length) return null;
  for (let index = current + direction; index >= 0 && index < items.length; index += direction) {
    if (readable(galleries.get(items[index]!.galleryId))) return index;
  }
  return null;
}
