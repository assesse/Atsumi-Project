import type { BackendClient } from "../api/backend";
import type {
  BackendThumbnailKey,
  ThumbnailCompletionEvent,
  ThumbnailRequestDto,
} from "../api/contracts";
import type {
  ThumbnailAsset,
  ThumbnailCoordinatorAdapter,
  ThumbnailImageAsset,
} from "./client";
import {
  thumbnailKeyIdentity,
  type ThumbnailKey,
  type ThumbnailRequest,
} from "./model";

type PendingResolution = {
  request: ThumbnailRequest;
  requestId?: string;
  cancelled: boolean;
  settled: boolean;
  timeoutId: number;
  earlyCompletions: Map<string, ThumbnailCompletionEvent>;
  resolve: (asset: ThumbnailAsset) => void;
  reject: (error: Error) => void;
};

const THUMBNAIL_COMPLETION_TIMEOUT_MS = 30_000;

const backendKey = (key: ThumbnailKey): BackendThumbnailKey => {
  if (key.kind === "gallery-cover") return { kind: "galleryCover", galleryId: key.galleryId };
  if (key.kind === "source-page") return { kind: "galleryPage", galleryId: key.galleryId, sourcePage: key.page };
  return { kind: "artifactPage", entryId: key.entryId, sourcePage: key.page };
};

const requestDto = (request: ThumbnailRequest): ThumbnailRequestDto => ({
  key: backendKey(request.key),
  consumer: request.consumer,
  priority: request.priority,
});

const keysEqual = (left: BackendThumbnailKey, right: BackendThumbnailKey): boolean => {
  if (left.kind !== right.kind) return false;
  if (left.kind === "galleryCover") return right.kind === "galleryCover" && left.galleryId === right.galleryId;
  if (left.kind === "galleryPage") {
    return right.kind === "galleryPage"
      && left.galleryId === right.galleryId
      && left.sourcePage === right.sourcePage;
  }
  return right.kind === "artifactPage"
    && left.entryId === right.entryId
    && left.sourcePage === right.sourcePage;
};

const backendKeyIdentity = (key: BackendThumbnailKey): string => {
  if (key.kind === "galleryCover") return `gallery-cover:${key.galleryId}`;
  if (key.kind === "galleryPage") return `source-page:${key.galleryId}:${key.sourcePage}`;
  return `artifact-page:${key.entryId}:${key.sourcePage}`;
};

const priorityRank: Record<ThumbnailRequest["priority"], number> = {
  prefetch: 0,
  visible: 1,
  critical: 2,
};

type RetryableError = Error & { retryable?: boolean };

const errorFrom = (message: string, code?: string, retryable?: boolean): RetryableError => {
  const error = new Error(message);
  if (code) error.name = code;
  if (retryable !== undefined) (error as RetryableError).retryable = retryable;
  return error;
};

/**
 * Bridges the React subscription registry to the process-wide Rust coordinator.
 * The backend owns scheduling and canonical cache bytes; this adapter owns only
 * short-lived WebView Blob URLs and revokes them with the final UI subscriber.
 */
export class BackendThumbnailAdapter implements ThumbnailCoordinatorAdapter {
  private readonly pendingByIdentity = new Map<string, PendingResolution>();
  private readonly pendingByRequestId = new Map<string, PendingResolution>();
  private readonly bufferedCompletions = new Map<string, ThumbnailCompletionEvent>();
  private readonly cancelledRequestIds = new Set<string>();
  private readonly displayUrls = new Map<string, string>();
  private readonly completionListenerReady: Promise<void>;
  private unlisten?: () => void;
  private disposed = false;

  constructor(private readonly backend: BackendClient) {
    this.completionListenerReady = backend.on("thumbnail:ready", (event) => this.complete(event)).then((unlisten) => {
      if (this.disposed) unlisten();
      else this.unlisten = unlisten;
    });
  }

