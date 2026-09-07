import type { Gallery } from "../core/types";
import type { ThumbnailAsset, ThumbnailClient } from "./client";
import { thumbnailKeyIdentity, type ThumbnailRequest } from "./model";
import {
  galleryCoverPrefetchRequests,
  type GalleryCoverPageView,
} from "./pagePrefetch";

/**
 * Eight times the ordinary short-lived display-handle count, guarded first by
 * a byte budget. In practice the retained value is a compressed thumbnail Blob
 * URL, not a mounted/decoded card. Both ceilings prevent a full library walk
 * from growing the WebView without bound while keeping a long session revisitable.
 */
export const SESSION_GALLERY_COVER_CAPACITY = 2_048;
export const SESSION_GALLERY_COVER_BYTE_BUDGET = 512 * 1024 * 1024;

const FALLBACK_IMAGE_BYTES_PER_PIXEL = 0.5;
const FALLBACK_IMAGE_MIN_BYTES = 16 * 1024;
const FALLBACK_SPRITE_MIN_BYTES = 8 * 1024;
const FALLBACK_MISSING_BYTES = 1024;

const estimatedAssetBytes = (asset: ThumbnailAsset): number => {
  if (asset.kind === "missing") return FALLBACK_MISSING_BYTES;
  if (asset.kind === "image") {
    if (asset.byteLength !== undefined && Number.isSafeInteger(asset.byteLength) && asset.byteLength >= 0) {
      return Math.max(1, asset.byteLength);
    }
    return Math.max(
      FALLBACK_IMAGE_MIN_BYTES,
      Math.ceil(asset.width * asset.height * FALLBACK_IMAGE_BYTES_PER_PIXEL),
    );
  }
  const cells = Math.max(1, asset.columns * asset.rows);
  return Math.max(
    FALLBACK_SPRITE_MIN_BYTES,
    Math.ceil((asset.sheetWidth * asset.sheetHeight * FALLBACK_IMAGE_BYTES_PER_PIXEL) / cells),
  );
};

type RetainedCover = {
  request: ThumbnailRequest;
  release: () => void;
  lastVisited: number;
  estimatedBytes: number;
};

/** Pins only covers belonging to a page the user has actually visited. */
export class GalleryCoverSessionRetainer {
  private readonly entries = new Map<string, RetainedCover>();
  private visitSequence = 0;
  private retainedBytesValue = 0;

  constructor(
    private readonly client: ThumbnailClient,
    private readonly capacity = SESSION_GALLERY_COVER_CAPACITY,
    private readonly byteBudget = SESSION_GALLERY_COVER_BYTE_BUDGET,
  ) {
    if (!Number.isInteger(capacity) || capacity < 1) {
      throw new RangeError("Gallery cover session capacity must be a positive integer");
    }
    if (!Number.isSafeInteger(byteBudget) || byteBudget < 1) {
      throw new RangeError("Gallery cover session byte budget must be a positive safe integer");
    }
  }

  get size(): number {
    return this.entries.size;
  }

  get retainedBytes(): number {
    return this.retainedBytesValue;
  }

  visit(view: GalleryCoverPageView, items: readonly Gallery[]): void {
    for (const request of galleryCoverPrefetchRequests(items, view)) {
      const identity = thumbnailKeyIdentity(request.key);
      const existing = this.entries.get(identity);
      if (existing) {
        existing.lastVisited = ++this.visitSequence;
        this.settle(identity, existing);
        continue;
      }

      const entry: RetainedCover = {
        request,
        release: () => undefined,
        lastVisited: ++this.visitSequence,
        estimatedBytes: 0,
      };
      this.entries.set(identity, entry);
      // A fixture adapter may resolve synchronously before subscribe returns.
      // Defer its listener so `release` is installed before settlement; the
      // immediate settle below still handles that synchronous result at once.
      entry.release = this.client.subscribe(request, () => {
        queueMicrotask(() => this.settle(identity, entry));
      });
      this.settle(identity, entry);
      this.evictLeastRecentlyVisited();
    }
  }

  clear(): void {
    const entries = [...this.entries.values()];
    this.entries.clear();
    this.retainedBytesValue = 0;
    for (const entry of entries) entry.release();
  }

  private settle(identity: string, entry: RetainedCover): void {
    if (this.entries.get(identity) !== entry) return;
    const snapshot = this.client.getSnapshot(entry.request.key);
    if (snapshot.status === "resolved") {
      const nextEstimate = estimatedAssetBytes(snapshot.asset);
      this.retainedBytesValue += nextEstimate - entry.estimatedBytes;
      entry.estimatedBytes = nextEstimate;
      this.evictLeastRecentlyVisited();
      return;
    }
    if (snapshot.status !== "error") return;
    this.remove(identity, entry);
  }

  private evictLeastRecentlyVisited(): void {
    while (this.retainedBytesValue > this.byteBudget) {
      const oldestResolved = this.oldestEntry((entry) => entry.estimatedBytes > 0);
      if (!oldestResolved) break;
      this.remove(oldestResolved[0], oldestResolved[1]);
    }
    while (this.entries.size > this.capacity) {
      const oldestEntry = this.oldestEntry(() => true);
      if (!oldestEntry) return;
      this.remove(oldestEntry[0], oldestEntry[1]);
    }
  }

  private oldestEntry(
    include: (entry: RetainedCover) => boolean,
  ): readonly [string, RetainedCover] | undefined {
    let oldestIdentity: string | undefined;
    let oldest: RetainedCover | undefined;
    for (const [identity, entry] of this.entries) {
      if (!include(entry)) continue;
      if (!oldest || entry.lastVisited < oldest.lastVisited) {
        oldestIdentity = identity;
        oldest = entry;
      }
    }
    return oldest && oldestIdentity !== undefined ? [oldestIdentity, oldest] : undefined;
  }

  private remove(identity: string, entry: RetainedCover): void {
    if (this.entries.get(identity) !== entry) return;
    this.entries.delete(identity);
    this.retainedBytesValue = Math.max(0, this.retainedBytesValue - entry.estimatedBytes);
    entry.release();
  }
}
