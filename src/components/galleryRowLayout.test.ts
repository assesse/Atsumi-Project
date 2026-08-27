import { describe, expect, it } from "vitest";
import { groupGalleryCardRows } from "./galleryRowLayout";

describe("groupGalleryCardRows", () => {
  it("uses the tallest intrinsic thumbnail per visual row", () => {
    expect(groupGalleryCardRows([
      { index: 0, top: 0, intrinsicThumbnailHeight: 160 },
      { index: 1, top: 0.5, intrinsicThumbnailHeight: 240 },
      { index: 2, top: 260, intrinsicThumbnailHeight: 180 },
      { index: 3, top: 260, intrinsicThumbnailHeight: 210 },
    ])).toEqual([
      { indices: [0, 1], height: 240 },
      { indices: [2, 3], height: 210 },
    ]);
  });

  it("supports one through four columns and incomplete final rows", () => {
    for (const columns of [1, 2, 3, 4]) {
      const metrics = Array.from({ length: columns * 2 - 1 }, (_, index) => ({
        index,
        top: Math.floor(index / columns) * 300,
        intrinsicThumbnailHeight: 100 + index,
      }));
      const rows = groupGalleryCardRows(metrics);
      expect(rows[0]?.indices).toHaveLength(columns);
      expect(rows.at(-1)?.indices).toHaveLength(columns - 1 || 1);
    }
  });

  it("is independent per grid and recalculates from replacement metrics", () => {
    const firstGrid = groupGalleryCardRows([
      { index: 0, top: 0, intrinsicThumbnailHeight: 160 },
      { index: 1, top: 0, intrinsicThumbnailHeight: 300 },
    ]);
    const secondGrid = groupGalleryCardRows([
      { index: 0, top: 0, intrinsicThumbnailHeight: 190 },
    ]);
    const reflowedFirstGrid = groupGalleryCardRows([
      { index: 0, top: 0, intrinsicThumbnailHeight: 220 },
      { index: 1, top: 240, intrinsicThumbnailHeight: 300 },
    ]);
    expect(firstGrid[0]?.height).toBe(300);
    expect(secondGrid[0]?.height).toBe(190);
    expect(reflowedFirstGrid.map(({ height }) => height)).toEqual([220, 300]);
  });
});
