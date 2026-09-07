import { describe, expect, it } from "vitest";
import { galleryId, type Gallery } from "../core/types";
import {
  galleryGroupStorageKey,
  groupGalleries,
  latestDownloadedGallery,
  recentDownloadedGalleries,
  summarizeArtistFolderTags,
} from "./galleryGrouping";

const gallery = (id: number, artist: string, publishedAt: string): Gallery => ({
  id: galleryId(id),
  title: `Gallery ${id}`,
  subtitle: "",
  artist,
  pages: 1,
  score: 0,
  publishedAt,
  coverIndex: 0,
  language: "korean",
  tags: [],
  series: [],
  characters: [],
});

describe("groupGalleries", () => {
  it("groups artists using a stable persistence key while retaining a readable label", () => {
    const groups = groupGalleries([
      gallery(1, "Mizuno", "2026-08-24"),
      gallery(2, "mizuno", "2026-08-23"),
      gallery(3, "Serein", "2026-08-23"),
    ], "artist", (item) => item.publishedAt);

    expect(groups).toHaveLength(2);
    expect(groups.find((group) => group.label === "Mizuno")?.items.map((item) => item.id)).toEqual([
      galleryId(1),
      galleryId(2),
    ]);
    expect(galleryGroupStorageKey("auto-find", groups[0]!)).toContain("auto-find\u001fartist\u001f");
  });

  it("groups daily dates newest first and keeps invalid dates in a final bucket", () => {
    const groups = groupGalleries([
      gallery(1, "A", "2026-08-23T12:34:00Z"),
      gallery(2, "B", "2026-08-25"),
      gallery(3, "C", "not-a-date"),
    ], "day", (item) => item.publishedAt);

    expect(groups.map((group) => group.label)).toEqual([
      "2026년 8월 25일",
      "2026년 8월 23일",
      "날짜 정보 없음",
    ]);
  });

  it("uses the most recently downloaded work as an artist folder cover", () => {
    const older = {
      ...gallery(10, "Folder artist", "2026-08-30"),
      download: { entryId: "older", state: "completed" as const, createdAt: "2026-09-01T10:00:00Z" },
    };
    const newer = {
      ...gallery(11, "Folder artist", "2025-01-01"),
      download: { entryId: "newer", state: "completed" as const, createdAt: "2026-09-02T10:00:00Z" },
    };

    expect(latestDownloadedGallery([older, newer])?.id).toBe(newer.id);
  });

  it("selects at most three artist-folder previews in stable download-recency order", () => {
    const items = [
      { ...gallery(21, "Folder artist", "2025-01-01"), download: { entryId: "21", state: "completed" as const, createdAt: "2026-09-01T10:00:00Z" } },
      { ...gallery(22, "Folder artist", "2025-01-01"), download: { entryId: "22", state: "completed" as const, createdAt: "2026-09-03T10:00:00Z" } },
      { ...gallery(23, "Folder artist", "2025-01-01"), download: { entryId: "23", state: "completed" as const, createdAt: "2026-09-02T10:00:00Z" } },
      { ...gallery(24, "Folder artist", "2025-01-01"), download: { entryId: "24", state: "completed" as const, createdAt: "2026-08-31T10:00:00Z" } },
    ];

    expect(recentDownloadedGalleries(items).map((item) => item.id)).toEqual([
      galleryId(22),
      galleryId(23),
      galleryId(21),
    ]);
    expect(recentDownloadedGalleries(items, 0)).toEqual([]);
  });

  it("summarizes resolved artist tags by favorite status and gallery frequency", () => {
    const items = [
      { ...gallery(31, "Folder artist", "2026-09-01"), tags: ["female:glasses", "full_color", "full_color"] },
      { ...gallery(32, "Folder artist", "2026-09-02"), tags: ["female:glasses", "sole_female"] },
      { ...gallery(33, "Folder artist", "2026-09-03"), tags: ["female:glasses", "full_color"] },
    ];

    expect(summarizeArtistFolderTags(items, new Set(["FULL_COLOR"]))).toEqual([
      { value: "full_color", count: 2, favorite: true },
      { value: "female:glasses", count: 3, favorite: false },
      { value: "sole_female", count: 1, favorite: false },
    ]);
    expect(summarizeArtistFolderTags(items, new Set(), 1)).toEqual([
      { value: "female:glasses", count: 3, favorite: false },
    ]);
  });
});
