import {
  thumbnailKeyIdentity,
  type ThumbnailKey,
  type ThumbnailPriority,
  type ThumbnailRequest,
} from "./model";

export type ThumbnailImageAsset = {
  readonly kind: "image";
  readonly url: string;
  readonly width: number;
  readonly height: number;
};

export type ThumbnailSpriteAsset = {
  readonly kind: "sprite";
  readonly url: string;
  readonly sheetWidth: number;
  readonly sheetHeight: number;
  readonly columns: number;
  readonly rows: number;
  readonly cell: number;
};

export type ThumbnailMissingAsset = {
  readonly kind: "missing";
  readonly reason?: string;
};

export type ThumbnailAsset = ThumbnailImageAsset | ThumbnailSpriteAsset | ThumbnailMissingAsset;

export interface ThumbnailCoordinatorAdapter {
  /**
   * Resolve a display handle. Network, disk cache, retry and eviction policy all
   * belong to the backend implementation of this adapter.
   */
  resolve(request: ThumbnailRequest): ThumbnailAsset | Promise<ThumbnailAsset>;
  /** Optional best-effort promotion when a later subscriber needs the same key sooner. */
  reprioritize?(request: ThumbnailRequest): void | Promise<void>;
  /** Cancels unresolved work after the final frontend subscriber leaves. */
  cancel?(request: ThumbnailRequest): void | Promise<void>;
  /** Releases display-only resources such as Blob URLs. Canonical cache data remains backend-owned. */
  release?(request: ThumbnailRequest, asset: ThumbnailAsset): void | Promise<void>;
  /** Reports that the returned display handle could not be decoded by the webview. */
  displayFailed?(request: ThumbnailRequest, reason: string): void | Promise<void>;
}

export type ThumbnailSnapshot =
  | { readonly status: "idle" }
  | { readonly status: "loading" }
  | { readonly status: "resolved"; readonly asset: ThumbnailAsset }
  | { readonly status: "error"; readonly message: string; readonly code?: string };

type Entry = {
  identity: string;
  active: boolean;
  resolutionVersion: number;
  retryTimer?: ReturnType<typeof setTimeout>;
  orphanTimer?: ReturnType<typeof setTimeout>;
  retainedTimer?: ReturnType<typeof setTimeout>;
  retained: boolean;
  retainedLastUsed: number;
  retryableError: boolean;
  priorityRetryTriggered: boolean;
  displayFailureRetries: number;
  snapshot: ThumbnailSnapshot;
  request: ThumbnailRequest;
  listeners: Set<() => void>;
};

const idleSnapshot: ThumbnailSnapshot = { status: "idle" };
const RETRYABLE_ERROR_DELAY_MS = 3_000;
/** Leave a short window for scroll/React reconciliation churn before cancelling work. */
const ORPHAN_GRACE_MS = 400;
/** Keep a decoded display handle briefly so a revisited card need not recreate it. */
const RETAINED_ASSET_TTL_MS = 120_000;
const RETAINED_ASSET_CAPACITY = 256;
const priorityRank: Record<ThumbnailPriority, number> = {
  prefetch: 0,
  visible: 1,
  critical: 2,
};

/** Full source pages can be much larger than covers. Keep their canonical bytes
 * in the backend cache, but never retain WebView Blob URLs between Detail windows. */
const retainsDisplayHandle = (request: ThumbnailRequest): boolean => request.key.kind !== "source-page";

const retryableThumbnailErrorNames = new Set([
  "THUMBNAIL_cancelled",
  "THUMBNAIL_temporarilyUnavailable",
  "THUMBNAIL_resolver",
  "THUMBNAIL_coordinatorClosed",
  "THUMBNAIL_COORDINATOR_CLOSED",
  "THUMBNAIL_COMPLETION_TIMEOUT",
  "THUMBNAIL_WORKER_UNAVAILABLE",
]);

const validPositiveDimension = (value: number): boolean =>
  Number.isFinite(value) && value > 0;