  resolve(request: ThumbnailRequest): Promise<ThumbnailAsset> {
    if (this.disposed) return Promise.reject(errorFrom("Thumbnail adapter is disposed"));
    const identity = thumbnailKeyIdentity(request.key);
    return new Promise<ThumbnailAsset>((resolve, reject) => {
      const pending: PendingResolution = {
        request,
        cancelled: false,
        settled: false,
        timeoutId: 0,
        earlyCompletions: new Map(),
        resolve,
        reject,
      };
      pending.timeoutId = window.setTimeout(() => {
        if (pending.cancelled || pending.settled) return;
        pending.cancelled = true;
        pending.settled = true;
        if (this.pendingByIdentity.get(identity) === pending) this.pendingByIdentity.delete(identity);
        if (pending.requestId) {
          this.pendingByRequestId.delete(pending.requestId);
          this.bufferedCompletions.delete(pending.requestId);
          this.rememberCancelledRequestId(pending.requestId);
          void this.backend.thumbnailCancel(pending.requestId).catch(() => undefined);
        }
        pending.earlyCompletions.clear();
        pending.reject(errorFrom("Thumbnail completion timed out", "THUMBNAIL_COMPLETION_TIMEOUT"));
      }, THUMBNAIL_COMPLETION_TIMEOUT_MS);
      this.pendingByIdentity.set(identity, pending);
      void this.start(pending, identity);
    });
  }

  reprioritize(request: ThumbnailRequest): void {
    const pending = this.pendingByIdentity.get(thumbnailKeyIdentity(request.key));
    if (!pending) return;
    pending.request = request;
    if (pending.requestId) {
      void this.backend.thumbnailReprioritize(pending.requestId, request.priority).catch(() => undefined);
    }
  }

  cancel(request: ThumbnailRequest): void {
    const identity = thumbnailKeyIdentity(request.key);
    const pending = this.pendingByIdentity.get(identity);
    if (!pending) return;
    pending.cancelled = true;
    window.clearTimeout(pending.timeoutId);
    pending.earlyCompletions.clear();
    this.pendingByIdentity.delete(identity);
    if (pending.requestId) {
      this.rememberCancelledRequestId(pending.requestId);
      void this.backend.thumbnailCancel(pending.requestId).catch(() => undefined);
    }
    if (!pending.settled) {
      pending.settled = true;
      pending.reject(errorFrom("Thumbnail request was cancelled", "THUMBNAIL_cancelled", true));
    }
  }

  release(request: ThumbnailRequest, asset: ThumbnailAsset): void {
    if (asset.kind !== "image") return;
    const identity = thumbnailKeyIdentity(request.key);
    if (this.displayUrls.get(identity) === asset.url) this.displayUrls.delete(identity);
    URL.revokeObjectURL(asset.url);
  }

  displayFailed(request: ThumbnailRequest, _reason?: string): void {
    const identity = thumbnailKeyIdentity(request.key);
    const url = this.displayUrls.get(identity);
    if (url) {
      this.displayUrls.delete(identity);
      URL.revokeObjectURL(url);
    }
    try {
      void this.backend.thumbnailInvalidate(backendKey(request.key)).catch(() => undefined);
    } catch {
      // Cache invalidation is best-effort; the client already exposes a safe error state.
    }
  }

  dispose(): void {
    this.disposed = true;
    this.unlisten?.();
    this.unlisten = undefined;
    for (const url of this.displayUrls.values()) URL.revokeObjectURL(url);
    this.displayUrls.clear();
    for (const pending of this.pendingByIdentity.values()) {
      pending.cancelled = true;
      window.clearTimeout(pending.timeoutId);
      pending.earlyCompletions.clear();
      if (pending.requestId) void this.backend.thumbnailCancel(pending.requestId).catch(() => undefined);
      if (pending.requestId) this.rememberCancelledRequestId(pending.requestId);
      if (!pending.settled) {
        pending.settled = true;
        pending.reject(errorFrom("Thumbnail adapter is disposed", "THUMBNAIL_cancelled", true));
      }
    }
    this.pendingByIdentity.clear();
    this.pendingByRequestId.clear();
    this.bufferedCompletions.clear();
    this.cancelledRequestIds.clear();
  }

