import type { DownloadChangedEvent } from "../api/contracts";
import { galleryId, type Gallery, type GalleryId } from "../core/types";

export type DownloadProjection = {
  galleries: ReadonlyMap<GalleryId, Gallery>;
  applied: boolean;
};

export function applyDownloadChanged(
  galleries: ReadonlyMap<GalleryId, Gallery>,
  event: DownloadChangedEvent,
): DownloadProjection {
  const id = galleryId(event.galleryId);
  const gallery = galleries.get(id);
  if (!gallery) return { galleries, applied: false };
  const currentDownload = gallery.download;
  if (
    currentDownload?.entryId === event.entryId
    && event.revision <= (currentDownload.revision ?? -1)
  ) {
    return { galleries, applied: false };
  }

  const nextGalleries = new Map(galleries);
  nextGalleries.set(id, {
    ...gallery,
    download: {
      entryId: event.entryId,
      revision: event.revision,
      state: event.state,
      progress: event.progress,
      attempt: event.attempt,
      errorCode: event.errorCode,
      errorMessage: event.errorMessage,
      reviewKind: event.reviewKind,
      reviewId: event.reviewId,
    },
  });
  return { galleries: nextGalleries, applied: true };
}
