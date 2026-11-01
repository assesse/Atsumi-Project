import type { Gallery } from "../core/types";
import type { ThumbnailClient } from "./client";
import { galleryCoverThumbnailKey, type ThumbnailRequest } from "./model";
import { galleryCoverPageSignature } from "./pagePrefetch";
import type { GalleryCoverSessionRetainer } from "./sessionCoverRetainer";

type CoverPreload = {
  identity: string;
  gallery: Gallery;
  request: ThumbnailRequest;
  status: "queued" | "loading" | "settled";
  release?: () => void;
};

/** Warms folder covers without mounting cards or pinning a second library cache. */
export class GalleryCoverBackgroundPreloader {
  private entries = new Map<string, CoverPreload>();
  private queued: CoverPreload[] = [];
  private queueIndex = 0;
  private active = 0;
  private pumping = false;
  private disposed = false;
  private pumpTimer: ReturnType<typeof setTimeout> | undefined;

  constructor(
    private readonly client: ThumbnailClient,
    private readonly retainer?: GalleryCoverSessionRetainer,
    private readonly concurrency = 4,
  ) {
    if (!Number.isInteger(concurrency) || concurrency < 1) {
      throw new RangeError("Background cover concurrency must be a positive integer");
    }
  }

  /** Reconcile saved selections and cover changes while preserving shared pending work. */
  update(items: readonly Gallery[]): void {
    if (this.disposed) return;
    const nextEntries = new Map<string, CoverPreload>();
    for (const gallery of items) {
      const identity = galleryCoverPageSignature([gallery]);
      if (nextEntries.has(identity)) continue;
      const entry = this.entries.get(identity) ?? {
        identity,
        gallery,
        request: { key: galleryCoverThumbnailKey(gallery), consumer: "downloads", priority: "prefetch" },
        status: "queued",
      } satisfies CoverPreload;
      entry.gallery = gallery;
      nextEntries.set(identity, entry);
    }
    for (const [identity, entry] of this.entries) {
      if (nextEntries.has(identity)) continue;
      if (entry.status === "loading") this.active -= 1;
      entry.release?.();
    }
    this.entries = nextEntries;
    this.queued = [...nextEntries.values()].filter((entry) => entry.status === "queued");
    this.queueIndex = 0;
    this.pump();
  }

  dispose(): void {
    this.disposed = true;
    clearTimeout(this.pumpTimer);
    this.pumpTimer = undefined;
    for (const entry of this.entries.values()) entry.release?.();
    this.entries.clear();
    this.queued = [];
    this.queueIndex = 0;
    this.active = 0;
  }

  private pump(): void {
    if (this.disposed || this.pumping || this.pumpTimer !== undefined) return;
    this.pumping = true;
    let started = 0;
    try {
      while (started < 16 && this.active < this.concurrency && this.queueIndex < this.queued.length) {
        started += 1;
        const entry = this.queued[this.queueIndex++]!;
        entry.status = "loading";
        this.active += 1;
        // Synchronous fixture/cached results can notify before subscribe returns.
        entry.release = this.client.subscribe(entry.request, () => {
          queueMicrotask(() => this.settle(entry));
        });
        this.settle(entry);
      }
    } finally {
      this.pumping = false;
    }
    if (this.active < this.concurrency && this.queueIndex < this.queued.length) this.schedulePump();
  }

  private schedulePump(): void {
    if (this.disposed || this.pumpTimer !== undefined || this.queueIndex >= this.queued.length) return;
    // Cache hits can settle synchronously. Yield a task instead of draining
    // thousands of covers in one effect/microtask chain ahead of user input.
    this.pumpTimer = setTimeout(() => {
      this.pumpTimer = undefined;
      this.pump();
    }, 0);
  }

  private settle(entry: CoverPreload): void {
    if (this.disposed || this.entries.get(entry.identity) !== entry || entry.status !== "loading") return;
    const snapshot = this.client.getSnapshot(entry.request.key);
    if (snapshot.status !== "resolved" && snapshot.status !== "error") return;
    entry.status = "settled";
    this.active -= 1;
    // Transfer completed covers to the app's existing count/byte-bounded session
    // budget before releasing this loader's subscription. Errors must not pin it.
    if (snapshot.status === "resolved") this.retainer?.visit("downloads", [entry.gallery]);
    entry.release?.();
    entry.release = undefined;
    this.schedulePump();
  }
}
