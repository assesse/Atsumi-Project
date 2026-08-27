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
