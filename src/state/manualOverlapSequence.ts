import type { BackendClient } from "../api/backend";
import type { GalleryId } from "../core/types";

export type ManualOverlapItem = { reviewId: string; incomingGalleryId: GalleryId };

/** Read all pending jobs, not the paginated history of already applied actions. */
export async function collectManualOverlapReviews(
  backend: Pick<BackendClient, "downloadEntriesList">, current: () => boolean,
): Promise<ManualOverlapItem[] | null> {
  const items = new Map<string, ManualOverlapItem>();
  const pages = new Set<string>();
  const pageSize = 200;
  for (let page = 1; ; page++) {
    const result = await backend.downloadEntriesList({ state: "review_required", page, pageSize });
    if (!current()) return null;
    if (!result.ok) throw new Error(result.error.message);
    if (result.data.page !== page) throw new Error("수동 검토 목록의 페이지를 확인하지 못했습니다.");
    const entries = result.data.entries;
    const signature = JSON.stringify(entries.map(entry => entry.entryId));
    if (entries.length && pages.has(signature)) throw new Error("수동 검토 목록이 반복되어 중단했습니다.");
    pages.add(signature);
    for (const entry of entries) {
      if (entry.state === "review_required" && entry.reviewKind === "gallery_duplicate" && entry.reviewId) {
        items.set(entry.reviewId, { reviewId: entry.reviewId, incomingGalleryId: entry.galleryId });
      }
    }
    if (page * pageSize >= result.data.totalItems) return [...items.values()];
    if (!entries.length) throw new Error("수동 검토 목록을 끝까지 불러오지 못했습니다.");
  }
}
