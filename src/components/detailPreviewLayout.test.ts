import { describe, expect, it } from "vitest";
import { DETAIL_ORIENTATION_SAMPLE_SIZE, detailPreviewLayout } from "./detailPreviewLayout";

const samples = (width: number, height: number, count = DETAIL_ORIENTATION_SAMPLE_SIZE) =>
  Array.from({ length: count }, () => ({ status: "resolved" as const, width, height }));

describe("detailPreviewLayout", () => {
  it("uses two columns for a landscape-majority sample", () => {
    expect(detailPreviewLayout(samples(1600, 900))).toEqual({ columns: 2, orientation: "landscape" });
    expect(detailPreviewLayout([...samples(1600, 900, 5), ...samples(900, 1600, 3)])).toEqual({ columns: 2, orientation: "landscape" });
  });

  it("keeps portrait, balanced, neutral, and insufficient samples at three columns", () => {
    expect(detailPreviewLayout(samples(900, 1600))).toEqual({ columns: 3, orientation: "portrait" });
    expect(detailPreviewLayout([...samples(1600, 900, 4), ...samples(900, 1600, 4)])).toEqual({ columns: 3, orientation: "mixed" });
    expect(detailPreviewLayout(samples(1000, 1000))).toEqual({ columns: 3, orientation: "mixed" });
    expect(detailPreviewLayout(samples(1600, 900, 2))).toEqual({ columns: 3, orientation: "mixed" });
  });

  it("ignores invalid metadata dimensions", () => {
    expect(detailPreviewLayout([
      ...samples(1600, 900, 3),
      ...Array.from({ length: 5 }, () => ({ width: 0, height: 0 })),
    ])).toEqual({ columns: 2, orientation: "landscape" });
  });
});
