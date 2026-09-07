import type { Gallery } from "../core/types";

export type GalleryGrouping = "all" | "day" | "artist";
export type GalleryAccordionGrouping = Exclude<GalleryGrouping, "all">;

export type GalleryGroup = {
  key: string;
  label: string;
  items: Gallery[];
};

export type ArtistFolderTag = {
  value: string;
  count: number;
  favorite: boolean;
};

const UNKNOWN_ARTIST = "작가 정보 없음";
const UNKNOWN_DAY = "날짜 정보 없음";

const dateKey = (value: string | undefined): string => {
  const match = value?.match(/^(\d{4})-(\d{2})-(\d{2})/);
  return match ? `${match[1]}-${match[2]}-${match[3]}` : "unknown";
};

const dateLabel = (key: string): string => {
  if (key === "unknown") return UNKNOWN_DAY;
  const [year, month, day] = key.split("-");
  return `${Number(year)}년 ${Number(month)}월 ${Number(day)}일`;
};

const groupKey = (grouping: GalleryAccordionGrouping, identity: string): string =>
  `${grouping}\u001f${identity.trim().toLocaleLowerCase()}`;

/**
 * Produces stable, presentation-neutral grouping buckets. The caller chooses
 * the date source because Auto Find discovery time and download update time
 * represent different user-facing timelines.
 */
export function groupGalleries(
  galleries: readonly Gallery[],
  grouping: GalleryAccordionGrouping,
  dateForGallery: (gallery: Gallery) => string | undefined,
): GalleryGroup[] {
  const groups = new Map<string, GalleryGroup>();

  for (const gallery of galleries) {
    const identity = grouping === "artist"
      ? gallery.artist.trim() || UNKNOWN_ARTIST
      : dateKey(dateForGallery(gallery));
    const key = groupKey(grouping, identity);
    const current = groups.get(key);
    if (current) {
      current.items.push(gallery);
      continue;
    }
    groups.set(key, {
      key,
      label: grouping === "artist" ? identity : dateLabel(identity),
      items: [gallery],
    });
  }

  const result = [...groups.values()];
  result.sort((left, right) => {
    if (grouping === "day") {
      const leftIdentity = left.key.split("\u001f")[1] ?? "unknown";
      const rightIdentity = right.key.split("\u001f")[1] ?? "unknown";
      const leftUnknown = leftIdentity === "unknown";
      const rightUnknown = rightIdentity === "unknown";
      if (leftUnknown !== rightUnknown) return leftUnknown ? 1 : -1;
      return rightIdentity.localeCompare(leftIdentity);
    }
    return left.label.localeCompare(right.label, "ko");
  });
  return result;
}

export const galleryGroupStorageKey = (
  view: "auto-find" | "downloads",
  group: Pick<GalleryGroup, "key">,
): string => `${view}\u001f${group.key}`;

const downloadRecency = (gallery: Gallery): string => (
  gallery.download?.createdAt
  ?? gallery.download?.updatedAt
  ?? gallery.publishedAt
);

/**
 * Returns the virtual folder covers in download-recency order. The gallery id
 * tie-breaker keeps the three-cover stack deterministic when legacy entries do
 * not have timestamps.
 */
export function recentDownloadedGalleries(
  galleries: readonly Gallery[],
  limit = 3,
): Gallery[] {
  if (limit <= 0) return [];
  return [...galleries]
    .sort((left, right) => {
      const order = downloadRecency(right).localeCompare(downloadRecency(left));
      return order || Number(right.id) - Number(left.id);
    })
    .slice(0, Math.trunc(limit));
}

/** Chooses the cover shown for a virtual artist folder. */
export function latestDownloadedGallery(galleries: readonly Gallery[]): Gallery | undefined {
  return recentDownloadedGalleries(galleries, 1)[0];
}

/**
 * Summarizes the metadata that has already been resolved for an artist folder.
 * A tag is counted once per gallery, favorite tags are kept ahead of ordinary
 * tags, and frequency then determines the display order.
 */
export function summarizeArtistFolderTags(
  galleries: readonly Gallery[],
  favoriteMetadata: ReadonlySet<string>,
  limit = 6,
): ArtistFolderTag[] {
  if (limit <= 0) return [];
  const normalizedFavorites = new Set([...favoriteMetadata].map((value) => value.trim().toLocaleLowerCase()));
  const counts = new Map<string, { value: string; count: number }>();

  for (const gallery of galleries) {
    const seen = new Set<string>();
    for (const value of gallery.tags) {
      const normalized = value.trim().toLocaleLowerCase();
      if (!normalized || seen.has(normalized)) continue;
      seen.add(normalized);
      const current = counts.get(normalized);
      if (current) current.count += 1;
      else counts.set(normalized, { value: value.trim(), count: 1 });
    }
  }

  return [...counts.entries()]
    .map(([normalized, tag]) => ({
      ...tag,
      favorite: normalizedFavorites.has(normalized),
    }))
    .sort((left, right) =>
      Number(right.favorite) - Number(left.favorite)
      || right.count - left.count
      || left.value.localeCompare(right.value, "en"))
    .slice(0, Math.trunc(limit));
}
