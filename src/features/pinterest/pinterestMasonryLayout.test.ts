import { describe, expect, it } from "vitest";
import { IMAGE_PREVIEW_WIDTHS } from "../platforms/imagePreviewPreferences";
import { layoutPinterestMasonry, pinterestImageDimensions, PINTEREST_MASONRY_GAP } from "./pinterestMasonryLayout";

describe("Pinterest shortest-column layout", () => {
  const items = Array.from({ length: 48 }, (_, index) => ({ key: `pin:${index}`, width: 800,
    height: [500, 800, 1200, 1800, 2400][index % 5]! }));

  it.each([120, 292, 480, 720, 1280])("preserves source/visual order without overlap at %ipx", width => {
    for (const preset of IMAGE_PREVIEW_WIDTHS) {
      const layout = layoutPinterestMasonry(items, width, preset);
      expect(layout.placements.map(item => item.key)).toEqual(items.map(item => item.key));
      for (const [index, item] of layout.placements.entries()) {
        expect(item.x).toBeGreaterThanOrEqual(0);
        expect(item.x + item.width).toBeLessThanOrEqual(width + .001);
        expect(item.y + item.height).toBeLessThanOrEqual(layout.height);
        expect(item.height).toBeGreaterThanOrEqual(120);
        if (index) {
          const previous = layout.placements[index - 1]!;
          expect(item.y).toBeGreaterThanOrEqual(previous.y);
          if (item.y === previous.y) expect(item.x).toBeGreaterThan(previous.x);
        }
        for (const previous of layout.placements.slice(0, index)) {
          const horizontallySeparated = item.x >= previous.x + previous.width + PINTEREST_MASONRY_GAP - .001
            || previous.x >= item.x + item.width + PINTEREST_MASONRY_GAP - .001;
          const verticallySeparated = item.y >= previous.y + previous.height + PINTEREST_MASONRY_GAP
            || previous.y >= item.y + item.height + PINTEREST_MASONRY_GAP;
          expect(horizontallySeparated || verticallySeparated).toBe(true);
        }
      }
    }
  });

  it("uses first shortest column, never mutates items, and leaves a sparse first row compact", () => {
    const input = Object.freeze([Object.freeze({ key: "a", width: 200, height: 400 }), Object.freeze({ key: "b", width: 200, height: 200 }), Object.freeze({ key: "c", width: 200, height: 200 })]);
    const layout = layoutPinterestMasonry(input, 374, 180);
    expect(layout.columns).toBe(2);
    expect(layout.placements.map(item => item.column)).toEqual([1, 2, 2]);
    expect(layout.placements[2]!.y).toBe(layout.placements[1]!.height + 14);
    const single = layoutPinterestMasonry(input.slice(0, 1), 1280, 180);
    expect(single.columns).toBeGreaterThan(1);
    expect(single.columnWidth).toBeLessThan(250);
  });

  it("retains ordinary proportions, reserves a stable missing-dimension fallback and bounds only extreme previews", () => {
    const layout = layoutPinterestMasonry([
      { key: "landscape", width: 800, height: 450 }, { key: "portrait", width: 800, height: 1200 },
      { key: "fallback" }, { key: "panorama", width: 100_000, height: 1 }, { key: "poster", width: 1, height: 100_000 },
    ], 340, 340);
    expect(layout.placements.map(item => item.height)).toEqual([192, 510, 454, 120, 1020]);
    expect(layout.height).toBe(layout.placements.at(-1)!.y + 1020);
  });

  it.each([[0, 1], [1, 0], [-1, 20], [NaN, 20], [10, Infinity], [1.2, 2], [100_001, 30], ["800", 1200]])("rejects malformed dimensions %s/%s", (width, height) => {
    expect(pinterestImageDimensions(width, height)).toBeNull();
  });
  it("accepts only bounded positive integer dimensions", () => {
    expect(pinterestImageDimensions(100_000, 1)).toEqual({ width: 100_000, height: 1 });
  });
  it.each([0, -1, NaN, Infinity, 1e12])("does not create pathological tracks for unavailable width %s", width => {
    expect(layoutPinterestMasonry(items, width, 340).placements).toEqual([]);
  });
  it("handles empty lists and invalid preview preference without losing source entries", () => {
    expect(layoutPinterestMasonry([], 720, 180).height).toBe(0);
    expect(layoutPinterestMasonry(items, 720, NaN)).toEqual(layoutPinterestMasonry(items, 720, 180));
  });
});
