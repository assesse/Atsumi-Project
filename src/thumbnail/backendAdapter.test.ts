import { afterEach, describe, expect, it, vi } from "vitest";
import { backend, type BackendClient } from "../api/backend";
import type { ApiResult, ThumbnailCompletionEvent, ThumbnailRequestToken } from "../api/contracts";
import { galleryId } from "../core/types";
import { BackendThumbnailAdapter } from "./backendAdapter";
import type { ThumbnailRequest } from "./model";

const request: ThumbnailRequest = {
  key: { kind: "gallery-cover", galleryId: galleryId(4_051_038) },
  consumer: "explore",
  priority: "visible",
};

const readyEvent = (
  requestId: string,
  gallery: number = 4_051_038,
): ThumbnailCompletionEvent => ({
  requestId,
  key: { kind: "galleryCover", galleryId: gallery },
  outcome: {
    status: "ready",
    delivery: {
      key: { kind: "galleryCover", galleryId: gallery },
      cacheStatus: "resolved",
      thumbnail: {
        contentType: "image/svg+xml",
        bytes: [60, 115, 118, 103, 47, 62],
        width: 512,
        height: 512,
      },
    },
  },
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("BackendThumbnailAdapter", () => {
  it("turns one backend completion into a revocable display URL", async () => {
    const createObjectURL = vi.fn(() => "blob:https://atsumi.local/thumbnail-1");
    const revokeObjectURL = vi.fn();
    Object.defineProperty(URL, "createObjectURL", { configurable: true, value: createObjectURL });
    Object.defineProperty(URL, "revokeObjectURL", { configurable: true, value: revokeObjectURL });
    const adapter = new BackendThumbnailAdapter(backend);

    const asset = await adapter.resolve(request);

    expect(asset).toEqual({
      kind: "image",
      url: "blob:https://atsumi.local/thumbnail-1",
      width: 512,
      height: 512,
    });
    expect(createObjectURL).toHaveBeenCalledOnce();

    adapter.release(request, asset);
    expect(revokeObjectURL).toHaveBeenCalledWith("blob:https://atsumi.local/thumbnail-1");
    adapter.dispose();
  });

  it("exposes one shared worker request through the typed browser transport", async () => {
    const before = await backend.thumbnailStats();
    if (!before.ok) throw new Error(before.error.message);
    Object.defineProperty(URL, "createObjectURL", {
      configurable: true,
      value: vi.fn(() => "blob:https://atsumi.local/thumbnail-2"),
    });
    Object.defineProperty(URL, "revokeObjectURL", { configurable: true, value: vi.fn() });
    const adapter = new BackendThumbnailAdapter(backend);
    const pageRequest: ThumbnailRequest = {
      ...request,
      key: { kind: "source-page", galleryId: galleryId(4_051_038), page: 7 },
      consumer: "review",
      priority: "critical",
    };
    const asset = await adapter.resolve(pageRequest);
    const after = await backend.thumbnailStats();

    expect(asset.kind).toBe("image");
    expect(after).toMatchObject({ ok: true, data: { requestsTotal: before.data.requestsTotal + 1 } });
    adapter.release(pageRequest, asset);
    adapter.dispose();
  });

  it("requests verified review evidence by artifact entry and immutable source page", async () => {
    Object.defineProperty(URL, "createObjectURL", {
      configurable: true,
      value: vi.fn(() => "blob:https://atsumi.local/artifact-page"),
    });
    Object.defineProperty(URL, "revokeObjectURL", { configurable: true, value: vi.fn() });
    const submitted = vi.spyOn(backend, "thumbnailRequest");
    const adapter = new BackendThumbnailAdapter(backend);
    const artifactRequest: ThumbnailRequest = {
      key: { kind: "artifact-page", entryId: "verified-entry-101", page: 11 },
      consumer: "review",
      priority: "critical",
    };

    const asset = await adapter.resolve(artifactRequest);

    expect(asset.kind).toBe("image");
    expect(submitted).toHaveBeenCalledWith({
      key: { kind: "artifactPage", entryId: "verified-entry-101", sourcePage: 11 },
      consumer: "review",
      priority: "critical",
    });
    adapter.release(artifactRequest, asset);
    adapter.dispose();
  });

  it("replays a priority promotion that happens during the request handshake", async () => {
    let completeToken: ((result: ApiResult<ThumbnailRequestToken>) => void) | undefined;
    let completeListener: ((unlisten: () => void) => void) | undefined;
    let completionHandler: ((event: ThumbnailCompletionEvent) => void) | undefined;
    const thumbnailReprioritize = vi.fn(async () => ({ ok: true, data: true } as const));
    const transport = {
      on: vi.fn((_event, handler) => new Promise<() => void>((resolve) => {
        completionHandler = handler;
        completeListener = resolve;
      })),
      thumbnailRequest: vi.fn(() => new Promise<ApiResult<ThumbnailRequestToken>>((resolve) => {
        completeToken = resolve;
      })),
      thumbnailReprioritize,
      thumbnailCancel: vi.fn(async () => ({ ok: true, data: true } as const)),
    } as unknown as BackendClient;
    Object.defineProperty(URL, "createObjectURL", {
      configurable: true,
      value: vi.fn(() => "blob:https://atsumi.local/promoted"),
    });
    Object.defineProperty(URL, "revokeObjectURL", { configurable: true, value: vi.fn() });
    const adapter = new BackendThumbnailAdapter(transport);
    const prefetch: ThumbnailRequest = { ...request, priority: "prefetch" };
    const critical: ThumbnailRequest = { ...request, consumer: "detail", priority: "critical" };

    const resolution = adapter.resolve(prefetch);
    adapter.reprioritize(critical);
    completeListener?.(() => undefined);
    await vi.waitFor(() => expect(transport.thumbnailRequest).toHaveBeenCalledOnce());
    completeToken?.({
      ok: true,
      data: { requestId: "thumbnail-promoted", key: { kind: "galleryCover", galleryId: 4_051_038 } },
    });
    await vi.waitFor(() => {
      expect(thumbnailReprioritize).toHaveBeenCalledWith("thumbnail-promoted", "critical");
    });
    completionHandler?.(readyEvent("thumbnail-promoted"));
    const asset = await resolution;
    adapter.release(critical, asset);
    adapter.dispose();
  });

  it("settles cancellation and drops a late completion instead of replaying it from the buffer", async () => {
    let completionHandler: ((event: ThumbnailCompletionEvent) => void) | undefined;
    const transport = {
      on: vi.fn(async (_event, handler) => {
        completionHandler = handler;
        return () => undefined;
      }),
      thumbnailRequest: vi.fn(async () => ({
        ok: true,
        data: { requestId: "cancelled-request", key: { kind: "galleryCover", galleryId: 4_051_038 } },
      } as const)),
      thumbnailCancel: vi.fn(async () => ({ ok: true, data: true } as const)),
      thumbnailReprioritize: vi.fn(async () => ({ ok: true, data: true } as const)),
    } as unknown as BackendClient;
    const createObjectURL = vi.fn(() => "blob:https://atsumi.local/non-stale");
    Object.defineProperty(URL, "createObjectURL", { configurable: true, value: createObjectURL });
    Object.defineProperty(URL, "revokeObjectURL", { configurable: true, value: vi.fn() });
    const adapter = new BackendThumbnailAdapter(transport);

    const first = adapter.resolve(request);
    await vi.waitFor(() => expect(transport.thumbnailRequest).toHaveBeenCalledOnce());
    adapter.cancel(request);
    await vi.waitFor(() => expect(transport.thumbnailCancel).toHaveBeenCalledWith("cancelled-request"));
    await expect(first).rejects.toMatchObject({ name: "THUMBNAIL_cancelled" });
    completionHandler?.(readyEvent("cancelled-request"));

    const second = adapter.resolve(request);
    await vi.waitFor(() => expect(transport.thumbnailRequest).toHaveBeenCalledTimes(2));
    await Promise.resolve();
    expect(createObjectURL).not.toHaveBeenCalled();
    completionHandler?.(readyEvent("cancelled-request"));
    await expect(second).resolves.toMatchObject({ kind: "image", url: "blob:https://atsumi.local/non-stale" });
    expect(createObjectURL).toHaveBeenCalledOnce();
    adapter.dispose();
  });

  it("keeps the expected early completion while unrelated buffered events are evicted", async () => {
    let completeToken: ((result: ApiResult<ThumbnailRequestToken>) => void) | undefined;
    let completionHandler: ((event: ThumbnailCompletionEvent) => void) | undefined;
    const transport = {
      on: vi.fn(async (_event, handler) => {
        completionHandler = handler;
        return () => undefined;
      }),
      thumbnailRequest: vi.fn(() => new Promise<ApiResult<ThumbnailRequestToken>>((resolve) => {
        completeToken = resolve;
      })),
      thumbnailReprioritize: vi.fn(async () => ({ ok: true, data: true } as const)),
      thumbnailCancel: vi.fn(async () => ({ ok: true, data: true } as const)),
      thumbnailInvalidate: vi.fn(async () => ({
        ok: true,
        data: {
          key: { kind: "galleryCover", galleryId: 4_051_038 },
          successCacheRemoved: false,
          negativeCacheRemoved: false,
        },
      } as const)),
    } as unknown as BackendClient;
    Object.defineProperty(URL, "createObjectURL", {
      configurable: true,
      value: vi.fn(() => "blob:https://atsumi.local/early"),
    });
    Object.defineProperty(URL, "revokeObjectURL", { configurable: true, value: vi.fn() });
    const adapter = new BackendThumbnailAdapter(transport);

    const resolution = adapter.resolve(request);
    await vi.waitFor(() => expect(transport.thumbnailRequest).toHaveBeenCalledOnce());
    for (let index = 0; index < 300; index += 1) {
      completionHandler?.(readyEvent(`unrelated-${index}`, 5_000_000 + index));
    }
    completionHandler?.(readyEvent("expected-early"));
    completeToken?.({
      ok: true,
      data: { requestId: "expected-early", key: { kind: "galleryCover", galleryId: 4_051_038 } },
    });

    const asset = await resolution;
    expect(asset).toMatchObject({ kind: "image", url: "blob:https://atsumi.local/early" });
    adapter.release(request, asset);
    adapter.dispose();
  });

  it("invalidates the backend cache after a decoded display failure", async () => {
    const thumbnailInvalidate = vi.fn(async () => ({
      ok: true,
      data: {
        key: { kind: "galleryCover", galleryId: 4_051_038 },
        successCacheRemoved: true,
        negativeCacheRemoved: false,
      },
    } as const));
    const transport = {
      on: vi.fn(async () => () => undefined),
      thumbnailInvalidate,
    } as unknown as BackendClient;
    const adapter = new BackendThumbnailAdapter(transport);

    adapter.displayFailed(request, "decode failed");

    await vi.waitFor(() => {
      expect(thumbnailInvalidate).toHaveBeenCalledWith({
        kind: "galleryCover",
        galleryId: 4_051_038,
      });
    });
    adapter.dispose();
  });

  it("preserves the backend retryability contract on typed failures", async () => {
    let completionHandler: ((event: ThumbnailCompletionEvent) => void) | undefined;
    const transport = {
      on: vi.fn(async (_event, handler) => {
        completionHandler = handler;
        return () => undefined;
      }),
      thumbnailRequest: vi.fn(async () => ({
        ok: true,
        data: {
          requestId: "thumbnail-retryable",
          key: { kind: "galleryCover", galleryId: 4_051_038 },
        },
      } as const)),
      thumbnailCancel: vi.fn(async () => ({ ok: true, data: true } as const)),
    } as unknown as BackendClient;
    const adapter = new BackendThumbnailAdapter(transport);

    const resolution = adapter.resolve(request);
    await vi.waitFor(() => expect(transport.thumbnailRequest).toHaveBeenCalledOnce());
    completionHandler?.({
      requestId: "thumbnail-retryable",
      key: { kind: "galleryCover", galleryId: 4_051_038 },
      outcome: {
        status: "failed",
        failure: {
          key: { kind: "galleryCover", galleryId: 4_051_038 },
          code: "responseInvalid",
          message: "thumbnail source returned a non-image response",
          retryable: true,
          negativeCacheHit: false,
        },
      },
    });

    await expect(resolution).rejects.toMatchObject({
      name: "THUMBNAIL_responseInvalid",
      retryable: true,
    });
    adapter.dispose();
  });
});