const validatedAsset = (asset: ThumbnailAsset): ThumbnailAsset => {
  if (asset.kind === "image") {
    if (!asset.url.trim()) throw new Error("Thumbnail adapter returned an empty image URL");
    if (!validPositiveDimension(asset.width) || !validPositiveDimension(asset.height)) {
      throw new Error("Thumbnail adapter returned invalid intrinsic dimensions");
    }
  } else if (asset.kind === "sprite") {
    if (!asset.url.trim()) throw new Error("Thumbnail adapter returned an empty sprite URL");
    if (
      !validPositiveDimension(asset.sheetWidth)
      || !validPositiveDimension(asset.sheetHeight)
      || !Number.isInteger(asset.columns)
      || asset.columns < 1
      || !Number.isInteger(asset.rows)
      || asset.rows < 1
      || !Number.isInteger(asset.cell)
      || asset.cell < 0
      || asset.cell >= asset.columns * asset.rows
    ) {
      throw new Error("Thumbnail adapter returned invalid sprite metadata");
    }
  }
  return asset;
};

const errorMessage = (error: unknown): string =>
  error instanceof Error && error.message.trim() ? error.message : "Thumbnail resolution failed";

const isRetryableThumbnailError = (error: unknown): boolean =>
  error instanceof Error
  && (("retryable" in error && error.retryable === true)
    || retryableThumbnailErrorNames.has(error.name));

const thumbnailErrorCode = (error: unknown): string | undefined =>
  error instanceof Error && error.name.startsWith("THUMBNAIL_") ? error.name : undefined;

/**
 * A deliberately thin in-memory subscription registry. It coalesces simultaneous
 * consumers and retries known transient coordinator failures after the backend's
 * negative-cache TTL, but never persists, evicts or claims canonical cache state.
 */
export class ThumbnailClient {
  private readonly entries = new Map<string, Entry>();
  private nextRetainedSequence = 0;

  constructor(private readonly adapter: ThumbnailCoordinatorAdapter) {}

  getSnapshot(key: ThumbnailKey): ThumbnailSnapshot {
    return this.entries.get(thumbnailKeyIdentity(key))?.snapshot ?? idleSnapshot;
  }

  subscribe(request: ThumbnailRequest, listener: () => void): () => void {
    const identity = thumbnailKeyIdentity(request.key);
    let entry = this.entries.get(identity);
    if (!entry) {
      entry = {
        identity,
        active: true,
        resolutionVersion: 0,
        retained: false,
        retainedLastUsed: 0,
        retryableError: false,
        priorityRetryTriggered: false,
        displayFailureRetries: 0,
        snapshot: { status: "loading" },
        request,
        listeners: new Set(),
      };
      this.entries.set(identity, entry);
      entry.listeners.add(listener);
      this.resolve(entry);
    } else {
      this.clearOrphanTimer(entry);
      this.clearRetainedTimer(entry);
      if (entry.retained) {
        entry.retained = false;
        // There is no in-flight work to promote; keep the latest consumer for
        // a later display-failure/release lifecycle callback.
        entry.request = request;
      }
      entry.listeners.add(listener);
      if (priorityRank[request.priority] > priorityRank[entry.request.priority]) {
        entry.request = request;
        try {
          const promotion = this.adapter.reprioritize?.(request);
          if (promotion instanceof Promise) void promotion.catch(() => undefined);
        } catch {
          // Promotion is best-effort; the original resolution remains valid.
        }
      }
      if (
        entry.snapshot.status === "error"
        && entry.retryableError
        && request.priority !== "prefetch"
        && !entry.priorityRetryTriggered
      ) {
        entry.priorityRetryTriggered = true;
        this.beginRetry(entry);
      } else if (entry.snapshot.status === "error" && entry.retryableError) {
        this.scheduleRetry(entry);
      }
    }
    return () => {
      entry?.listeners.delete(listener);
      if (entry?.listeners.size === 0) this.scheduleOrphanCleanup(entry);
    };
  }

