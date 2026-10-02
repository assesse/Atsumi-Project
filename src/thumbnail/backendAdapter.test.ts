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
  const binaryHarness = (read: () => Promise<ArrayBuffer>) => {
    let completion: ((event: ThumbnailCompletionEvent) => void) | undefined;
    const transport = {
      on: vi.fn(async (_event, handler) => { completion=handler; return () => undefined; }),
      thumbnailRequest: vi.fn(async () => ({ok:true,data:{requestId:"binary-request",key:{kind:"galleryCover",galleryId:4_051_038}}})),
      thumbnailRead: vi.fn(read), thumbnailRelease:vi.fn(async()=>({ok:true,data:true})),
      thumbnailCancel:vi.fn(async()=>({ok:true,data:true})),
    } as unknown as BackendClient;
    const adapter=new BackendThumbnailAdapter(transport);
    const event=readyEvent("binary-request");
    if (event.outcome.status!=="ready") throw new Error("fixture");
    event.outcome.delivery.thumbnail={contentType:"image/webp",resourceToken:"one-read-capability",byteLength:6,width:2,height:2};
    return {transport,adapter,event,emit:()=>completion?.(event)};
  };
  it("reads native bodies as binary, acknowledges the lease and owns only a Blob URL", async () => {
    Object.defineProperty(URL,"createObjectURL",{configurable:true,value:vi.fn(()=>"blob:binary")});
    Object.defineProperty(URL,"revokeObjectURL",{configurable:true,value:vi.fn()});
    const h=binaryHarness(async()=>new Uint8Array(6).buffer);
    const resolution=h.adapter.resolve(request); await vi.waitFor(()=>expect(h.transport.thumbnailRequest).toHaveBeenCalledOnce());
    h.emit(); const asset=await resolution;
    expect(h.transport.thumbnailRead).toHaveBeenCalledWith("one-read-capability");
    expect(h.transport.thumbnailRelease).toHaveBeenCalledWith("one-read-capability");
    expect(asset).toMatchObject({kind:"image",byteLength:6});
    expect(JSON.stringify(h.event)).not.toContain('"bytes"');
    h.adapter.release(request,asset); h.adapter.dispose();
  });
  it("does not resurrect an image cancelled while its binary response was in flight",async()=>{
    Object.defineProperty(URL,"createObjectURL",{configurable:true,value:vi.fn(()=>"blob:late")});
    let resolveBody!: (body:ArrayBuffer)=>void;
    const h=binaryHarness(()=>new Promise((resolve)=>{resolveBody=resolve;}));
    const resolution=h.adapter.resolve(request); const rejected=expect(resolution).rejects.toThrow(/cancelled/);
    await vi.waitFor(()=>expect(h.transport.thumbnailRequest).toHaveBeenCalledOnce()); h.emit();
    expect(h.transport.thumbnailRead).toHaveBeenCalledOnce(); h.adapter.cancel(request); resolveBody(new ArrayBuffer(6));
    await rejected; await vi.waitFor(()=>expect(h.transport.thumbnailRelease).toHaveBeenCalledOnce());
    expect(URL.createObjectURL).not.toHaveBeenCalled(); h.adapter.dispose();
  });
  it("rejects truncated binary bodies and releases unused late capabilities",async()=>{
    const h=binaryHarness(async()=>new ArrayBuffer(2)); const resolution=h.adapter.resolve(request);
    const rejected=expect(resolution).rejects.toMatchObject({name:"THUMBNAIL_INVALID_BODY"});
    await vi.waitFor(()=>expect(h.transport.thumbnailRequest).toHaveBeenCalledOnce()); h.emit(); await rejected;
    expect(h.transport.thumbnailRelease).toHaveBeenCalledOnce(); h.emit();
    expect(h.transport.thumbnailRelease).toHaveBeenCalledTimes(2); h.adapter.dispose();
  });
  it("keeps the saved review, candidate, revision and side in the image transport", async () => {
    Object.defineProperty(URL, "createObjectURL", { configurable: true, value: vi.fn(() => "blob:review") });
    Object.defineProperty(URL, "revokeObjectURL", { configurable: true, value: vi.fn() });
    const submitted = vi.spyOn(backend, "thumbnailRequest");
    const adapter = new BackendThumbnailAdapter(backend);
    const reviewRequest: ThumbnailRequest = {
      key: { kind: "overlap-review-page", reviewId: "review-101", candidateId: "candidate-9", reviewRevision: 4, side: "existing", page: 11 },
      consumer: "review", priority: "critical",
    };
    const asset = await adapter.resolve(reviewRequest);
    expect(asset.kind).toBe("image");
    expect(submitted).toHaveBeenCalledWith({
      key: { kind: "overlapReviewPage", reviewId: "review-101", candidateId: "candidate-9", reviewRevision: 4, side: "existing", sourcePage: 11 },
      consumer: "review", priority: "critical",
    });
    adapter.release(reviewRequest, asset);
    adapter.dispose();
  });
  it("turns one backend completion into a revocable display URL", async () => {
    const createObjectURL = vi.fn((_blob: Blob) => "blob:https://atsumi.local/thumbnail-1");
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
      byteLength: expect.any(Number),
    });
    expect(createObjectURL).toHaveBeenCalledOnce();
    expect(asset.kind === "image" ? asset.byteLength : undefined)
      .toBe((createObjectURL.mock.calls[0]?.[0] as Blob | undefined)?.size);

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
    expect((adapter as unknown as { pendingByRequestId: Map<string, unknown> }).pendingByRequestId.size).toBe(0);
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

  it("keeps the expected early completion but does not retain unsolicited byte arrays", async () => {
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
    expect(adapter).not.toHaveProperty("bufferedCompletions");
    expect((adapter as unknown as { pendingByRequestId: Map<string, unknown> }).pendingByRequestId.size).toBe(0);
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
