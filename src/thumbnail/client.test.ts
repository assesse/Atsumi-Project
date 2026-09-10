import { describe, expect, it, vi } from "vitest";
import { galleryId } from "../core/types";
import { ThumbnailClient, type ThumbnailAsset } from "./client";
import { type ThumbnailRequest } from "./model";

const coverKey = {
  kind: "gallery-cover" as const,
  galleryId: galleryId(4051038),
  sourceKey: "opaque-cover-key",
  fallback: { kind: "fixture-sheet-cell" as const, index: 0 },
};

describe("ThumbnailClient", () => {
  it("invalidates an active display handle and refreshes all existing listeners", async () => {
    const oldAsset: ThumbnailAsset = { kind: "image", url: "blob:old-cover", width: 100, height: 150 };
    const freshAsset: ThumbnailAsset = { kind: "image", url: "blob:fresh-cover", width: 100, height: 150 };
    let finish: ((asset: ThumbnailAsset) => void) | undefined;
    const pending = new Promise<ThumbnailAsset>((resolve) => { finish = resolve; });
    const resolve = vi.fn().mockReturnValueOnce(oldAsset).mockReturnValueOnce(pending);
    const release = vi.fn();
    const client = new ThumbnailClient({ resolve, release });
    const request: ThumbnailRequest = { key: coverKey, consumer: "downloads", priority: "visible" };
    const first = vi.fn();
    const second = vi.fn();
    const unsubscribeFirst = client.subscribe(request, first);
    const unsubscribeSecond = client.subscribe(request, second);
    first.mockClear();

    expect(client.invalidate((key) => key.kind === "gallery-cover" && key.galleryId === coverKey.galleryId)).toBe(1);
    expect(client.getSnapshot(coverKey)).toEqual({ status: "loading" });
    expect(release).toHaveBeenCalledExactlyOnceWith(request, oldAsset);
    expect(resolve).toHaveBeenCalledTimes(2);
    expect(first).toHaveBeenCalledOnce();
    expect(second).toHaveBeenCalledOnce();

    finish?.(freshAsset);
    await pending;
    await Promise.resolve();
    expect(client.getSnapshot(coverKey)).toEqual({ status: "resolved", asset: freshAsset });
    expect(first).toHaveBeenCalledTimes(2);
    expect(second).toHaveBeenCalledTimes(2);
    unsubscribeFirst();
    unsubscribeSecond();
    client.dispose();
  });

  it("invalidates only matching inactive keys without evicting unrelated retained albums", async () => {
    vi.useFakeTimers();
    try {
      const asset: ThumbnailAsset = { kind: "image", url: "blob:retained", width: 100, height: 150 };
      const resolve = vi.fn(() => asset);
      const release = vi.fn();
      const client = new ThumbnailClient({ resolve, release });
      const request: ThumbnailRequest = { key: coverKey, consumer: "downloads", priority: "visible" };
      const artifactRequest: ThumbnailRequest = {
        ...request, key: { kind: "artifact-page", entryId: "merged-entry", page: 1 },
      };
      const unrelated: ThumbnailRequest = { ...request, key: { ...coverKey, galleryId: galleryId(9) } };
      client.subscribe(request, vi.fn())();
      client.subscribe(unrelated, vi.fn())();
      await vi.advanceTimersByTimeAsync(400);
      // Also invalidate an inactive handle still inside its orphan grace period.
      client.subscribe(artifactRequest, vi.fn())();

      const predicate = (key: ThumbnailRequest["key"]) => key.kind === "artifact-page"
        ? key.entryId === "merged-entry"
        : key.galleryId === coverKey.galleryId;
      expect(client.invalidate(predicate)).toBe(2);
      expect(client.invalidate(predicate)).toBe(0);
      expect(client.getSnapshot(coverKey)).toEqual({ status: "idle" });
      expect(client.getSnapshot(artifactRequest.key)).toEqual({ status: "idle" });
      expect(client.getSnapshot(unrelated.key)).toEqual({ status: "resolved", asset });
      expect(release).toHaveBeenCalledTimes(2);
      expect(release).not.toHaveBeenCalledWith(unrelated, expect.anything());
      const unsubscribe = client.subscribe(unrelated, vi.fn());
      expect(resolve).toHaveBeenCalledTimes(3);
      unsubscribe();
      client.dispose();
    } finally {
      vi.useRealTimers();
    }
  });

  it("cancels invalidated in-flight work and releases a late old image without replacing the fresh image", async () => {
    let finishOld: ((asset: ThumbnailAsset) => void) | undefined;
    let finishFresh: ((asset: ThumbnailAsset) => void) | undefined;
    const oldPending = new Promise<ThumbnailAsset>((resolve) => { finishOld = resolve; });
    const freshPending = new Promise<ThumbnailAsset>((resolve) => { finishFresh = resolve; });
    const resolve = vi.fn().mockReturnValueOnce(oldPending).mockReturnValueOnce(freshPending);
    const cancel = vi.fn();
    const release = vi.fn();
    const client = new ThumbnailClient({ resolve, cancel, release });
    const request: ThumbnailRequest = {
      key: { kind: "source-page", galleryId: coverKey.galleryId, page: 1 },
      consumer: "detail", priority: "critical",
    };
    const listener = vi.fn();
    const unsubscribe = client.subscribe(request, listener);

    expect(client.invalidate((key) => key.kind === "source-page" && key.galleryId === coverKey.galleryId)).toBe(1);
    expect(cancel).toHaveBeenCalledExactlyOnceWith(request);
    expect(resolve).toHaveBeenCalledTimes(2);
    const freshAsset: ThumbnailAsset = { kind: "image", url: "blob:fresh-page", width: 100, height: 150 };
    finishFresh?.(freshAsset);
    await freshPending;
    await Promise.resolve();
    listener.mockClear();

    const oldAsset: ThumbnailAsset = { kind: "image", url: "blob:late-old-page", width: 100, height: 150 };
    finishOld?.(oldAsset);
    await oldPending;
    await Promise.resolve();
    expect(client.getSnapshot(request.key)).toEqual({ status: "resolved", asset: freshAsset });
    expect(release).toHaveBeenCalledExactlyOnceWith(request, oldAsset);
    expect(listener).not.toHaveBeenCalled();
    unsubscribe();
    client.dispose();
  });

  it("resets display retry counters and clears old retry timers when invalidating", async () => {
    vi.useFakeTimers();
    try {
      const asset: ThumbnailAsset = { kind: "image", url: "blob:retry-display", width: 100, height: 150 };
      const resolve = vi.fn(() => asset);
      const client = new ThumbnailClient({ resolve });
      const request: ThumbnailRequest = { key: coverKey, consumer: "downloads", priority: "visible" };
      const unsubscribe = client.subscribe(request, vi.fn());
      client.reportDisplayFailure(request, "old asset decode failed");
      await vi.advanceTimersByTimeAsync(1_000);

      expect(client.invalidate(() => true)).toBe(1);
      expect(resolve).toHaveBeenCalledTimes(2);
      await vi.advanceTimersByTimeAsync(2_000);
      expect(resolve).toHaveBeenCalledTimes(2);
      client.reportDisplayFailure(request, "new asset needs one fresh retry");
      await vi.advanceTimersByTimeAsync(3_000);
      expect(resolve).toHaveBeenCalledTimes(3);
      expect(client.getSnapshot(coverKey)).toEqual({ status: "resolved", asset });
      await vi.advanceTimersByTimeAsync(6_000);
      expect(resolve).toHaveBeenCalledTimes(3);
      unsubscribe();
      client.dispose();
    } finally {
      vi.useRealTimers();
    }
  });

  it("coalesces consumers for one structured key and promotes the shared work", async () => {
    let finish: ((asset: ThumbnailAsset) => void) | undefined;
    const pending = new Promise<ThumbnailAsset>((resolve) => { finish = resolve; });
    const adapter = {
      resolve: vi.fn(() => pending),
      reprioritize: vi.fn(),
    };
    const client = new ThumbnailClient(adapter);
    const first = vi.fn();
    const second = vi.fn();
    const exploreRequest: ThumbnailRequest = {
      key: coverKey,
      consumer: "explore",
      priority: "prefetch",
    };
    const reviewRequest: ThumbnailRequest = {
      key: { ...coverKey, sourceKey: "newer-projection-hint" },
      consumer: "review",
      priority: "critical",
    };

    const unsubscribeFirst = client.subscribe(exploreRequest, first);
    const unsubscribeSecond = client.subscribe(reviewRequest, second);

    expect(adapter.resolve).toHaveBeenCalledTimes(1);
    expect(adapter.resolve).toHaveBeenCalledWith(exploreRequest);
    expect(adapter.reprioritize).toHaveBeenCalledOnce();
    expect(adapter.reprioritize).toHaveBeenCalledWith(reviewRequest);
    expect(client.getSnapshot(coverKey)).toEqual({ status: "loading" });

    finish?.({
      kind: "image",
      url: "https://images.example.test/cover.jpg",
      width: 720,
      height: 1080,
    });
    await pending;
    await Promise.resolve();

    expect(client.getSnapshot(coverKey)).toEqual({
      status: "resolved",
      asset: {
        kind: "image",
        url: "https://images.example.test/cover.jpg",
        width: 720,
        height: 1080,
      },
    });
    expect(first).toHaveBeenCalled();
    expect(second).toHaveBeenCalled();
    unsubscribeFirst();
    unsubscribeSecond();
    client.dispose();
  });

  it("gives orphaned loading work a 400ms grace period, then cancels and releases a late handle", async () => {
    vi.useFakeTimers();
    try {
    let finish: ((asset: ThumbnailAsset) => void) | undefined;
    const pending = new Promise<ThumbnailAsset>((resolve) => { finish = resolve; });
    const cancel = vi.fn();
    const release = vi.fn();
    const client = new ThumbnailClient({ resolve: () => pending, cancel, release });
    const request: ThumbnailRequest = { key: coverKey, consumer: "explore", priority: "prefetch" };

    const unsubscribe = client.subscribe(request, vi.fn());
    unsubscribe();
    await vi.advanceTimersByTimeAsync(399);

    expect(cancel).not.toHaveBeenCalled();
    expect(client.getSnapshot(coverKey).status).toBe("loading");
    await vi.advanceTimersByTimeAsync(1);

    expect(cancel).toHaveBeenCalledOnce();
    expect(cancel).toHaveBeenCalledWith(request);
    expect(client.getSnapshot(coverKey)).toEqual({ status: "idle" });

    const lateAsset: ThumbnailAsset = {
      kind: "image",
      url: "blob:https://app.local/late-thumbnail",
      width: 512,
      height: 512,
    };
    finish?.(lateAsset);
    await pending;
    await Promise.resolve();

    expect(release).toHaveBeenCalledWith(request, lateAsset);
    expect(client.getSnapshot(coverKey)).toEqual({ status: "idle" });
    } finally {
      vi.useRealTimers();
    }
  });

  it("reuses in-flight work when a subscriber returns during the orphan grace period", async () => {
    vi.useFakeTimers();
    try {
      let finish: ((asset: ThumbnailAsset) => void) | undefined;
      const pending = new Promise<ThumbnailAsset>((resolve) => { finish = resolve; });
      const resolve = vi.fn(() => pending);
      const cancel = vi.fn();
      const client = new ThumbnailClient({ resolve, cancel });
      const request: ThumbnailRequest = { key: coverKey, consumer: "explore", priority: "prefetch" };

      const unsubscribe = client.subscribe(request, vi.fn());
      unsubscribe();
      await vi.advanceTimersByTimeAsync(399);
      const returningUnsubscribe = client.subscribe({ ...request, priority: "visible" }, vi.fn());
      await vi.advanceTimersByTimeAsync(1);
      expect(resolve).toHaveBeenCalledOnce();
      expect(cancel).not.toHaveBeenCalled();

      finish?.({ kind: "missing", reason: "returned during grace" });
      await pending;
      await Promise.resolve();
      expect(client.getSnapshot(coverKey)).toEqual({
        status: "resolved",
        asset: { kind: "missing", reason: "returned during grace" },
      });
      returningUnsubscribe();
      client.dispose();
    } finally {
      vi.useRealTimers();
    }
  });

  it("retains a resolved display asset for a returning subscriber, then releases it after its TTL", async () => {
    vi.useFakeTimers();
    try {
    const asset: ThumbnailAsset = {
      kind: "image",
      url: "blob:https://app.local/thumbnail",
      width: 512,
      height: 512,
    };
    const release = vi.fn();
    const resolve = vi.fn(() => asset);
    const client = new ThumbnailClient({ resolve, release });
    const request: ThumbnailRequest = { key: coverKey, consumer: "detail", priority: "critical" };

    const unsubscribe = client.subscribe(request, vi.fn());
    expect(client.getSnapshot(coverKey)).toEqual({ status: "resolved", asset });
    unsubscribe();
    await vi.advanceTimersByTimeAsync(400);

    expect(release).not.toHaveBeenCalled();
    const returningListener = vi.fn();
    const unsubscribeReturning = client.subscribe(request, returningListener);
    expect(client.getSnapshot(coverKey)).toEqual({ status: "resolved", asset });
    expect(resolve).toHaveBeenCalledOnce();
    unsubscribeReturning();
    await vi.advanceTimersByTimeAsync(400);
    await vi.advanceTimersByTimeAsync(120_000);

    expect(release).toHaveBeenCalledWith(request, asset);
    expect(client.getSnapshot(coverKey)).toEqual({ status: "idle" });
    } finally {
      vi.useRealTimers();
    }
  });

  it("evicts only retained assets when the bounded display cache is full", async () => {
    vi.useFakeTimers();
    try {
      const release = vi.fn();
      const client = new ThumbnailClient({
        resolve: ({ key }) => ({
          kind: "image" as const,
          url: `blob:https://app.local/${key.kind === "gallery-cover" ? key.galleryId : "other"}`,
          width: 20,
          height: 30,
        }),
        release,
      });
      const activeRequest: ThumbnailRequest = {
        key: { ...coverKey, galleryId: galleryId(9_999_999) },
        consumer: "detail",
        priority: "critical",
      };
      const activeUnsubscribe = client.subscribe(activeRequest, vi.fn());
      for (let index = 0; index < 257; index += 1) {
        const request: ThumbnailRequest = {
          key: { ...coverKey, galleryId: galleryId(index + 1) },
          consumer: "explore",
          priority: "prefetch",
        };
        const unsubscribe = client.subscribe(request, vi.fn());
        unsubscribe();
      }
      await vi.advanceTimersByTimeAsync(400);

      expect(release).toHaveBeenCalledTimes(1);
      expect(client.getSnapshot(activeRequest.key)).toMatchObject({ status: "resolved" });
      activeUnsubscribe();
      client.dispose();
    } finally {
      vi.useRealTimers();
    }
  });

  it("turns malformed adapter output into a shared error state", () => {
    const client = new ThumbnailClient({
      resolve: () => ({ kind: "image", url: "", width: 0, height: 0 }),
    });

    client.subscribe({ key: coverKey, consumer: "downloads", priority: "visible" }, vi.fn());

    expect(client.getSnapshot(coverKey)).toEqual({
      status: "error",
      message: "Thumbnail adapter returned an empty image URL",
    });
  });

  it("retries a transient backend failure after the negative-cache TTL and recovers", async () => {
    vi.useFakeTimers();
    try {
      const transient = new Error("fixture outage");
      transient.name = "THUMBNAIL_temporarilyUnavailable";
      const asset: ThumbnailAsset = {
        kind: "image",
        url: "blob:https://app.local/recovered-thumbnail",
        width: 512,
        height: 512,
      };
      const resolve = vi.fn()
        .mockRejectedValueOnce(transient)
        .mockResolvedValueOnce(asset);
      const client = new ThumbnailClient({ resolve });
      const request: ThumbnailRequest = { key: coverKey, consumer: "explore", priority: "prefetch" };
      const unsubscribe = client.subscribe(request, vi.fn());

      await Promise.resolve();
      expect(client.getSnapshot(coverKey)).toEqual({
        status: "error",
        message: "fixture outage",
        code: "THUMBNAIL_temporarilyUnavailable",
      });
      expect(resolve).toHaveBeenCalledTimes(1);

      await vi.advanceTimersByTimeAsync(2_999);
      expect(resolve).toHaveBeenCalledTimes(1);
      await vi.advanceTimersByTimeAsync(1);

      expect(resolve).toHaveBeenCalledTimes(2);
      expect(client.getSnapshot(coverKey)).toEqual({ status: "resolved", asset });
      unsubscribe();
      await vi.advanceTimersByTimeAsync(400);
    } finally {
      vi.useRealTimers();
    }
  });

  it("cancels a scheduled retry when the final subscriber leaves", async () => {
    vi.useFakeTimers();
    try {
      const transient = new Error("temporary resolver failure");
      transient.name = "THUMBNAIL_resolver";
      const resolve = vi.fn().mockRejectedValue(transient);
      const cancel = vi.fn();
      const client = new ThumbnailClient({ resolve, cancel });
      const request: ThumbnailRequest = { key: coverKey, consumer: "downloads", priority: "visible" };

      const unsubscribe = client.subscribe(request, vi.fn());
      await Promise.resolve();
      expect(resolve).toHaveBeenCalledTimes(1);
      unsubscribe();
      await vi.advanceTimersByTimeAsync(400);
      await vi.advanceTimersByTimeAsync(3_000);

      expect(resolve).toHaveBeenCalledTimes(1);
      expect(cancel).not.toHaveBeenCalled();
      expect(client.getSnapshot(coverKey)).toEqual({ status: "idle" });
    } finally {
      vi.useRealTimers();
    }
  });

  it("cancels an in-flight retry and releases its late asset after unsubscribe", async () => {
    vi.useFakeTimers();
    try {
      const transient = new Error("temporary worker failure");
      transient.name = "THUMBNAIL_WORKER_UNAVAILABLE";
      let finishRetry: ((asset: ThumbnailAsset) => void) | undefined;
      const pendingRetry = new Promise<ThumbnailAsset>((resolve) => { finishRetry = resolve; });
      const resolve = vi.fn()
        .mockRejectedValueOnce(transient)
        .mockImplementationOnce(() => pendingRetry);
      const cancel = vi.fn();
      const release = vi.fn();
      const client = new ThumbnailClient({ resolve, cancel, release });
      const request: ThumbnailRequest = { key: coverKey, consumer: "explore", priority: "prefetch" };
      const unsubscribe = client.subscribe(request, vi.fn());

      await Promise.resolve();
      await vi.advanceTimersByTimeAsync(3_000);
      expect(resolve).toHaveBeenCalledTimes(2);
      expect(client.getSnapshot(coverKey)).toEqual({ status: "loading" });

      unsubscribe();
      await vi.advanceTimersByTimeAsync(400);
      expect(cancel).toHaveBeenCalledOnce();
      expect(cancel).toHaveBeenCalledWith(request);

      const lateAsset: ThumbnailAsset = {
        kind: "image",
        url: "blob:https://app.local/late-retry-thumbnail",
        width: 512,
        height: 768,
      };
      finishRetry?.(lateAsset);
      await pendingRetry;
      await Promise.resolve();

      expect(release).toHaveBeenCalledOnce();
      expect(release).toHaveBeenCalledWith(request, lateAsset);
      expect(client.getSnapshot(coverKey)).toEqual({ status: "idle" });
      await vi.advanceTimersByTimeAsync(6_000);
      expect(resolve).toHaveBeenCalledTimes(2);
    } finally {
      vi.useRealTimers();
    }
  });

  it("lets one new foreground subscriber accelerate a transient error without duplicate timers", async () => {
    vi.useFakeTimers();
    try {
      const transient = new Error("temporary coordinator outage");
      transient.name = "THUMBNAIL_coordinatorClosed";
      const asset: ThumbnailAsset = {
        kind: "image",
        url: "blob:https://app.local/foreground-recovery",
        width: 320,
        height: 480,
      };
      const resolve = vi.fn()
        .mockRejectedValueOnce(transient)
        .mockResolvedValueOnce(asset);
      const client = new ThumbnailClient({ resolve, reprioritize: vi.fn() });
      const prefetch: ThumbnailRequest = { key: coverKey, consumer: "explore", priority: "prefetch" };
      const critical: ThumbnailRequest = { key: coverKey, consumer: "detail", priority: "critical" };

      const unsubscribePrefetch = client.subscribe(prefetch, vi.fn());
      await Promise.resolve();
      expect(client.getSnapshot(coverKey).status).toBe("error");

      const unsubscribeCritical = client.subscribe(critical, vi.fn());
      await Promise.resolve();
      expect(resolve).toHaveBeenCalledTimes(2);
      expect(client.getSnapshot(coverKey)).toEqual({ status: "resolved", asset });

      await vi.advanceTimersByTimeAsync(3_000);
      expect(resolve).toHaveBeenCalledTimes(2);
      unsubscribeCritical();
      unsubscribePrefetch();
      await vi.advanceTimersByTimeAsync(400);
    } finally {
      vi.useRealTimers();
    }
  });

  it("does not automatically retry permanent thumbnail failures", async () => {
    vi.useFakeTimers();
    try {
      const permanent = new Error("source thumbnail does not exist");
      permanent.name = "THUMBNAIL_notFound";
      const resolve = vi.fn().mockRejectedValue(permanent);
      const client = new ThumbnailClient({ resolve });
      const prefetch: ThumbnailRequest = { key: coverKey, consumer: "explore", priority: "prefetch" };
      const critical: ThumbnailRequest = { key: coverKey, consumer: "review", priority: "critical" };

      const unsubscribePrefetch = client.subscribe(prefetch, vi.fn());
      await Promise.resolve();
      const unsubscribeCritical = client.subscribe(critical, vi.fn());
      await vi.advanceTimersByTimeAsync(30_000);

      expect(resolve).toHaveBeenCalledTimes(1);
      expect(client.getSnapshot(coverKey)).toEqual({
        status: "error",
        message: "source thumbnail does not exist",
        code: "THUMBNAIL_notFound",
      });
      unsubscribeCritical();
      unsubscribePrefetch();
      await vi.advanceTimersByTimeAsync(400);
    } finally {
      vi.useRealTimers();
    }
  });

  it("retries one decoded-display failure after invalidation without looping forever", async () => {
    vi.useFakeTimers();
    try {
      const assets: ThumbnailAsset[] = [
        { kind: "image", url: "blob:broken-first", width: 512, height: 512 },
        { kind: "image", url: "blob:broken-second", width: 512, height: 512 },
      ];
      const resolve = vi.fn(() => assets.shift()!);
      const displayFailed = vi.fn();
      const release = vi.fn();
      const client = new ThumbnailClient({ resolve, displayFailed, release });
      const request: ThumbnailRequest = { key: coverKey, consumer: "detail", priority: "critical" };
      const unsubscribe = client.subscribe(request, vi.fn());

      client.reportDisplayFailure(request, "first decode failed");
      expect(displayFailed).toHaveBeenCalledWith(request, "first decode failed");
      expect(client.getSnapshot(coverKey)).toEqual({
        status: "error",
        message: "first decode failed",
        code: "THUMBNAIL_decodeFailed",
      });

      await vi.advanceTimersByTimeAsync(3_000);
      expect(resolve).toHaveBeenCalledTimes(2);
      expect(client.getSnapshot(coverKey)).toMatchObject({
        status: "resolved",
        asset: { url: "blob:broken-second" },
      });

      client.reportDisplayFailure(request, "second decode failed");
      await vi.advanceTimersByTimeAsync(3_000);
      expect(resolve).toHaveBeenCalledTimes(2);
      expect(client.getSnapshot(coverKey)).toEqual({
        status: "error",
        message: "second decode failed",
        code: "THUMBNAIL_decodeFailed",
      });

      unsubscribe();
      await vi.advanceTimersByTimeAsync(400);
    } finally {
      vi.useRealTimers();
    }
  });

  it("releases source-page Blob URLs after the orphan grace instead of retaining them", async () => {
    vi.useFakeTimers();
    try {
      const release = vi.fn();
      const cancel = vi.fn();
      const client = new ThumbnailClient({
        resolve: () => ({ kind: "image", url: "blob:detail-page", width: 1200, height: 1800 }),
        release,
        cancel,
      });
      const pageKey = { kind: "source-page" as const, galleryId: galleryId(4051038), page: 1 };
      const request: ThumbnailRequest = { key: pageKey, consumer: "detail", priority: "visible" };
      const unsubscribe = client.subscribe(request, vi.fn());
      await Promise.resolve();
      unsubscribe();
      await vi.advanceTimersByTimeAsync(400);
      expect(release).toHaveBeenCalledWith(request, expect.objectContaining({ url: "blob:detail-page" }));
      expect(client.getSnapshot(pageKey)).toEqual({ status: "idle" });
      expect(cancel).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });
});
