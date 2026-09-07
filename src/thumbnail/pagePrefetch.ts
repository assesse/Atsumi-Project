import type { Gallery } from "../core/types";
import type { ThumbnailClient } from "./client";
import {
  galleryCoverThumbnailKey,
  thumbnailConsumerForView,
  thumbnailKeyIdentity,
  type ThumbnailRequest,
} from "./model";

export type GalleryCoverPageView = "auto-find" | "downloads";

/** Stable across download progress/detail projection changes that do not affect a cover request. */
export const galleryCoverPageSignature = (items: readonly Gallery[]): string => JSON.stringify(
  items.map((gallery) => [
    Number(gallery.id),
    gallery.coverIndex,
    gallery.thumbnailKey?.trim() ?? "",
    thumbnailKeyIdentity(galleryCoverThumbnailKey(gallery)),
  ]),
);

export const galleryCoverPrefetchRequests = (
  items: readonly Gallery[],
  view: GalleryCoverPageView,
): ThumbnailRequest[] => {
  const consumer = thumbnailConsumerForView(view);
  const identities = new Set<string>();
  return items.flatMap((gallery) => {
    const key = galleryCoverThumbnailKey(gallery);
    const identity = thumbnailKeyIdentity(key);
    if (identities.has(identity)) return [];
    identities.add(identity);
    return [{ key, consumer, priority: "prefetch" as const }];
  });
};

const terminal = (client: ThumbnailClient, request: ThumbnailRequest): boolean => {
  const status = client.getSnapshot(request.key).status;
  return status === "resolved" || status === "error";
};

/**
 * Warms every cover in the current client-side page at background priority.
 * Only after they have all reached a terminal state does it warm the next page.
 * Visible GalleryThumbnail subscribers can promote the same coalesced requests,
 * while disposal releases both page subscriptions without leaving hidden DOM.
 */
export function prefetchNextGalleryPageAfterCurrent(
  client: ThumbnailClient,
  view: GalleryCoverPageView,
  currentItems: readonly Gallery[],
  nextItems: readonly Gallery[],
): () => void {
  const currentRequests = galleryCoverPrefetchRequests(currentItems, view);
  const nextRequests = galleryCoverPrefetchRequests(nextItems, view);
  const currentReleases: Array<() => void> = [];
  let nextReleases: Array<() => void> | null = null;
  let disposed = false;

  const beginNextPage = () => {
    if (disposed || nextReleases !== null) return;
    if (!currentRequests.every((request) => terminal(client, request))) return;
    nextReleases = nextRequests.map((request) => client.subscribe(request, () => undefined));
  };

  for (const request of currentRequests) {
    currentReleases.push(client.subscribe(request, beginNextPage));
  }
  beginNextPage();

  return () => {
    disposed = true;
    for (const release of currentReleases) release();
    for (const release of nextReleases ?? []) release();
  };
}