  private async start(pending: PendingResolution, identity: string): Promise<void> {
    const submittedPriority = pending.request.priority;
    try {
      await this.completionListenerReady;
      if (pending.cancelled) return;
      const result = await this.backend.thumbnailRequest(requestDto(pending.request));
      if (!result.ok) throw errorFrom(result.error.message, result.error.code);
      pending.requestId = result.data.requestId;
      this.pendingByRequestId.set(result.data.requestId, pending);
      if (pending.cancelled) {
        this.pendingByRequestId.delete(result.data.requestId);
        this.bufferedCompletions.delete(result.data.requestId);
        this.rememberCancelledRequestId(result.data.requestId);
        void this.backend.thumbnailCancel(result.data.requestId).catch(() => undefined);
        return;
      }
      if (priorityRank[pending.request.priority] > priorityRank[submittedPriority]) {
        void this.backend
          .thumbnailReprioritize(result.data.requestId, pending.request.priority)
          .catch(() => undefined);
      }
      const buffered = pending.earlyCompletions.get(result.data.requestId)
        ?? this.bufferedCompletions.get(result.data.requestId);
      pending.earlyCompletions.clear();
      if (buffered) {
        this.bufferedCompletions.delete(result.data.requestId);
        this.complete(buffered);
      }
    } catch (error) {
      window.clearTimeout(pending.timeoutId);
      pending.earlyCompletions.clear();
      if (this.pendingByIdentity.get(identity) === pending) this.pendingByIdentity.delete(identity);
      if (pending.requestId) this.pendingByRequestId.delete(pending.requestId);
      if (!pending.cancelled && !pending.settled) {
        pending.settled = true;
        pending.reject(error instanceof Error ? error : errorFrom("Thumbnail request failed"));
      }
    }
  }

  private complete(event: ThumbnailCompletionEvent): void {
    if (this.cancelledRequestIds.delete(event.requestId)) return;
    const pending = this.pendingByRequestId.get(event.requestId);
    if (!pending) {
      const handshaking = this.pendingByIdentity.get(backendKeyIdentity(event.key));
      if (handshaking && !handshaking.requestId && !handshaking.cancelled && !handshaking.settled) {
        handshaking.earlyCompletions.set(event.requestId, event);
        return;
      }
      this.bufferedCompletions.set(event.requestId, event);
      if (this.bufferedCompletions.size > 256) {
        const oldest = this.bufferedCompletions.keys().next().value as string | undefined;
        if (oldest) this.bufferedCompletions.delete(oldest);
      }
      return;
    }

    this.pendingByRequestId.delete(event.requestId);
    window.clearTimeout(pending.timeoutId);
    pending.earlyCompletions.clear();
    const identity = thumbnailKeyIdentity(pending.request.key);
    if (this.pendingByIdentity.get(identity) === pending) this.pendingByIdentity.delete(identity);
    if (pending.cancelled || pending.settled) return;
    pending.settled = true;
    if (!keysEqual(event.key, backendKey(pending.request.key))) {
      pending.reject(errorFrom("Thumbnail completion key did not match its request", "THUMBNAIL_KEY_MISMATCH"));
      return;
    }
    if (event.outcome.status === "failed") {
      pending.reject(errorFrom(
        event.outcome.failure.message,
        `THUMBNAIL_${event.outcome.failure.code}`,
        event.outcome.failure.retryable,
      ));
      return;
    }

    const { thumbnail } = event.outcome.delivery;
    try {
      const bytes = Uint8Array.from(thumbnail.bytes);
      const blob = new Blob([bytes.buffer as ArrayBuffer], { type: thumbnail.contentType });
      const url = URL.createObjectURL(blob);
      const asset: ThumbnailImageAsset = {
        kind: "image",
        url,
        width: thumbnail.width,
        height: thumbnail.height,
      };
      this.displayUrls.set(identity, url);
      pending.resolve(asset);
    } catch (error) {
      pending.reject(error instanceof Error ? error : errorFrom("Thumbnail payload could not be displayed"));
    }
  }

  private rememberCancelledRequestId(requestId: string): void {
    this.cancelledRequestIds.delete(requestId);
    this.cancelledRequestIds.add(requestId);
    while (this.cancelledRequestIds.size > 256) {
      const oldest = this.cancelledRequestIds.values().next().value as string | undefined;
      if (oldest) this.cancelledRequestIds.delete(oldest);
    }
  }
}
