import { describe, expect, it, vi } from "vitest";
import { galleryId, type Gallery } from "../core/types";
import { ThumbnailClient, type ThumbnailAsset } from "./client";
import { galleryCoverThumbnailKey, type ThumbnailRequest } from "./model";
import { galleryCoverPageSignature, prefetchNextGalleryPageAfterCurrent } from "./pagePrefetch";

const gallery = (value: number): Gallery => ({
  id: galleryId(value),
  title: `Gallery ${value}`,
  subtitle: "",
  artist: "Artist",
  pages: 1,
  score: 0,
  publishedAt: "2026-09-07",
  coverIndex: value % 6,
  language: "korean",
  tags: [],
  series: [],
  characters: [],
});

const asset = (value: number): ThumbnailAsset => ({
  kind: "image",
  url: `https://images.example.test/${value}.jpg`,
  width: 512,
  height: 768,
});

const requestGalleryId = (request: ThumbnailRequest): number => (
  request.key.kind === "gallery-cover" ? Number(request.key.galleryId) : -1
);

describe("prefetchNextGalleryPageAfterCurrent", () => {
  it("uses the saved local representative page and invalidates prefetch identity when it changes", () => {
    const original = gallery(91);
    const selected: Gallery = {
      ...original,
      download: { entryId: "entry-91", state: "completed", progress: 100 },
      representativePreview: {
        galleryId: original.id, mode: "manual", sourcePage: 3, manualSourcePage: 3,
        entryId: "entry-91", width: 800, height: 1200, candidates: [2, 3],
        algorithmVersion: 1, updatedAt: "2026-09-07T10:00:00Z",
      },
    };
    expect(galleryCoverThumbnailKey(selected)).toMatchObject({ kind: "artifact-page", entryId: "entry-91", page: 3 });
    expect(galleryCoverPageSignature([selected])).not.toBe(galleryCoverPageSignature([original]));
    expect(galleryCoverPageSignature([{ ...selected, representativePreview: { ...selected.representativePreview!, sourcePage: 4 } }]))
      .not.toBe(galleryCoverPageSignature([selected]));
    expect(galleryCoverThumbnailKey({ ...selected, download: { ...selected.download!, state: "quarantined" } })).toMatchObject({ kind: "gallery-cover" });
    expect(galleryCoverThumbnailKey({ ...selected, download: { ...selected.download!, entryId: "replacement" } })).toMatchObject({ kind: "gallery-cover" });
  });

  it("keeps the cover signature stable across progress updates and changes it for cover identity", () => {
    const original = gallery(91);
    const progressUpdate: Gallery = {
      ...original,
      download: { entryId: "entry-91", state: "downloading", progress: 73 },
    };
    expect(galleryCoverPageSignature([progressUpdate])).toBe(galleryCoverPageSignature([original]));
    expect(galleryCoverPageSignature([{ ...original, thumbnailKey: "new-source" }]))
      .not.toBe(galleryCoverPageSignature([original]));
    expect(galleryCoverPageSignature([{ ...original, coverIndex: original.coverIndex + 1 }]))
      .not.toBe(galleryCoverPageSignature([original]));
    expect(galleryCoverPageSignature([gallery(92), original]))
      .not.toBe(galleryCoverPageSignature([original, gallery(92)]));
  });

  it("waits for every current cover before starting the next page at prefetch priority", async () => {
    const resolvers = new Map<number, (resolved: ThumbnailAsset) => void>();
    const resolve = vi.fn((request: ThumbnailRequest) => new Promise<ThumbnailAsset>((done) => {
      resolvers.set(requestGalleryId(request), done);
    }));
    const client = new ThumbnailClient({ resolve });

    const dispose = prefetchNextGalleryPageAfterCurrent(
      client,
      "downloads",
      [gallery(101), gallery(102)],
      [gallery(201), gallery(202)],
    );

    expect(resolve.mock.calls.map(([request]) => requestGalleryId(request))).toEqual([101, 102]);
    expect(resolve.mock.calls.every(([request]) => request.priority === "prefetch")).toBe(true);

    resolvers.get(101)?.(asset(101));
    await Promise.resolve();
    await Promise.resolve();
    expect(resolve).toHaveBeenCalledTimes(2);

    resolvers.get(102)?.(asset(102));
    await Promise.resolve();
    await Promise.resolve();
    expect(resolve.mock.calls.map(([request]) => requestGalleryId(request))).toEqual([101, 102, 201, 202]);
    expect(resolve.mock.calls.slice(2).every(([request]) => (
      request.consumer === "downloads" && request.priority === "prefetch"
    ))).toBe(true);

    dispose();
    client.dispose();
  });

  it("treats a current-page failure as terminal", async () => {
    let rejectCurrent: ((reason: Error) => void) | undefined;
    const resolve = vi.fn((request: ThumbnailRequest) => {
      if (requestGalleryId(request) === 301) {
        return new Promise<ThumbnailAsset>((_done, reject) => { rejectCurrent = reject; });
      }
      return asset(requestGalleryId(request));
    });
    const client = new ThumbnailClient({ resolve });
    const dispose = prefetchNextGalleryPageAfterCurrent(
      client,
      "auto-find",
      [gallery(301)],
      [gallery(401)],
    );
    rejectCurrent?.(new Error("permanent fixture failure"));
    await Promise.resolve();
    await Promise.resolve();
    expect(resolve.mock.calls.map(([request]) => requestGalleryId(request))).toEqual([301, 401]);
    expect(resolve.mock.calls[1]?.[0]).toMatchObject({ consumer: "review", priority: "prefetch" });
    dispose();
    client.dispose();
  });
});
