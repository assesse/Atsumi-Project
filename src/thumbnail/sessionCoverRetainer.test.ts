import { afterEach, describe, expect, it, vi } from "vitest";
import { galleryId, type Gallery } from "../core/types";
import { ThumbnailClient, type ThumbnailAsset } from "./client";
import { galleryCoverThumbnailKey } from "./model";
import { GalleryCoverSessionRetainer } from "./sessionCoverRetainer";

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

const asset = (value: number, byteLength?: number): ThumbnailAsset => ({
  kind: "image",
  url: `blob:https://atsumi.test/${value}`,
  width: 512,
  height: 768,
  ...(byteLength === undefined ? {} : { byteLength }),
});

afterEach(() => {
  vi.useRealTimers();
});

describe("GalleryCoverSessionRetainer", () => {
  it("keeps a visited resolved cover beyond the ordinary TTL and clears it explicitly", () => {
    vi.useFakeTimers();
    const release = vi.fn();
    const client = new ThumbnailClient({
      resolve: (request) => asset(request.key.kind === "gallery-cover" ? Number(request.key.galleryId) : 0),
      release,
    });
    const retainer = new GalleryCoverSessionRetainer(client, 4);
    const visited = gallery(101);

    retainer.visit("downloads", [visited]);
    vi.advanceTimersByTime(5 * 60_000);
    expect(retainer.size).toBe(1);
    expect(retainer.retainedBytes).toBe(512 * 768 * 0.5);
    expect(client.getSnapshot(galleryCoverThumbnailKey(visited)).status).toBe("resolved");
    expect(release).not.toHaveBeenCalled();

    retainer.clear();
    expect(retainer.size).toBe(0);
    expect(retainer.retainedBytes).toBe(0);
    expect(client.clearRetainedCache()).toBe(1);
    expect(client.getSnapshot(galleryCoverThumbnailKey(visited)).status).toBe("idle");
    expect(release).toHaveBeenCalledOnce();
    client.dispose();
  });

  it("evicts by exact backend delivery bytes before reaching the count ceiling", async () => {
    vi.useFakeTimers();
    const client = new ThumbnailClient({
      resolve: (request) => asset(
        request.key.kind === "gallery-cover" ? Number(request.key.galleryId) : 0,
        60,
      ),
    });
    const retainer = new GalleryCoverSessionRetainer(client, 10, 100);
    const first = gallery(501);
    const second = gallery(502);

    retainer.visit("downloads", [first, second]);
    expect(retainer.size).toBe(1);
    expect(retainer.retainedBytes).toBe(60);
    await vi.advanceTimersByTimeAsync(121_000);
    expect(client.getSnapshot(galleryCoverThumbnailKey(first)).status).toBe("idle");
    expect(client.getSnapshot(galleryCoverThumbnailKey(second)).status).toBe("resolved");

    retainer.clear();
    client.dispose();
  });

  it("uses visited recency to cap session pins without retaining failed covers", async () => {
    vi.useFakeTimers();
    const client = new ThumbnailClient({
      resolve: (request) => {
        const id = request.key.kind === "gallery-cover" ? Number(request.key.galleryId) : 0;
        if (id === 404) throw new Error("missing fixture cover");
        return asset(id);
      },
    });
    const retainer = new GalleryCoverSessionRetainer(client, 2);
    const first = gallery(201);
    const second = gallery(202);
    const third = gallery(203);

    retainer.visit("auto-find", [first, second]);
    retainer.visit("auto-find", [first]);
    retainer.visit("auto-find", [third]);
    expect(retainer.size).toBe(2);

    // The second cover was least recently visited and now follows only the
    // ordinary client retention lifecycle; the touched first and new third stay pinned.
    await vi.advanceTimersByTimeAsync(121_000);
    expect(client.getSnapshot(galleryCoverThumbnailKey(second)).status).toBe("idle");
    expect(client.getSnapshot(galleryCoverThumbnailKey(first)).status).toBe("resolved");
    expect(client.getSnapshot(galleryCoverThumbnailKey(third)).status).toBe("resolved");

    retainer.visit("downloads", [gallery(404)]);
    await Promise.resolve();
    expect(retainer.size).toBe(2);
    retainer.clear();
    client.dispose();
  });

  it("rejects an unbounded or invalid capacity", () => {
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing" }) });
    expect(() => new GalleryCoverSessionRetainer(client, 0)).toThrow(RangeError);
    expect(() => new GalleryCoverSessionRetainer(client, Number.POSITIVE_INFINITY)).toThrow(RangeError);
    expect(() => new GalleryCoverSessionRetainer(client, 2, 0)).toThrow(RangeError);
    client.dispose();
  });
});
