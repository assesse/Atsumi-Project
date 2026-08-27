import type { Gallery, GalleryId } from "../core/types";

export type ThumbnailConsumer = "explore" | "downloads" | "detail" | "review";

export type ThumbnailPriority = "critical" | "visible" | "prefetch";

type FixtureCellFallback = {
  readonly kind: "fixture-sheet-cell";
  readonly index: number;
};

export type GalleryCoverThumbnailKey = {
  readonly kind: "gallery-cover";
  readonly galleryId: GalleryId;
  /** Opaque source identifier. It is never interpreted as a display URL. */
  readonly sourceKey?: string;
  readonly fallback?: FixtureCellFallback;
};

export type SourcePageThumbnailKey = {
  readonly kind: "source-page";
  readonly galleryId: GalleryId;
  /** One-based source page number. */
  readonly page: number;
  /** Opaque gallery/source identifier owned by the backend coordinator. */
  readonly sourceKey?: string;
  readonly fallback?: FixtureCellFallback;
};

export type ArtifactPageThumbnailKey = {
  readonly kind: "artifact-page";
  readonly entryId: string;
  /** One-based immutable source page number recorded by the verified artifact. */
  readonly page: number;
  readonly fallback?: FixtureCellFallback;
};

export type ThumbnailKey = GalleryCoverThumbnailKey | SourcePageThumbnailKey | ArtifactPageThumbnailKey;

export type ThumbnailRequest = {
  readonly key: ThumbnailKey;
  readonly consumer: ThumbnailConsumer;
  readonly priority: ThumbnailPriority;
};

const normalizedFixtureCell = (value: number): number => {
  if (!Number.isInteger(value)) return 0;
  return ((value % 6) + 6) % 6;
};

export function galleryCoverThumbnailKey(
  gallery: Pick<Gallery, "id" | "thumbnailKey" | "coverIndex">,
): GalleryCoverThumbnailKey {
  return {
    kind: "gallery-cover",
    galleryId: gallery.id,
    ...(gallery.thumbnailKey?.trim() ? { sourceKey: gallery.thumbnailKey.trim() } : {}),
    fallback: { kind: "fixture-sheet-cell", index: normalizedFixtureCell(gallery.coverIndex) },
  };
}

export function sourcePageThumbnailKey(
  gallery: Pick<Gallery, "id" | "thumbnailKey" | "coverIndex">,
  page: number,
): SourcePageThumbnailKey {
  if (!Number.isInteger(page) || page < 1) {
    throw new RangeError("Thumbnail source pages are one-based positive integers");
  }
  return {
    kind: "source-page",
    galleryId: gallery.id,
    page,
    ...(gallery.thumbnailKey?.trim() ? { sourceKey: gallery.thumbnailKey.trim() } : {}),
    fallback: {
      kind: "fixture-sheet-cell",
      index: normalizedFixtureCell(gallery.coverIndex + page - 1),
    },
  };
}

export function artifactPageThumbnailKey(
  entryId: string,
  page: number,
  fallbackIndex = page - 1,
): ArtifactPageThumbnailKey {
  const normalizedEntryId = entryId.trim();
  if (!normalizedEntryId) throw new RangeError("Artifact entry ID must not be empty");
  if (!Number.isInteger(page) || page < 1) {
    throw new RangeError("Artifact source pages are one-based positive integers");
  }
  return {
    kind: "artifact-page",
    entryId: normalizedEntryId,
    page,
    fallback: { kind: "fixture-sheet-cell", index: normalizedFixtureCell(fallbackIndex) },
  };
}

/**
 * Stable identity used only to merge frontend subscriptions. The backend remains
 * the canonical cache/network owner and receives the structured key as-is.
 */
export function thumbnailKeyIdentity(key: ThumbnailKey): string {
  if (key.kind === "gallery-cover") return `gallery-cover:${key.galleryId}`;
  if (key.kind === "source-page") return `source-page:${key.galleryId}:${key.page}`;
  return `artifact-page:${key.entryId}:${key.page}`;
}

export function thumbnailConsumerForView(view: "explore" | "auto-find" | "downloads"): ThumbnailConsumer {
  if (view === "downloads") return "downloads";
  if (view === "auto-find") return "review";
  return "explore";
}
