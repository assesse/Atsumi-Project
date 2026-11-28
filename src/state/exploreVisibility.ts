import type { Gallery, GalleryId } from "../core/types";
import { galleryProcessingStatus } from "./downloadStatus";

export const exploreVisibilityLabels = { excluded: "제외됨", duplicate: "중복 후보", completed: "다운로드 완료" } as const;
export type ExploreVisibility = Record<keyof typeof exploreVisibilityLabels, boolean>;
const defaults: ExploreVisibility = { excluded: true, duplicate: true, completed: true };
const storageKey = "atsumi.hitomi.explore.visibility.v1";

export function readExploreVisibility(): ExploreVisibility {
  try {
    const saved = JSON.parse(localStorage.getItem(storageKey) ?? "null");
    return {
      excluded: saved?.excluded !== false,
      duplicate: saved?.duplicate !== false,
      completed: saved?.completed !== false,
    };
  } catch { return { ...defaults }; }
}

export function saveExploreVisibility(value: ExploreVisibility): void {
  try { localStorage.setItem(storageKey, JSON.stringify(value)); } catch { /* Still usable for this session. */ }
}

export function omittedExploreIds(
  visibility: ExploreVisibility, galleries: ReadonlyMap<GalleryId, Gallery>,
  excludedIds: ReadonlySet<GalleryId>, duplicateCounts: ReadonlyMap<GalleryId, number>,
): GalleryId[] {
  if (visibility.excluded && visibility.duplicate && visibility.completed) return [];
  const omitted = new Set<GalleryId>(visibility.excluded ? [] : excludedIds);
  for (const id of duplicateCounts.keys()) {
    if (!visibility.duplicate && !excludedIds.has(id)) omitted.add(id);
  }
  for (const [id, gallery] of galleries) {
    if (!gallery.download) continue;
    const state = galleryProcessingStatus(gallery.download, {
      duplicateCount: duplicateCounts.get(id),
      exclusionLabel: excludedIds.has(id) ? "제외됨" : undefined,
    })?.state;
    const category = state === "quarantined" ? "excluded" : state;
    if (category === "excluded" || category === "duplicate" || category === "completed") {
      if (visibility[category]) omitted.delete(id);
      else omitted.add(id);
    }
  }
  return Array.from(omitted).sort((a, b) => a - b);
}
