import { afterEach, describe, expect, it, vi } from "vitest";
import { galleryId, type Gallery } from "../core/types";
import { ThumbnailClient, type ThumbnailAsset } from "./client";
import { GalleryCoverBackgroundPreloader } from "./backgroundCoverPreloader";
import { galleryCoverThumbnailKey, thumbnailKeyIdentity, type ThumbnailRequest } from "./model";
import { GalleryCoverSessionRetainer } from "./sessionCoverRetainer";

const gallery = (id: number): Gallery => ({
  id: galleryId(id), title: `Gallery ${id}`, subtitle: "", artist: "Artist", pages: 20,
  score: 0, publishedAt: "2026-09-07", coverIndex: 0, language: "korean", tags: [], series: [], characters: [],
  download: { entryId: `entry-${id}`, state: "completed", progress: 100 },
});

const asset: ThumbnailAsset = { kind: "image", url: "blob:https://atsumi.test/cover", width: 400, height: 600, byteLength: 60 };

afterEach(() => vi.useRealTimers());

describe("GalleryCoverBackgroundPreloader", () => {
  it("reconciles changed representative pages without restarting unrelated pending covers", async () => {
    vi.useFakeTimers();
    const resolvers = new Map<string, (asset: ThumbnailAsset) => void>();
    const resolve = vi.fn((request: ThumbnailRequest) => new Promise<ThumbnailAsset>((done) => {
      resolvers.set(thumbnailKeyIdentity(request.key), done);
    }));
    const cancel = vi.fn();
    const client = new ThumbnailClient({ resolve, cancel });
    const subscribe = vi.spyOn(client, "subscribe");
    const loader = new GalleryCoverBackgroundPreloader(client, undefined, 2);
    const items = [gallery(1), gallery(2), gallery(3), gallery(4)];
    const identities = () => resolve.mock.calls.map(([request]) => thumbnailKeyIdentity(request.key));

    try {
      loader.update(items);
      expect(identities()).toEqual(["gallery-cover:1", "gallery-cover:2"]);
      loader.update(items.map((item) => ({ ...item, tags: ["new_tag"], title: "Resolved title" })));
      expect(subscribe).toHaveBeenCalledTimes(2);

      const selected = {
        ...items[0]!,
        representativePreview: {
          galleryId: galleryId(1), mode: "manual" as const, sourcePage: 7, manualSourcePage: 7,
          entryId: "entry-1", width: 400, height: 600, candidates: [7], algorithmVersion: 1, updatedAt: "2026-09-07T10:00:00Z",
        },
      };
      loader.update([selected, ...items.slice(1)]);
      expect(identities()).toEqual(["gallery-cover:1", "gallery-cover:2", "artifact-page:entry-1:7"]);
      await vi.advanceTimersByTimeAsync(400);
      expect(cancel.mock.calls.map(([request]) => thumbnailKeyIdentity(request.key))).toEqual(["gallery-cover:1"]);

      resolvers.get("gallery-cover:1")?.(asset);
      await Promise.resolve();
      await Promise.resolve();
      expect(resolve).toHaveBeenCalledTimes(3);
      resolvers.get("gallery-cover:2")?.(asset);
      await Promise.resolve();
      await Promise.resolve();
      expect(identities()).toEqual(["gallery-cover:1", "gallery-cover:2", "artifact-page:entry-1:7", "gallery-cover:3"]);
      expect(subscribe.mock.calls.filter(([request]) => thumbnailKeyIdentity(request.key) === "gallery-cover:2")).toHaveLength(1);
    } finally {
      loader.dispose();
      client.dispose();
    }
  });

  it("transfers completed covers into the shared byte budget and releases its own subscriptions", () => {
    const resolve = vi.fn((request: ThumbnailRequest) => {
      if (request.key.kind === "gallery-cover" && Number(request.key.galleryId) === 3) throw new Error("missing cover");
      return asset;
    });
    const client = new ThumbnailClient({ resolve });
    const retainer = new GalleryCoverSessionRetainer(client, 10, 100);
    const loader = new GalleryCoverBackgroundPreloader(client, retainer);
    const items = [gallery(1), gallery(2), gallery(3), gallery(4)];

    try {
      loader.update(items);
      expect(resolve).toHaveBeenCalledTimes(4);
      expect(retainer.size).toBe(1);
      expect(retainer.retainedBytes).toBe(60);
      // The loader has no completed subscriptions left to defeat LRU eviction.
      expect(client.clearRetainedCache()).toBe(2);
      expect(client.getSnapshot(galleryCoverThumbnailKey(items[0]!)).status).toBe("idle");
      expect(client.getSnapshot(galleryCoverThumbnailKey(items[3]!)).status).toBe("resolved");
      loader.update(items.map((item) => ({ ...item, tags: ["metadata_update"] })));
      expect(resolve).toHaveBeenCalledTimes(4);
    } finally {
      loader.dispose();
      retainer.clear();
      client.dispose();
    }
  });

  it("coalesces an already-visible cover and stops the queue after disposal", async () => {
    vi.useFakeTimers();
    const resolvers = new Map<string, (asset: ThumbnailAsset) => void>();
    const resolve = vi.fn((request: ThumbnailRequest) => new Promise<ThumbnailAsset>((done) => {
      resolvers.set(thumbnailKeyIdentity(request.key), done);
    }));
    const cancel = vi.fn();
    const client = new ThumbnailClient({ resolve, cancel });
    const items = [gallery(1), gallery(2), gallery(3)];
    const releaseVisible = client.subscribe({ key: galleryCoverThumbnailKey(items[0]!), consumer: "downloads", priority: "visible" }, () => undefined);
    const loader = new GalleryCoverBackgroundPreloader(client, undefined, 2);

    try {
      loader.update(items);
      expect(resolve).toHaveBeenCalledTimes(2);
      loader.dispose();
      await vi.advanceTimersByTimeAsync(400);
      expect(cancel.mock.calls.map(([request]) => thumbnailKeyIdentity(request.key))).toEqual(["gallery-cover:2"]);
      resolvers.get("gallery-cover:1")?.(asset);
      resolvers.get("gallery-cover:2")?.(asset);
      await Promise.resolve();
      await Promise.resolve();
      loader.update(items);
      expect(resolve).toHaveBeenCalledTimes(2);
    } finally {
      releaseVisible();
      loader.dispose();
      client.dispose();
    }
  });
});