  dispose(): void {
    for (const entry of this.entries.values()) this.cleanup(entry);
    this.entries.clear();
  }

  /** Clears only inactive, recreatable display handles; visible subscribers stay intact. */
  clearRetainedCache(): number {
    const retained = [...this.entries.values()].filter(
      (entry) => entry.active && entry.retained && entry.listeners.size === 0,
    );
    for (const entry of retained) this.cleanup(entry);
    return retained.length;
  }

  reportDisplayFailure(request: ThumbnailRequest, reason: string): void {
    const entry = this.entries.get(thumbnailKeyIdentity(request.key));
    if (!entry || entry.snapshot.status === "error") return;
    if (entry.snapshot.status === "resolved") this.release(entry, entry.snapshot.asset);
    this.clearRetryTimer(entry);
    const shouldRetry = entry.displayFailureRetries === 0;
    entry.displayFailureRetries += 1;
    entry.retryableError = shouldRetry;
    this.publish(entry, {
      status: "error",
      message: reason,
      code: "THUMBNAIL_decodeFailed",
    });
    try {
      const report = this.adapter.displayFailed?.(request, reason);
      if (report instanceof Promise) void report.catch(() => undefined);
    } catch {
      // The shared UI state is already safe; reporting is best-effort.
    }
    if (shouldRetry) this.scheduleRetry(entry);
  }

  private publish(entry: Entry, snapshot: ThumbnailSnapshot): void {
    if (!entry.active || this.entries.get(entry.identity) !== entry) return;
    entry.snapshot = snapshot;
    for (const listener of entry.listeners) listener();
  }

  private scheduleOrphanCleanup(entry: Entry): void {
    if (!entry.active || entry.listeners.size > 0 || entry.orphanTimer !== undefined) return;
    entry.orphanTimer = setTimeout(() => {
      entry.orphanTimer = undefined;
      if (
        !entry.active
        || entry.listeners.size > 0
        || this.entries.get(entry.identity) !== entry
      ) return;
      if (entry.snapshot.status === "resolved" && retainsDisplayHandle(entry.request)) {
        this.retain(entry);
      } else {
        this.cleanup(entry);
      }
    }, ORPHAN_GRACE_MS);
  }

  private cleanup(entry: Entry): void {
    if (!entry.active) return;
    entry.active = false;
    this.clearRetryTimer(entry);
    this.clearOrphanTimer(entry);
    this.clearRetainedTimer(entry);
    entry.retained = false;
    if (this.entries.get(entry.identity) === entry) this.entries.delete(entry.identity);
    if (entry.snapshot.status === "loading") {
      this.callLifecycleHook(() => this.adapter.cancel?.(entry.request));
    } else if (entry.snapshot.status === "resolved") {
      this.release(entry, entry.snapshot.asset);
    }
  }

  private release(entry: Entry, asset: ThumbnailAsset): void {
    this.callLifecycleHook(() => this.adapter.release?.(entry.request, asset));
  }

  private callLifecycleHook(hook: () => void | Promise<void> | undefined): void {
    try {
      const result = hook();
      if (result instanceof Promise) void result.catch(() => undefined);
    } catch {
      // UI subscription cleanup must not be blocked by an adapter hook.
    }
  }

  private clearRetryTimer(entry: Entry): void {
    if (entry.retryTimer === undefined) return;
    clearTimeout(entry.retryTimer);
    entry.retryTimer = undefined;
  }

  private clearOrphanTimer(entry: Entry): void {
    if (entry.orphanTimer === undefined) return;
    clearTimeout(entry.orphanTimer);
    entry.orphanTimer = undefined;
  }

  private clearRetainedTimer(entry: Entry): void {
    if (entry.retainedTimer === undefined) return;
    clearTimeout(entry.retainedTimer);
    entry.retainedTimer = undefined;
  }

