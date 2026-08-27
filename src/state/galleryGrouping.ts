import type { Gallery } from "../core/types";

export type GalleryGrouping = "all" | "day" | "artist";
export type GalleryAccordionGrouping = Exclude<GalleryGrouping, "all">;

export type GalleryGroup = {
  key: string;
  label: string;
  items: Gallery[];
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
