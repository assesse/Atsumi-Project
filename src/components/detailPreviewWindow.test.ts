import { describe, expect, it } from "vitest";
import {
  detailPreviewWindowClampStart,
  detailPreviewWindowRange,
  detailPreviewWindowSize,
  detailPreviewWindowStart,
} from "./detailPreviewWindow";

describe("detailPreviewWindow", () => {
  it("uses only the fixed two- and three-column contracts", () => {
    expect(detailPreviewWindowSize(5, 2)).toBe(5);
    expect(detailPreviewWindowSize(5, 3)).toBe(5);
    expect(detailPreviewWindowSize(5_000, 2)).toBe(8);
    expect(detailPreviewWindowSize(5_000, 3)).toBe(9);
  });

  it("keeps final partial windows reachable without allocating every page", () => {
    const size = 9;
    const start = detailPreviewWindowStart(5000, 5000, size);
    expect(start).toBe(4996);
    expect(detailPreviewWindowRange(start, 5000, size)).toEqual([4996, 4997, 4998, 4999, 5000]);
  });

  it("keeps a preserved start when stable metrics change and reaches page 5000 by bounded next windows", () => {
    const size = 9;
    expect(detailPreviewWindowClampStart(97, 5000, 9)).toBe(97);
    let start = 1;
    const lastStart = Math.floor((5000 - 1) / size) * size + 1;
    while (start < lastStart) start = Math.min(lastStart, start + size);
    expect(detailPreviewWindowRange(start, 5000, size).at(-1)).toBe(5000);
  });
});