  private retain(entry: Entry): void {
    if (
      !entry.active
      || entry.listeners.size > 0
      || entry.snapshot.status !== "resolved"
      || this.entries.get(entry.identity) !== entry
    ) return;
    entry.retained = true;
    entry.retainedLastUsed = ++this.nextRetainedSequence;
    entry.retainedTimer = setTimeout(() => {
      entry.retainedTimer = undefined;
      if (
        entry.active
        && entry.retained
        && entry.listeners.size === 0
        && this.entries.get(entry.identity) === entry
      ) this.cleanup(entry);
    }, RETAINED_ASSET_TTL_MS);
    this.evictRetainedEntries();
  }

  private evictRetainedEntries(): void {
    while (this.retainedEntryCount() > RETAINED_ASSET_CAPACITY) {
      let oldest: Entry | undefined;
      for (const entry of this.entries.values()) {
        if (!entry.active || !entry.retained || entry.listeners.size > 0) continue;
        if (!oldest || entry.retainedLastUsed < oldest.retainedLastUsed) oldest = entry;
      }
      if (!oldest) return;
      this.cleanup(oldest);
    }
  }

  private retainedEntryCount(): number {
    let count = 0;
    for (const entry of this.entries.values()) {
      if (entry.active && entry.retained && entry.listeners.size === 0) count += 1;
    }
    return count;
  }

  private scheduleRetry(entry: Entry): void {
    if (
      !entry.active
      || entry.listeners.size === 0
      || !entry.retryableError
      || entry.snapshot.status !== "error"
      || entry.retryTimer !== undefined
    ) return;

    entry.retryTimer = setTimeout(() => {
      entry.retryTimer = undefined;
      if (
        !entry.active
        || entry.listeners.size === 0
        || !entry.retryableError
        || entry.snapshot.status !== "error"
        || this.entries.get(entry.identity) !== entry
      ) return;
      this.beginRetry(entry);
    }, RETRYABLE_ERROR_DELAY_MS);
  }

  private beginRetry(entry: Entry): void {
    if (
      !entry.active
      || entry.listeners.size === 0
      || !entry.retryableError
      || entry.snapshot.status !== "error"
      || this.entries.get(entry.identity) !== entry
    ) return;
    this.clearRetryTimer(entry);
    entry.retryableError = false;
    this.publish(entry, { status: "loading" });
    this.resolve(entry);
  }

  private fail(entry: Entry, resolutionVersion: number, error: unknown): void {
    if (
      !entry.active
      || resolutionVersion !== entry.resolutionVersion
      || this.entries.get(entry.identity) !== entry
    ) return;
    entry.retryableError = isRetryableThumbnailError(error);
    this.publish(entry, {
      status: "error",
      message: errorMessage(error),
      code: thumbnailErrorCode(error),
    });
    if (entry.retryableError) this.scheduleRetry(entry);
  }

  private resolve(entry: Entry): void {
    const resolutionVersion = ++entry.resolutionVersion;
    try {
      const resolution = this.adapter.resolve(entry.request);
      if (resolution instanceof Promise) {
        void resolution.then(
          (asset) => {
            try {
              const validated = validatedAsset(asset);
              if (!entry.active || this.entries.get(entry.identity) !== entry) {
                this.release(entry, validated);
                return;
              }
              if (resolutionVersion !== entry.resolutionVersion) {
                this.release(entry, validated);
                return;
              }
              this.clearRetryTimer(entry);
              entry.retryableError = false;
              entry.priorityRetryTriggered = false;
              this.publish(entry, { status: "resolved", asset: validated });
            } catch (error) {
              this.fail(entry, resolutionVersion, error);
            }
          },
          (error) => this.fail(entry, resolutionVersion, error),
        );
      } else {
        const asset = validatedAsset(resolution);
        if (resolutionVersion !== entry.resolutionVersion) {
          this.release(entry, asset);
          return;
        }
        this.clearRetryTimer(entry);
        entry.retryableError = false;
        entry.priorityRetryTriggered = false;
        this.publish(entry, { status: "resolved", asset });
      }
    } catch (error) {
      this.fail(entry, resolutionVersion, error);
    }
  }
}
