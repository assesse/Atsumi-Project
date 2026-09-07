import { describe, expect, it, vi } from "vitest";
import { galleryId, type DownloadState, type Gallery } from "../core/types";
import { pickArtistBalancedCompletedDownload } from "./downloadRandom";

const gallery = (id: number, artist: string, state: DownloadState = "completed"): Gallery => ({
  id: galleryId(id),
  title: `Gallery ${id}`,
  subtitle: "",
  artist,
  pages: 1,
  score: 0,
  publishedAt: "2026-09-07",
  coverIndex: 0,
  language: "korean",
  tags: [],
  series: [],
  characters: [],
  download: { entryId: `entry-${id}`, state },
});

describe("pickArtistBalancedCompletedDownload", () => {
  it("draws an artist first and then an album within that artist", () => {
    const random = vi.fn()
      .mockReturnValueOnce(0.1)
      .mockReturnValueOnce(0.99);
    const picked = pickArtistBalancedCompletedDownload([
      gallery(1, "ankoman"),
      gallery(2, "ANKOMAN "),
      gallery(3, "ankoman"),
      gallery(4, "another artist"),
    ], new Set(), random);

    expect(picked?.id).toBe(galleryId(3));
    expect(random).toHaveBeenCalledTimes(2);
  });

  it("gives prolific and single-album artists the same artist-level share", () => {
    const candidates = [
      gallery(1, "ankoman"),
      gallery(2, "ankoman"),
      gallery(3, "ankoman"),
      gallery(4, "single artist"),
    ];

    expect(pickArtistBalancedCompletedDownload(candidates, new Set(), () => 0.49)?.artist).toBe("ankoman");
    expect(pickArtistBalancedCompletedDownload(candidates, new Set(), () => 0.5)?.artist).toBe("single artist");
  });

  it("does not give duplicate probability shares to Unicode or whitespace variants", () => {
    const random = vi.fn()
      .mockReturnValueOnce(0.1)
      .mockReturnValueOnce(0.99);
    const picked = pickArtistBalancedCompletedDownload([
      gallery(1, "ＡＮＫＯＭＡＮ"),
      gallery(2, "ankoman   "),
      gallery(3, "ankoman\t"),
      gallery(4, "another artist"),
    ], new Set(), random);

    expect(picked?.id).toBe(galleryId(3));
    expect(random).toHaveBeenCalledTimes(2);
  });

  it("excludes hidden, quarantined, and incomplete albums", () => {
    const excludedId = galleryId(1);
    const picked = pickArtistBalancedCompletedDownload([
      gallery(1, "hidden"),
      gallery(2, "quarantined", "quarantined"),
      gallery(3, "active", "downloading"),
      gallery(4, "eligible"),
    ], new Set([excludedId]), () => 0);

    expect(picked?.id).toBe(galleryId(4));
  });

  it("keeps blank artist metadata in one shared deterministic bucket", () => {
    const random = vi.fn()
      .mockReturnValueOnce(0.6)
      .mockReturnValueOnce(0.99);
    const picked = pickArtistBalancedCompletedDownload([
      gallery(1, "known"),
      gallery(2, ""),
      gallery(3, "정보 불러오는 중"),
    ], new Set(), random);

    expect(picked?.id).toBe(galleryId(3));
  });

  it("returns undefined when no eligible completed album remains", () => {
    expect(pickArtistBalancedCompletedDownload([
      gallery(1, "hidden"),
      gallery(2, "failed", "failed"),
    ], new Set([galleryId(1)]), () => 0)).toBeUndefined();
  });
});
