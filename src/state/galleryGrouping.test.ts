import { describe, expect, it } from "vitest";
import { galleryId, type Gallery } from "../core/types";
import { galleryGroupStorageKey, groupGalleries } from "./galleryGrouping";

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
});
